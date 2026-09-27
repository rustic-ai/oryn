"""Actual build execution and final artifact joins, separate from metadata preparation."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from . import builds
from .generation import REQUIREMENTS, source_info
from .inputs import InputLease, resolve_input, snapshot_input, verify_inputs
from .manifests import manifest_digest
from .processes import ResourceLease, run_owned


ROLES = frozenset({"runtime", "driver", "worker", "worker_app", "worker_manifest", "engine_worker", "startup_snapshot"})
FEATURES = {"native", "core-full", "webdriver-server", "v8-host", "v8-sandbox"}
ENGINE_ROLES = ("archive", "binding", "icu_data", "startup_snapshot", "external_references",
                "worker", "bridge_abi", "rusty_v8_source_manifest")


def _read(path):
    from .production import _read as read
    return read(path)


def _hash(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _write(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write("\n")


def verify_generation_review(gate, review, source, *, gate_sha256):
    if (gate.get("schema_version") != 2 or gate.get("kind") != "committed-source-generation"
            or gate.get("source") != source or set(gate.get("requirements", {})) != REQUIREMENTS):
        raise ValueError("build requires current committed full-generation gate")
    fields = {"schema_version", "kind", "gate_sha256", "source", "author", "reviewer", "decision"}
    if (set(review) != fields or review["schema_version"] != 1
            or review["kind"] != "generation-review" or review["decision"] != "accepted"
            or review["source"] != source or review["gate_sha256"] != gate_sha256
            or any(not isinstance(review[k], str) or not review[k].strip() for k in ("author", "reviewer"))
            or review["author"].strip().casefold() == review["reviewer"].strip().casefold()):
        raise ValueError("build generation review does not bind current source and exact gate")


def _successful(process):
    return (type(process.get("returncode")) is int and process["returncode"] == 0 and process.get("error") is None
            and process.get("timed_out") is False and process.get("cleanup_complete") is True
            and process.get("cleanup_errors") == [] and process.get("cancellation_signal") is None
            and process.get("forced_cleanup") is False)


def _reject_implicit_cargo_configuration(source, home):
    """First migration admits the canonical scripts with no implicit Cargo overrides."""
    roots = {Path(home), Path(source), *Path(source).parents}
    for root in roots:
        for name in ("config", "config.toml"):
            if (root / ".cargo" / name).exists():
                raise ValueError("implicit Cargo configuration requires an explicit reviewed build contract: " + str(root / ".cargo" / name))


def _toolchain(environment, source):
    paths = {}
    for name in ("rustc", "cargo"):
        value = subprocess.check_output(["rustup", "which", name], cwd=source, env=environment, text=True).strip()
        path = Path(value).resolve(strict=True)
        paths[name] = {"path": str(path), **builds._identity(path)}
    return paths


def _covered(record, declarations):
    for declared in declarations:
        if record == declared:
            return True
        prefix = declared["path"] + "/"
        if (declared["scope"] == record["scope"] and declared["kind"] == "directory"
                and record["path"].startswith(prefix)):
            member = {key: value for key, value in record.items() if key not in {"scope", "kind"}}
            member["path"] = record["path"][len(prefix):]
            if member in declared["members"]:
                return True
    return False


def _engine_files(profile_path, source, store, declarations):
    """Resolve the reviewed schema6 build-time fields once before compilation."""
    from .runtime_metadata import _legacy_validator
    validator = _legacy_validator(source)
    profile = _read(profile_path)
    validator.validate(profile, source, profile="core-full", allow_relocated_source_manifest=True)
    bridge = _read(profile["bridge_abi_path"])
    paths = [profile[name + "_path"] for name in validator.ARTIFACTS]
    paths += [row["path"] for row in bridge["inputs"]]
    files = {}
    for value in paths:
        path = Path(value).absolute()
        if not path.is_relative_to(store):
            raise ValueError("native compiler/engine input is outside the consolidated store")
        record = snapshot_input(store, path.relative_to(store).as_posix())
        if not _covered(record, declarations):
            raise ValueError("native compiler/engine file absent from prelaunch input closure: " + record["path"])
        files[record["path"]] = record
    return [files[name] for name in sorted(files)]


def _verify_engine_closure(execution, store):
    """Join every original schema6 operand to current declared bytes after relocation.

    Historical absolute strings are parsed only relative to the recorded store;
    they are never opened. Only typed current-store records authorize reads.
    """
    store = Path(store)
    original = Path(execution.get("original_store_root", ""))
    if not original.is_absolute() or ".." in original.parts:
        raise ValueError("build execution lacks its original store root")
    declared = execution["build_inputs"]
    files = execution.get("engine_input_files", [])
    if not files or any(not _covered(row, declared) for row in files):
        raise ValueError("compiled engine/compiler inputs are outside original build input declarations")
    by_path = {row["path"]: row for row in files}
    if len(by_path) != len(files):
        raise ValueError("duplicate compiled engine input")
    verify_inputs(files, store_root=store)
    expected = set()

    def operand(value, digest, size):
        path = Path(value)
        if not path.is_absolute() or ".." in path.parts or not path.is_relative_to(original):
            raise ValueError("build engine operand escapes original store")
        relative = path.relative_to(original).as_posix()
        row = by_path.get(relative)
        if (row is None or row["scope"] != "store" or row["kind"] != "file"
                or row["sha256"] != digest or row["size"] != size):
            raise ValueError("build engine operand absent or different in exact compiled closure: " + relative)
        expected.add(relative)
        return resolve_input(store, relative)

    profile_record = execution["environment_inputs"]["ORYN_V8_PROFILE_MANIFEST"]
    verify_inputs([profile_record], store_root=store)
    profile = _read(resolve_input(store, profile_record["path"]))
    if profile.get("schema_version") != 6 or profile.get("profile") != "core-full":
        raise ValueError("original compile profile must preserve schema6 core-full operands")
    paths = {name: operand(profile[name + "_path"], profile[name + "_sha256"],
                           profile[name + "_size_bytes"]) for name in ENGINE_ROLES}
    bridge = _read(paths["bridge_abi"])
    if not bridge.get("inputs") or not any(row["path"] == bridge.get("compiler") for row in bridge["inputs"]):
        raise ValueError("original compile descriptor lacks compiler inputs")
    for row in bridge["inputs"]:
        operand(row["path"], row["sha256"], row["size_bytes"])
    if expected != set(by_path):
        raise ValueError("compiled engine input population differs from original operands")


def verify_build_evidence(bound, build):
    """Recompute source/gate/command/outputs joins from the original build attempt."""
    assets, records = bound["assets"], bound["run"]["assets"]
    required = {"build_execution", "build_process", "build_stdout", "build_stderr",
                "build_generation_gate", "build_generation_review"}
    if not required <= assets.keys():
        raise ValueError("production admission lacks original build execution evidence")
    for name in required:
        if records[name] not in bound["records"] or records[name] not in build["immutable_inputs"]:
            raise ValueError("build evidence is outside the declared current build closure: " + name)
    execution = _read(assets["build_execution"])
    process = _read(assets["build_process"])
    gate, review = _read(assets["build_generation_gate"]), _read(assets["build_generation_review"])
    verify_generation_review(gate, review, build["source"], gate_sha256=_hash(assets["build_generation_gate"]))
    source = Path(assets["source_export"])
    if gate["plan_sha256"] != _hash(source / "docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md"):
        raise ValueError("build gate plan differs from exported source")
    for identifier, row in gate["requirements"].items():
        if row.get("implementation_generated") is not True or row.get("tests_generated") is not True:
            raise ValueError("build source generation incomplete: " + identifier)
        for field in ("implementation_sources", "test_sources"):
            if not row.get(field):
                raise ValueError("build generation source mapping missing")
            for record in row[field]:
                if _hash(resolve_input(source, record["path"])) != record["sha256"]:
                    raise ValueError("build generation source differs: " + identifier)
    if (execution.get("schema_version") != 1 or execution.get("kind") != "typed-build-execution"
            or execution.get("source") != build["source"]
            or execution.get("configuration") != build["configuration"]
            or execution.get("build_inputs") != build["build_inputs"]
            or execution.get("command") not in build["commands"]
            or execution.get("command") != process.get("argv")
            or execution.get("generation_gate_sha256") != _hash(assets["build_generation_gate"])
            or execution.get("generation_review_sha256") != _hash(assets["build_generation_review"])
            or execution.get("integrity_errors") != [] or not _successful(process)):
        raise ValueError("original build command, source, configuration, or custody differs")
    command = execution["command"]
    if command != ["/bin/bash", "scripts/build-sandboxed-worker.sh"]:
        raise ValueError("build evidence must execute a canonical complete production build")
    if set(build["configuration"]["features"]) != FEATURES:
        raise ValueError("build feature declaration differs from canonical worker/parent/driver features")
    observed = execution.get("toolchain_observation", "").splitlines()
    if (not observed or observed[0] != build["configuration"]["toolchain"]
            or "host: " + build["configuration"]["host"] not in observed):
        raise ValueError("original build toolchain observation differs from configuration")
    _verify_environment(execution["environment_inputs"], execution["literal_environment"],
                        execution["build_inputs"])
    if execution.get("environment_sha256") != manifest_digest({"inputs": execution["environment_inputs"],
                                                              "literals": execution["literal_environment"]}):
        raise ValueError("original build environment identity changed")
    _verify_engine_closure(execution, bound["store_root"])
    for role in ("process", "stdout", "stderr"):
        if execution.get("evidence", {}).get(role) != _hash(assets["build_" + role]):
            raise ValueError("original build evidence changed: " + role)
    if set(execution.get("outputs", {})) != ROLES:
        raise ValueError("original build output population differs")
    for role, original in execution["outputs"].items():
        # A copy may relocate an object but cannot change its contents/mode.
        fingerprint = lambda record: {key: value for key, value in record.items() if key not in {"scope", "path"}}
        if role not in records or fingerprint(original) != fingerprint(records[role]):
            raise ValueError("selected runtime differs from original build output: " + role)
    return {"source_commit": build["source"]["commit"],
            "build_execution_sha256": _hash(assets["build_execution"]), "outputs_joined": sorted(ROLES)}


def canonical_outputs(configuration, artifact_directory):
    from .manifests import relative_path
    relative_path(artifact_directory)
    cache = "cache/cargo/" + builds.cargo_cache_key(configuration) + "/release/"
    app = artifact_directory + "/OrynPageWorker.app"
    return {"runtime": cache + "oryn", "driver": cache + "oryn-webdriver",
            "startup_snapshot": artifact_directory + "/oryn-startup-core-full.blob",
            "engine_worker": cache + "oryn-page-worker", "worker_app": app,
            "worker": app + "/Contents/MacOS/oryn-page-worker",
            "worker_manifest": app + "/Contents/Resources/worker-manifest.json"}


def _verify_environment(environment_inputs, literal_environment, immutable_inputs):
    allowed_literals = {"ORYN_CODESIGN_IDENTITY", "ORYN_APPLE_TEAM_ID", "ORYN_EXPECTED_BUNDLE_ID",
                        "ORYN_TYPED_RUNTIME_OUTPUT", "DEVELOPER_DIR", "SDKROOT"}
    required_inputs = {"ORYN_V8_PROFILE_MANIFEST", "ORYN_TYPED_SOURCE_IDENTITY"}
    if (set(literal_environment) - allowed_literals or set(environment_inputs) != required_inputs
            or not {"ORYN_CODESIGN_IDENTITY", "ORYN_TYPED_RUNTIME_OUTPUT"} <= literal_environment.keys()
            or any(not isinstance(v, str) or not v or "\0" in v for v in literal_environment.values())):
        raise ValueError("undeclared or incomplete canonical build environment")
    for key, record in environment_inputs.items():
        if record not in immutable_inputs or record["scope"] != "store" or record["kind"] != "file":
            raise ValueError("build environment file is not in declared immutable inputs: " + key)


def execute_build(source_root, store_root, *, configuration, source_identity, generation_gate,
                  generation_review, script, output, immutable_inputs, output_paths,
                  environment_inputs, literal_environment, artifact_directory,
                  estimated_build_bytes, timeout_seconds=14400):
    """Run one reviewed canonical production build and bind its actual products.

    environment_inputs maps variable names to declared store-relative file inputs.
    Credentials stay in the process environment; the receipt contains no values.
    A failed attempt is preserved and cannot be reused as production evidence.
    """
    source = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    output = Path(output).absolute()
    if not output.is_relative_to(store) or output.exists():
        raise ValueError("build attempt needs a new directory inside data store")
    from .generation import new_store_path
    new_store_path(store, output.relative_to(store).as_posix())
    if output_paths != canonical_outputs(configuration, artifact_directory):
        raise ValueError("output roles must be the exact canonical build destinations")
    if set(configuration["features"]) != FEATURES:
        raise ValueError("features must describe the canonical parent/driver/worker build")
    if type(estimated_build_bytes) is not int or estimated_build_bytes < 0:
        raise ValueError("measured build/staging size estimate is required")
    free_bytes = shutil.disk_usage(store).free
    if free_bytes < estimated_build_bytes + 50 * 1024**3:
        raise ValueError("insufficient free space for measured build demand plus 50 GiB reserve")
    current = source_info(source, source_identity)
    gate, review = _read(generation_gate), _read(generation_review)
    verify_generation_review(gate, review, current, gate_sha256=_hash(generation_gate))
    plan = builds.build_command_plan(source, store, configuration, script=script,
                                    artifact_directory=artifact_directory)
    if script != "scripts/build-sandboxed-worker.sh":
        raise ValueError("build-run requires a complete production build script")
    # Do not inherit unrecorded compiler/runner overrides from a developer shell.
    environment = {key: value for key, value in os.environ.items() if key in
                   {"PATH", "HOME", "USER", "LOGNAME", "TMPDIR", "LANG", "LC_ALL"}}
    _reject_implicit_cargo_configuration(source, environment["HOME"])
    tools = _toolchain(environment, source)
    environment["RUSTC"] = tools["rustc"]["path"]
    environment["PATH"] = str(Path(tools["cargo"]["path"]).parent) + os.pathsep + environment.get("PATH", "")
    _verify_environment(environment_inputs, literal_environment, immutable_inputs)
    for path in (Path(source_identity), Path(generation_gate), Path(generation_review)):
        path = path.absolute()
        if not path.is_relative_to(store) or snapshot_input(store, path.relative_to(store).as_posix()) not in immutable_inputs:
            raise ValueError("source identity, gate and review must be immutable prelaunch inputs")
    for key, record in environment_inputs.items():
        environment[key] = str(resolve_input(store, record["path"]))
    engine_files = _engine_files(environment["ORYN_V8_PROFILE_MANIFEST"], source, store, immutable_inputs)
    if Path(environment["ORYN_TYPED_SOURCE_IDENTITY"]) != Path(source_identity).absolute():
        raise ValueError("build source identity differs from canonical script's source identity")
    environment.update(literal_environment)
    environment.update(plan["environment_overlay"])
    environment.update(ORYN_DATA_ROOT=str(store), ORYN_GENERATION_GATE=str(Path(generation_gate).absolute()),
                       ORYN_GENERATION_GATE_SHA256=_hash(generation_gate), PYTHONDONTWRITEBYTECODE="1")
    environment["RUSTFLAGS"] = " ".join(configuration["flags"])
    from .runtime_metadata import observe_sdk, verify_sdk_observation
    sdkroot = environment.get("SDKROOT") or subprocess.check_output(
        ["xcrun", "--sdk", "macosx", "--show-sdk-path"], env=environment, text=True).strip()
    sdk_observation = observe_sdk(sdkroot)
    environment["SDKROOT"] = sdkroot
    observed = subprocess.check_output(["rustc", "-vV"], cwd=source, env=environment, text=True)
    if observed.splitlines()[0] != configuration["toolchain"] or "host: " + configuration["host"] not in observed.splitlines():
        raise ValueError("fresh Rust toolchain observation differs from build configuration")
    if _hash(source / "Cargo.lock") != configuration["cargo_lock_sha256"]:
        raise ValueError("Cargo.lock differs from requested build")
    for path in output_paths.values():
        # Resolve safe relative structure now; products need not exist yet.
        from .generation import new_store_path
        new_store_path(store, path)
    artifact_path = new_store_path(store, artifact_directory)
    typed_path = new_store_path(store, literal_environment["ORYN_TYPED_RUNTIME_OUTPUT"])
    if artifact_path.exists() or typed_path.exists():
        raise ValueError("canonical build staging and typed metadata require new output directories")
    output.mkdir(parents=True, exist_ok=False)
    result = {"schema_version": 1, "kind": "typed-build-execution", "source": current,
              "configuration": configuration, "command": plan["command"], "original_store_root": str(store),
              "disk_observation": {"free_bytes_before": free_bytes, "estimated_build_bytes": estimated_build_bytes,
                                   "reserve_bytes": 50 * 1024**3},
              "generation_gate_sha256": _hash(generation_gate),
              "generation_review_sha256": _hash(generation_review),
              "toolchain_observation": observed, "toolchain_executables": tools,
              "sdk_observation": sdk_observation, "engine_input_files": engine_files,
              "outputs": {}, "evidence": {}, "integrity_errors": []}
    result.update(build_inputs=immutable_inputs, environment_inputs=environment_inputs,
                  literal_environment=literal_environment,
                  environment_sha256=manifest_digest({"inputs": environment_inputs, "literals": literal_environment}))
    with ResourceLease(store, plan["resource_keys"]) as resources:
        process = None
        try:
            from .retention import register_object, restore_cache_registration, touch_object
            cache_path = Path(plan["environment_overlay"]["CARGO_TARGET_DIR"])
            cache_path.mkdir(parents=True, exist_ok=True)
            cache_id = "cargo-cache:" + builds.cargo_cache_key(configuration)
            index = _read(store / "retention.json")
            if cache_id not in index["objects"]:
                register_object(store, cache_id, path=cache_path.relative_to(store).as_posix(), kind="cargo-cache")
            elif "removed_ns" in index["objects"][cache_id]:
                restore_cache_registration(store, store / "retention.json", cache_id)
            elif (index["objects"][cache_id]["path"] != cache_path.relative_to(store).as_posix()
                  or index["objects"][cache_id]["kind"] != "cargo-cache"):
                raise ValueError("cache ID belongs to another object")
            resources.attach_objects([cache_id])
            touch_object(store, cache_id)
            with InputLease(immutable_inputs, store_root=store):
                process = run_owned(plan["command"], cwd=source, output=output / "process", env=environment,
                                    timeout_seconds=timeout_seconds)
                if source_info(source, source_identity) != current:
                    raise ValueError("committed source changed during build")
                _reject_implicit_cargo_configuration(source, environment["HOME"])
                for identity in tools.values():
                    if builds._identity(Path(identity["path"])) != {key: value for key, value in identity.items() if key != "path"}:
                        raise ValueError("actual compiler/Cargo executable changed during build")
                verify_sdk_observation(sdk_observation)
                if result["generation_gate_sha256"] != _hash(generation_gate) or result["generation_review_sha256"] != _hash(generation_review):
                    raise ValueError("generation gate or review changed during build")
                if _successful(process):
                    for role, relative in output_paths.items():
                        result["outputs"][role] = snapshot_input(store, relative,
                            kind="directory" if role == "worker_app" else "file")
        except BaseException as error:
            result["integrity_errors"].append(repr(error))
            raise
        finally:
            for role, name in {"process": "process-result.json", "stdout": "stdout.log", "stderr": "stderr.log"}.items():
                path = output / "process" / name
                if path.is_file():
                    result["evidence"][role] = _hash(path)
            _write(output / "build-execution.json", result)
            if process is not None:
                resources.mark_settled(process)
    if not _successful(process):
        raise RuntimeError("canonical build failed; original process and output receipts preserved")
    return result

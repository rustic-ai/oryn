"""Git admission and sealed artifacts; existing scripts remain the build engine.

These functions do not launch Cargo or sign files. The coordinator must hold the
shared cache/signing resource lease when executing a returned command plan.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import sys


def _digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _relative(value):
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError("expected a nonempty POSIX relative path")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in value.split("/")):
        raise ValueError("path must stay inside its declared root")
    return path


def _file(root, relative):
    root = Path(root).resolve(strict=True)
    path = root
    for component in _relative(relative).parts:
        path = path / component
        if path.is_symlink():
            raise ValueError("undeclared symlink: " + relative)
    if not path.is_file() or not stat.S_ISREG(path.stat().st_mode):
        raise ValueError("not a regular input: " + relative)
    return path


def _identity(path, *, git_object_id=None):
    before = path.stat()
    hasher = hashlib.sha256()
    git_hasher = None
    if git_object_id is not None:
        git_hasher = hashlib.new("sha1" if len(git_object_id) == 40 else "sha256")
        git_hasher.update(f"blob {before.st_size}\0".encode())
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(block)
            if git_hasher is not None:
                git_hasher.update(block)
    after = path.stat()
    if (before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns):
        raise ValueError("input changed during hashing")
    if git_hasher is not None and git_hasher.hexdigest() != git_object_id:
        raise ValueError("working source differs from committed Git blob: " + str(path))
    return {"sha256": hasher.hexdigest(), "size": after.st_size,
            "executable": bool(after.st_mode & 0o111)}


def _git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE,
                                   env={**os.environ, "GIT_OPTIONAL_LOCKS": "0"})


def audit_source(source_root, *, effective_inputs, external_inputs=(), official=True,
                 expected_commit=None):
    """Capture tracked bytes and explicitly account for ignored effective inputs.

    effective_inputs is the reviewed complete list of files consumed by the
    build, including ignored generators/vendor sources. External inputs are
    exact relative paths with a pinned SHA256; directory exemptions are forbidden.
    Effective-input completeness is a required independent review: this helper
    cannot infer arbitrary compiler/generator file reads. The ignored inventory
    is a collapsed diagnostic, never a dependency closure or a source exemption.
    An ignored effective file still requires its own exact pinned input record.
    """
    root = Path(source_root).resolve(strict=True)
    if Path(os.fsdecode(_git(root, "rev-parse", "--show-toplevel")).strip()).resolve() != root:
        raise ValueError("source root must be the Git checkout root")
    commit = _git(root, "rev-parse", "HEAD").decode().strip()
    if expected_commit is not None and commit != expected_commit:
        raise ValueError("source commit mismatch")
    status = _git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all",
                  "--ignore-submodules=none")
    if official and status:
        raise ValueError("official builds require a clean committed checkout")
    tracked = [os.fsdecode(path) for path in _git(root, "ls-files", "-z").split(b"\0") if path]
    committed = {}
    for record in _git(root, "ls-tree", "-r", "-z", "HEAD").split(b"\0"):
        if record:
            metadata, name = record.split(b"\t", 1)
            mode, kind, object_id = metadata.decode().split()
            committed[os.fsdecode(name)] = (mode, kind, object_id)
    if official and set(tracked) != set(committed):
        raise ValueError("index file population differs from committed source")
    ignored = [os.fsdecode(path) for path in _git(root, "ls-files", "--others", "--ignored",
              "--exclude-standard", "--directory", "-z").split(b"\0") if path]
    # Gitlinks omit nested dependency bytes. Require vendoring or explicit pinned
    # external inputs instead of accepting a clean-looking top-level submodule.
    if any(row.startswith(b"160000 ") for row in _git(root, "ls-files", "--stage", "-z").split(b"\0")):
        raise ValueError("nested Git dependencies need explicit immutable external admission")
    inventory = {}
    for name in sorted(tracked):
        expected = committed.get(name)
        identity = _identity(_file(root, name), git_object_id=expected[2] if official else None)
        if official and identity["executable"] != (expected[0] == "100755"):
            raise ValueError("source executable mode differs from committed Git mode")
        inventory[name] = identity
    external = {}
    for record in external_inputs:
        if set(record) != {"path", "sha256"} or record["path"] in external:
            raise ValueError("external input needs one exact path and SHA256")
        _relative(record["path"])
        if _identity(_file(root, record["path"]))["sha256"] != record["sha256"]:
            raise ValueError("external input digest mismatch")
        external[record["path"]] = record["sha256"]
    effective = sorted(set(effective_inputs))
    if not effective or "Cargo.lock" not in effective or "Cargo.toml" not in effective:
        raise ValueError("effective input review must include Cargo.toml and Cargo.lock")
    for name in effective:
        _relative(name)
        if name not in inventory and name not in external:
            raise ValueError("unversioned effective input is not explicitly pinned: " + name)
    if _git(root, "rev-parse", "HEAD").decode().strip() != commit or _git(
            root, "status", "--porcelain=v1", "-z", "--untracked-files=all",
            "--ignore-submodules=none") != status:
        raise ValueError("source changed during admission")
    return {"schema_version": 1, "commit": commit, "dirty": bool(status),
            "development_only": not official, "tree_sha256": _digest(inventory),
            "tracked": inventory, "ignored_entries_diagnostic": sorted(ignored),
            "effective_inputs": effective, "external_inputs": external}


def cargo_cache_key(configuration):
    """One cache per compiler configuration, never per source commit or lockfile."""
    keys = {"toolchain", "host", "target", "profile", "features", "flags"}
    if not keys <= set(configuration):
        raise ValueError("incomplete Cargo configuration")
    selected = {name: configuration[name] for name in sorted(keys)}
    if any(not isinstance(selected[key], str) or not selected[key] for key in
           ("toolchain", "host", "target", "profile")):
        raise ValueError("compiler identity, host, target and profile are required")
    for name in ("features", "flags"):
        if not isinstance(selected[name], list) or any(not isinstance(x, str) for x in selected[name]):
            raise ValueError("features and flags must be string arrays")
    selected["features"] = sorted(set(selected["features"]))
    return _digest(selected)[:24]


def build_command_plan(source_root, store_root, configuration, *, script, arguments=(),
                       artifact_directory=None):
    """Plan an existing build script; execution requires coordinator admission."""
    allowed = {"scripts/build-v8-runtime-profile.sh", "scripts/build-sandboxed-worker.sh",
               "scripts/build-startup-snapshot.sh", "scripts/build-platform-release.sh",
               "scripts/package-macos-app.sh"}
    if script not in allowed:
        raise ValueError("use a maintained canonical build/signing script")
    _file(source_root, script)
    key = cargo_cache_key(configuration)
    if configuration["profile"] != "release" or configuration["host"] != configuration["target"]:
        raise ValueError("canonical production scripts require a native release configuration")
    if not isinstance(arguments, (list, tuple)) or any(not isinstance(x, str) or not x or "\0" in x for x in arguments):
        raise ValueError("build arguments must be nonempty strings")
    count = {"scripts/build-v8-runtime-profile.sh": 2, "scripts/build-startup-snapshot.sh": 3}.get(script, 0)
    if len(arguments) != count:
        raise ValueError("canonical build script argument count mismatch")
    if arguments and arguments[0] not in {"core-full", "core-lite"}:
        raise ValueError("unsupported V8 profile")
    store = Path(store_root).resolve()
    overlay = {"CARGO_TARGET_DIR": str(store / "cache/cargo" / key)}
    resources = ["cargo-cache:" + key, "production-signing"]
    if script == "scripts/build-v8-runtime-profile.sh":
        overlay["ORYN_ENGINE_CACHE_ROOT"] = str(store / "cache/engines" / key)
        overlay["ORYN_RUSTY_V8_SOURCE"] = str(store / "cache/engines/sources/rusty-v8-150.4.0")
        resources.extend(["engine-cache:" + key, "engine-source:rusty-v8-150.4.0"])
    if artifact_directory is not None:
        _relative(artifact_directory)
        overlay["ORYN_ARTIFACT_ROOT"] = str(store / artifact_directory)
    return {"command": ["/bin/bash", script, *arguments],
            "cwd": str(Path(source_root).resolve()),
            "environment_overlay": overlay, "resource_keys": resources,
            "configuration_expected": configuration,
            "required_admission": "GenerationLease.environment with reviewed 51-requirement gate",
            "execution_authorized": False}


def native_inputs_digest(source_admission, configuration, *, runner_only_paths=(),
                         external_inputs=()):
    """Conservatively bind every tracked file except exact reviewed runner paths.

    The exclusion list itself is part of identity; expanding it never proves
    equivalence to a previous build. Native build files cannot be excluded.
    """
    cargo_cache_key(configuration)
    lock = source_admission["tracked"].get("Cargo.lock")
    if not lock or configuration.get("cargo_lock_sha256") != lock["sha256"]:
        raise ValueError("configuration lockfile differs from admitted committed Cargo.lock")
    excluded = sorted(set(runner_only_paths))
    permitted = ("tools/validation/", "tools/wptrunner-oryn/", "docs/", "benchmarks/")
    for name in excluded:
        _relative(name)
        if not name.startswith(permitted) or name not in source_admission["tracked"]:
            raise ValueError("runner exclusion requires an exact reviewed tracked runner path")
    return _digest({"tracked": {k: v for k, v in source_admission["tracked"].items() if k not in excluded},
                    "runner_exclusions": excluded, "external": list(external_inputs),
                    "effective_external": source_admission["external_inputs"],
                    "configuration": configuration})


def native_reuse_proof(original_build, new_build):
    from .manifests import manifest_digest, validate_manifest
    for manifest in (original_build, new_build):
        validate_manifest(manifest, expected_kind="build")
        if manifest["development_only"] or manifest["source"]["dirty"]:
            raise ValueError("development or historical dirty-source builds cannot be relabeled official")
    for key in ("build_inputs_sha256", "build_inputs", "configuration"):
        if original_build[key] != new_build[key]:
            raise ValueError("native inputs changed; a new native build is required")
    for key in ("artifacts", "immutable_inputs", "signatures"):
        if original_build[key] != new_build[key]:
            raise ValueError("reuse must retain exact original artifacts, dependencies and signatures")
    if original_build.get("platform") != new_build.get("platform"):
        raise ValueError("reuse must retain original platform observations")
    return {"schema_version": 1, "reusable": True,
            "original_build_sha256": manifest_digest(original_build),
            "original_source_commit": original_build["source"]["commit"],
            "runner_source_commit": new_build["source"]["commit"],
            "build_inputs_sha256": original_build["build_inputs_sha256"]}


def verify_worker_signature(source_root, bundle_root, *, worker_path, manifest_path):
    """Fresh read-only production verification through the existing strict checker."""
    worker = _file(bundle_root, worker_path)
    manifest = _file(bundle_root, manifest_path)
    scripts = Path(source_root).resolve() / "scripts"
    old_path = list(sys.path)
    try:
        sys.path.insert(0, str(scripts))
        spec = importlib.util.spec_from_file_location("oryn_sealed_security", scripts / "security_prebuilt.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        def read_only(command, **kwargs):
            if command[0] != "codesign" or command[1] not in ("--verify", "-dvvv", "-d"):
                raise ValueError("signature verifier attempted a mutating operation")
            return subprocess.run(["/usr/bin/codesign", *command[1:]], timeout=30, **kwargs)

        module.verify_signature({"artifacts": {"worker": worker, "worker_manifest": manifest}}, run=read_only)
    finally:
        sys.path[:] = old_path
    identity = json.loads(manifest.read_text())["signing_cert_sha1"]
    return {"path": worker_path, "verifier": "security_prebuilt.verify_signature",
            "identity": identity, "valid": True}


def bundle_inventory(bundle_root):
    """All regular files and empty directories; reject every symlink/special file."""
    root = Path(bundle_root)
    if root.is_symlink() or not root.is_dir():
        raise ValueError("bundle root must be a real directory")
    files, directories = {}, []
    for path in sorted(root.rglob("*")):
        name = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ValueError("bundle contains a symlink: " + name)
        if path.is_dir():
            directories.append(name)
        else:
            files[name] = _identity(_file(root, name))
    if not files:
        raise ValueError("empty runtime bundle")
    return {"files": files, "directories": directories}


def seal_bundle(source_root, destination, files, *, signature_verifier, directories=()):
    """Copy exact supplied runtime membership, verify production signing, seal.

    files maps relative bundle paths to exact source file paths. The signing
    callback MUST perform fresh platform verification and return signature rows;
    the canonical production adapter is verify_worker_signature. Sealing does not
    establish that a caller supplied every required dynamic runtime dependency.
    That separate admission requires a reviewed runtime-input manifest.
    """
    del source_root  # Kept explicit in the caller API; provenance belongs to the build manifest.
    destination = Path(destination)
    if destination.exists() or destination.is_symlink() or not files:
        raise ValueError("sealing requires a new nonempty bundle destination")
    if not isinstance(directories, (list, tuple)) or len(directories) != len(set(directories)):
        raise ValueError("sealed directories must be an explicit unique list")
    for name in directories:
        _relative(name)
        if name in files:
            raise ValueError("sealed directory collides with file")
    sources = {}
    for name, source in files.items():
        _relative(name)
        source = Path(source).absolute()
        checked = _file(source.anchor, source.relative_to(source.anchor).as_posix())
        sources[name] = (checked, _identity(checked))
    # Do not traverse a symlinked store parent while publishing a sealed object.
    parent = destination.parent.absolute()
    if parent.resolve(strict=True) != parent:
        raise ValueError("bundle store parent contains symlinks")
    destination.mkdir()
    for name in sorted(directories):
        (destination / name).mkdir(parents=True, exist_ok=True)
    for name, (source, identity) in sources.items():
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)  # Never link mutable Cargo cache inodes.
        target.chmod(0o755 if identity["executable"] else 0o644)
        if _identity(source) != identity or _identity(target) != identity:
            raise ValueError("runtime input changed during staging")
    before = bundle_inventory(destination)
    signatures = signature_verifier(destination)
    if not signatures or any(row.get("valid") is not True or not row.get("identity") for row in signatures):
        raise ValueError("fresh production signature verification is mandatory")
    if bundle_inventory(destination) != before:
        raise ValueError("signature verification changed bundle membership or content")
    for path in sorted(destination.rglob("*"), reverse=True):
        path.chmod(0o555 if path.is_dir() or path.stat().st_mode & 0o111 else 0o444)
    destination.chmod(0o555)
    return {"schema_version": 1, "sealed": True, "membership": before,
            "sha256": _digest(before), "signatures": signatures}


def verify_sealed_bundle(bundle_root, receipt, *, signature_verifier):
    actual = bundle_inventory(bundle_root)
    if receipt.get("sealed") is not True or actual != receipt.get("membership") or _digest(actual) != receipt.get("sha256"):
        raise ValueError("sealed runtime membership or bytes changed")
    signatures = signature_verifier(Path(bundle_root))
    if signatures != receipt.get("signatures"):
        raise ValueError("production signature observation changed")
    if bundle_inventory(bundle_root) != actual:
        raise ValueError("runtime changed during signature admission")
    return {"valid": True, "sha256": receipt["sha256"]}

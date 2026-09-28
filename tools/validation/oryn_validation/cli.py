"""One explicit command interface; inspection never implicitly launches work."""
import argparse
import json
from pathlib import Path
import sys

from .manifests import load_manifest, manifest_digest


def _json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result
    return json.loads(Path(path).read_text(), object_pairs_hook=unique)


def parser():
    root = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    commands = root.add_subparsers(dest="command", required=True)
    def command(name, help):
        return commands.add_parser(name, help=help, allow_abbrev=False)
    check = command("manifest-check", "Validate one typed document without executing it")
    check.add_argument("manifest", type=Path)
    check.add_argument("--kind", choices=("build", "run", "result", "release"))
    export = command("source-export", "Copy committed source plus verifiable Git objects outside source")
    export.add_argument("--source-root", type=Path, required=True)
    export.add_argument("--store-root", type=Path, required=True)
    export.add_argument("--export-path", required=True)
    export.add_argument("--identity-path", required=True)
    export.add_argument("--commit")
    dataset = command("dataset-export", "Export the pinned WPT Git dataset with exact normalization proof")
    dataset.add_argument("--source-root", type=Path, required=True)
    dataset.add_argument("--store-root", type=Path, required=True)
    dataset.add_argument("--export-path", required=True)
    dataset.add_argument("--identity-path", required=True)
    environment = command("environment-prepare", "Stage the pinned offline Python environment and verify its actual runtime")
    for name in ("donor-site-packages", "system-python", "stdlib", "store-root"):
        environment.add_argument("--" + name, type=Path, required=True)
    for name in ("environment-path", "identity-path", "diagnostics-path"):
        environment.add_argument("--" + name, required=True, help="Exact store-relative path")
    environment.add_argument("--lock-path", type=Path)
    environment_check = command("environment-verify", "Verify pinned Python bytes, dependencies, and system observations")
    environment_check.add_argument("identity", type=Path)
    environment_check.add_argument("--store-root", type=Path, required=True)
    reference = command("reference-environment-stage", "Stage and verify the pinned reference app and driver")
    for name in ("source-app", "source-driver", "store-root"):
        reference.add_argument("--" + name, type=Path, required=True)
    reference.add_argument("--source-archive", type=Path,
                           help="Pinned publisher archive required for archive-based app authentication")
    reference.add_argument("--output-directory", required=True, help="New store-relative environment directory")
    relay = command("fixture-relay-identity", "Record dependency identity for an already copied payload/bin/socat; does not copy or launch")
    relay.add_argument("--store-root", type=Path, required=True)
    relay.add_argument("--executable-path", required=True, help="Store-relative regular payload/bin/socat")
    relay.add_argument("--identity-path", required=True, help="New store-relative identity outside payload")
    gate = command("generation-prepare", "Prepare the full 51-requirement source gate for review")
    gate.add_argument("--source-root", type=Path, required=True)
    gate.add_argument("--predecessor", type=Path, required=True)
    gate.add_argument("--output", type=Path, required=True)
    gate.add_argument("--commit")
    gate.add_argument("--mapping", help="Committed source-relative migration mapping")
    info = command("source-info", "Verify committed checkout against exported source identity")
    info.add_argument("--source-root", type=Path, required=True)
    info.add_argument("--source-identity", type=Path, required=True)
    runtime = command("typed-runtime-prepare", "Prepare typed runtime inputs for a new native build")
    runtime.add_argument("--source-root", type=Path, required=True)
    runtime.add_argument("--store-root", type=Path, required=True)
    runtime.add_argument("--legacy-profile", type=Path, required=True)
    runtime.add_argument("--output-directory", required=True)
    runtime.add_argument("--source-identity", type=Path, required=True)
    profile = command("typed-profile-check", "Verify typed metadata against compiled engine inputs")
    profile.add_argument("--source-root", type=Path, required=True)
    profile.add_argument("--legacy-profile", type=Path, required=True)
    profile.add_argument("--typed-profile", type=Path, required=True)
    profile.add_argument("--source-identity", type=Path, required=True)
    refresh = command("engine-profile-refresh", "Bind freshly compiled snapshot/worker in a new profile")
    refresh.add_argument("--source-root", type=Path, required=True)
    refresh.add_argument("--legacy-profile", type=Path, required=True)
    refresh.add_argument("--output", type=Path, required=True)
    refresh.add_argument("--worker", type=Path)
    refresh.add_argument("--runtime-info", type=Path)
    refresh.add_argument("--startup-snapshot", type=Path)
    stage = command("engine-inputs-stage", "Copy the declared compiler/engine closure into a new data object")
    stage.add_argument("--source-root", type=Path, required=True)
    stage.add_argument("--store-root", type=Path, required=True)
    stage.add_argument("--legacy-profile", type=Path, required=True)
    stage.add_argument("--output-directory", required=True)
    stage.add_argument("--diagnostics-directory", required=True)
    controls = command("controls-prepare", "Generate pinned upstream canary/placement manifests")
    controls.add_argument("--records", type=Path, required=True)
    controls.add_argument("--store-root", type=Path, required=True)
    controls.add_argument("--runner-source", type=Path, required=True)
    controls.add_argument("--wpt-checkout", type=Path, required=True)
    controls.add_argument("--upstream-identity", type=Path, required=True)
    controls.add_argument("--output", type=Path, required=True)
    prerequisite = command("prerequisites-run", "Collect current runtime/native/topology prerequisites")
    prerequisite.add_argument("manifest", type=Path)
    prerequisite.add_argument("--store-root", type=Path, required=True)
    prerequisite.add_argument("--bundle-root", type=Path, required=True)
    prerequisite.add_argument("--stage", choices=("runtime", "native", "topology"), required=True)
    prerequisite.add_argument("--timeout-seconds", type=float, default=600)
    build = command("build-run", "Run a reviewed canonical build under source/input/resource custody")
    build.add_argument("--source-root", type=Path, required=True)
    build.add_argument("--store-root", type=Path, required=True)
    build.add_argument("--configuration", type=Path, required=True)
    build.add_argument("--source-identity", type=Path, required=True)
    build.add_argument("--generation-gate", type=Path, required=True)
    build.add_argument("--generation-review", type=Path, required=True)
    build.add_argument("--output", type=Path, required=True)
    build.add_argument("--build-inputs", type=Path, required=True)
    build.add_argument("--environment-inputs", type=Path, required=True)
    build.add_argument("--literal-environment", type=Path, required=True)
    build.add_argument("--artifact-directory", required=True)
    build.add_argument("--estimated-build-bytes", type=int, required=True)
    build.add_argument("--timeout-seconds", type=float, default=14400)
    assembly = command("build-assemble", "Join actual build receipts, seal products, and publish the typed build manifest")
    assembly.add_argument("--source-root", type=Path, required=True)
    assembly.add_argument("--store-root", type=Path, required=True)
    assembly.add_argument("--build-id", required=True)
    for name in ("execution-path", "source-export", "source-identity", "generation-gate", "generation-review", "output-directory"):
        assembly.add_argument("--" + name, required=True, help="Exact store-relative path")
    seal = command("bundle-seal", "Copy, verify signing, and seal a new explicitly inventoried bundle")
    seal.add_argument("--source-root", type=Path, required=True)
    seal.add_argument("--store-root", type=Path, required=True)
    seal.add_argument("--build-id", required=True)
    seal.add_argument("--files", type=Path, required=True, help="JSON mapping of bundle paths to source files")
    seal.add_argument("--worker-path", required=True)
    seal.add_argument("--worker-manifest-path", required=True)
    seal.add_argument("--dependency", action="append", default=[])
    snap = command("input-snapshot", "Hash one explicitly declared file or directory")
    snap.add_argument("--root", type=Path, required=True)
    snap.add_argument("--path", required=True)
    snap.add_argument("--kind", choices=("file", "directory"), default="file")
    snap.add_argument("--scope", choices=("store", "bundle"), default="store")
    verify = command("inputs-check", "Verify only declared immutable inputs and assets")
    verify.add_argument("manifest", type=Path)
    verify.add_argument("--store-root", type=Path, required=True)
    verify.add_argument("--bundle-root", type=Path)
    audit = command("source-audit", "Verify committed source and reviewed effective inputs")
    audit.add_argument("--source-root", type=Path, required=True)
    audit.add_argument("--effective-inputs", type=Path, required=True,
                       help="JSON array of reviewed exact source-relative file paths")
    audit.add_argument("--external-inputs", type=Path)
    audit.add_argument("--commit")
    audit.add_argument("--development-only", action="store_true")
    plan = command("build-plan", "Print a canonical command plan; does not build or admit source")
    plan.add_argument("--source-root", type=Path, required=True)
    plan.add_argument("--store-root", type=Path, required=True)
    plan.add_argument("--configuration", type=Path, required=True)
    plan.add_argument("--script", required=True)
    plan.add_argument("--argument", action="append", default=[], help="Repeat for each script argument")
    plan.add_argument("--artifact-directory", help="Store-relative staging directory")
    reuse = command("native-reuse-check", "Check exact native input and artifact equivalence")
    reuse.add_argument("original", type=Path)
    reuse.add_argument("proposed", type=Path)
    dev = command("wpt-dev", "Run selected WPT URLs with an existing build; lightweight developer startup")
    dev.add_argument("manifest", type=Path)
    for name in ("store-root", "bundle-root", "output", "include-file"):
        dev.add_argument("--" + name, type=Path, required=True)
    dev.add_argument("--workers", type=int, default=6)
    dev.add_argument("--timeout-seconds", type=float, default=7200)
    dev.add_argument("--audit-inputs", action="store_true", help="Expensive: hash all inputs and verify the native build once before running")
    for name in ("wpt-inspect", "wpt-run"):
        wpt = command(name, "Qualification audit (expensive): inspect typed admission" if name.endswith("inspect") else
                      "Qualification run (expensive): execute after production gates")
        wpt.add_argument("manifest", type=Path)
        wpt.add_argument("--store-root", type=Path, required=True)
        wpt.add_argument("--bundle-root", type=Path, required=True)
        if name == "wpt-run":
            wpt.add_argument("--output", type=Path, required=True)
            wpt.add_argument("--timeout-seconds", type=float, default=None)
    result = command("wpt-reconcile", "Count a single original's events; not release qualification")
    result.add_argument("--include", type=Path, required=True)
    result.add_argument("--events", type=Path, required=True)
    result.add_argument("--execution", type=Path, required=True)
    result.add_argument("--assessment-total", type=int, default=752)
    for name in ("cleanup-preview", "cleanup-execute"):
        cleanup = command(name, "Preview cache eviction" if name.endswith("preview") else
                          "Execute an unchanged reviewed cache-only preview")
        cleanup.add_argument("--store-root", type=Path, required=True)
        cleanup.add_argument("--index", type=Path, required=True)
        if name == "cleanup-preview":
            cleanup.add_argument("--cache-budget-bytes", type=int, default=120 * 1024**3)
        else:
            cleanup.add_argument("--preview", type=Path, required=True)
            cleanup.add_argument("--preview-sha256", required=True)
            cleanup.add_argument("--receipt", type=Path, required=True)
    return root


def dispatch(args):
    if args.command == "environment-prepare":
        from .environments import prepare_environment
        return prepare_environment(args.donor_site_packages, args.system_python, args.stdlib, args.store_root,
            environment_path=args.environment_path, identity_path=args.identity_path,
            diagnostics_path=args.diagnostics_path, lock_path=args.lock_path)
    if args.command == "environment-verify":
        from .environments import verify_environment
        return verify_environment(_json(args.identity), args.store_root)
    if args.command == "reference-environment-stage":
        from .reference_environment import stage_reference_environment
        return stage_reference_environment(args.source_app, args.source_driver, args.store_root,
            args.output_directory, source_archive=args.source_archive)
    if args.command == "fixture-relay-identity":
        from .fixture_relay import prepare_relay_identity
        return prepare_relay_identity(args.store_root, executable_path=args.executable_path,
                                      identity_path=args.identity_path)
    if args.command == "dataset-export":
        from .datasets import export_dataset
        return export_dataset(args.source_root, args.store_root, export_path=args.export_path,
                              identity_path=args.identity_path)
    if args.command == "source-export":
        from .generation import export_source
        return export_source(args.source_root, args.store_root, export_path=args.export_path,
                             identity_path=args.identity_path, expected_commit=args.commit)
    if args.command == "generation-prepare":
        from .generation import prepare_generation_gate
        return prepare_generation_gate(args.source_root, args.predecessor, args.output,
                                       expected_commit=args.commit, mapping_path=args.mapping)
    if args.command == "source-info":
        from .generation import source_info
        return source_info(args.source_root, args.source_identity)
    if args.command == "typed-runtime-prepare":
        from .runtime_metadata import prepare_typed_runtime
        return prepare_typed_runtime(args.source_root, args.store_root, legacy_profile=args.legacy_profile,
                                     output_directory=args.output_directory,
                                     source_identity=args.source_identity)
    if args.command == "typed-profile-check":
        from .runtime_metadata import validate_profile_replacement
        return validate_profile_replacement(args.source_root, args.legacy_profile,
                                            args.typed_profile, args.source_identity)
    if args.command == "engine-profile-refresh":
        from .runtime_metadata import refresh_engine_profile
        return refresh_engine_profile(args.source_root, args.legacy_profile, args.output,
                                      worker=args.worker, runtime_info=args.runtime_info,
                                      startup_snapshot=args.startup_snapshot)
    if args.command == "engine-inputs-stage":
        from .runtime_metadata import stage_engine_inputs
        return stage_engine_inputs(args.source_root, args.store_root, legacy_profile=args.legacy_profile,
                                   output_directory=args.output_directory, diagnostics_directory=args.diagnostics_directory)
    if args.command == "controls-prepare":
        from .wpt import prepare_controls
        return prepare_controls(_json(args.records), store_root=args.store_root,
                                runner_source=args.runner_source, wpt_checkout=args.wpt_checkout,
                                upstream_identity=args.upstream_identity, output=args.output)
    if args.command == "prerequisites-run":
        from .prerequisites import execute
        return execute(args.manifest, store_root=args.store_root, bundle_root=args.bundle_root,
                       stage=args.stage, timeout_seconds=args.timeout_seconds)
    if args.command == "build-run":
        from .build_evidence import execute_build, canonical_outputs
        config = _json(args.configuration)
        return execute_build(args.source_root, args.store_root, configuration=config,
            source_identity=args.source_identity, generation_gate=args.generation_gate,
            generation_review=args.generation_review, script="scripts/build-sandboxed-worker.sh",
            output=args.output, immutable_inputs=_json(args.build_inputs),
            output_paths=canonical_outputs(config, args.artifact_directory),
            environment_inputs=_json(args.environment_inputs), literal_environment=_json(args.literal_environment),
            artifact_directory=args.artifact_directory, estimated_build_bytes=args.estimated_build_bytes,
            timeout_seconds=args.timeout_seconds)
    if args.command == "build-assemble":
        from .assembly import assemble_build
        return assemble_build(args.source_root, args.store_root, execution_path=args.execution_path,
            source_export=args.source_export, source_identity=args.source_identity,
            generation_gate=args.generation_gate, generation_review=args.generation_review,
            output_directory=args.output_directory, build_id=args.build_id)
    if args.command == "bundle-seal":
        from .builds import seal_bundle, verify_worker_signature
        from .manifests import relative_path
        from .processes import ResourceLease
        from .retention import register_object
        relative_path(args.build_id)
        if "/" in args.build_id:
            raise ValueError("build ID must be a single path component")
        store = args.store_root.resolve(strict=True)
        parent = store / "builds" / args.build_id
        for ancestor in (parent, *parent.parents):
            if ancestor.is_symlink():
                raise ValueError("build destination cannot traverse symlinks")
        files = _json(args.files)
        if not isinstance(files, dict) or any(not isinstance(v, str) for v in files.values()):
            raise ValueError("bundle file map must contain exact string paths")
        with ResourceLease(store, ["production-signing", "build:" + args.build_id]) as lease:
            # No long-lived child exists in this operation; fresh verification
            # uses bounded codesign queries. A failed seal remains unregistered.
            parent.mkdir(parents=True, exist_ok=True)
            if (parent / "runtime").exists() or (parent / "seal.json").exists():
                raise ValueError("build ID already has a sealed destination")
            receipt = seal_bundle(args.source_root, parent / "runtime", files,
                signature_verifier=lambda root: [verify_worker_signature(args.source_root, root,
                    worker_path=args.worker_path, manifest_path=args.worker_manifest_path)])
            with (parent / "seal.json").open("x") as output:
                json.dump(receipt, output, indent=2)
                output.write("\n")
            register_object(store, "build:" + args.build_id, path="builds/" + args.build_id,
                            kind="build", dependencies=args.dependency, pinned=True)
            lease.mark_settled({"cleanup_complete": True, "cleanup_errors": []})
        return {"build_id": args.build_id, "sealed": True, "receipt": str(parent / "seal.json"),
                "production_admitted": False}
    if args.command == "manifest-check":
        document = load_manifest(args.manifest, expected_kind=args.kind)
        return {"schema_valid": True, "kind": document["kind"],
                "manifest_sha256": manifest_digest(document), "execution": "not-requested"}
    if args.command == "input-snapshot":
        from .inputs import snapshot_input
        return snapshot_input(args.root, args.path, kind=args.kind, scope=args.scope)
    if args.command == "inputs-check":
        from .inputs import verify_inputs
        document = load_manifest(args.manifest)
        records = [*document.get("immutable_inputs", []), *document.get("artifacts", []),
                   *document.get("assets", {}).values()]
        if not records:
            raise ValueError("manifest has no immutable input declarations")
        unique = {}
        for record in records:
            key = record["scope"], record["path"]
            if key in unique and unique[key] != record:
                raise ValueError("conflicting input declarations")
            unique[key] = record
        verify_inputs(unique.values(), store_root=args.store_root, bundle_root=args.bundle_root)
        return {"inputs_valid": True, "declared_inputs": len(unique)}
    if args.command == "source-audit":
        from .builds import audit_source
        effective = _json(args.effective_inputs)
        if not isinstance(effective, list) or any(not isinstance(x, str) for x in effective):
            raise ValueError("effective inputs must be an explicit JSON string array")
        return audit_source(args.source_root, effective_inputs=effective,
                            external_inputs=_json(args.external_inputs) if args.external_inputs else [],
                            official=not args.development_only, expected_commit=args.commit)
    if args.command == "build-plan":
        from .builds import build_command_plan
        return build_command_plan(args.source_root, args.store_root,
                                  _json(args.configuration), script=args.script,
                                  arguments=args.argument, artifact_directory=args.artifact_directory)
    if args.command == "native-reuse-check":
        from .builds import native_reuse_proof
        return native_reuse_proof(load_manifest(args.original, "build"),
                                  load_manifest(args.proposed, "build"))
    if args.command == "wpt-dev":
        from .developer_wpt import execute
        code = execute(args.manifest, args.store_root, args.bundle_root, args.output, args.include_file,
                       workers=args.workers, timeout_seconds=args.timeout_seconds, audit=args.audit_inputs)
        if code:
            raise RuntimeError('developer run incomplete or infrastructure failure; inspect ' + str(args.output))
        return _json(args.output / 'timing-result.json')
    if args.command.startswith("wpt-"):
        if args.command == "wpt-reconcile":
            from .results import read_mozlog, reconcile_wpt
            return reconcile_wpt(args.include.read_text().splitlines(), read_mozlog(args.events),
                                 execution=_json(args.execution), assessment_total=args.assessment_total)
        from .wpt import load_inputs, inspect_admission, execute
        if args.command == "wpt-run":
            return execute(args.manifest, store_root=args.store_root, bundle_root=args.bundle_root,
                           output=args.output, timeout_seconds=args.timeout_seconds)
        from .admission import UNMIGRATED_GATES
        selected = inspect_admission(load_inputs(args.manifest, store_root=args.store_root,
                                                bundle_root=args.bundle_root))
        return {"migrated_checks_passed": True, "automated": len(selected["urls"]),
                "unmigrated_gates": list(UNMIGRATED_GATES), "official_launch_admitted": False}
    if args.command.startswith("cleanup-"):
        from .retention import retention_lock, preview_cleanup, execute_cleanup
        if args.index.absolute() != args.store_root.absolute() / "retention.json":
            raise ValueError("cleanup must use the same retention.json as resource leases")
        if args.command == "cleanup-preview":
            with retention_lock(args.store_root):
                return preview_cleanup(args.store_root, _json(args.index),
                                       cache_budget_bytes=args.cache_budget_bytes)
        from .processes import assert_no_active_writers
        def settled(_object_ids):
            assert_no_active_writers(args.store_root)
            return True
        return execute_cleanup(args.store_root, args.index, _json(args.preview),
                               expected_preview_sha256=args.preview_sha256,
                               receipt_path=args.receipt, confirm_no_active_writers=settled)
    raise ValueError("unknown command")


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        result = dispatch(args)
    except (ValueError, RuntimeError, OSError, KeyError, TypeError) as error:
        print(json.dumps({"error": str(error), "command": args.command}), file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True, indent=2, default=str))
    return 0

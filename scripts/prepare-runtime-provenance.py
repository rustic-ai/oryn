#!/usr/bin/env python3
"""Own one canonical profile build, then publish separate runtime provenance."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(args):
    provenance = load(ROOT / 'scripts/runtime_provenance.py', 'owned_runtime_provenance')
    gate, admitted_inputs = provenance.gate_inputs(
        ROOT, args.gate, args.gate_sha256, args.whole_source_lease, args.whole_source_lease_sha256)
    custody_record = provenance.record(args.custody, args.custody_sha256)
    provenance.bound(gate, custody_record, ROOT)
    custody = load(args.custody, 'owned_profile_custody')
    candidate = custody.configure_candidate(args.gate,args.gate_sha256,
        args.whole_source_lease,args.whole_source_lease_sha256)
    provenance.require(custody.ROOT.resolve() == ROOT.resolve() and
                       custody.GATE.resolve() == args.gate.resolve(), 'custody uses different root/gate')
    before = custody.check(args.whole_source_lease, args.whole_source_lease_sha256, args.gate_sha256)
    provenance.require(not before['errors'], 'source custody failed before profile build')
    admission = load(ROOT / 'scripts/generation_gate.py', 'owned_profile_gate').GenerationLease(
        args.gate, expected=args.gate_sha256)
    output = args.output_directory.absolute()
    evidence = args.evidence_directory.absolute()
    for path in (output, evidence):
        provenance.require(path.resolve().is_relative_to(ROOT / 'artifacts'), 'outputs must be repository artifacts')
        provenance.require(not path.exists() and not path.is_symlink(), 'refusing existing output: ' + str(path))
    output_destination, evidence_destination = output.resolve(), evidence.resolve()
    provenance.require(output_destination != evidence_destination and
                       not output_destination.is_relative_to(evidence_destination) and
                       not evidence_destination.is_relative_to(output_destination), 'overlapping output directories')
    provenance.require(output == output_destination and evidence == evidence_destination,
                       'output directories must use canonical paths')
    engine_source = args.engine_source.absolute()
    provenance.require(engine_source.resolve().is_relative_to(ROOT / 'target'), 'engine source must be isolated in repository target')
    environment = dict(os.environ)
    import re
    for name in list(environment):
        if re.fullmatch(r'ORYN_REPAIR[0-9]+_SUPERVISED_GROUP',name):
            environment.pop(name)
    for name in ('V8_FROM_SOURCE', 'RUSTY_V8_ARCHIVE', 'RUSTY_V8_SRC_BINDING_PATH',
                 'ORYN_V8_BRIDGE_ABI', 'ORYN_V8_PROFILE_MANIFEST', 'ORYN_STARTUP_SNAPSHOT',
                 'ORYN_ICU_DATA_PATH', 'ORYN_ICU_DATA_FINGERPRINT_PATH', 'ORYN_SECURITY_PREBUILT_CANDIDATE',
                 'ORYN_V8_ENGINE_REUSE_RECEIPT', 'ORYN_V8_ENGINE_REUSE_SHA256',
                 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR', 'GN_ARGS', 'EXTRA_GN_ARGS', 'DOCS_RS',
                 'DENO_TRYBUILD', 'CXXSTDLIB', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
                 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'ORYN_LITE_ICU_DATA'):
        environment.pop(name, None)
    selected_environment = dict(
        ORYN_RUSTY_V8_SOURCE=str(engine_source), GN=str(args.gn.absolute()), NINJA=str(args.ninja.absolute()),
        CLANG_BASE_PATH=str(args.clang_base.absolute()), LIBCLANG_PATH=str(args.libclang.absolute()),
        SDKROOT=str(args.sdkroot.absolute()), PYTHONDONTWRITEBYTECODE='1', RUST_BACKTRACE='1',
        **{candidate['supervisor_key']:'runtime-profile-build'})
    environment.update(selected_environment)
    inputs = provenance.merge_records(admitted_inputs, [custody_record],
        [provenance.record(path) for path in (args.gn, args.ninja, args.clang_base / 'bin/clang',
                                             args.libclang / 'libclang.dylib', args.sdkroot / 'SDKSettings.json')])
    producers = [provenance.record(ROOT / 'scripts' / name) for name in
                 ('prepare-runtime-provenance.py', 'runtime_provenance.py', 'build-v8-runtime-profile.sh')]
    producers.append(custody_record)
    provenance.check_records(inputs + producers)
    evidence.mkdir(parents=True, exist_ok=False)
    command = ['bash', str(ROOT / 'scripts/build-v8-runtime-profile.sh'), args.profile, str(output)]
    start_path, log_path = evidence / 'start.json', evidence / 'build.log'
    build_path, final_path = evidence / 'build-receipt.json', evidence / 'runtime-provenance.json'
    environment = admission.environment(environment)  # Bind exact generation last.
    record = dict(schema_version=1, kind='oryn_owned_runtime_build', generation_gate=admitted_inputs[0],
                  whole_source_lease=admitted_inputs[1], custody_source=custody_record, producer_sources=producers, inputs=inputs,
                  command=command, profile_name=args.profile, output_directory=str(output),
                  environment={name: environment[name] for name in selected_environment} |
                              {name: environment[name] for name in ('ORYN_GENERATION_GATE', 'ORYN_GENERATION_GATE_SHA256')},
                  before=before, started_at=time.time(), engine_reuse=None, qualification_complete=False)
    started = time.monotonic()
    stdout_copy, stderr_copy = os.dup(1), os.dup(2)
    try:
        with log_path.open('xb') as output_log:
            os.dup2(output_log.fileno(), 1)
            os.dup2(output_log.fileno(), 2)
            record['process'] = custody.run_owned_dispatch(command, environment, start_path)
    finally:
        os.dup2(stdout_copy, 1)
        os.dup2(stderr_copy, 2)
        os.close(stdout_copy)
        os.close(stderr_copy)
        record['elapsed_seconds'] = time.monotonic() - started
        record['after'] = custody.check(args.whole_source_lease, args.whole_source_lease_sha256, args.gate_sha256)
        record['log'] = provenance.record(log_path)
        record['start_receipt'] = provenance.record(start_path) if start_path.is_file() else None
        process = record.get('process', {})
        record['success'] = (type(process.get('returncode')) is int and process.get('returncode') == 0 and process.get('group_settled') is True and
                             not process.get('execution_error') and not process.get('cleanup_errors') and
                             not record['after']['errors'] and record['start_receipt'] is not None)
        record['validation_error'] = None
        try:
            provenance.check_records(inputs + producers)
            if record['success']:
                profile_path = output / (args.profile + '-manifest.json')
                profile, outputs = provenance.profile_inputs(profile_path, provenance.digest(profile_path), ROOT)
                provenance.require(profile['profile'] == args.profile, 'canonical output profile changed')
                record['profile'], record['outputs'] = provenance.record(profile_path), outputs
                admission.check()
                latest = custody.check(args.whole_source_lease, args.whole_source_lease_sha256, args.gate_sha256)
                provenance.require(not latest['errors'], 'source changed during output validation')
                record['after'] = latest
                provenance.check_records(inputs + producers + outputs)
        except (ValueError, OSError, RuntimeError, KeyError, TypeError, json.JSONDecodeError) as error:
            record['success'] = False
            record['validation_error'] = str(error)
        provenance.save_new(build_path, record)
    if not record['success']:
        return record.get('process', {}).get('returncode') or 1
    receipt = dict(schema_version=1, kind='oryn_runtime_provenance', status='complete',
                   generation_gate=admitted_inputs[0], whole_source_lease=admitted_inputs[1],
                   profile=record['profile'], build_receipt=provenance.record(build_path), engine_reuse=None,
                   inputs=provenance.merge_records(record['outputs'], producers), qualification_complete=False)
    checked = provenance.validate_runtime_provenance_data(
        receipt, root=ROOT, gate_path=args.gate, gate_sha256=args.gate_sha256,
        lease_path=args.whole_source_lease, lease_sha256=args.whole_source_lease_sha256,
        profile_path=record['profile']['path'], profile_sha256=record['profile']['sha256'])
    admission.check()
    provenance.require(not custody.check(args.whole_source_lease, args.whole_source_lease_sha256,
                                        args.gate_sha256)['errors'], 'source drift before publication')
    provenance.check_records(checked['records'])
    provenance.save_new(final_path, receipt)  # Publish only after all validation; never overwrite.
    print(json.dumps(dict(runtime_provenance=provenance.record(final_path), profile=record['profile'],
                          build_receipt=provenance.record(build_path), qualification_complete=False)))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('gate', 'whole-source-lease', 'custody', 'output-directory', 'evidence-directory',
                 'engine-source', 'gn', 'ninja', 'clang-base', 'libclang', 'sdkroot'):
        parser.add_argument('--' + flag, type=Path, required=True)
    for flag in ('gate-sha256', 'whole-source-lease-sha256', 'custody-sha256'):
        parser.add_argument('--' + flag, required=True)
    parser.add_argument('--profile', choices=('core-full', 'core-lite'), default='core-full')
    args = parser.parse_args()
    try:
        code = run(args)
    except (ValueError, OSError, RuntimeError, KeyError, TypeError) as error:
        parser.exit(1, str(error) + '\n')
    sys.exit(code)


if __name__ == '__main__':
    main()

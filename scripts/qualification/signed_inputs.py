"""Prepare signed worker inputs explicitly; acceptance only validates frozen outputs."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'artifacts/completion-validation-2026-09-18'
KEYS = ('ORYN_TEST_NATIVE_BIN', 'ORYN_PAGE_WORKER', 'ORYN_WPT_BINARY')
KIND = 'repair49_signed_python_inputs'
BUILD = ROOT / 'scripts/build-sandboxed-worker.sh'
COMMAND = ['/bin/bash', str(BUILD)]
BUNDLE_ID = 'ai.rustic.oryn.page-worker'
TEAM_ID = '5HVA9VFF8K'
CERT_SHA1 = '9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90'
SOURCES = ('scripts/build-sandboxed-worker.sh', 'scripts/security_prebuilt.py',
           'scripts/completion_provenance.py', 'scripts/generation_gate.py',
           'scripts/native_protocol_versions.py', 'scripts/source-tree-hash.sh',
           'scripts/validate-v8-runtime-profile.py', 'scripts/validate-v8-rust-runtime.py',
           'scripts/rusty_v8_source_provenance.py', 'scripts/runtime_provenance.py', 'sandbox/oryn-page-worker.entitlements',
           'sandbox/oryn-page-worker-Info.plist')


def module_at(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def custody_module():
    return module_at(ROOT / 'scripts/qualification/custody.py', 'repair49_signed_custody')


def read_identity(path):
    path = Path(path).absolute()
    destination = str(path.resolve(strict=True))
    before = path.stat()
    data = path.read_bytes()
    after = path.stat()
    if (destination != str(path.resolve(strict=True))
            or (before.st_size, before.st_mtime_ns, before.st_ino)
            != (after.st_size, after.st_mtime_ns, after.st_ino)):
        raise ValueError('input changed during capture: ' + str(path))
    return (dict(path=str(path), resolved_path=destination,
                 sha256=hashlib.sha256(data).hexdigest(), size_bytes=len(data)), data)


def identity(path):
    return read_identity(path)[0]


def read_json_record(record):
    actual, data = read_identity(record['path'])
    if actual != record:
        raise ValueError('JSON input differs from its captured identity')
    return json.loads(data)


def source_identities(custody):
    paths = [ROOT / path for path in SOURCES]
    paths.extend((Path(__file__).resolve(), ROOT / 'scripts/qualification/custody.py'))
    for path in paths:
        custody.bound_source(path)
    return {str(path.relative_to(ROOT)): custody.sha(path) for path in paths}


def security_module(custody):
    for name in ('security_prebuilt.py', 'completion_provenance.py', 'generation_gate.py'):
        custody.bound_source(ROOT / 'scripts' / name)
    sys.path.insert(0, str(ROOT / 'scripts'))
    return module_at(ROOT / 'scripts/security_prebuilt.py', 'repair49_security_prebuilt')


def signature_check(security, worker, manifest):
    # Only read/verify options are admitted. This route can never sign or build.
    def run(command, **kwargs):
        if command[0] != 'codesign' or command[1] not in ('--verify', '-dvvv', '-d'):
            raise ValueError('signature validator requested a mutating command')
        return subprocess.run(['/usr/bin/codesign', *command[1:]], timeout=30, **kwargs)
    security.verify_signature(dict(artifacts=dict(worker=Path(worker), worker_manifest=Path(manifest))), run=run)


def process_ok(process):
    return (type(process.get('returncode')) is int and process['returncode'] == 0
            and process.get('execution_error') is None and process.get('cleanup_errors') == []
            and process.get('group_settled') is True)


def layout(profile):
    raw = Path(profile['worker_path']).absolute()
    if (raw.name != 'oryn-page-worker' or raw.parent.name != 'release'
            or not raw.resolve().is_relative_to(ROOT.resolve())):
        raise ValueError('profile worker does not identify the canonical release output')
    release = raw.parent
    worker = release / 'OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
    return dict(cli=release / 'oryn', worker=worker,
                worker_manifest=worker.parents[1] / 'Resources/worker-manifest.json',
                bundle_profile=worker.parents[1] / 'Resources/v8-profile-manifest.json',
                target=release.parent)


def overlay_for(artifacts):
    return dict(ORYN_TEST_NATIVE_BIN=artifacts['cli']['resolved_path'],
                ORYN_PAGE_WORKER=artifacts['worker']['resolved_path'],
                ORYN_WPT_BINARY=artifacts['cli']['resolved_path'])


def effective_environment(base, overlay, admission):
    if set(overlay) != set(KEYS) or any(key in base for key in KEYS):
        raise ValueError('signed overlay must replace exactly the three absent baseline keys')
    if overlay['ORYN_WPT_BINARY'] != overlay['ORYN_TEST_NATIVE_BIN']:
        raise ValueError('WPT must consume the exact admitted native CLI')
    effective = dict(base)
    effective.update(overlay)
    effective = admission.environment(effective)
    validate_environment(base, effective, overlay)
    return effective


def validate_environment(base, effective, overlay):
    if (set(overlay) != set(KEYS) or any(key in base for key in KEYS)
            or {key: effective.get(key) for key in KEYS} != overlay
            or {key: value for key, value in effective.items() if key not in KEYS} != base
            or set(effective) != set(base) | set(KEYS)):
        raise ValueError('effective Python environment differs beyond the admitted three keys')


def validate_record(value, custody, lease_path, lease_sha, gate_sha):
    if (type(value.get('schema_version')) is not int or value['schema_version'] != 1
            or value.get('prepared') is not True or value.get('kind') != f'repair{custody.CANDIDATE_NUMBER}_signed_python_inputs' or value.get('mode') != 'canonical_preparation'
            or value.get('generation') != dict(path=str(custody.GATE.resolve()), sha256=gate_sha)
            or value.get('whole_lease') != dict(path=str(lease_path.resolve()), sha256=lease_sha)
            or value.get('producer_sources') != source_identities(custody)
            or value.get('command') != COMMAND or not process_ok(value.get('process', {}))):
        raise ValueError('signed preparation provenance, command or custody mismatch')
    for phase in ('before', 'after'):
        result = value['whole_source_' + phase]
        if (result.get('errors') != [] or result.get('declaration_sha256') != gate_sha
                or result.get('lease_sha256') != lease_sha):
            raise ValueError('signed preparation lacks exact whole-source custody')
    if value.get('signing_identity') != CERT_SHA1:
        raise ValueError('signed preparation signer differs from the native contract')
    if value.get('overlay') != overlay_for(value['artifacts']):
        raise ValueError('signed overlay paths disagree with captured artifacts')


def load_inputs(path, expected, custody, lease_path, lease_sha, gate_sha):
    # Capture original aliases before any validator or codesign child can run.
    record, data = read_identity(path)
    if record['sha256'] != expected:
        raise ValueError('explicit signed-input receipt digest changed')
    value = json.loads(data)
    validate_record(value, custody, lease_path, lease_sha, gate_sha)
    rows = [record, value['log'], value['process_receipt'], value['parent_runtime_output'],
            value['parent_runtime_process_receipt'], *value['artifacts'].values()]
    errors = custody.runtime_records_check(rows)
    if errors:
        raise ValueError('signed inputs changed before validation: ' + repr(errors))
    if read_json_record(value['process_receipt']) != value['process']:
        raise ValueError('signed preparation process receipt differs')
    profile = value['profile']
    runtime = custody.load_runtime(profile['path'], profile['sha256'])
    provenance = custody.load_runtime_provenance(value['runtime_provenance']['path'],
        value['runtime_provenance']['sha256'], lease_path, lease_sha, gate_sha, runtime)
    manifest = read_json_record(next(row for row in runtime['records'] if row['path'] == profile['path']))
    paths = layout(manifest)
    artifacts = value['artifacts']
    if set(artifacts) != {'cli', 'worker', 'worker_manifest', 'bundle_profile'}:
        raise ValueError('signed preparation artifact set changed')
    if any(artifacts[name]['path'] != str(paths[name]) for name in artifacts):
        raise ValueError('signed preparation layout differs from the profile build directory')
    for name in ('cli', 'worker'):
        if not os.access(artifacts[name]['resolved_path'], os.X_OK):
            raise ValueError('prepared executable lacks execute permission')
    worker_manifest = read_json_record(artifacts['worker_manifest'])
    if (worker_manifest.get('worker_build_sha256') != manifest['worker_sha256']
            or worker_manifest.get('worker_build_bytes') != manifest['worker_size_bytes']
            or worker_manifest.get('v8_profile_sha256') != profile['sha256']
            or artifacts['bundle_profile']['sha256'] != profile['sha256']
            or worker_manifest.get('bundle_id') != BUNDLE_ID
            or worker_manifest.get('team_id') != TEAM_ID
            or worker_manifest.get('signing_cert_sha1') != CERT_SHA1
            or worker_manifest.get('source_tree_sha256') != value['source_tree_sha256']):
        raise ValueError('signed worker/raw worker/profile/source provenance mismatch')
    parent = read_json_record(value['parent_runtime_output'])
    if (read_json_record(value['parent_runtime_process_receipt'])
            != value['parent_runtime_process'] or not process_ok(value['parent_runtime_process'])
            or value['parent_runtime_command'] != [artifacts['cli']['resolved_path'], 'native', '--runtime-info']):
        raise ValueError('parent runtime-info command or process custody changed')
    validate_parent(parent, manifest, artifacts, custody)
    security = security_module(custody)
    state = dict(receipt=record, value=value, records=rows, runtime=runtime, provenance=provenance)
    check_inputs(state, custody, security)
    return state


def validate_parent(parent, profile, artifacts, custody):
    custody.bound_source(ROOT / 'scripts/native_protocol_versions.py')
    protocols = module_at(ROOT / 'scripts/native_protocol_versions.py', 'repair49_protocols')
    expected = dict(native_v8=True, build_profile='release', execution_mode='sandboxed_worker',
                    sandboxed=True, sandbox_state='app_sandbox', worker_bundle_id=BUNDLE_ID,
                    worker_team_id=TEAM_ID, worker_signing_cert_sha1=CERT_SHA1,
                    worker_binary_sha256=artifacts['worker']['sha256'],
                    v8_archive_sha256=profile['archive_sha256'],
                    v8_binding_sha256=profile['binding_sha256'],
                    v8_bridge_abi_sha256=profile['bridge_abi_sha256'])
    if not protocols.runtime_matches(parent, ROOT) or any(parent.get(key) != value for key, value in expected.items()):
        raise ValueError('parent runtime-info does not attest the admitted production worker/profile')


def check_inputs(state, custody, security=None):
    value = state['value']
    security = security_module(custody) if security is None else security
    worker = value['artifacts']['worker']['path']
    errors = (custody.runtime_records_check(state['records']) + custody.runtime_check(state['runtime'])['errors']
              + custody.check_runtime_provenance(state['provenance'], state['runtime'])['errors'])
    if errors or security.bundle_inventory(worker) != value['bundle_files']:
        raise ValueError('signed input or complete bundle inventory drift: ' + repr(errors))
    signature_check(security, worker, value['artifacts']['worker_manifest']['path'])
    errors = (custody.runtime_records_check(state['records']) + custody.runtime_check(state['runtime'])['errors']
              + custody.check_runtime_provenance(state['provenance'], state['runtime'])['errors'])
    if errors or security.bundle_inventory(worker) != value['bundle_files']:
        raise ValueError('signed inputs changed during live signature validation: ' + repr(errors))
    return dict(receipt=state['receipt'], overlay=value['overlay'], bundle_file_count=len(value['bundle_files']),
                live_signature_verified=True, profile=value['profile'])


def preparation_environment(admission, custody, runtime, identity_value):
    if identity_value != CERT_SHA1:
        raise ValueError('canonical preparation requires the native trusted certificate')
    profile = read_json_record(next(row for row in runtime['records'] if row['path'] == runtime['profile']['path']))
    paths = layout(profile)
    env = custody.runtime_environment(os.environ, runtime)
    for key in list(env):
        if re.fullmatch(r'ORYN_REPAIR[0-9]+_SUPERVISED_GROUP',key):
            env.pop(key)
    for key in (*KEYS, 'ORYN_SECURITY_PREBUILT_CANDIDATE', 'ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256',
                'ORYN_EXPECTED_BUNDLE_ID', 'ORYN_APPLE_TEAM_ID', 'ORYN_REPAIR48_SUPERVISED_GROUP',
                'ORYN_REPAIR49_SUPERVISED_GROUP'):
        env.pop(key, None)
    env.update(CARGO_TARGET_DIR=str(paths['target']), ORYN_CODESIGN_IDENTITY=identity_value,
               ORYN_EXPECTED_BUNDLE_ID=BUNDLE_ID, ORYN_APPLE_TEAM_ID=TEAM_ID,
               PYTHONDONTWRITEBYTECODE='1')
    return admission.environment(env), paths


def prepare(args, custody, admission):
    output = args.output.absolute()
    if not output.resolve().is_relative_to(BASE.resolve()):
        raise ValueError('preparation output must be a fresh completion artifact directory')
    output.mkdir(exist_ok=False)
    record = dict(kind=f'repair{custody.CANDIDATE_NUMBER}_signed_python_inputs', schema_version=1, mode='canonical_preparation',
                  generation=dict(path=str(custody.GATE.resolve()), sha256=args.gate_sha256),
                  whole_lease=dict(path=str(args.whole_lease.resolve()), sha256=args.whole_lease_sha256),
                  command=COMMAND, signing_identity=args.signing_identity)
    runtime, env, provenance = None, None, None
    try:
        record['whole_source_before'] = custody.check(args.whole_lease, args.whole_lease_sha256, args.gate_sha256)
        if record['whole_source_before']['errors']:
            raise ValueError('preparation source admission failed')
        record['producer_sources'] = source_identities(custody)
        runtime = custody.load_runtime(args.runtime_profile, args.runtime_profile_sha256)
        provenance = custody.load_runtime_provenance(args.runtime_provenance, args.runtime_provenance_sha256,
            args.whole_lease, args.whole_lease_sha256, args.gate_sha256, runtime)
        record['runtime_provenance'] = dict(path=provenance['path'], sha256=provenance['sha256'])
        record['profile'] = runtime['profile']
        env, paths = preparation_environment(admission, custody, runtime, args.signing_identity)
        record['build_environment'] = {name: env[name] for name in
            ('CARGO_TARGET_DIR', 'ORYN_CODESIGN_IDENTITY', 'ORYN_V8_PROFILE_MANIFEST',
             'ORYN_GENERATION_GATE', 'ORYN_GENERATION_GATE_SHA256')}
        admission.check()
        record['process'] = custody.run_owned_dispatch(COMMAND, env,
            start_receipt=output / 'build-start.json', log_path=output / 'build.log')
        custody.save(output / 'build-process.json', record['process'])
        record['process_receipt'] = identity(output / 'build-process.json')
        record['log'] = identity(output / 'build.log')
        if not process_ok(record['process']):
            raise ValueError('canonical build/signing or process custody failed')
        if custody.runtime_check(runtime)['errors']:
            raise ValueError('canonical build changed inputs owned by the supplied runtime profile')
        record['artifacts'] = {name: identity(paths[name]) for name in
                               ('cli', 'worker', 'worker_manifest', 'bundle_profile')}
        record['overlay'] = overlay_for(record['artifacts'])
        parent_env = dict(env, ORYN_PAGE_WORKER=record['overlay']['ORYN_PAGE_WORKER'])
        parent_env = admission.environment(parent_env)
        record['parent_runtime_command'] = [record['overlay']['ORYN_TEST_NATIVE_BIN'], 'native', '--runtime-info']
        record['parent_runtime_process'] = custody.run_owned_dispatch(record['parent_runtime_command'], parent_env,
            start_receipt=output / 'parent-runtime-start.json', log_path=output / 'parent-runtime.json')
        custody.save(output / 'parent-runtime-process.json', record['parent_runtime_process'])
        record['parent_runtime_process_receipt'] = identity(output / 'parent-runtime-process.json')
        record['parent_runtime_output'] = identity(output / 'parent-runtime.json')
        if not process_ok(record['parent_runtime_process']):
            raise ValueError('parent runtime-info or cleanup failed')
        validate_parent(read_json_record(record['parent_runtime_output']),
            read_json_record(next(row for row in runtime['records'] if row['path'] == runtime['profile']['path'])),
            record['artifacts'], custody)
        security = security_module(custody)
        record['bundle_files'] = security.bundle_inventory(paths['worker'])
        # The canonical script captured this hash while the same whole-source
        # lease was held and checked parent runtime-info, raw worker ABI and smokes.
        record['source_tree_sha256'] = read_json_record(record['artifacts']['worker_manifest'])['source_tree_sha256']
        if not re.fullmatch('[0-9a-f]{64}', record['source_tree_sha256']):
            raise ValueError('canonical manifest source fingerprint is invalid')
        signature_check(security, paths['worker'], paths['worker_manifest'])
        admission.check()
    except BaseException as error:
        record['preparation_error'] = repr(error)
    finally:
        record['whole_source_after'] = custody.check(args.whole_lease, args.whole_lease_sha256, args.gate_sha256)
        if runtime is not None:
            record['runtime_after'] = custody.runtime_check(runtime)
        if provenance is not None:
            record['runtime_provenance_after'] = custody.check_runtime_provenance(provenance, runtime)
        record['prepared'] = not (record.get('runtime_provenance_after', {}).get('errors') or record.get('preparation_error') or record['whole_source_after']['errors']
            or record.get('runtime_after', {}).get('errors') or not process_ok(record.get('process', {})))
        custody.save(output / 'preparation.json', record)
    if not record['prepared']:
        return 1
    # Validate the complete persisted record through the same acceptance path.
    receipt = output / 'preparation.json'
    load_inputs(receipt, custody.sha(receipt), custody, args.whole_lease,
                args.whole_lease_sha256, args.gate_sha256)
    custody.save(output / 'signed-inputs.json', dict(path=str(receipt), sha256=custody.sha(receipt)))
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gate', type=Path, required=True)
    parser.add_argument('--gate-sha256', required=True)
    parser.add_argument('--whole-lease', type=Path, required=True)
    parser.add_argument('--whole-lease-sha256', required=True)
    parser.add_argument('--runtime-profile', type=Path, required=True)
    parser.add_argument('--runtime-profile-sha256', required=True)
    parser.add_argument('--runtime-provenance', type=Path, required=True)
    parser.add_argument('--runtime-provenance-sha256', required=True)
    parser.add_argument('--signing-identity', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    for value in (args.gate_sha256, args.whole_lease_sha256, args.runtime_profile_sha256, args.runtime_provenance_sha256):
        if not re.fullmatch('[0-9a-f]{64}', value):
            parser.error('explicit lowercase SHA256 digests are required')
    sys.dont_write_bytecode = True
    custody = custody_module()
    custody.configure_candidate(args.gate,args.gate_sha256,args.whole_lease,args.whole_lease_sha256)
    custody.bound_source(Path(__file__).resolve())
    sys.path.insert(0, str(ROOT / 'scripts'))
    from generation_gate import GenerationLease
    admission = GenerationLease(custody.GATE, expected=args.gate_sha256)
    return prepare(args, custody, admission)


if __name__ == '__main__':
    raise SystemExit(main())

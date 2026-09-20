"""Separate source/build provenance for canonical schema6 runtime profiles.

Validation is read-only. The producer always runs the ordinary canonical profile
builder; Cargo may reuse or rebuild its inputs through its normal dependency graph.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
PATCHES = ('window-proxy-structural-semantics', 'shared-memory-growth',
           'shared-memory-wait', 'shared-wasm-size', 'shared-wasm-backing',
           'shared-memory-factory', 'frame-context-reattachment',
           'html-constructor-deferred-prototype')
ARTIFACTS = ('archive', 'binding', 'icu_data', 'startup_snapshot',
             'external_references', 'worker', 'bridge_abi', 'rusty_v8_source_manifest')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def record(path, expected=None):
    path = Path(os.path.abspath(path))
    destination = path.resolve(strict=True)
    before = path.stat()
    actual = digest(path)
    after = path.stat()
    require(destination == path.resolve(strict=True) and
            (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns),
            'input changed during capture: ' + str(path))
    require(expected is None or actual == expected, 'input digest mismatch: ' + str(path))
    return dict(path=str(path), resolved_path=str(destination), sha256=actual,
                size_bytes=after.st_size)


def check_records(records):
    seen = {}
    for item in records:
        current = record(item['path'], item['sha256'])
        require(current == item, 'input identity changed: ' + item['path'])
        prior = seen.setdefault(item['path'], item)
        require(prior == item, 'conflicting input assertion: ' + item['path'])
    return records


def merge_records(*groups):
    result = {}
    destinations = {}
    for group in groups:
        for item in group:
            previous = result.setdefault(item['path'], item)
            require(previous == item, 'conflicting input assertion: ' + item['path'])
            identity = (item['sha256'], item['size_bytes'])
            old = destinations.setdefault(item['resolved_path'], identity)
            require(old == identity, 'conflicting resolved input digest')
    return list(result.values())


def read_json(path, expected=None):
    original = record(path, expected)
    raw = Path(original['path']).read_bytes()
    require(hashlib.sha256(raw).hexdigest() == original['sha256'], 'parsed JSON bytes differ from asserted digest')
    value = json.loads(raw)
    check_records([original])
    return value, original


def save_new(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write('\n')


def bound(gate, item, root, *, review=False):
    relative = str(Path(item['path']).relative_to(root))
    rows = list(gate.get('review_sources', [])) if review else []
    for requirement in gate['requirements'].values():
        for field in ('implementation_sources', 'test_sources'):
            rows.extend(requirement[field])
    require(any(row['path'] == relative and row['sha256'] == item['sha256']
                for row in rows), 'input not admitted by exact declaration: ' + relative)


def candidate_number(root, gate_path):
    """Derive a label only from the explicitly supplied canonical declaration path."""
    root, path = Path(root).resolve(), Path(gate_path)
    require(path.is_absolute() and path == path.resolve() and
            path.parent == root / 'docs/oryn2', 'canonical generation declaration path required')
    match = re.fullmatch(r'GENERATION-COMPLETE-REPAIR-([1-9][0-9]*)-[0-9]{4}-[0-9]{2}-[0-9]{2}\.json', path.name)
    require(match is not None, 'canonical repair declaration filename required')
    return match.group(1)


def gate_inputs(root, gate_path, gate_sha, lease_path, lease_sha):
    root = Path(root).resolve()
    number = candidate_number(root, gate_path)
    gate, gate_record = read_json(gate_path, gate_sha)
    lease, lease_record = read_json(lease_path, lease_sha)
    require(lease['kind'] == f'repair{number}_whole_source_and_review_lease_inputs', 'wrong whole-source lease')
    bound(gate, lease_record, root, review=True)
    require(lease['expected_declaration_path'] == str(Path(gate_path).relative_to(root)),
            'whole lease names another declaration')
    code_records = []
    for name in ('runtime_provenance.py', 'prepare-runtime-provenance.py',
                 'build-v8-runtime-profile.sh', 'validate-v8-runtime-profile.py',
                 'rusty_v8_source_provenance.py', 'generation_gate.py'):
        item = record(root / 'scripts' / name)
        bound(gate, item, root)
        code_records.append(item)
    return gate, [gate_record, lease_record] + code_records


def load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def profile_inputs(profile_path, expected, root):
    profile, item = read_json(profile_path, expected)
    records = [item]
    for name in ('worker-runtime-info.json', 'rusty-v8-source-inputs.json'):
        records.append(record(Path(profile_path).parent / name))
    for name in PATCHES:
        records.extend(record(Path(root) / 'patches/v8' / (name + suffix)) for suffix in ('.patch', '.json'))
    regression, regression_record = read_json(Path(root) / 'patches/v8/tests/manifest.json')
    records.append(regression_record)
    for name, expected_source in regression['sources'].items():
        records.append(record(Path(root) / 'patches/v8/tests' / name, expected_source))
    for name, expected_metadata in regression.get('test_only_compatibility', {}).items():
        metadata_path = Path(root) / 'patches/v8/tests' / (name + '.json')
        metadata, metadata_record = read_json(metadata_path, expected_metadata)
        records.extend([metadata_record, record(metadata_path.with_suffix('.patch'), metadata['patch_sha256'])])
    for key in ARTIFACTS:
        captured = record(profile[key + '_path'], profile[key + '_sha256'])
        require(captured['size_bytes'] == profile[key + '_size_bytes'], 'profile artifact size mismatch')
        records.append(captured)
    bridge, bridge_record = read_json(profile['bridge_abi_path'], profile['bridge_abi_sha256'])
    records.append(bridge_record)
    for row in bridge['inputs']:
        captured = record(row['path'], row['sha256'])
        require(captured['size_bytes'] == row['size_bytes'], 'ABI input size mismatch')
        records.append(captured)
    for name in ('validate-v8-runtime-profile.py', 'rusty_v8_source_provenance.py'):
        records.append(record(Path(root) / 'scripts' / name))
    validator = load_module(Path(root) / 'scripts/validate-v8-runtime-profile.py', 'provenance_profile_validator')
    validator.validate(profile, Path(root), profile['profile'], 'aarch64-apple-darwin')
    check_records(records)
    return profile, merge_records(records)


def validate_runtime_provenance(path, expected_sha256, *, root, gate_path, gate_sha256,
                                lease_path, lease_sha256, profile_path, profile_sha256):
    receipt, receipt_record = read_json(path, expected_sha256)
    checked = validate_runtime_provenance_data(
        receipt, root=root, gate_path=gate_path, gate_sha256=gate_sha256,
        lease_path=lease_path, lease_sha256=lease_sha256,
        profile_path=profile_path, profile_sha256=profile_sha256)
    checked['records'] = merge_records([receipt_record], checked['records'])
    check_records(checked['records'])
    return checked


def validate_runtime_provenance_data(receipt, *, root, gate_path, gate_sha256,
                                     lease_path, lease_sha256, profile_path, profile_sha256):
    root = Path(root).resolve()
    gate, inputs = gate_inputs(root, gate_path, gate_sha256, lease_path, lease_sha256)
    require(receipt.get('schema_version') == 1 and receipt.get('kind') == 'oryn_runtime_provenance' and
            receipt.get('status') == 'complete', 'runtime provenance is not complete')
    require(receipt['generation_gate'] == inputs[0] and receipt['whole_source_lease'] == inputs[1],
            'runtime provenance generation mismatch')
    selected_profile = record(profile_path, profile_sha256)
    require(receipt['profile'] == selected_profile, 'runtime provenance profile mismatch')
    build, build_record = read_json(receipt['build_receipt']['path'], receipt['build_receipt']['sha256'])
    require(build_record == receipt['build_receipt'], 'build receipt identity mismatch')
    require(build['kind'] == 'oryn_owned_runtime_build' and build['schema_version'] == 1 and
            build['generation_gate'] == inputs[0] and build['whole_source_lease'] == inputs[1],
            'owned build generation mismatch')
    require(build['command'] == ['bash', str(root / 'scripts/build-v8-runtime-profile.sh'),
                                  build['profile_name'], build['output_directory']], 'unexpected build command')
    require(str(Path(profile_path)) == str(Path(build['output_directory']) / (build['profile_name'] + '-manifest.json')),
            'profile was not an output of this build')
    process = build['process']
    require(build['success'] is True and type(process['returncode']) is int and process['returncode'] == 0 and process['group_settled'] is True and
            not process['execution_error'] and not process['cleanup_errors'], 'owned build failed or unsettled')
    lease, _ = read_json(lease_path, lease_sha256)
    start, start_record = read_json(build['start_receipt']['path'], build['start_receipt']['sha256'])
    require(start_record == build['start_receipt'] and start['command'] == build['command'] and
            type(start['pid']) is int and start['pid'] > 0 and
            start['pid'] == start['group_id'] == process['pid'] and
            start['generation_gate'] == str(Path(gate_path)) and
            start['generation_gate_sha256'] == gate_sha256 and
            start['supervision'] == 'runtime-profile-build' and
            {'event': 'started', 'pid': start['pid'], 'group_id': start['group_id']} in process['events'],
            'owned start identity differs from build')
    require(lease['source_count'] == len(lease['sources']), 'whole lease source count differs')
    for check in (build['before'], build['after']):
        require(check['errors'] == [] and check['declaration_sha256'] == gate_sha256 and
                check['lease_sha256'] == lease_sha256 and check['source_count'] == lease['source_count'],
                'owned build source custody failed')
    for item in build['producer_sources']:
        bound(gate, item, root)
    require(build['custody_source'] in build['producer_sources'], 'owned custody source missing')
    environment = build['environment']
    require(environment['ORYN_GENERATION_GATE'] == str(Path(gate_path)) and
            environment['ORYN_GENERATION_GATE_SHA256'] == gate_sha256 and
            environment.get(f'ORYN_REPAIR{candidate_number(root, gate_path)}_SUPERVISED_GROUP') == 'runtime-profile-build',
            'build environment generation mismatch')
    retained_paths = {item['path'] for item in build['inputs']}
    for selected in (environment['GN'], environment['NINJA'],
                     str(Path(environment['CLANG_BASE_PATH']) / 'bin/clang'),
                     str(Path(environment['LIBCLANG_PATH']) / 'libclang.dylib'),
                     str(Path(environment['SDKROOT']) / 'SDKSettings.json')):
        require(selected in retained_paths, 'build tool/SDK input missing: ' + selected)
    require({str(root / 'scripts' / name) for name in
             ('prepare-runtime-provenance.py', 'runtime_provenance.py', 'build-v8-runtime-profile.sh')}
            .issubset({item['path'] for item in build['producer_sources']}), 'build producer custody missing')
    profile, runtime_inputs = profile_inputs(profile_path, profile_sha256, root)
    require(build['profile'] == selected_profile and build['outputs'] == runtime_inputs,
            'owned build output inventory differs')
    require(build['engine_reuse'] == receipt['engine_reuse'], 'reuse receipt differs from build')
    require(receipt['engine_reuse'] is None, 'custom engine reuse is unsupported; canonical build required')
    inputs = merge_records(inputs, [build_record], runtime_inputs,
                           build['producer_sources'], build['inputs'], [build['log'], build['start_receipt']])
    require(receipt['inputs'] == merge_records(runtime_inputs, build['producer_sources']),
            'runtime provenance output/producer inventory differs')
    check_records(inputs)
    return dict(receipt=receipt, profile=profile, records=inputs)


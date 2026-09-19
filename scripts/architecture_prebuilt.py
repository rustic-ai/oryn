"""Frozen architecture executables, reviewed platform scope and attestation joins.

Preparation is not qualification. Final execution uses only recorded executables;
macOS release acceptance additionally needs the independently validated original
security component. No function in this module builds or signs an executable.
"""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile

from completion_provenance import ROOT, digest, read_record

MANIFEST = ROOT / 'benchmarks/conformance/runtime-architecture-v1.json'
SHADOW_MANIFEST = ROOT / 'benchmarks/conformance/runtime-shadow-v1.json'
HISTORICAL = ROOT / 'benchmarks/evidence/runtime-shadow-v1.json'
HARNESS = 'crates/oryn-native/examples/runtime_shadow.rs'
LEGACY_COMMIT = '35ff762ac35716bb891bf980654d434eed5dfcf2'
RELEASE_PLATFORMS = {'macos-app-sandbox': 'macOS',
                     'linux-namespaces-landlock-seccomp': 'Linux'}
BUILD_COMMANDS = {
    'current': ['cargo', 'build', '--locked', '-p', 'oryn-native', '--example',
                'runtime_shadow', '--features', 'v8-host in-process-probe'],
    'legacy': ['cargo', 'build', '--locked', '-p', 'oryn-native', '--example',
               'runtime_shadow', '--features', 'v8-host in-process-probe'],
    'webidl': ['cargo', 'build', '--locked', '-p', 'oryn-webidl'],
}
TREE_COMMAND = ['cargo', 'tree', '--locked', '-p', 'oryn-native', '--prefix', 'none']
SOURCE_INPUTS = {
    'manifest': 'benchmarks/conformance/runtime-architecture-v1.json',
    'shadow_manifest': 'benchmarks/conformance/runtime-shadow-v1.json',
    'historical': 'benchmarks/evidence/runtime-shadow-v1.json',
    'harness': HARNESS,
    'lock': 'Cargo.lock', 'workspace': 'Cargo.toml',
    'idl': 'crates/oryn-webidl/idl/browser.webidl',
    'generated': 'crates/oryn-js/src/generated/browser.rs',
}
PRODUCER_SOURCES = ('architecture_prebuilt.py', 'prepare-architecture-prebuilt.py',
                    'validate_runtime_architecture.py')
ENGINE_FIELDS = ('archive', 'binding', 'bridge_abi', 'icu_data', 'startup_snapshot',
                 'external_references', 'worker', 'rusty_v8_source_manifest')
LEGACY_MARKERS = {'sync_dom': 'full-DOM synchronization',
                  '__oryn_export': 'JavaScript DOM export',
                  '__oryn_fetch_sync': 'synchronous Fetch bridge',
                  '_frame_documents': 'isolate-per-frame document path'}


def record(path):
    path = Path(path).resolve()
    return {'path': str(path), 'sha256': digest(path)}


def object_file(path):
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict): raise ValueError('architecture input must be an object')
    return value


def source_hash():
    return subprocess.check_output([str(ROOT / 'scripts/source-tree-hash.sh')],
                                   cwd=ROOT, text=True).strip()


def exact_record(value):
    if not isinstance(value, dict) or set(value) != {'path', 'sha256'}:
        raise ValueError('architecture input needs an exact path/SHA-256 record')
    path = read_record(value)
    if path.is_symlink() or str(path.resolve()) != value['path']:
        raise ValueError('architecture input path must be canonical and not a symlink')
    return path


def engine_inputs(path):
    """Validate explicit engine selection; never select an engine from the host."""
    path = Path(path).resolve()
    before = record(path)
    spec = importlib.util.spec_from_file_location('architecture_engine_profile',
                                                 ROOT / 'scripts/validate-v8-runtime-profile.py')
    validator = importlib.util.module_from_spec(spec); spec.loader.exec_module(validator)
    value = validator.validate(object_file(path), ROOT, profile='core-full')
    result = {'profile': before}
    for name in ENGINE_FIELDS:
        item = {'path': value[name + '_path'], 'sha256': value[name + '_sha256']}
        exact_record(item); result[name] = item
    for index, item in enumerate(object_file(exact_record(result['bridge_abi']))['inputs']):
        bound = {'path': item['path'], 'sha256': item['sha256']}
        exact_record(bound); result['bridge_input_' + str(index)] = bound
    if record(path) != before: raise ValueError('architecture engine profile changed')
    return result


def engine_environment(inputs, environment=None):
    env = dict(os.environ if environment is None else environment)
    # Both builds and final executions explicitly select their own recorded engine.
    for name in ('CARGO_TARGET_DIR', 'V8_FROM_SOURCE', 'ORYN_ICU_DATA_PATH',
                 'ORYN_LITE_ICU_DATA', 'ORYN_V8_PROFILE_MANIFEST',
                 'ORYN_SECURITY_PREBUILT_CANDIDATE', 'ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256'):
        env.pop(name, None)
    fields = {'RUSTY_V8_ARCHIVE': 'archive', 'RUSTY_V8_SRC_BINDING_PATH': 'binding',
              'ORYN_V8_BRIDGE_ABI': 'bridge_abi', 'ORYN_STARTUP_SNAPSHOT': 'startup_snapshot',
              'ORYN_ICU_DATA_FINGERPRINT_PATH': 'icu_data'}
    for name, key in fields.items(): env[name] = str(exact_record(inputs[key]))
    return env


def preparation_artifacts(value):
    result = {}
    def add(name, item):
        exact_record(item); result['architecture_' + name] = item
    for name, item in value['sources'].items(): add('source_' + name, item)
    for name, item in value['producer_sources'].items(): add('producer_' + name, item)
    for role, inputs in value['engines'].items():
        for name, item in inputs.items(): add(role + '_engine_' + name, item)
    for index, name in enumerate(sorted(value['legacy_sources'])):
        add('legacy_source_' + str(index), value['legacy_sources'][name])
    for name, build in value['builds'].items():
        add(name, build['executable']); add(name + '_build_log', build['log'])
    add('legacy_archive', value['legacy_archive'])
    add('generation_gate', value['generation_gate'])
    add('dependency_graph', value['dependency_graph']['output'])
    return result


def archive_sources(path):
    """Read the exact pinned Git archive without extracting or rebuilding it."""
    result, seen = {}, set()
    with tarfile.open(path, 'r:') as archive:
        if archive.pax_headers.get('comment') != LEGACY_COMMIT:
            raise ValueError('legacy archive does not identify the pinned commit')
        for member in archive.getmembers():
            name = PurePosixPath(member.name)
            if (name.is_absolute() or '..' in name.parts or str(name) != member.name.rstrip('/')
                    or member.name in seen or not (member.isfile() or member.isdir())):
                raise ValueError('legacy archive contains an unsafe or duplicate source entry')
            seen.add(member.name)
            if member.isfile():
                with archive.extractfile(member) as stream:
                    result[member.name] = hashlib.file_digest(stream, 'sha256').hexdigest()
    if not {'Cargo.toml', 'Cargo.lock'} <= result.keys():
        raise ValueError('legacy archive lacks its workspace and locked dependency inputs')
    return result


def validate_preparation(candidate):
    """Read-only candidate-load hook; deliberately does not load_candidate again."""
    records = candidate['record']['artifacts']
    item = records.get('architecture_prebuilt')
    if item is None: raise ValueError('candidate lacks frozen architecture preparation')
    path = exact_record(item); value = object_file(path)
    required = {'schema_version', 'source_tree_sha256', 'generation_gate', 'sources',
                'producer_sources', 'engines', 'builds', 'legacy_commit', 'legacy_archive',
                'legacy_source_root', 'legacy_sources', 'dependency_graph'}
    if (set(value) != required or type(value['schema_version']) is not int or value['schema_version'] != 1
            or value['source_tree_sha256'] != candidate['record']['source_tree_sha256']
            or value['legacy_commit'] != LEGACY_COMMIT):
        raise ValueError('architecture preparation source/schema/baseline mismatch')
    exact_record(value['generation_gate'])
    legacy_root = Path(value['legacy_source_root'])
    if not legacy_root.is_absolute() or str(legacy_root.resolve()) != str(legacy_root):
        raise ValueError('legacy source root must be canonical')
    if not {'Cargo.toml', 'Cargo.lock', HARNESS} <= value['legacy_sources'].keys():
        raise ValueError('legacy source inventory incomplete')
    for name, item in value['legacy_sources'].items():
        relative = PurePosixPath(name)
        if (relative.is_absolute() or '..' in relative.parts
                or exact_record(item) != legacy_root / name):
            raise ValueError('legacy source input escapes its recorded tree')
    if {p.relative_to(legacy_root).as_posix() for p in legacy_root.rglob('*') if p.is_file()} != set(value['legacy_sources']):
        raise ValueError('legacy source inventory membership changed')
    if value['legacy_sources'][HARNESS]['sha256'] != value['sources']['harness']['sha256']:
        raise ValueError('legacy fixture harness differs from current source')
    archived = archive_sources(exact_record(value['legacy_archive']))
    if set(value['legacy_sources']) != set(archived) | {HARNESS}:
        raise ValueError('legacy source membership differs from pinned archive plus harness overlay')
    for name, expected in archived.items():
        if name != HARNESS and value['legacy_sources'][name]['sha256'] != expected:
            raise ValueError('legacy source differs from pinned archive: ' + name)
    if set(value['sources']) != set(SOURCE_INPUTS): raise ValueError('architecture source inventory incomplete')
    for name, relative in SOURCE_INPUTS.items():
        if exact_record(value['sources'][name]) != (ROOT / relative).resolve():
            raise ValueError('architecture source path substituted: ' + name)
    if set(value['producer_sources']) != set(PRODUCER_SOURCES):
        raise ValueError('architecture producer inventory incomplete')
    for name, item in value['producer_sources'].items():
        if exact_record(item) != (ROOT / 'scripts' / name).resolve():
            raise ValueError('architecture producer source substituted')
    if set(value['engines']) != {'current', 'legacy'}: raise ValueError('architecture engine roles incomplete')
    for role, inputs in value['engines'].items():
        if engine_inputs(exact_record(inputs['profile'])) != inputs:
            raise ValueError('architecture engine inputs differ: ' + role)
    if value['engines']['current']['profile'] != records['engine_profile']:
        raise ValueError('current architecture engine differs from candidate')
    if set(value['builds']) != set(BUILD_COMMANDS): raise ValueError('architecture executable inventory incomplete')
    executables = []
    for name, build in value['builds'].items():
        if (set(build) != {'argv', 'returncode', 'executable', 'log'}
                or build['argv'] != BUILD_COMMANDS[name]
                or type(build['returncode']) is not int or build['returncode'] != 0):
            raise ValueError('architecture build custody invalid: ' + name)
        executable = exact_record(build['executable']); executables.append(executable)
        if not os.access(executable, os.X_OK): raise ValueError('architecture input is not executable')
        exact_record(build['log'])
    if len(set(executables)) != 3: raise ValueError('architecture executables share an output path')
    graph = value['dependency_graph']
    if (set(graph) != {'argv', 'returncode', 'output'} or graph['argv'] != TREE_COMMAND
            or type(graph['returncode']) is not int or graph['returncode'] != 0):
        raise ValueError('architecture dependency graph custody invalid')
    tree = exact_record(graph['output']).read_text()
    if not all(package in tree for package in ('oryn-dom', 'oryn-js', 'oryn-net')):
        raise ValueError('native orchestrator dependency graph incomplete')
    historical = object_file(exact_record(value['sources']['historical']))
    if historical.get('legacy_commit') != LEGACY_COMMIT or historical.get('harness') != HARNESS:
        raise ValueError('architecture historical baseline changed')
    expected = preparation_artifacts(value)
    actual_names = {name for name in records if name.startswith('architecture_')}
    if actual_names != set(expected) | {'architecture_prebuilt'}:
        raise ValueError('architecture candidate input membership differs')
    for name, input_record in expected.items():
        if records.get(name) != input_record:
            raise ValueError('architecture input differs from candidate: ' + name)
        exact_record(input_record)
    exact_record(records['architecture_prebuilt'])
    return value


def scope(candidate):
    from core_profile_contract import validate_registry
    registry = validate_registry(candidate, required=True)
    declared = registry['value']
    platforms = sorted({declared['profiles'][name]['platform']
                        for name in declared['required_profile_ids']})
    if not platforms or not set(platforms) <= set(RELEASE_PLATFORMS.values()):
        raise ValueError('architecture has no attestation route for a declared platform')
    return {'registry_sha256': registry['sha256'], 'platforms': platforms,
            'required_profile_ids': declared['required_profile_ids'],
            'architecture_profile_id': declared['non_core_claims']['architecture']}


def manifest_inputs():
    value = object_file(MANIFEST); shadow = object_file(SHADOW_MANIFEST)
    cases = value.get('cases', []); ids = [case.get('id') for case in cases]
    if (value.get('schema_version') != 1 or value.get('frozen_before_validation') is not True
            or value.get('shadow_side_effect_policy') != 'fixtures_and_recorded_network_only'
            or len(ids) != 32 or len(set(ids)) != 32
            or any(case.get('required') is not True for case in cases)
            or {case['id'] for case in cases if case.get('gate') == 'release'} != set(RELEASE_PLATFORMS)):
        raise ValueError('architecture manifest/denominator changed')
    fixtures = [item.get('id') for item in shadow.get('fixtures', [])]
    if (shadow.get('schema_version') != 1 or shadow.get('side_effect_policy') != 'repository_fixtures_only'
            or len(fixtures) != 4 or len(set(fixtures)) != 4):
        raise ValueError('architecture shadow fixture population changed')
    return value, fixtures


def check_shadow(rows, fixtures):
    if (not isinstance(rows, list) or [item.get('id') for item in rows if isinstance(item, dict)] != fixtures
            or any(not isinstance(item, dict) or set(item) != {'id', 'passed'}
                   or item['passed'] is not True for item in rows)):
        raise ValueError('architecture shadow failed or fixture ledger differs')


def software_result(candidate, shadow):
    manifest, fixtures = manifest_inputs(); declared = scope(candidate)
    for rows in shadow.values(): check_shadow(rows, fixtures)
    if set(shadow) != {'current', 'legacy'}: raise ValueError('both architecture shadow roles required')
    source = '\n'.join((ROOT / path).read_text() for path in (
        'crates/oryn-native/src/rust_executable.rs', 'crates/oryn-native/src/host.rs',
        'crates/oryn-js/src/v8_bindings.rs'))
    remaining = [label for marker, label in LEGACY_MARKERS.items() if marker in source]
    ledger, pending, releases, outside = [], [], [], []
    for case in manifest['cases']:
        row = {'id': case['id'], 'manifest_status': case.get('status'), 'required': True}
        if case.get('gate', 'software') == 'software':
            row.update(applicability='required', status=case.get('status'))
            if case.get('status') != 'passed': pending.append(case['id'])
        else:
            platform = RELEASE_PLATFORMS[case['id']]
            required = platform in declared['platforms']
            row.update(platform=platform, applicability='required' if required else 'outside_declared_scope',
                       status='pending_attestation' if required else 'outside_declared_scope')
            (releases if required else outside).append(case['id'])
        ledger.append(row)
    return {'schema_version': 2, 'candidate_manifest_sha256': candidate['sha256'],
            'source_tree_sha256': candidate['record']['source_tree_sha256'],
            'manifest': record(MANIFEST), 'shadow_manifest': record(SHADOW_MANIFEST),
            'preparation': candidate['record']['artifacts']['architecture_prebuilt'],
            'scope': declared, 'cases': len(ledger), 'case_ledger': ledger,
            'shadow': shadow, 'pending': pending, 'pending_release_attestations': releases,
            'outside_declared_scope': outside, 'legacy_production_markers': remaining,
            'cutover_ready': not pending and not remaining}


def execute(candidate, output, lease):
    preparation = validate_preparation(candidate)
    scope(candidate); manifest_inputs()
    if preparation['generation_gate'] != {'path': str(lease.path), 'sha256': lease.sha256}:
        raise ValueError('architecture preparation belongs to another generation declaration')
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('architecture source differs from candidate')
    output = Path(output).resolve()
    protected = [exact_record(item) for item in candidate['record']['artifacts'].values()]
    if output == ROOT or any(output == p or output in p.parents for p in protected):
        raise ValueError('architecture output overlaps source or frozen inputs')
    lease.check(); output.mkdir(parents=True, exist_ok=False)
    shadow = {}
    for name in ('current', 'legacy', 'webidl'):
        validate_preparation(candidate); lease.check()
        executable = exact_record(preparation['builds'][name]['executable'])
        argv = [str(executable)]
        if name == 'webidl': argv += ['check', str(ROOT / SOURCE_INPUTS['idl']), str(ROOT / SOURCE_INPUTS['generated'])]
        inputs = preparation['engines']['legacy' if name == 'legacy' else 'current']
        env = lease.environment(engine_environment(inputs))
        execution = {'argv': argv, 'executable': preparation['builds'][name]['executable'],
                     'engine_profile': inputs['profile'], 'returncode': None, 'error': None}
        try:
            with (output / (name + '.stdout')).open('xb') as stdout, (output / (name + '.stderr')).open('xb') as stderr:
                completed = subprocess.run(argv, cwd=ROOT, env=env, stdout=stdout, stderr=stderr, timeout=300)
                execution['returncode'] = completed.returncode
        except Exception as error:
            execution['error'] = str(error)
            raise
        finally:
            with (output / (name + '.execution.json')).open('x') as stream:
                json.dump(execution, stream, indent=2, sort_keys=True)
        lease.check(); validate_preparation(candidate)
        if completed.returncode != 0: raise ValueError('architecture executable failed: ' + name)
        if name != 'webidl': shadow[name] = json.loads((output / (name + '.stdout')).read_text())
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('architecture source changed during execution')
    result = software_result(candidate, shadow)
    with (output / 'result.json').open('x') as stream: json.dump(result, stream, indent=2, sort_keys=True)
    lease.check(); validate_preparation(candidate)
    if not result['cutover_ready']: raise ValueError('architecture software cutover remains open')
    return result


def qualify_architecture(receipt, candidate, security_result, security_receipt):
    """Call after the full raw security validator and candidate fingerprint checks.

    Recheck original receipts (including live signature verification), match the
    already validated security bytes, and derive release applicability afresh.
    """
    from completion_execution import verify_receipt
    validate_preparation(candidate)
    verify_receipt(receipt, 'architecture', candidate)
    value = object_file(Path(receipt).parent / 'raw/result.json')
    expected = software_result(candidate, value.get('shadow', {}))
    if value != expected or expected['cutover_ready'] is not True:
        raise ValueError('architecture raw result or software cutover differs')
    # Raw role outputs must independently agree with the aggregate.
    for name in ('current', 'legacy'):
        rows = json.loads((Path(receipt).parent / ('raw/' + name + '.stdout')).read_text())
        if rows != expected['shadow'][name]: raise ValueError('architecture raw shadow result differs')
    prepared = validate_preparation(candidate)
    for name in ('current', 'legacy', 'webidl'):
        execution = object_file(Path(receipt).parent / ('raw/' + name + '.execution.json'))
        argv = [prepared['builds'][name]['executable']['path']]
        if name == 'webidl': argv += ['check', str(ROOT / SOURCE_INPUTS['idl']), str(ROOT / SOURCE_INPUTS['generated'])]
        profile = prepared['engines']['legacy' if name == 'legacy' else 'current']['profile']
        if (execution != {'argv': argv, 'executable': prepared['builds'][name]['executable'],
                          'engine_profile': profile, 'returncode': 0, 'error': None}
                or type(execution.get('returncode')) is not int):
            raise ValueError('architecture raw executable custody differs: ' + name)
    if security_result is None or security_receipt is None:
        raise ValueError('architecture release attestations require qualified security evidence')
    verify_receipt(security_receipt, 'security', candidate)
    security_path = Path(security_receipt).parent / 'raw/security-result-v3.json'
    if object_file(security_path) != security_result:
        raise ValueError('architecture security result differs from validated original bytes')
    fingerprint = security_result.get('fingerprint', {})
    if (fingerprint.get('source_tree_sha256') != candidate['record']['source_tree_sha256']
            or fingerprint.get('worker', {}).get('sha256') != candidate['record']['artifacts']['worker']['sha256']
            or fingerprint.get('parent', {}).get('sha256') != candidate['record']['artifacts']['runtime']['sha256']
            or security_result.get('outcome', {}).get('status') != 'passed'):
        raise ValueError('architecture macOS security attestation candidate mismatch or failure')
    result = copy.deepcopy(expected)
    if 'macos-app-sandbox' in result['pending_release_attestations']:
        result['pending_release_attestations'].remove('macos-app-sandbox')
        for row in result['case_ledger']:
            if row['id'] == 'macos-app-sandbox':
                row['status'] = 'passed'
                row['attestation'] = {'receipt': record(security_receipt), 'result': record(security_path)}
    if result['pending_release_attestations']:
        raise ValueError('architecture required release attestations remain open: ' + ', '.join(result['pending_release_attestations']))
    result['release_ready'] = True
    return result

"""Bound execution tooling; explicit candidate, whole lease and fresh outputs are required."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT / 'scripts') not in sys.path:
    sys.path.insert(0, str(ROOT / 'scripts'))
from completion_process_supervisor import Cancellation, supervise
BASE = ROOT / 'artifacts/completion-validation-2026-09-18'
GATE = ROOT / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-49-2026-09-18.json'
PREVIOUS_LEASE = BASE / 'repair48-whole-source-lease-inputs-01.json'
PREVIOUS_LEASE_SHA = 'cc4c2e3970cf376e6d07be4ad7a8a2cad015698cc54a8967ed7eb128ced83bc1'
HISTORICAL_LEASE = PREVIOUS_LEASE
HISTORICAL_LEASE_SHA = PREVIOUS_LEASE_SHA
CANDIDATE_NUMBER = '49'
SUPERVISOR_KEY = 'ORYN_REPAIR49_SUPERVISED_GROUP'
CANDIDATE_BINDING = None
ENVIRONMENT = BASE / 'repair40-python09-environment.json'
ENVIRONMENT_SHA = '5081d2bc2cbb85974794de5ca593c7fd1a089cb7c6a24eac73fab013f06754cb'
ENVIRONMENT_CHECKER = BASE / 'check-python11-environment.py'
ENVIRONMENT_CHECKER_SHA = 'abb2e1fb93ded5a47025b062b015278ccbd398c5c3ba5568654c3d762e9822c6'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def read_bound_json(path, expected):
    path = Path(path).absolute()
    destination = path.resolve(strict=True)
    before = path.stat()
    data = path.read_bytes()
    after = path.stat()
    if (not re.fullmatch('[0-9a-f]{64}', expected) or hashlib.sha256(data).hexdigest() != expected
            or path.resolve(strict=True) != destination
            or (before.st_size, before.st_mtime_ns, before.st_ino)
            != (after.st_size, after.st_mtime_ns, after.st_ino)):
        raise ValueError('candidate input changed during capture: ' + str(path))
    return json.loads(data)


def candidate_number(root, gate_path):
    path = Path(gate_path)
    if not path.is_absolute() or path != path.resolve(strict=True):
        raise ValueError('candidate declaration path must be absolute and canonical')
    match = re.fullmatch(r'GENERATION-COMPLETE-REPAIR-([1-9][0-9]*)-([0-9]{4}-[0-9]{2}-[0-9]{2})\.json', path.name)
    if path.parent != Path(root).resolve() / 'docs/oryn2' or match is None:
        raise ValueError('candidate requires a canonical declaration path')
    return match.group(1)


def configure_candidate(gate_path, gate_sha256, lease_path, lease_sha256):
    """One explicit binding per imported helper; repeating that binding is safe."""
    global GATE, PREVIOUS_LEASE, PREVIOUS_LEASE_SHA, CANDIDATE_NUMBER, SUPERVISOR_KEY, CANDIDATE_BINDING
    number = candidate_number(ROOT, gate_path)
    gate = Path(gate_path)
    lease = Path(lease_path).resolve(strict=True)
    if int(number) < 49 or not lease.is_relative_to(BASE.resolve()):
        raise ValueError('candidate predates the retained floor or lease escapes completion artifacts')
    binding = (str(gate), gate_sha256, str(lease), lease_sha256)
    if CANDIDATE_BINDING is not None and CANDIDATE_BINDING != binding:
        raise ValueError('execution helper is already bound to a different candidate')
    declaration = read_bound_json(gate, gate_sha256)
    data = read_bound_json(lease, lease_sha256)
    if (data.get('kind') != f'repair{number}_whole_source_and_review_lease_inputs'
            or data.get('expected_declaration_path') != str(gate.relative_to(ROOT))):
        raise ValueError('declaration identity and whole-lease metadata disagree')
    reviews = {(row['path'], row['sha256']) for row in declaration['review_sources']}
    if (str(lease.relative_to(ROOT)), lease_sha256) not in reviews:
        raise ValueError('selected whole lease is not bound by the selected declaration')
    baseline = data['baseline']
    previous_path = ROOT / baseline['path']
    if (Path(baseline['path']).is_absolute() or '..' in Path(baseline['path']).parts
            or not previous_path.resolve(strict=True).is_relative_to(BASE.resolve())
            or (baseline['path'], baseline['sha256']) not in reviews):
        raise ValueError('predecessor lease is missing from exact declaration reviews')
    previous = read_bound_json(previous_path, baseline['sha256'])
    parent = declaration['previous_declaration']
    if ((parent['path'],parent['sha256']) not in reviews
            or declaration.get('parent_declaration',parent) != parent):
        raise ValueError('predecessor declaration metadata or review binding disagrees')
    parent_path = ROOT / parent['path']
    if (Path(parent['path']).is_absolute() or '..' in Path(parent['path']).parts
            or int(candidate_number(ROOT, parent_path)) != int(number) - 1
            or previous.get('kind') != f'repair{int(number)-1}_whole_source_and_review_lease_inputs'
            or previous.get('expected_declaration_path') != parent['path']):
        raise ValueError('candidate predecessor declaration and lease disagree')
    parent_data = read_bound_json(parent_path, parent['sha256'])
    if (baseline['path'], baseline['sha256']) not in {(row['path'],row['sha256']) for row in parent_data['review_sources']}:
        raise ValueError('predecessor declaration did not bind predecessor lease')
    GATE, PREVIOUS_LEASE, PREVIOUS_LEASE_SHA = gate, previous_path, baseline['sha256']
    CANDIDATE_NUMBER, SUPERVISOR_KEY = number, f'ORYN_REPAIR{number}_SUPERVISED_GROUP'
    CANDIDATE_BINDING = binding
    return dict(number=number, supervisor_key=SUPERVISOR_KEY, gate=str(gate), gate_sha256=gate_sha256,
                whole_lease=str(lease), whole_lease_sha256=lease_sha256)


def fresh_output_directory(path):
    path = Path(path).absolute()
    if not path.resolve().is_relative_to(BASE.resolve()) or path.exists() or path.is_symlink():
        raise ValueError('execution requires a fresh completion output directory')
    path.mkdir(parents=True, exist_ok=False)
    return path


def included(path):
    # Same authored discovery population as repair47; do not shrink exclusions.
    p = Path(path)
    return (p.parts[0] in ('crates', 'vendor', 'scripts', 'tools', 'docs')
            and not any(x in ('target', 'node_modules', '__pycache__', '.git') for x in p.parts[:-1])
            and not p.name.endswith(('.pyc', '.pyo', '.orig', '.rej', '.bak'))
            and not p.name.startswith('GENERATION-COMPLETE'))


def retained_review_identities(data, previous, declaration_reviews):
    """Keep prior evidence exact; permit only allowlisted, reviewed archive transitions."""
    prior = {(row['path'], row['sha256']) for row in previous['review_sources']}
    current = {(row['path'], row['sha256']) for row in data['review_sources']}
    sources = {(row['path'], row['sha256']) for row in data['sources']}
    allowed = {
        'docs/oryn2/BUILD-AND-QUALIFICATION-PROGRESS-2026-09-18.md',
        'docs/oryn2/CORE-AND-OIL-GENERATION-CHECKLIST-2026-09-15.md',
        # This gate is both live admission code and retained review evidence.
        # A reviewed update still binds its current source digest, while its
        # predecessor bytes remain exact in the canonical artifact archive.
        # Do not generalize this to implementation paths or a directory prefix.
        'scripts/generation_gate.py',
    }
    replaced, seen = set(), set()
    for row in data.get('review_identity_transitions', []):
        if set(row) != {'path', 'before_sha256', 'after_sha256', 'archived_before', 'review'}:
            raise ValueError('invalid status review transition fields')
        name = row['path']
        old, new = (name, row['before_sha256']), (name, row['after_sha256'])
        if (name not in allowed or name in seen or old not in prior or old == new
                or new not in current or new not in declaration_reviews or new not in sources):
            raise ValueError('status review transition is not an explicit current source replacement')
        seen.add(name)
        archive, review = row['archived_before'], row['review']
        for ref in (archive, review):
            if set(ref) != {'path', 'sha256'}:
                raise ValueError('invalid status transition evidence reference')
            relative = Path(ref['path'])
            target = ROOT / relative
            if (relative.is_absolute() or '..' in relative.parts
                    or not target.resolve(strict=True).is_relative_to(BASE.resolve())
                    or target != target.resolve(strict=True)
                    or (ref['path'], ref['sha256']) not in current
                    or (ref['path'], ref['sha256']) not in declaration_reviews):
                raise ValueError('status transition evidence must be bound canonical artifacts')
        if archive['sha256'] != row['before_sha256']:
            raise ValueError('status archive does not preserve predecessor identity')
        archived = ROOT / archive['path']
        before = archived.stat()
        digest = sha(archived)
        after = archived.stat()
        if (digest != archive['sha256'] or (before.st_size, before.st_mtime_ns, before.st_ino)
                != (after.st_size, after.st_mtime_ns, after.st_ino)):
            raise ValueError('status archive bytes changed')
        evidence = read_bound_json(ROOT / review['path'], review['sha256'])
        core = {key: value for key, value in row.items() if key != 'review'}
        if (evidence.get('approved') is not True
                or core not in evidence.get('review_identity_transitions', [])):
            raise ValueError('status identity transition lacks exact explicit review')
        replaced.add(old)
    if not (prior - replaced).issubset(declaration_reviews):
        raise ValueError('whole custody dropped historical review inputs')


def retained_floor_review_identities(data, previous, historical, declaration_reviews):
    """A floor identity may move only through an exact approved archive transition.

    Later successors need not repeat a transition: its hash-bound predecessor
    lease records it. Both original archive and approval remain current inputs.
    """
    missing = {(row['path'], row['sha256']) for row in historical['review_sources']} - declaration_reviews
    cursor, parent = data, previous
    visited = set()
    while missing:
        local_reviews = {(row['path'], row['sha256']) for row in cursor['review_sources']}
        retained_review_identities(cursor, parent, local_reviews)
        for row in cursor.get('review_identity_transitions', []):
            identity = (row['path'], row['before_sha256'])
            if identity not in missing:
                continue
            evidence = {(row[key]['path'], row[key]['sha256']) for key in ('archived_before', 'review')}
            if not evidence.issubset(declaration_reviews):
                raise ValueError('historical floor archive and approval are not retained')
            missing.remove(identity)
        if not missing:
            return
        cursor = parent
        if cursor.get('kind') == historical.get('kind'):
            if cursor != historical:
                raise ValueError('historical floor ancestry differs from pinned floor')
            break
        ref = cursor.get('baseline')
        if not isinstance(ref, dict) or set(ref) != {'path', 'sha256'}:
            raise ValueError('historical floor transition ancestry is incomplete')
        relative = Path(ref['path'])
        target = ROOT / relative
        if (relative.is_absolute() or '..' in relative.parts or target != target.resolve(strict=True)
                or not target.resolve(strict=True).is_relative_to(BASE.resolve())
                or (ref['path'], ref['sha256']) in visited
                or (ref['path'], ref['sha256']) not in {(r['path'], r['sha256']) for r in cursor['review_sources']}):
            raise ValueError('historical floor transition ancestry is not bound and canonical')
        visited.add((ref['path'], ref['sha256']))
        parent = read_bound_json(target, ref['sha256'])
        current_number = re.fullmatch(r'repair([1-9][0-9]*)_whole_source_and_review_lease_inputs', cursor.get('kind', ''))
        parent_number = re.fullmatch(r'repair([1-9][0-9]*)_whole_source_and_review_lease_inputs', parent.get('kind', ''))
        if (current_number is None or parent_number is None
                or int(current_number.group(1)) != int(parent_number.group(1)) + 1):
            raise ValueError('historical floor transition ancestry skips a predecessor')
    if missing:
        raise ValueError('whole custody dropped historical floor review inputs')


def check(lease_path, lease_sha, gate_sha):
    errors, count = [], None
    try:
        if CANDIDATE_BINDING != (str(GATE), gate_sha, str(Path(lease_path).resolve()), lease_sha):
            raise ValueError('whole-source check requires the exact explicit candidate binding')
        if sha(HISTORICAL_LEASE) != HISTORICAL_LEASE_SHA:
            raise ValueError('retained historical floor changed')
        if sha(PREVIOUS_LEASE) != PREVIOUS_LEASE_SHA:
            raise ValueError('historical whole-source lease changed')
        if sha(lease_path) != lease_sha or sha(GATE) != gate_sha:
            raise ValueError('whole lease or declaration changed')
        data = read_bound_json(lease_path, lease_sha)
        previous = read_bound_json(PREVIOUS_LEASE, PREVIOUS_LEASE_SHA)
        historical = read_bound_json(HISTORICAL_LEASE, HISTORICAL_LEASE_SHA)
        declaration = read_bound_json(GATE, gate_sha)
        if data.get('kind') != f'repair{CANDIDATE_NUMBER}_whole_source_and_review_lease_inputs':
            raise ValueError('whole lease is not the selected candidate source/review contract')
        declaration_reviews = {(row['path'], row['sha256']) for row in declaration['review_sources']}
        if (str(lease_path.relative_to(ROOT)), lease_sha) not in declaration_reviews:
            raise ValueError('actual declaration does not bind this whole lease')
        if not {(row['path'], row['sha256']) for row in data['review_sources']}.issubset(declaration_reviews):
            raise ValueError('whole lease review inventory is not fully declaration-bound')
        retained_review_identities(data, previous, declaration_reviews)
        retained_floor_review_identities(data, previous, historical, declaration_reviews)
        rows = data['sources']
        count = len(rows)
        paths = [row['path'] for row in rows]
        if len(set(paths)) != count or data['source_count'] != count:
            raise ValueError('inconsistent whole-source population')
        if not {row['path'] for row in historical['sources']}.issubset(paths):
            raise ValueError('whole lease dropped historical floor source paths')
        if not {row['path'] for row in previous['sources']}.issubset(paths):
            raise ValueError('whole lease dropped historical source paths')
        if data['authored_discovery_roots'] != previous['authored_discovery_roots']:
            raise ValueError('authored discovery root denominator changed')
        if data['expected_declaration_path'] != str(GATE.relative_to(ROOT)):
            raise ValueError('whole lease names a different generation')
        for row in rows + declaration['review_sources']:
            p = ROOT / row['path']
            if (Path(row['path']).is_absolute() or '..' in Path(row['path']).parts
                    or not p.resolve().is_relative_to(ROOT.resolve())):
                raise ValueError('source path escapes repository')
            try:
                before = p.stat()
                actual = sha(p)
                after = p.stat()
                if actual != row['sha256'] or (before.st_size, before.st_mtime_ns, before.st_ino) != (after.st_size, after.st_mtime_ns, after.st_ino):
                    errors.append(row['path'])
            except OSError as error:
                errors.append(str(error))
        expected = {path for path in paths if included(path)}
        found = set()
        for top in data['authored_discovery_roots']:
            for folder, dirs, files in os.walk(ROOT / top):
                dirs[:] = [x for x in dirs if x not in ('target', 'node_modules', '__pycache__', '.git')]
                for name in files:
                    path = str((Path(folder) / name).relative_to(ROOT))
                    if included(path):
                        found.add(path)
        errors.extend('added:' + path for path in sorted(found - expected))
        errors.extend('missing:' + path for path in sorted(expected - found))
        if sha(lease_path) != lease_sha or sha(GATE) != gate_sha:
            raise ValueError('whole lease/declaration changed during check')
    except Exception as error:
        errors.append(repr(error))
    return dict(errors=errors, source_count=count, declaration_sha256=gate_sha,
                lease_sha256=lease_sha, checked_at=time.time())


def bound_harness(launch, python):
    declaration = json.loads(GATE.read_bytes())
    bound = {}
    for requirement in declaration['requirements'].values():
        for field in ('implementation_sources', 'test_sources'):
            for row in requirement[field]:
                if row['path'] in bound and bound[row['path']] != row['sha256']:
                    raise ValueError('conflicting source binding: ' + row['path'])
                bound[row['path']] = row['sha256']
    required = [Path(__file__), launch]
    if python:
        required.extend((launch.parent / 'runner.py', ENVIRONMENT_CHECKER))
    for path in required:
        if bound.get(str(path.relative_to(ROOT))) != sha(path):
            raise ValueError('execution helper is not bound: ' + str(path))


# Runtime outputs are produced only after source freeze. Root supplies their exact
# reviewed profile digest at launch; they must never be refreshed from live bytes.
RUNTIME_VALIDATOR = ROOT / 'scripts/validate-v8-runtime-profile.py'
OLD_ARCHIVE_SHA = '7e4604e068025681bd0788e1720a948576363f581ac55c7d46d11e7b14de00ef'


def bound_source(path):
    values = {row['sha256'] for item in json.loads(GATE.read_bytes())['requirements'].values()
              for field in ('implementation_sources', 'test_sources')
              for row in item[field] if row['path'] == str(path.relative_to(ROOT))}
    if values != {sha(path)}:
        raise ValueError('runtime admission source is not uniquely bound: ' + str(path))


def canonical_profile_validator(manifest):
    import importlib.util
    for path in (RUNTIME_VALIDATOR, ROOT / 'scripts/rusty_v8_source_provenance.py'):
        bound_source(path)
    spec = importlib.util.spec_from_file_location('repair49_runtime_validator', RUNTIME_VALIDATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.validate(manifest, ROOT, 'core-full', 'aarch64-apple-darwin')


def runtime_records_check(records):
    errors = []
    for row in records:
        path = Path(row['path'])
        try:
            before_destination = str(path.resolve(strict=True))
            before = path.stat()
            actual = sha(path)
            after = path.stat()
            if (before_destination != row['resolved_path']
                    or str(path.resolve(strict=True)) != row['resolved_path']
                    or actual != row['sha256'] or after.st_size != row['size_bytes']
                    or (before.st_size, before.st_mtime_ns, before.st_ino)
                    != (after.st_size, after.st_mtime_ns, after.st_ino)):
                errors.append(str(path))
        except (OSError, RuntimeError) as error:
            errors.append(str(error))
    return errors


def load_runtime(profile_path, profile_sha):
    profile_path = Path(profile_path).absolute()
    profile_destination = str(profile_path.resolve(strict=True))
    if not Path(profile_destination).is_relative_to(ROOT / 'artifacts'):
        raise ValueError('runtime profile must be a retained repository artifact')
    data = profile_path.read_bytes()
    if hashlib.sha256(data).hexdigest() != profile_sha:
        raise ValueError('runtime profile digest changed')
    manifest = json.loads(data)
    if manifest.get('archive_sha256') == OLD_ARCHIVE_SHA:
        raise ValueError('historical native33 archive cannot execute repair49 engine changes')
    records = {}
    canonical_records = {}
    def retain(path, digest, size, expected_destination=None):
        # Keep the original asserted path as well as canonical alias identity.
        path = Path(path)
        if not path.is_absolute() or not re.fullmatch('[0-9a-f]{64}', digest):
            raise ValueError('invalid runtime input record')
        key = str(path.resolve(strict=True))
        if expected_destination is not None and key != expected_destination:
            raise ValueError('runtime input alias changed during capture')
        row = dict(path=str(path), resolved_path=key, sha256=digest, size_bytes=size)
        if key in canonical_records and canonical_records[key] != (digest, size):
            raise ValueError('conflicting runtime canonical input identity')
        if str(path) in records and records[str(path)] != row:
            raise ValueError('conflicting runtime input identity')
        canonical_records[key] = (digest, size)
        records[str(path)] = row
    retain(profile_path, profile_sha, len(data), profile_destination)
    for name in ('archive', 'binding', 'icu_data', 'startup_snapshot', 'external_references',
                 'worker', 'bridge_abi', 'rusty_v8_source_manifest'):
        retain(manifest[name + '_path'], manifest[name + '_sha256'], manifest[name + '_size_bytes'])
    bridge_path = Path(manifest['bridge_abi_path'])
    bridge_data = bridge_path.read_bytes()
    if hashlib.sha256(bridge_data).hexdigest() != manifest['bridge_abi_sha256']:
        raise ValueError('bridge ABI changed during runtime admission')
    bridge = json.loads(bridge_data)
    for row in bridge['inputs']:
        retain(row['path'], row['sha256'], row['size_bytes'])
    sdk = Path(bridge['sdkroot'])
    if not sdk.is_absolute() or not sdk.is_dir():
        raise ValueError('runtime ABI SDK is missing')
    sdk_destination = str(sdk.resolve(strict=True))
    flags = bridge['flags']
    if flags.count('-isysroot') != 1 or Path(flags[flags.index('-isysroot') + 1]).resolve() != sdk.resolve():
        raise ValueError('runtime ABI SDK flags disagree')
    rows = list(records.values())
    # Capture every asserted input and alias before the canonical validator can
    # read them; a validator-time retarget or mutation must not become authority.
    canonical_profile_validator(manifest)
    errors = runtime_records_check(rows)
    if str(sdk.resolve(strict=True)) != sdk_destination:
        errors.append('runtime SDK alias changed during admission')
    if errors:
        raise ValueError('runtime inputs changed during admission: ' + repr(errors))
    return dict(profile=dict(path=str(profile_path), sha256=profile_sha), records=rows,
                sdkroot=str(sdk), sdkroot_resolved=sdk_destination,
                environment=dict(RUSTY_V8_ARCHIVE=manifest['archive_path'],
                    RUSTY_V8_SRC_BINDING_PATH=manifest['binding_path'],
                    ORYN_V8_BRIDGE_ABI=manifest['bridge_abi_path'],
                    ORYN_STARTUP_SNAPSHOT=manifest['startup_snapshot_path'],
                    ORYN_ICU_DATA_FINGERPRINT_PATH=manifest['icu_data_path'],
                    ORYN_V8_PROFILE_MANIFEST=str(profile_path), SDKROOT=str(sdk), RUST_BACKTRACE='1'))


def runtime_check(runtime):
    errors = runtime_records_check(runtime['records'])
    try:
        if str(Path(runtime['sdkroot']).resolve(strict=True)) != runtime['sdkroot_resolved']:
            errors.append('runtime SDK alias changed')
    except (OSError, RuntimeError) as error:
        errors.append(str(error))
    return dict(errors=errors, profile=runtime['profile'], input_count=len(runtime['records']))


def load_runtime_provenance(path, expected, lease_path, lease_sha, gate_sha, runtime):
    validator_path = ROOT / 'scripts/runtime_provenance.py'
    bound_source(validator_path)
    import importlib.util
    spec = importlib.util.spec_from_file_location('repair49_runtime_provenance', validator_path)
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    arguments = dict(root=ROOT, gate_path=GATE, gate_sha256=gate_sha,
                     lease_path=lease_path, lease_sha256=lease_sha,
                     profile_path=Path(runtime['profile']['path']), profile_sha256=runtime['profile']['sha256'])
    result = validator.validate_runtime_provenance(path, expected, **arguments)
    return dict(path=str(Path(path).absolute()), sha256=expected, records=result['records'],
                lease_path=str(lease_path), lease_sha256=lease_sha, gate_sha256=gate_sha)


def check_runtime_provenance(provenance, runtime):
    try:
        repeated = load_runtime_provenance(provenance['path'], provenance['sha256'],
            Path(provenance['lease_path']), provenance['lease_sha256'], provenance['gate_sha256'], runtime)
        if repeated != provenance:
            raise ValueError('runtime provenance inputs or original aliases changed')
        return dict(errors=[], path=provenance['path'], sha256=provenance['sha256'])
    except Exception as error:
        return dict(errors=[repr(error)], path=provenance['path'], sha256=provenance['sha256'])


def runtime_environment(env, runtime):
    env = dict(env)
    for key in ('V8_FROM_SOURCE', 'ORYN_STARTUP_SNAPSHOT', 'ORYN_ICU_DATA_PATH',
                'ORYN_ICU_DATA_FINGERPRINT_PATH', 'RUSTY_V8_ARCHIVE', 'RUSTY_V8_SRC_BINDING_PATH',
                'ORYN_V8_BRIDGE_ABI', 'ORYN_V8_PROFILE_MANIFEST', 'ORYN_SECURITY_PREBUILT_CANDIDATE'):
        env.pop(key, None)
    env.update(runtime['environment'])
    return env


def group_exists(group_id):
    try:
        os.killpg(group_id, 0)
    except ProcessLookupError:
        return False
    return True


def run_owned_dispatch(command, env, start_receipt=None, log_path=None):
    """Allow nested completion wrappers to settle before forced cleanup."""
    process, log = None, None
    result = dict(returncode=None, error=None, cleanup_complete=True,
                  cleanup_errors=[], events=[], cancellation_signal=None, forced_cleanup=False)
    with Cancellation() as cancellation:
        try:
            if log_path is not None:
                log = Path(log_path).open('x')
            if cancellation.signum is not None:
                raise InterruptedError('cancelled before dispatch launch')
            process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True,
                stdout=log, stderr=subprocess.STDOUT if log is not None else None)
            primary = None
            try:
                if start_receipt is not None:
                    save(start_receipt, dict(pid=process.pid, group_id=process.pid, command=command,
                        generation_gate=env.get('ORYN_GENERATION_GATE'),
                        generation_gate_sha256=env.get('ORYN_GENERATION_GATE_SHA256'),
                        supervision=env.get(SUPERVISOR_KEY)))
            except BaseException as failure:
                primary = repr(failure)
            result = supervise(process, cancellation, grace=90, primary_error=primary)
            result['events'].insert(0, dict(event='started', pid=process.pid, group_id=process.pid))
        except BaseException as failure:
            result['error'] = result['error'] or repr(failure)
            if process is not None:
                result['cleanup_complete'] = False
                result['cleanup_errors'].append('launched process supervision escaped')
        finally:
            if log is not None:
                try:
                    log.close()
                except BaseException as failure:
                    result['cleanup_errors'].append('log close failed: ' + repr(failure))
                    result['cleanup_complete'] = False
        result['cancellation_signal'] = cancellation.signum
        if cancellation.signum is not None:
            result['error'] = result['error'] or 'owned execution cancelled by signal ' + str(cancellation.signum)
    return dict(returncode=result['returncode'], execution_error=result['error'],
                cleanup_errors=result['cleanup_errors'], group_settled=result['cleanup_complete'],
                pid=process.pid if process is not None else None, events=result['events'],
                cancellation_signal=result['cancellation_signal'],
                forced_cleanup=result['forced_cleanup'], cleanup_completed_at=time.time())


def environment_check(env, phase, before_sha=None, output_directory=None):
    if sha(ENVIRONMENT) != ENVIRONMENT_SHA or sha(ENVIRONMENT_CHECKER) != ENVIRONMENT_CHECKER_SHA:
        raise ValueError('historical environment/checker changed')
    directory = BASE if output_directory is None else Path(output_directory)
    output = directory / ('python14-base-environment-' + phase + '.json')
    command = [env['ORYN_WPT_PYTHON'], str(ENVIRONMENT_CHECKER), '--phase', phase,
               '--output', str(output), '--check-process-environment']
    if phase == 'after':
        command.extend(('--before', str(directory / 'python14-base-environment-before.json'),
                        '--before-sha256', before_sha))
    child_env = dict(env)
    child_env.pop(SUPERVISOR_KEY, None)
    process = run_owned_dispatch(command, child_env)
    receipt = directory / ('python14-base-environment-' + phase + '-process.json')
    save(receipt, process)
    code = process['returncode']
    if process['execution_error'] or process['cleanup_errors'] or not process['group_settled']:
        raise ValueError('Python environment process custody failed: ' + repr(process))
    result = json.loads(output.read_bytes())
    if code or result.get('status') != 'passed':
        raise ValueError('Python environment custody failed: ' + repr(result.get('errors')))
    return dict(scope='baseline82_without_signed_overlay', path=str(output.relative_to(ROOT)), sha256=sha(output),
                process_receipt=dict(path=str(receipt.relative_to(ROOT)), sha256=sha(receipt)))


def build_environment(admission, python, launch, gate_sha, runtime=None):
    env = dict(os.environ)
    env['PYTHONDONTWRITEBYTECODE'] = '1'
    for key in list(env):
        if re.fullmatch(r'ORYN_REPAIR[0-9]+_SUPERVISED_GROUP', key):
            env.pop(key)
    if python:
        if sha(ENVIRONMENT) != ENVIRONMENT_SHA:
            raise ValueError('historical Python environment record changed')
        config = json.loads(ENVIRONMENT.read_bytes())
        env.update(config['environment'])
        env['PATH'] = config['path_prepend'] + os.pathsep + env.get('PATH', '')
        for key in config['unset']:
            env.pop(key, None)
        command = [config['environment']['ORYN_WPT_PYTHON'], str(launch)]
    else:
        command = [sys.executable, str(launch)]
    if python:
        env[SUPERVISOR_KEY] = 'outer-owned-session'
    else:
        env.pop(SUPERVISOR_KEY, None)
    if not python:
        if runtime is None:
            raise ValueError('native35 requires an exact fresh runtime profile')
        env = runtime_environment(env, runtime)
        command.extend(('--runtime-profile', runtime['profile']['path'],
                        '--runtime-profile-sha256', runtime['profile']['sha256']))
    env[SUPERVISOR_KEY] = 'outer-owned-session'
    # Bind last: historical environment dictionaries must never restore an older gate.
    env = admission.environment(env)
    if env['ORYN_GENERATION_GATE'] != str(GATE.resolve()) or env['ORYN_GENERATION_GATE_SHA256'] != gate_sha:
        raise ValueError('final child environment is not bound to the explicit candidate')
    return env, command


def signed_module():
    import importlib.util
    path = ROOT / 'scripts/qualification/signed_inputs.py'
    bound_source(path)
    spec = importlib.util.spec_from_file_location('repair49_signed_inputs', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('attempt', choices=('native35', 'python14'))
    parser.add_argument('--gate', type=Path, required=True)
    parser.add_argument('--gate-sha256', required=True)
    parser.add_argument('--output-directory', type=Path, required=True)
    parser.add_argument('--whole-lease', type=Path, required=True)
    parser.add_argument('--whole-lease-sha256', required=True)
    parser.add_argument('--runtime-profile', type=Path)
    parser.add_argument('--runtime-profile-sha256')
    parser.add_argument('--runtime-provenance', type=Path)
    parser.add_argument('--runtime-provenance-sha256')
    parser.add_argument('--signed-inputs', type=Path)
    parser.add_argument('--signed-inputs-sha256')
    args = parser.parse_args()
    if args.attempt == 'native35' and (args.runtime_profile is None or not args.runtime_profile_sha256
            or args.runtime_provenance is None or not args.runtime_provenance_sha256):
        parser.error('native35 requires explicit runtime profile and provenance paths/digests')
    if args.attempt == 'python14' and any(value is not None for value in (args.runtime_profile,
            args.runtime_profile_sha256, args.runtime_provenance, args.runtime_provenance_sha256)):
        parser.error('Python14 does not consume a native runtime profile')
    if python_requested := (args.attempt == 'python14'):
        if args.signed_inputs is None or not args.signed_inputs_sha256:
            parser.error('Python14 requires explicit --signed-inputs and --signed-inputs-sha256')
    elif args.signed_inputs is not None or args.signed_inputs_sha256 is not None:
        parser.error('native35 does not consume a Python signed overlay')
    if python_requested and not re.fullmatch('[0-9a-f]{64}', args.signed_inputs_sha256):
        parser.error('signed inputs require an actual lowercase SHA256 digest')
    for value in (args.gate_sha256, args.whole_lease_sha256) + ((args.runtime_profile_sha256,args.runtime_provenance_sha256) if args.attempt == 'native35' else ()):
        if not re.fullmatch('[0-9a-f]{64}', value):
            parser.error('freeze digests must be actual lowercase SHA256 values')
    lease_path = args.whole_lease.resolve()
    if not lease_path.is_relative_to(BASE):
        parser.error('whole lease must be a reviewed completion artifact')
    python = args.attempt == 'python14'
    launch = ROOT / 'scripts/qualification' / ('launch.py' if python else 'native.py')
    configure_candidate(args.gate, args.gate_sha256, lease_path, args.whole_lease_sha256)
    run_directory = fresh_output_directory(args.output_directory)
    child_directory = run_directory / ('python' if python else 'native')
    output = run_directory / 'whole-custody.json'
    preflight = run_directory / 'whole-preflight.json'
    process_start = run_directory / 'process-start.json'
    process_receipt = run_directory / 'process.json'
    record = dict(operation_sha256=sha(__file__), gate=str(GATE), whole_lease=str(lease_path),
                  before=check(lease_path, args.whole_lease_sha256, args.gate_sha256),
                  returncode=None, execution_error=None)
    if record['before']['errors']:
        save(output, record)
        return 1
    env = None
    runtime = None
    provenance = None
    signed = None
    overlay_helper = None
    base_env = None
    try:
        sys.dont_write_bytecode = True
        sys.path.insert(0, str(ROOT / 'scripts'))
        from generation_gate import GenerationLease
        admission = GenerationLease(GATE, expected=args.gate_sha256)
        bound_harness(launch, python)
        if not python:
            runtime = load_runtime(args.runtime_profile, args.runtime_profile_sha256)
            provenance = load_runtime_provenance(args.runtime_provenance, args.runtime_provenance_sha256,
                lease_path, args.whole_lease_sha256, args.gate_sha256, runtime)
            record['runtime_provenance_before'] = provenance
            record['runtime_before'] = runtime_check(runtime)
            record['runtime_inputs'] = runtime['records']
            if record['runtime_before']['errors']:
                raise ValueError('runtime inputs changed before launch')
        env, command = build_environment(admission, python, launch, args.gate_sha256, runtime)
        command.extend(('--gate', str(GATE), '--gate-sha256', args.gate_sha256,
                        '--output-directory', str(child_directory)))
        if not python:
            command.extend(('--runtime-provenance', provenance['path'],
                '--runtime-provenance-sha256', provenance['sha256'],
                '--whole-lease', str(lease_path), '--whole-lease-sha256', args.whole_lease_sha256))
        if python:
            overlay_helper = signed_module()
            signed = overlay_helper.load_inputs(args.signed_inputs, args.signed_inputs_sha256,
                sys.modules[__name__], lease_path, args.whole_lease_sha256, args.gate_sha256)
            base_env = dict(env)
            env = overlay_helper.effective_environment(base_env, signed['value']['overlay'], admission)
            record['signed_before'] = overlay_helper.check_inputs(signed, sys.modules[__name__])
            command.extend(('--signed-inputs', str(args.signed_inputs.absolute()),
                            '--signed-inputs-sha256', args.signed_inputs_sha256,
                            '--whole-lease', str(lease_path), '--whole-lease-sha256', args.whole_lease_sha256))
            record['effective_environment'] = dict(scope='baseline82_plus_exact_signed_three',
                baseline=ENVIRONMENT_SHA, changed_keys=list(overlay_helper.KEYS),
                overlay=signed['value']['overlay'])
        record.update(command=command, final_generation_environment={key: env[key] for key in ('ORYN_GENERATION_GATE', 'ORYN_GENERATION_GATE_SHA256')})
        if python:
            record['environment_before'] = environment_check(base_env, 'before', output_directory=run_directory)
        save(preflight, record)
        admission.check()
        record['process'] = run_owned_dispatch(command, env, start_receipt=process_start)
        save(process_receipt, record['process'])
        record['process_receipt'] = dict(path=str(process_receipt.relative_to(ROOT)), sha256=sha(process_receipt))
        record['returncode'] = record['process']['returncode']
        record['execution_error'] = record['process']['execution_error']
    except BaseException as error:
        record['execution_error'] = repr(error)
    finally:
        if python and env is not None and 'environment_before' in record:
            try:
                record['environment_after'] = environment_check(base_env, 'after', record['environment_before']['sha256'], output_directory=run_directory)
            except BaseException as error:
                record['environment_error'] = repr(error)
        if signed is not None:
            try:
                overlay_helper.validate_environment(base_env, env, signed['value']['overlay'])
                record['signed_after'] = overlay_helper.check_inputs(signed, sys.modules[__name__])
            except BaseException as error:
                record['signed_error'] = repr(error)
        if runtime is not None:
            record['runtime_after'] = runtime_check(runtime)
        if provenance is not None:
            record['runtime_provenance_after'] = check_runtime_provenance(provenance, runtime)
        record['after'] = check(lease_path, args.whole_lease_sha256, args.gate_sha256)
        record['custody_valid'] = not (record['before']['errors'] or record['after']['errors']
            or record.get('environment_error') or record.get('signed_error') or record['execution_error']
            or record.get('runtime_provenance_after', {}).get('errors')
            or record.get('runtime_before', {}).get('errors') or record.get('runtime_after', {}).get('errors')
            or record.get('process', {}).get('cleanup_errors')
            or not record.get('process', {}).get('group_settled', False))
        save(output, record)
    print(json.dumps(record), flush=True)
    return record['returncode'] or int(not record['custody_valid'] or record['execution_error'] is not None)


if __name__ == '__main__':
    sys.exit(main())

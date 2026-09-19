"""Original-run supervisor custody, separate from individual WPT outcomes."""
import json
import sys
from pathlib import Path
from completion_provenance import ROOT, digest, read_record
sys.path.insert(0, str(ROOT / 'tools/wptrunner-oryn'))
from oryn_wptrunner import feature_profile, cookie_profile


def verify(report_path, execution_path, candidate, collection, *, profile_id=None):
    report_path = Path(report_path).resolve()
    execution_path = Path(execution_path).resolve()
    original_hashes = {str(path): digest(path) for path in (report_path, execution_path)}
    execution = json.loads(execution_path.read_text())
    if (type(execution.get('schema_version')) is not int or execution['schema_version'] not in (1, 2) or execution.get('candidate_sha256') != candidate
            or execution.get('collection') != collection):
        raise ValueError('official execution identity mismatch')
    if (execution.get('custody_complete') is not True or execution.get('cleanup_complete') is not True
            or execution.get('timed_out') is not False or execution.get('launch_error')
            or execution.get('errors') != [] or type(execution.get('returncode')) is not int):
        raise ValueError('official execution custody or cleanup incomplete')
    # Core may contain genuine failures (exit 1); retain those raw results.
    if execution['returncode'] not in (0, 1):
        raise ValueError('official executor did not reach an ordinary terminal result')
    if report_path.parent != execution_path.parent or execution.get('artifacts', {}).get(report_path.name) != digest(report_path):
        raise ValueError('official report not bound to original execution')
    custody_path = execution_path.parent / 'protocol-custody.jsonl'
    if execution.get('artifacts', {}).get(custody_path.name) != digest(custody_path):
        raise ValueError('official protocol custody not bound to execution')
    protocol_custody(custody_path, candidate)
    preflight_path = read_record(execution.get('preflight'))
    if preflight_path != execution_path.parent / 'preflight.json':
        raise ValueError('official execution preflight belongs to another run')
    preflight = json.loads(preflight_path.read_text())
    if (preflight.get('schema_version') != execution['schema_version'] or preflight.get('candidate_sha256') != candidate
            or preflight.get('collection') != collection or preflight.get('argv') != execution.get('argv')):
        raise ValueError('official execution preflight identity mismatch')
    for field in ('generation_gate', 'run_contract', 'workflow_lock', 'producer_source', 'supervisor_source'):
        read_record(preflight.get(field))
    for field, name in (('producer_source', 'wpt_official_workflow.py'), ('supervisor_source', 'completion_execution.py')):
        if read_record(preflight[field]) != ROOT / 'scripts' / name:
            raise ValueError('official execution producer substituted')
    if preflight['workflow_lock']['sha256'] != execution.get('workflow_lock_sha256'):
        raise ValueError('official execution workflow lock mismatch')
    lock = json.loads(read_record(preflight['workflow_lock']).read_text())
    contract = json.loads(read_record(preflight['run_contract']).read_text())
    if (lock.get('candidate_sha256') != candidate or lock.get('run_contract_sha256') != preflight['run_contract']['sha256']
            or contract.get('candidate', {}).get('sha256') != candidate):
        raise ValueError('official execution candidate custody mismatch')
    candidate_record = json.loads(read_record(contract['candidate']).read_text())
    envelope = None
    if contract.get('schema_version') == 2:
        from core_profile_contract import resolve_envelope, identity, check_identity, BASE_FIELDS, verify_platform
        envelope = resolve_envelope(preflight['run_contract']['path'])
        expected = identity(envelope, collection)
        if profile_id is not None and profile_id != expected['profile_id']:
            raise ValueError('official report belongs to another required profile')
        if (execution['schema_version'] != 2 or lock.get('schema_version') != 2
                or any(lock.get(key) != envelope[key] for key in BASE_FIELDS)
                or report_path.parent != Path(envelope['envelope']['outputs'][collection])
                or read_record(preflight['workflow_lock']).parent != Path(envelope['envelope']['prepared'])):
            raise ValueError('official envelope output/lock identity changed')
        check_identity(execution, expected); check_identity(preflight, expected)
        fixture = envelope['fixture']
    else:
        if profile_id is not None or 'core_profile_registry' in candidate_record.get('artifacts', {}) or execution['schema_version'] != 1:
            raise ValueError('required profile cannot use historical execution custody')
        fixture = json.loads(read_record(candidate_record.get('artifacts', {}).get('fixture_contract')).read_text())
    features = feature_profile.from_fixture(fixture)
    cookies = cookie_profile.from_fixture(fixture)
    report_info = json.loads(report_path.read_text()).get('run_info', {})
    if envelope:
        check_identity(report_info, expected, prefix='oryn_')
        verify_platform(report_info, envelope)
        if report_info.get('oryn_fixture_sha256') != envelope['profile_sha256'] or report_info.get('oryn_run_contract_sha256') != envelope['envelope_sha256']:
            raise ValueError('raw profile fixture/envelope binding differs')
        execution['profile_context'] = expected
    for recorded in (lock, execution):
        if (recorded.get('feature_environment_sha256') != feature_profile.environment_digest(fixture)
                or recorded.get('fixture_environment_sha256') != cookie_profile.environment_digest(fixture)):
            raise ValueError('native profile comparison environment differs from candidate fixture')
    for recorded in (lock, execution, {'feature_profile': report_info.get('oryn_feature_profile'),
            'feature_profile_sha256': report_info.get('oryn_feature_profile_sha256')}):
        feature_profile.verify_record(recorded, features)
    for recorded in (lock, execution, {'cookie_profile': report_info.get('oryn_cookie_profile'),
            'cookie_profile_sha256': report_info.get('oryn_cookie_profile_sha256')}):
        if recorded.get('cookie_profile') != cookies or recorded.get('cookie_profile_sha256') != cookie_profile.digest(cookies):
            raise ValueError('native cookie profile differs from candidate fixture')
    for path, expected in lock.get('inputs', {}).items():
        read_record({'path': path, 'sha256': expected})
    if envelope:
        from core_profile_contract import validate_inventory_binding
        validate_inventory_binding(envelope['registry'], lock['manifest'], lock['policy'], Path(lock['inventory'])/'coverage.jsonl')
        for name, expected_hash in execution.get('artifacts', {}).items():
            file = execution_path.parent/name
            if file.parent != execution_path.parent or digest(file) != expected_hash:
                raise ValueError('original profile artifact changed')
        if any(file.is_symlink() or not file.is_file() for file in execution_path.parent.iterdir()):
            raise ValueError('original profile evidence contains an unrecorded directory or symlink')
        if {file.name for file in execution_path.parent.iterdir() if file.is_file()} != set(execution['artifacts']) | {'execution.json'}:
            raise ValueError('original profile artifact inventory changed')
    for path, expected_hash in original_hashes.items():
        read_record({'path': path, 'sha256': expected_hash})
    read_record(execution['preflight'])
    for field in ('generation_gate', 'run_contract', 'workflow_lock', 'producer_source', 'supervisor_source'):
        read_record(preflight[field])
    return execution


def protocol_custody(path, candidate):
    path = Path(path)
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError('official protocol custody exceeds evidence bound')
    records = [json.loads(line) for line in path.read_text().splitlines()]
    sessions = {}
    for record in records:
        identity = record.get('session')
        if (record.get('schema_version') != 1 or record.get('candidate_sha256') != candidate
                or not isinstance(identity, str) or len(identity) != 32
                or any(char not in '0123456789abcdef' for char in identity)
                or record.get('success') is not True or record.get('error_type') is not None):
            raise ValueError('official protocol connection or teardown failed')
        phases = sessions.setdefault(identity, [])
        expected = 'connect' if not phases else 'teardown' if len(phases) == 1 else None
        if record.get('phase') != expected:
            raise ValueError('official protocol custody duplicate or out of order')
        phases.append(record['phase'])
    if not sessions or any(phases != ['connect', 'teardown'] for phases in sessions.values()):
        raise ValueError('official protocol teardown evidence missing')
    return len(sessions)

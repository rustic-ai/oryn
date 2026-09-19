"""Fail-closed verdicts for the separately mounted official-executor canaries."""
from pathlib import Path
import json
from wpt_full_qualification import REVISION, sha256, unique

MOUNT = '/__oryn_qualification__/'
EXPECTED = {
    'assertion-failure.html': ('OK', 'intentional assertion failure', 'FAIL'),
    'missing-helper.html': ('OK', 'missing helper cannot produce a valid pass', 'FAIL'),
    'missing-completion.html': ('TIMEOUT', 'completed subtest without harness completion', 'PASS'),
    'reference-never-ready.html': ('TIMEOUT', None, None),
    'reference-mismatch.html': ('FAIL', None, None),
}


def qualify(report_path, execution_path, audit_path, candidate):
    report = json.loads(Path(report_path).read_text())
    execution = json.loads(Path(execution_path).read_text())
    audit = json.loads(Path(audit_path).read_text())
    info = report.get('run_info', {})
    if (info.get('oryn_candidate_sha256') != candidate or info.get('oryn_wpt_revision') != REVISION
            or info.get('product') != 'oryn' or info.get('oryn_executor') != 'upstream-wptrunner'):
        raise ValueError('canary report candidate/executor mismatch')
    if execution.get('candidate_sha256') != candidate or execution.get('collection') != 'canaries':
        raise ValueError('canary execution identity mismatch')
    if execution.get('artifacts', {}).get('wptreport.json') != sha256(report_path):
        raise ValueError('canary report not bound to execution')
    if audit.get('candidate_sha256') != candidate or audit.get('report_sha256') != sha256(report_path):
        raise ValueError('canary audit identity mismatch')
    rows = unique(report.get('results', []), 'test', 'canary result')
    if set(rows) != {MOUNT + name for name in EXPECTED}:
        raise ValueError('canary result set incomplete or contaminated with other tests')
    verdicts = []
    for name, (status, subtest, expected_substatus) in EXPECTED.items():
        row = rows[MOUNT + name]
        errors = []
        if row.get('status') != status:
            errors.append('intentional_control_did_not_reach_expected_failure')
        observed = row.get('subtests', [])
        if subtest is not None and (not isinstance(observed, list) or len(observed) != 1
                or not isinstance(observed[0], dict) or observed[0].get('name') != subtest or observed[0].get('status') != expected_substatus):
            errors.append('canary_subtest_identity_or_outcome_mismatch')
        if name == 'missing-helper.html':
            resources = audit.get('resources', {}).get(MOUNT + name, [])
            if not any(record.get('path') == '/__oryn_qualification_missing_helper__.js'
                       and record.get('status') == 404 for record in resources):
                errors.append('missing_helper_failure_not_observed')
        if name.startswith('reference-'):
            if audit.get('native_reference_rendering') is not True:
                errors.append('native_reference_provenance_missing')
        verdicts.append({'id': MOUNT + name, 'control_rejected': not errors, 'errors': errors})
    global_errors = []
    if (execution.get('custody_complete') is not True or execution.get('cleanup_complete') is not True
            or execution.get('timed_out') is not False or execution.get('errors') != [] or execution.get('launch_error')):
        global_errors.append('executor_custody_not_proven')
    if execution.get('returncode') != 1:
        global_errors.append('expected_failing_suite_exit_missing')
    if audit.get('cleanup_complete') is not True or audit.get('all_workers_reaped') is not True:
        global_errors.append('cleanup_not_proven')
    return {'schema_version': 1, 'candidate_sha256': candidate,
            'report_sha256': sha256(report_path), 'execution_sha256': sha256(execution_path),
            'audit_sha256': sha256(audit_path), 'controls': verdicts, 'errors': global_errors,
            'controls_rejected': not global_errors and all(row['control_rejected'] for row in verdicts),
            'scope': 'Five fixture/executor controls only; native fault and TLS/focus controls are separately required'}

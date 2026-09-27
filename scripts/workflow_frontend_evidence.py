"""Concrete frontend covering witnesses, independent of family-wide labels.

These witnesses supplement the complete cell/oracle denominator. A fixture file
or successful native action alone is not a witness of a frontend behavior.
"""
import json
from pathlib import Path
from completion_provenance import ROOT, digest, read_record

WITNESSES = {
    ('research', 2, 'hydrated_pagination'): ('client_rendering', 'ssr_hydration', 'routing_chunks'),
    ('forms', 3, 'multistep_custom_role'): ('portals_custom_controls',),
    ('shopping', 2, 'framed_persisted_checkout'): ('cross_origin_frames', 'async_persistence'),
    ('booking', 2, 'client_fold_first'): ('client_timezones',),
}


def sources(recipe):
    family, version, _ = recipe
    files = [f'test-harness/workflows/{family}-cases-v{version}.json',
             f'test-harness/workflows/{family}_fixture_v{version}.py',
             f'test-harness/workflows/v2/{family}-client.mjs',
             f'scripts/run-oil-{family}-outcomes.py', 'scripts/workflow_native_adapter.py',
             'scripts/workflow_frontend_evidence.py', 'scripts/workflow_outcome_evidence.py']
    if family == 'research': files.append('test-harness/workflows/v2/research-search.mjs')
    if family == 'forms': files.append('test-harness/workflows/forms_fixture_v2.py')
    if family == 'booking': files.append('test-harness/workflows/v2/timezone-resolution.mjs')
    return [ROOT / name for name in files]


def source_records(recipe):
    return [{'path': str(path), 'sha256': digest(path)} for path in sources(recipe)]


def validate_claim(root, claim, attempt, candidate, artifact):
    """Read preserved native inputs/results and rerun the independent family validator."""
    from workflow_native_adapter import (read_case_contract, family_lines, load, research,
                                          validate_forms)
    recipe = tuple(claim.get('recipe', []))
    if recipe not in WITNESSES or attempt.get('fault') is not None:
        raise ValueError('unknown or fault-modified frontend witness recipe')
    expected_sources = sources(recipe)
    records = claim.get('sources', [])
    if len(records) != len(expected_sources) or [read_record(record) for record in records] != expected_sources:
        raise ValueError('frontend witness source set differs')
    plan = json.loads(artifact(root, claim.get('native_plan')))
    evidence = json.loads(artifact(root, claim.get('native_evidence')))
    if (plan.get('attempt_id') != attempt['attempt_id'] or evidence.get('attempt_id') != attempt['attempt_id']
            or evidence.get('infrastructure_errors') or evidence.get('schema_version') != 1
            or evidence.get('runtime', {}).get('worker_binary_sha256') != candidate['worker_sha256']):
        raise ValueError('frontend witness native run identity differs')
    session_id = claim.get('session')
    inputs = [row for row in plan.get('sessions', []) if row.get('id') == session_id]
    results = [row for row in evidence.get('sessions', []) if row.get('id') == session_id]
    if len(inputs) != 1 or len(results) != 1:
        raise ValueError('frontend witness native session missing or duplicate')
    planned, session = inputs[0], results[0]
    if (planned.get('fault') or planned.get('adversarial_for') or session.get('errors')
            or session.get('cleanup_errors') or session.get('workers_retired') is not True):
        raise ValueError('frontend witness native session incomplete')
    family, version, name = recipe
    contract = read_case_contract(family, version, name)
    matches = [case for case in contract['cases'] if case['id'] == name]
    if len(matches) != 1:
        raise ValueError('frontend witness concrete case missing or duplicate')
    case = matches[0]
    runner = research if family == 'research' else load('frontend_' + family, ROOT / f'scripts/run-oil-{family}-outcomes.py')
    base = claim['base']
    lines = family_lines(family, runner, contract, case, base)
    if [step.get('oil') for step in planned.get('steps', [])] != lines:
        raise ValueError('frontend witness task differs from concrete recipe')
    steps = session.get('steps', [])
    if (len(steps) != len(lines) or any(step.get('index') != index or step.get('error')
            or step.get('timed_out') is not False for index, step in enumerate(steps))):
        raise ValueError('frontend witness native task truncated or failed')
    from workflow_outcome_evidence import command_frames
    frames = command_frames(steps, lines)
    data = json.loads(artifact(root, claim.get('sealed_oracle')))
    if family == 'forms':
        from urllib.parse import urlsplit
        url = urlsplit(base)
        errors = validate_forms(case, contract, base.rsplit('/', 1)[1], url.scheme + '://' + url.netloc, frames, data)
    else:
        errors = runner.validate(case, lines, base, frames, data)
    if errors:
        raise ValueError('frontend witness independent outcome rejected: ' + '; '.join(errors))
    return set(WITNESSES[recipe])


def validate(root, reference, attempt, candidate, artifact):
    claims = json.loads(artifact(root, reference))
    if not isinstance(claims, list):
        raise ValueError('frontend witnesses must be a list')
    identities = set(); dimensions = set()
    for claim in claims:
        identity = (claim.get('native_plan', {}).get('path'), claim.get('session'))
        if identity in identities:
            raise ValueError('duplicate frontend native session witness')
        identities.add(identity)
        dimensions.update(validate_claim(root, claim, attempt, candidate, artifact))
    return dimensions

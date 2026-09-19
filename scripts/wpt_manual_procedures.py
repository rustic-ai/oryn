"""Source-bound procedure selection; observations cannot rewrite procedures."""
import hashlib
import json
from pathlib import Path

REVISION = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
DEFAULT_CORPUS = Path(__file__).resolve().parents[1] / 'docs/oryn2/wpt-procedures/core-manual-visual-2026-09-16.jsonl'


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def select(identifier, path=None):
    path = DEFAULT_CORPUS if path is None else path
    matches = [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    matches = [record for record in matches if record.get('id') == identifier]
    if len(matches) != 1:
        raise ValueError('procedure identity missing or duplicated')
    return matches[0]


def assessment_errors(row, assessment, checkout):
    binding = assessment.get('procedure_source') or {}
    path = Path(binding.get('path', ''))
    if not path.is_file() or digest(path) != binding.get('sha256'):
        return ['procedure_corpus_unbound']
    try:
        record = select(row['id'], path)
        if record != select(row['id']):
            return ['procedure_differs_from_generated_source']
    except (ValueError, OSError):
        return ['procedure_identity_invalid']
    if (record.get('source_sha256') != row['sha256'] or record.get('inventory_source_sha256') != row['sha256']
            or record.get('path') != row['path'] or record.get('url') != row['url']
            or record.get('upstream_revision') != REVISION):
        return ['procedure_source_identity_mismatch']
    if record.get('source_issues'):
        manifest = Path(__file__).resolve().parents[1] / 'tools/wptrunner-oryn/oryn_wptrunner/fixture-repairs.json'
        if assessment.get('fixture_repairs_sha256') != digest(manifest):
            return ['procedure_has_unresolved_source_prerequisites']
        repairs = json.loads(manifest.read_text())['repairs']
        receipt = assessment.get('fixture_repair_evidence') or {}
        receipt_path = Path(receipt.get('path', ''))
        if receipt not in assessment.get('artifacts', []) or not receipt_path.is_file() or digest(receipt_path) != receipt.get('sha256'):
            return ['procedure_repair_observation_unbound']
        observed = json.loads(receipt_path.read_text())
        if observed.get('candidate_sha256') != assessment.get('candidate_sha256') or observed.get('fixture_repairs_sha256') != digest(manifest):
            return ['procedure_repair_observation_identity_mismatch']
        if assessment.get('profile_identity') is not None and observed.get('profile_identity') != assessment['profile_identity']:
            return ['procedure_repair_observation_profile_mismatch']
        for issue in record['source_issues']:
            matches = [repair for repair in repairs if repair['request_path'] == '/' + issue.get('requested_path', '')
                       and repair['source_path'] == row['path'] and repair['source_sha256'] == row['sha256']]
            if len(matches) != 1:
                return ['procedure_has_unresolved_source_prerequisites']
            repair = matches[0]
            expected = {'method': 'GET', 'request_path': repair['request_path'], 'status': 200,
                        'target_sha256': repair['target_sha256']}
            deliveries = [item for item in observed.get('requests', []) if all(item.get(key) == value for key, value in expected.items()) and not item.get('error')]
            if len(deliveries) != 1:
                return ['procedure_repair_delivery_missing']
    if not record.get('procedure') or not record.get('expected_observations'):
        return ['procedure_steps_missing']
    root = Path(checkout).resolve()
    for helper in record.get('instruction_helpers', []):
        source = (root / helper['path']).resolve()
        if root not in source.parents or not source.is_file() or digest(source) != helper['sha256']:
            return ['procedure_helper_changed']
    branches = record.get('required_platforms', [])
    selection = assessment.get('platform_selection') or {}
    platform = selection.get('platform')
    api = selection.get('api')
    supported = {'macOS': {'AXAPI'}, 'Linux': {'ATK'},
                 'Windows': {'IAccessible2', 'MSAA', 'UIA'}}
    if platform not in supported or assessment.get('platform') != platform:
        return ['procedure_platform_selection_unbound']
    if 'atta' in record:
        if api not in supported[platform]:
            return ['procedure_platform_api_mismatch']
        required = [branch for branch in branches if branch.get('api') == api]
        # Missing source assertions are an open obligation, never a vacuous pass.
        if len(required) != 1:
            return ['procedure_native_api_assertions_missing']
    else:
        if api is not None:
            return ['procedure_platform_api_mismatch']
        required = [branch for branch in branches
                    if branch.get('platform', '').split(' ', 1)[0] == platform]
        if not required:
            return ['procedure_native_platform_instructions_missing']
    platforms = assessment.get('platform_observations') or []
    if len(platforms) != len(required):
        return ['procedure_platform_observations_incomplete']
    for requirement, observation in zip(required, platforms):
        if observation.get('requirement') != requirement or not observation.get('observations') or not observation.get('artifacts'):
            return ['procedure_platform_evidence_missing']
        if any(artifact not in assessment.get('artifacts', []) for artifact in observation['artifacts']):
            return ['procedure_platform_artifact_unbound']
        if 'api' in requirement:
            expected = [{'element': step.get('element'), 'title': step.get('title'), 'assertions': step.get('test', {}).get(requirement['api'], [])}
                        for step in record.get('atta', {}).get('steps', []) if requirement['api'] in step.get('test', {})]
            if observation.get('assertions') != expected:
                return ['procedure_atta_assertions_changed']
    from wpt_assessment_packet import validate as validate_packet
    try:
        validate_packet(assessment, row['id'], checkout)
    except (ValueError, OSError, KeyError, TypeError):
        return ['assessment_packet_missing_or_changed']
    return []

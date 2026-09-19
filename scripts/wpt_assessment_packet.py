"""Exact source/context for manual review; packets contain no observations."""
import hashlib
import json
from pathlib import Path
from wpt_manual_procedures import DEFAULT_CORPUS, REVISION, digest, select


def source(root, relative, expected):
    root = Path(root).resolve(); path = (root / relative).resolve()
    if root not in path.parents or not path.is_file() or digest(path) != expected:
        raise ValueError('assessment source missing, changed or outside pinned checkout')
    # Preserve original markup (links, instructions and script), not lossy text.
    return {'path': relative, 'sha256': expected, 'text': path.read_text(encoding='utf-8')}


def create(identifier, candidate_sha256, checkout, platform, api=None, corpus=None, *, profile_context=None):
    if not isinstance(candidate_sha256, str) or len(candidate_sha256) != 64 or any(c not in '0123456789abcdef' for c in candidate_sha256):
        raise ValueError('exact candidate manifest SHA-256 required')
    corpus = DEFAULT_CORPUS if corpus is None else corpus
    record = select(identifier, corpus)
    if record.get('upstream_revision') != REVISION or record.get('qualification') is not None:
        raise ValueError('assessment procedure pin or generation state invalid')
    supported = {'macOS': {'AXAPI'}, 'Linux': {'ATK'}, 'Windows': {'IAccessible2', 'MSAA', 'UIA'}}
    if platform not in supported: raise ValueError('unknown assessment platform')
    if 'atta' in record:
        if api not in supported[platform]: raise ValueError('foreign platform accessibility API')
        branches = [value for value in record['required_platforms'] if value.get('api') == api]
        if len(branches) != 1: raise ValueError('selected native API has no unique pinned assertion branch')
        assertions = [{'element': step.get('element'), 'title': step.get('title'), 'assertions': step.get('test', {}).get(api, [])}
                      for step in record['atta'].get('steps', []) if api in step.get('test', {})]
        if not assertions: raise ValueError('selected native API assertions absent')
    else:
        if api is not None: raise ValueError('non-AT procedure cannot invent API assertions')
        branches = [value for value in record['required_platforms'] if value.get('platform', '').split(' ', 1)[0] == platform]
        if not branches: raise ValueError('selected platform instructions absent')
        assertions = []
    profile = {}
    if profile_context is not None:
        from wpt_reviewed_dispositions import profile_identity
        identity = profile_identity(profile_context, candidate_sha256)
        if identity['platform'] != platform:
            raise ValueError('assessment platform differs from selected profile')
        profile = {'profile_identity': identity}
    return {'schema_version': 2 if profile else 1, **profile, 'id': identifier, 'candidate_sha256': candidate_sha256,
            'upstream_revision': REVISION, 'procedure_source_sha256': digest(corpus),
            'source': source(checkout, record['path'], record['source_sha256']),
            'helpers': [source(checkout, item['path'], item['sha256']) for item in record.get('instruction_helpers', [])],
            'platform_selection': {'platform': platform, 'api': api}, 'required_platforms': branches,
            'atta_assertions': assertions, 'procedure': record['procedure'],
            'expected_observations': record['expected_observations'],
            'source_issues': record.get('source_issues', []), 'cleanup': record['cleanup'],
            'observations': None, 'qualified': False}


def validate(assessment, identifier, checkout):
    binding = assessment.get('assessment_packet') or {}
    path = Path(binding.get('path', ''))
    if binding not in assessment.get('artifacts', []) or not path.is_file() or digest(path) != binding.get('sha256'):
        raise ValueError('assessment packet artifact unbound')
    packet = json.loads(path.read_text())
    selection = assessment.get('platform_selection') or {}
    expected = create(identifier, assessment.get('candidate_sha256'), checkout,
                      selection.get('platform'), selection.get('api'),
                      profile_context=assessment.get('profile_identity'))
    if packet != expected: raise ValueError('assessment packet differs from candidate/pinned source/platform')
    return packet

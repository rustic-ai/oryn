"""Reviewed Core profile scope and immutable whole-run envelope custody.

This module never chooses a profile set or creates an upstream disposition. A
registry must independently declare both required policy pairs before execution.
"""
import copy
import hashlib
import json
from pathlib import Path
import re
import sys

from completion_provenance import ROOT, digest, read_record

REVISION = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
MANIFEST_SHA256 = '566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101'
POLICY_SHA256 = 'a655bd38bbd3cc90e49344f7ceae1db1a3bfb09c8b84419b336ebf72f81eb84f'
CORE_TOTAL = 16737
COLLECTIONS = {'core', 'canaries', 'placement'}
NON_CORE = {'workflows', 'frameworks', 'q', 'security', 'architecture', 'reliability',
            'booking_resolution', 'native_prerequisites', 'fixture_topology', 'native_media'}
REQUIREMENT_SOURCES = {
    'completion': 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md',
    'cookies': 'docs/oryn2/PARALLEL-AUTHORITY-STORAGE-2026-09-18.md',
    'features': 'docs/oryn2/PARALLEL-FAMILY-QUALIFICATION-2026-09-18.md',
    'executor': 'tools/wptrunner-oryn/README.md',
    'profile_scope': 'docs/oryn2/CORE-PROFILE-SCOPE-2026-09-18.md',
}
IDENTITY_FIELDS = ('candidate_sha256', 'registry_sha256', 'profile_id', 'profile_sha256',
                   'envelope_sha256', 'run_id', 'collection_id', 'attempt_id',
                   'original_run_id', 'platform')
BASE_FIELDS = tuple(key for key in IDENTITY_FIELDS if key != 'collection_id')
PLATFORM_OS = {'macOS': 'mac', 'Linux': 'linux', 'Windows': 'win'}
PLUGIN = ROOT / 'tools/wptrunner-oryn'
if str(PLUGIN) not in sys.path: sys.path.insert(0, str(PLUGIN))


def object_file(path):
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict):
        raise ValueError('Core profile input must be an object')
    return value


def identifier(value):
    return isinstance(value, str) and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,127}', value) is not None


def sha(value):
    return isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None


def reference(path):
    path = Path(path).resolve()
    return {'path': str(path), 'sha256': digest(path)}


def checked_record(record):
    if not isinstance(record, dict) or set(record) != {'path', 'sha256'}:
        raise ValueError('Core profile input needs an exact path/SHA-256 record')
    return read_record(record)


def record_inputs(value, *, checkout=None):
    """Preserve asserted hashes through the transitive JSON evidence graph."""
    result, visited = {}, set()
    checkout = Path(checkout).resolve() if checkout is not None else None
    def source_record(name, expected):
        path = (checkout/ name).resolve()
        if checkout not in path.parents:
            raise ValueError('Core source evidence escapes pinned checkout')
        visit({'path': str(path), 'sha256': expected})
    def visit(item):
        if isinstance(item, dict):
            if set(item) == {'path', 'sha256'} and isinstance(item['path'], str) and Path(item['path']).is_absolute():
                path = checked_record(item); key = str(path)
                if key in result and result[key]['sha256'] != item['sha256']:
                    raise ValueError('conflicting hashes for one Core input')
                result[key] = dict(item)
                if key not in visited:
                    visited.add(key)
                    # Artifact JSON may use any filename. Binary/source files
                    # stay bound but are not interpreted as evidence graphs.
                    with path.open('rb') as stream:
                        prefix = stream.read(256).lstrip()
                    if prefix.startswith((b'{', b'[')):
                        try:
                            nested = json.loads(path.read_text())
                        except (UnicodeError, ValueError):
                            nested = None
                        visit(nested)
                    checked_record(item)
            else:
                for key, child in item.items():
                    if key == 'source_files' and isinstance(child, dict) and checkout is not None:
                        for name, expected in child.items(): source_record(name, expected)
                    elif key == 'evidence_artifacts' and isinstance(child, dict):
                        for name, expected in child.items():
                            visit(expected if isinstance(expected, dict) else {'path': name, 'sha256': expected})
                    elif key in ('source', 'helpers') and checkout is not None:
                        for source in child if isinstance(child, list) else [child]:
                            if isinstance(source, dict) and isinstance(source.get('path'), str) and sha(source.get('sha256')):
                                source_record(source['path'], source['sha256'])
                        visit(child)
                    else: visit(child)
        elif isinstance(item, list):
            for child in item: visit(child)
    visit(value)
    for record in result.values(): checked_record(record)
    return list(result.values())


def profile_fields(fixture):
    from oryn_wptrunner import cookie_profile, feature_profile
    features, cookies = feature_profile.from_fixture(fixture), cookie_profile.from_fixture(fixture)
    return {'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features),
            'cookie_profile': cookies, 'cookie_profile_sha256': cookie_profile.digest(cookies),
            'feature_environment_sha256': feature_profile.environment_digest(fixture),
            'fixture_environment_sha256': cookie_profile.environment_digest(fixture)}


def compare_axis(left, right, axis):
    key = {'cookies': 'third_party_cookies', 'features': 'web_features'}[axis]
    allowed = {'cookies': {'allow', 'block'}, 'features': {'standard', 'legacy'}}[axis]
    modes = {value[key] if axis == 'cookies' else value[key]['usermedia_element'] for value in (left, right)}
    if modes != allowed or {k: v for k, v in left.items() if k != key} != {k: v for k, v in right.items() if k != key}:
        raise ValueError('required ' + axis + ' pair changes another fixture setting or lacks both policies')


def validate_registry(candidate, *, required=False):
    """Validate a candidate-bound registry without loading the candidate again."""
    records = candidate['record']['artifacts']
    record = records.get('core_profile_registry')
    if record is None:
        if required: raise ValueError('required Core profile registry is undeclared')
        return None
    path = checked_record(record)
    value = object_file(path)
    keys = {'schema_version', 'registry_id', 'revision', 'previous_registry', 'scope_review',
            'upstream_revision', 'manifest_sha256', 'policy_sha256', 'coverage_sha256', 'core_total',
            'primary_profile_id', 'profiles', 'required_profile_ids', 'required_pairs', 'non_core_claims'}
    if set(value) != keys or type(value['schema_version']) is not int or value['schema_version'] != 1:
        raise ValueError('Core profile registry schema/fields differ')
    if not identifier(value['registry_id']) or type(value['revision']) is not int or value['revision'] < 1:
        raise ValueError('Core profile registry revision identity invalid')
    if (value['upstream_revision'] != REVISION or value['manifest_sha256'] != MANIFEST_SHA256
            or value['policy_sha256'] != POLICY_SHA256 or not sha(value['coverage_sha256'])
            or type(value['core_total']) is not int or value['core_total'] != CORE_TOTAL):
        raise ValueError('Core profile registry changes pinned population')
    current, seen, history_inputs = value, {str(path)}, []
    while current['revision'] > 1:
        prior_record = current.get('previous_registry')
        prior_path = checked_record(prior_record)
        history_inputs.append(dict(prior_record))
        if str(prior_path) in seen: raise ValueError('Core registry history cycle')
        seen.add(str(prior_path)); prior = object_file(prior_path)
        if prior.get('registry_id') != value['registry_id'] or prior.get('revision') != current['revision'] - 1:
            raise ValueError('Core registry review history is discontinuous')
        current = prior
    if current.get('previous_registry') is not None:
        raise ValueError('initial Core registry must have no previous revision')
    profiles, required_ids = value['profiles'], value['required_profile_ids']
    if (not isinstance(profiles, dict) or not profiles or any(not identifier(key) for key in profiles)
            or not isinstance(required_ids, list) or any(not identifier(name) for name in required_ids) or len(required_ids) != len(set(required_ids))
            or set(required_ids) != set(profiles) or value['primary_profile_id'] not in profiles):
        raise ValueError('Core registry requires an exact unique declared profile set')
    review = value['scope_review']
    if (not isinstance(review, dict) or set(review) != {'author', 'reviewer', 'evidence', 'requirement_sources'}
            or any(not isinstance(review.get(k), str) or not review[k].strip() for k in ('author', 'reviewer'))
            or review['author'].strip().casefold() == review['reviewer'].strip().casefold()):
        raise ValueError('Core scope requires a distinct independent reviewer')
    sources = review['requirement_sources']
    if not isinstance(sources, dict) or set(sources) != set(REQUIREMENT_SOURCES):
        raise ValueError('Core scope omits pinned policy requirements')
    for name, relative in REQUIREMENT_SOURCES.items():
        if checked_record(sources[name]) != (ROOT / relative).resolve():
            raise ValueError('Core scope requirement source substituted: ' + name)
    decision = object_file(checked_record(review['evidence']))
    body = copy.deepcopy(value)
    del body['scope_review']['evidence']
    body_sha = hashlib.sha256(json.dumps(body, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    expected_decision = dict(schema_version=1, status='approved', author=review['author'],
                             reviewer=review['reviewer'], registry_body_sha256=body_sha)
    if decision != expected_decision:
        raise ValueError('independent Core scope review does not approve this exact registry selection')
    fixtures = {}
    from wpt_fixture_contract import validate as validate_fixture
    for name, profile in profiles.items():
        if (not isinstance(profile, dict) or set(profile) != {'fixture', 'platform', 'processor', 'execution_policy'}
                or profile['platform'] not in PLATFORM_OS or profile['processor'] not in {'aarch64', 'x86_64'}
                or profile['execution_policy'] != {'scope': 'entire_run', 'renderer': 'native'}):
            raise ValueError('Core profile has unsupported/per-test execution policy')
        fixture_path = checked_record(profile['fixture']); fixture = object_file(fixture_path)
        validate_fixture(fixture, fixture_path.parent)
        fixtures[name] = fixture
    primary = value['primary_profile_id']
    if profiles[primary]['fixture'] != records.get('fixture_contract'):
        raise ValueError('primary Core profile differs from production fixture')
    if (fixtures[primary]['web_features'] != {'usermedia_element': 'standard'}
            or fixtures[primary]['third_party_cookies'] != 'block'):
        raise ValueError('primary profile differs from source-owned non-Core standard/block defaults')
    pairs = value['required_pairs']
    if not isinstance(pairs, dict) or set(pairs) != {'cookies', 'features'}:
        raise ValueError('both required cookie and feature profile pairs must be declared')
    for axis, names in pairs.items():
        if not isinstance(names, list) or len(names) != 2 or any(not identifier(name) for name in names) or len(set(names)) != 2 or not set(names) <= set(profiles):
            raise ValueError('required Core profile pair is missing or ambiguous')
        a, b = names
        if {k: v for k, v in profiles[a].items() if k != 'fixture'} != {k: v for k, v in profiles[b].items() if k != 'fixture'}:
            raise ValueError('required pair changes platform or execution policy')
        compare_axis(fixtures[a], fixtures[b], axis)
    claims = value['non_core_claims']
    if not isinstance(claims, dict) or set(claims) != NON_CORE or any(v != primary for v in claims.values()):
        raise ValueError('non-Core evidence may claim only its executed production profile')
    if 'core_disposition_policy' not in records:
        raise ValueError('Core registry requires a frozen reviewed-disposition policy')
    checked_record(records['core_disposition_policy'])
    # Recheck nested records and registry after interpreting review and fixtures.
    inputs = record_inputs(value) + history_inputs + [dict(record), dict(records['core_disposition_policy'])]
    for name, profile in profiles.items():
        for material in fixtures[name]['tls'].values():
            inputs.append({'path': str((Path(profile['fixture']['path']).parent / material['path']).resolve()), 'sha256': material['sha256']})
    inputs += record_inputs(records['core_disposition_policy'])
    for item in inputs: checked_record(item)
    if reference(path) != record: raise ValueError('Core profile registry changed while reading')
    return {'path': path, 'sha256': record['sha256'], 'value': value, 'fixtures': fixtures, 'inputs': inputs}


def select_profile(candidate, profile_id):
    registry = validate_registry(candidate, required=True)
    if profile_id not in registry['value']['required_profile_ids']:
        raise ValueError('Core profile is not in the reviewed required set')
    profile = registry['value']['profiles'][profile_id]
    return {'candidate_sha256': candidate['sha256'], 'registry_sha256': registry['sha256'],
            'profile_id': profile_id, 'profile_sha256': profile['fixture']['sha256'],
            'platform': profile['platform'], 'processor': profile['processor'],
            'fixture_path': profile['fixture']['path'], 'fixture': registry['fixtures'][profile_id],
            'registry_path': str(registry['path']), 'registry': registry, **profile_fields(registry['fixtures'][profile_id])}


def stable_path(value):
    if not isinstance(value, str) or not Path(value).is_absolute() or str(Path(value).resolve()) != value:
        raise ValueError('Core envelope requires canonical absolute output paths')
    return Path(value)


def resolve_envelope(path, candidate=None):
    from completion_provenance import load_candidate
    path = Path(path).resolve(); before = reference(path); value = object_file(path)
    keys = {'schema_version', 'upstream_revision', 'candidate', 'profile_id', 'core_profile_registry_sha256',
            'fixture_sha256', 'run_id', 'attempt_id', 'original_run_id', 'collection_ids', 'prepared', 'outputs'}
    if set(value) != keys or type(value.get('schema_version')) is not int or value['schema_version'] != 2 or value['upstream_revision'] != REVISION:
        raise ValueError('Core run envelope schema/revision mismatch')
    candidate_path = checked_record(value['candidate'])
    if candidate is None: candidate = load_candidate(candidate_path)
    if candidate['path'] != candidate_path or candidate['sha256'] != value['candidate']['sha256']:
        raise ValueError('Core run envelope candidate changed')
    selected = select_profile(candidate, value['profile_id'])
    if value['core_profile_registry_sha256'] != selected['registry_sha256'] or value['fixture_sha256'] != selected['profile_sha256']:
        raise ValueError('Core run envelope registry/fixture differs')
    if any(not identifier(value[k]) for k in ('run_id', 'attempt_id', 'original_run_id')):
        raise ValueError('Core run envelope original/attempt identity missing')
    ids, outputs = value['collection_ids'], value['outputs']
    if (not isinstance(ids, dict) or set(ids) != COLLECTIONS or any(not identifier(v) for v in ids.values())
            or len(set(ids.values())) != len(COLLECTIONS) or not isinstance(outputs, dict) or set(outputs) != COLLECTIONS):
        raise ValueError('Core run envelope collection identities incomplete or duplicate')
    paths = [stable_path(value['prepared']), *[stable_path(v) for v in outputs.values()]]
    if len(set(paths)) != len(paths) or any(a in b.parents for a in paths for b in paths if a != b):
        raise ValueError('Core envelope output directories overlap')
    if any(p == path or p in path.parents for p in paths):
        raise ValueError('Core run envelope would be overwritten by execution')
    if reference(path) != before: raise ValueError('Core run envelope changed while reading')
    selected.update(envelope_sha256=before['sha256'], run_id=value['run_id'], attempt_id=value['attempt_id'],
                    original_run_id=value['original_run_id'], envelope=value, envelope_path=str(path),
                    candidate=candidate)
    return selected


def identity(envelope, collection):
    result = {key: envelope[key] for key in BASE_FIELDS}
    result['collection_id'] = envelope['envelope']['collection_ids'][collection]
    return result


def check_identity(record, expected, *, prefix=''):
    if any(record.get(prefix + key) != expected[key] for key in IDENTITY_FIELDS):
        raise ValueError('Core profile/envelope identity changed')


def verify_platform(info, selected):
    processor = {'arm64': 'aarch64', 'AMD64': 'x86_64'}.get(info.get('processor'), info.get('processor'))
    if info.get('os') != PLATFORM_OS[selected['platform']] or processor != selected['processor']:
        raise ValueError('Core report platform differs from declared profile')


def validate_inventory_binding(registry, manifest, policy, coverage):
    value = registry['value']
    for key, path in (('manifest_sha256', manifest), ('policy_sha256', policy), ('coverage_sha256', coverage)):
        if digest(path) != value[key]: raise ValueError('Core registry inventory binding differs: ' + key)


def load_profile_context(candidate_path, profile_id, report_paths):
    from completion_provenance import load_candidate
    from wpt_execution_evidence import verify
    candidate = load_candidate(candidate_path); selected = select_profile(candidate, profile_id)
    if not report_paths: raise ValueError('required profile lacks its original full Core collection')
    contexts, executions, inputs = {}, [], list(selected['registry']['inputs'])
    originals = []
    for path in map(Path, report_paths):
        original_records = [reference(path), reference(path.parent/'execution.json')]
        execution = verify(path, path.parent/'execution.json', candidate['sha256'], 'core', profile_id=profile_id)
        for original_record in original_records: checked_record(original_record)
        context = execution['profile_context']; report_sha = original_records[0]['sha256']
        inputs += original_records
        if report_sha in contexts: raise ValueError('duplicate original Core report')
        contexts[report_sha] = context; executions.append(execution)
        if context['run_id'] == context['original_run_id']: originals.append(report_sha)
        preflight = object_file(read_record(execution['preflight']))
        lock = object_file(read_record(preflight['workflow_lock']))
        inputs += record_inputs(preflight)
        inputs += [{'path': p, 'sha256': h} for p, h in lock['inputs'].items()]
        for name, expected in execution['artifacts'].items():
            file = path.parent/name
            if file.parent != path.parent: raise ValueError('Core evidence artifact escapes original run')
            inputs.append({'path': str(file.resolve()), 'sha256': expected})
    if len(originals) != 1: raise ValueError('profile requires exactly one designated original full Core collection')
    original = contexts[originals[0]]
    for field in ('run_id', 'attempt_id', 'collection_id', 'envelope_sha256'):
        if len({c[field] for c in contexts.values()}) != len(contexts):
            raise ValueError('Core attempt/envelope identity reused across reports')
    if any(c['original_run_id'] != original['run_id'] for c in contexts.values()):
        raise ValueError('Core retry omits or substitutes the designated original')
    for record in inputs: checked_record(record)
    # Re-load after all report custody reads; no stale parsed candidate survives.
    if load_candidate(candidate_path)['sha256'] != candidate['sha256']:
        raise ValueError('Core candidate changed during profile consumption')
    return {**original, 'reports': contexts, 'original_report_sha256': originals[0],
            'registry_path': selected['registry_path'], 'fixture_path': selected['fixture_path'],
            'disposition_policy_path': str(candidate['artifacts']['core_disposition_policy']),
            'coverage_sha256': selected['registry']['value']['coverage_sha256'],
            'profiles': {name: profile['fixture']['sha256'] for name, profile in selected['registry']['value']['profiles'].items()},
            'inputs': list({r['path']: r for r in inputs}.values())}


def validate_prerequisite_equivalence(candidate):
    """Only configuration-independent native/TLS receipts may be shared.

    The existing probes test TLS/topology/focus, not cookie or legacy-element
    semantics. Every other fixture field and platform must equal the primary.
    """
    registry = validate_registry(candidate, required=True); value = registry['value']
    primary = value['primary_profile_id']; base = value['profiles'][primary]
    def relevant(fixture):
        return {k: v for k, v in fixture.items() if k not in {'web_features', 'third_party_cookies'}}
    for name, profile in value['profiles'].items():
        if (profile['platform'] != base['platform'] or profile['processor'] != base['processor']
                or profile['execution_policy'] != base['execution_policy']
                or relevant(registry['fixtures'][name]) != relevant(registry['fixtures'][primary])):
            raise ValueError('profile needs separately admitted native/topology prerequisites: ' + name)
    return {'source': reference(__file__), 'registry_sha256': registry['sha256'],
            'primary_profile_id': primary, 'profile_ids': value['required_profile_ids'],
            'excluded_semantic_axes': ['web_features', 'third_party_cookies']}


def validate_plan(plan, candidate):
    registry = validate_registry(candidate, required=True)
    if type(plan.get('schema_version')) is not int or plan['schema_version'] != 2:
        raise ValueError('current complete qualification requires a schema2 per-profile plan')
    core = plan.get('core')
    if not isinstance(core, dict) or set(core) != {'inventory', 'manifest', 'policy', 'checkout', 'profiles'}:
        raise ValueError('complete per-profile Core plan is missing common pinned inputs')
    for key in ('inventory', 'manifest', 'policy', 'checkout'): stable_path(core[key])
    profiles = core['profiles']
    if not isinstance(profiles, dict) or set(profiles) != set(registry['value']['required_profile_ids']):
        raise ValueError('Core plan must contain exactly every reviewed required profile')
    owned = []
    keys = {'reports', 'reviews', 'assessments', 'attribution', 'disposition_applications',
            'run_contract', 'prepared', 'canaries', 'placement'}
    for name, profile in profiles.items():
        if not isinstance(profile, dict) or set(profile) != keys:
            raise ValueError('Core profile plan lacks original/assessment/control evidence: ' + name)
        if not isinstance(profile['reports'], list) or not profile['reports']:
            raise ValueError('Core profile needs its full original report')
        for key in ('reviews', 'assessments', 'attribution', 'disposition_applications', 'run_contract', 'prepared'):
            stable_path(profile[key])
        for path in profile['reports']: stable_path(path)
        if (not isinstance(profile['canaries'], dict) or set(profile['canaries']) != {'directory', 'audit'}
                or not isinstance(profile['placement'], dict) or set(profile['placement']) != {'directory'}):
            raise ValueError('each Core profile requires original canary/placement evidence')
        for path in (*profile['canaries'].values(), *profile['placement'].values()): stable_path(path)
        owned.extend(profile['reports'])
        owned.extend(profile[key] for key in ('reviews', 'assessments', 'disposition_applications', 'run_contract', 'prepared'))
        owned.extend((profile['canaries']['directory'], profile['canaries']['audit'], profile['placement']['directory']))
    if len(owned) != len(set(owned)):
        raise ValueError('Core profiles cannot share original/assessment/envelope/control evidence paths')
    if plan.get('non_core_scope') != registry['value']['non_core_claims']:
        raise ValueError('non-Core policy scope differs from actual primary production profile')
    validate_inventory_binding(registry, core['manifest'], core['policy'], Path(core['inventory'])/'coverage.jsonl')
    return registry


def validate_profile_ledger(summary, rows, core_ids, context):
    """Check each complete ledger independently; no pass-preferred union."""
    from collections import Counter
    if (len(core_ids) != CORE_TOTAL or len(rows) != CORE_TOTAL
            or len({row.get('id') for row in rows}) != CORE_TOTAL
            or {row.get('id') for row in rows} != set(core_ids)):
        raise ValueError('each required profile must retain all 16,737 unique Core entries')
    check_identity(summary, context)
    if 'reports' in context and (summary.get('report_identities') != context['reports']
            or summary.get('original_report_sha256') != context['original_report_sha256']):
        raise ValueError('profile summary omits or changes original/retry identity')
    counts = dict(Counter(row.get('obligation_state') for row in rows))
    if (summary.get('schema_version') != 2 or summary.get('core_total') != CORE_TOTAL
            or summary.get('automatic_total') != 15985 or summary.get('assessment_total') != 752
            or summary.get('counts') != counts or summary.get('complete') is not True
            or not set(counts) <= {'qualified_pass', 'satisfied_by_reviewed_disposition'}):
        raise ValueError('required profile remains independently incomplete')
    for row in rows:
        state = row['obligation_state']
        if row.get('state') != state or row.get('qualified_pass') is not (state == 'qualified_pass'):
            raise ValueError('Core ledger confuses raw qualification with reviewed disposition')
        if type(row.get('raw_pass')) is not bool:
            raise ValueError('Core ledger must retain raw pass status')
        if state == 'satisfied_by_reviewed_disposition' and row['raw_pass'] is not False:
            raise ValueError('reviewed disposition cannot relabel raw failure')
    observed = {'raw_pass_count': sum(row['raw_pass'] for row in rows),
                'qualified_pass_count': counts.get('qualified_pass', 0),
                'reviewed_disposition_count': counts.get('satisfied_by_reviewed_disposition', 0),
                'open_count': 0}
    if any(summary.get(key) != value for key, value in observed.items()):
        raise ValueError('profile summary does not match distinct ledger accounting')
    return {'counts': counts, **observed}


def validate_core_aggregate(aggregate, candidate):
    registry = validate_registry(candidate, required=True)
    plan_path = candidate['artifacts'].get('qualification_plan')
    if plan_path is None: raise ValueError('profile aggregate needs the frozen qualification plan')
    plan = object_file(plan_path); validate_plan(plan, candidate)
    value = registry['value']; profiles = aggregate.get('profiles')
    if (aggregate.get('schema_version') != 2 or aggregate.get('aggregation') != 'independent_required_profiles'
            or aggregate.get('candidate_sha256') != candidate['sha256']
            or aggregate.get('registry_sha256') != registry['sha256']
            or aggregate.get('core_total_per_profile') != CORE_TOTAL
            or aggregate.get('profile_count') != len(value['required_profile_ids'])
            or aggregate.get('required_profile_ids') != value['required_profile_ids']
            or aggregate.get('primary_profile_id') != value['primary_profile_id']
            or not isinstance(profiles, dict) or set(profiles) != set(value['required_profile_ids'])
            or aggregate.get('complete') is not True):
        raise ValueError('Core aggregate does not independently bind every required profile')
    from wpt_full_qualification import validate_inventory
    folder = Path(plan['core']['inventory'])
    core = validate_inventory([json.loads(line) for line in (folder/'coverage.jsonl').read_text().splitlines()],
                              object_file(folder/'plan.json'))
    for name, member in profiles.items():
        summary_path, ledger_path = checked_record(member['summary']), checked_record(member['ledger'])
        summary = object_file(summary_path)
        if summary.get('ledger_sha256') != member['ledger']['sha256']:
            raise ValueError('Core profile summary names a different ledger')
        context = load_profile_context(candidate['path'], name, plan['core']['profiles'][name]['reports'])
        rows = [json.loads(line) for line in ledger_path.read_text().splitlines()]
        observed = validate_profile_ledger(summary, rows, core, context)
        if (member.get('profile_sha256') != context['profile_sha256']
                or member.get('original_report_sha256') != context['original_report_sha256']):
            raise ValueError('Core aggregate substitutes a profile or original report')
        if any(member.get(key) != expected for key, expected in observed.items()):
            raise ValueError('Core aggregate conceals per-profile accounting')
        for path, expected in summary.get('inputs', {}).items():
            checked_record({'path': path, 'sha256': expected})
    primary = profiles[value['primary_profile_id']]
    raw_primary = primary['raw_pass_count'] == 15985 and primary['reviewed_disposition_count'] == 0
    if aggregate.get('non_core_scope') != value['non_core_claims']:
        raise ValueError('Core aggregate changes non-Core policy scope')
    if aggregate.get('errors') != []:
        raise ValueError('Core aggregate retains unresolved profile failures')
    if aggregate.get('single_profile_conformance') is not raw_primary:
        raise ValueError('Core aggregate derives conformance from another profile or disposition')
    return aggregate

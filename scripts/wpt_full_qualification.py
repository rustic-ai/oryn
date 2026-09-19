"""Full-manifest attribution for the official executor; raw evidence is immutable.

This is deliberately separate from legacy runner qualification: executor identity,
upstream report semantics and prerequisite reviews cannot be silently exchanged.
"""
from completion_provenance import render_contracts, validate_render_contracts
from collections import Counter
import hashlib
import json
from pathlib import Path

REVISION = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
MANIFEST_SHA256 = '566f2c77524c69cc249c5fe18507ad4c11866d9077b201681b19807c4ee28101'
POLICY_SHA256 = 'a655bd38bbd3cc90e49344f7ceae1db1a3bfb09c8b84419b336ebf72f81eb84f'
CORE_TOTAL = 16737
EXECUTORS = {'testharness', 'reftest', 'crashtest', 'print-reftest'}
OWNERS = {
    'html': ['R', 'L', 'S', 'V'],
    **dict.fromkeys(('dom', 'domparsing', 'domxpath', 'xml', 'webidl'), ['S', 'L']),
    **dict.fromkeys(('encoding', 'encoding-detection', 'url', 'mimesniff'), ['L']),
    **dict.fromkeys(('fetch', 'xhr', 'cors', 'websockets', 'FileAPI', 'streams'), ['L', 'W']),
    **dict.fromkeys(('custom-elements', 'shadow-dom', 'import-maps', 'webmessaging'), ['S', 'L', 'W']),
    **dict.fromkeys(('selection', 'focus', 'inert', 'uievents', 'input-events', 'accname', 'wai-aria', 'html-aam'), ['S', 'V']),
    'css': ['V'],
    **dict.fromkeys(('content-security-policy', 'referrer-policy', 'mixed-content',
                    'upgrade-insecure-requests', 'permissions-policy', 'trusted-types',
                    'secure-contexts', 'x-frame-options', 'subresource-integrity',
                    'cookies', 'webstorage'), ['L', 'P', 'W']),
}


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def unique(rows, key, label):
    result = {}
    for row in rows:
        value = row[key]
        if value in result:
            raise ValueError('duplicate ' + label + ': ' + value)
        result[value] = row
    return result


def validate_inventory(coverage, plan):
    for key, expected in [('upstream_revision', REVISION),
                          ('official_manifest_sha256', MANIFEST_SHA256),
                          ('policy_sha256', POLICY_SHA256)]:
        if plan.get(key) != expected:
            raise ValueError('pinned inventory identity mismatch: ' + key)
    all_rows = unique(coverage, 'id', 'manifest ID')
    if plan.get('total_manifest_tests') != len(all_rows):
        raise ValueError('full inventory count mismatch')
    core = {key: row for key, row in all_rows.items() if row['tier'] == 'core'}
    if len(core) != CORE_TOTAL:
        raise ValueError('Core denominator changed')
    for key, row in core.items():
        if (key != row['test_type'] + ':' + row['url'] or row['root'] not in OWNERS
                or row['root'] != row['path'].split('/', 1)[0]):
            raise ValueError('unattributed manifest entry: ' + key)
    return core


def bind_inventory(core, manifest_path, policy_path, checkout):
    """Check the inventory against actual pinned bytes, including every variant."""
    if sha256(manifest_path) != MANIFEST_SHA256 or sha256(policy_path) != POLICY_SHA256:
        raise ValueError('manifest/policy bytes differ from completion contract')
    import importlib.util
    spec = importlib.util.spec_from_file_location('oryn_full_inventory_source', Path(__file__).with_name('prepare-full-wpt-baseline.py'))
    inventory = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(inventory)
    manifest = json.loads(Path(manifest_path).read_text())
    policy = json.loads(Path(policy_path).read_text())
    if manifest.get('version') != 9:
        raise ValueError('unexpected pinned manifest schema')
    expected = {}
    root = Path(checkout).resolve()
    for kind, tree in manifest['items'].items():
        if kind in {'support', 'spec'}:
            continue
        for path, leaf in inventory.leaves(tree):
            tier, _ = inventory.tiers.classify(path, policy)
            if tier != 'core':
                continue
            source = (root / path).resolve()
            if root not in source.parents:
                raise ValueError('source escapes checkout')
            content = source.read_bytes()
            git_hash = hashlib.sha1(b'blob ' + str(len(content)).encode() + b'\0' + content).hexdigest()
            if git_hash != leaf[0]:
                raise ValueError('test source differs from pinned manifest blob: ' + path)
            digest = hashlib.sha256(content).hexdigest()
            prerequisites = sorted({rule['requirement'] for rule in policy.get('environment_rules', [])
                                    if any(path == prefix or path.startswith(prefix + '/') for prefix in rule['prefixes'])})
            for entry in leaf[1:]:
                url = entry[0] or '/' + path
                identifier = kind + ':' + url
                if identifier in expected:
                    raise ValueError('duplicate pinned manifest ID')
                expected[identifier] = (path, url, kind, digest, prerequisites)
    actual = {key: (row['path'], row['url'], row['test_type'], row['sha256'], row.get('environment_requirements', [])) for key, row in core.items()}
    if actual != expected:
        raise ValueError('coverage does not match actual full pinned Core manifest')


def navigation_outcome_error(item):
    if item.get('stage') != 'Execution':
        return 'native_navigation_admission_is_not_completion'
    if type(item.get('error_truncated')) is not bool or 'execution' not in item:
        return 'native_navigation_outcome_malformed'
    if item.get('target') is not None and not isinstance(item.get('target'), str):
        return 'native_navigation_outcome_malformed'
    result = item.get('result')
    if not isinstance(result, dict) or set(result) not in ({'Ok'}, {'Err'}):
        return 'native_navigation_outcome_malformed'
    if 'Ok' in result and (type(result['Ok']) is not int or not 0 <= result['Ok'] < 2**64):
        return 'native_navigation_outcome_malformed'
    if 'Err' in result and not isinstance(result['Err'], str):
        return 'native_navigation_outcome_malformed'
    execution = item['execution']
    if execution is None:  # Native tasks that never transfer execution custody.
        return None
    if execution == 'Completed':
        expected = {'Ok': 0}
    elif execution == 'Canceled':
        expected = {'Err': 'navigation canceled'}
    elif isinstance(execution, dict) and set(execution) == {'Committed'}:
        committed = execution['Committed']
        if not isinstance(committed, dict) or set(committed) != {'document_generation'} or type(committed['document_generation']) is not int or not 0 <= committed['document_generation'] < 2**64:
            return 'native_navigation_outcome_malformed'
        expected = {'Ok': committed['document_generation']}
    elif isinstance(execution, dict) and set(execution) == {'Failed'}:
        failed = execution['Failed']
        if not isinstance(failed, dict) or set(failed) != {'message'} or not isinstance(failed['message'], str):
            return 'native_navigation_outcome_malformed'
        expected = {'Err': failed['message']}
    else:
        return 'native_navigation_outcome_malformed'
    return None if expected == result else 'native_navigation_execution_mismatch'


def navigation_errors(row, result, review):
    evidence = (result.get('extra') or {}).get('oryn_navigation')
    if not isinstance(evidence, dict) or evidence.get('schema_version') != 1 or evidence.get('test') != row['url'] or not evidence.get('interval_start'):
        return ['native_navigation_evidence_missing']
    snapshots = []
    for phase in ('before', 'after'):
        snapshot = evidence.get(phase)
        if not isinstance(snapshot, dict) or snapshot.get('collection_error') or not isinstance(snapshot.get('pages'), list):
            return ['native_navigation_collection_incomplete:' + phase]
        pages = {}
        for page in snapshot['pages']:
            if not isinstance(page, dict) or type(page.get('page_id')) is not int or page['page_id'] < 0 or page['page_id'] in pages:
                return ['native_navigation_page_identity_invalid']
            value = page.get('snapshot')
            if not isinstance(value, dict) or not isinstance(value.get('outcomes'), list) or type(value.get('active')) is not int or type(value.get('pruned_outcomes')) is not int:
                return ['native_navigation_snapshot_malformed']
            if value['active'] < 0 or value['pruned_outcomes'] < 0 or type(value.get('retired')) is not bool:
                return ['native_navigation_snapshot_malformed']
            identities = set()
            for outcome in value['outcomes']:
                if not isinstance(outcome, dict) or type(outcome.get('id')) is not int or outcome['id'] <= 0 or outcome['id'] in identities:
                    return ['native_navigation_outcome_identity_invalid']
                error = navigation_outcome_error(outcome)
                if error:
                    return [error]
                identities.add(outcome['id'])
            if value['active'] != 0:
                return ['native_navigation_nonterminal:' + phase]
            pages[page['page_id']] = value
        snapshots.append(pages)
    before, after = snapshots
    if not set(before) <= set(after):
        return ['native_navigation_retired_history_missing']
    failures = []
    for page, value in after.items():
        previous = before.get(page, {'outcomes': [], 'pruned_outcomes': 0})
        if value['pruned_outcomes'] != previous['pruned_outcomes']:
            return ['native_navigation_history_pruned']
        old = {item['id']: item for item in previous['outcomes']}
        seen = set()
        for item in value['outcomes']:
            if not isinstance(item, dict) or type(item.get('id')) is not int or item['id'] in seen:
                return ['native_navigation_outcome_identity_invalid']
            seen.add(item['id'])
            if item['id'] in old:
                if item != old[item['id']]:
                    return ['native_navigation_outcome_rewritten']
                continue
            if item.get('error_truncated') is not False:
                return ['native_navigation_error_truncated']
            outcome = item.get('result')
            if not isinstance(outcome, dict) or set(outcome) not in ({'Ok'}, {'Err'}):
                return ['native_navigation_outcome_malformed']
            if 'Err' in outcome:
                failures.append({'page_id': page, **item})
        if not set(old) <= seen:
            return ['native_navigation_outcomes_missing']
    expected = review.get('expected_navigation_errors', [])
    if expected != failures:
        return ['native_navigation_failure_review_mismatch']
    if failures and not review.get('navigation_error_source_evidence'):
        return ['native_navigation_expected_error_source_missing']
    return []


def review_errors(row, result, review, report_hash, candidate, checkout):
    """Per-entry independent review is required, even for an upstream raw PASS."""
    if not review:
        return ['independent_prerequisite_review_missing']
    errors = []
    for key, expected in [('id', row['id']), ('report_sha256', report_hash),
                          ('candidate_sha256', candidate), ('source_sha256', row['sha256'])]:
        if review.get(key) != expected:
            errors.append('review_identity_mismatch:' + key)
    for field in ('reviewer', 'fixture_audit', 'automation_audit', 'cleanup_evidence',
                  'negative_control_evidence', 'resource_audit'):
        if not isinstance(review.get(field), str) or not review[field].strip():
            errors.append('review_missing:' + field)
    for field in ('tls_validation_verified', 'focus_verified', 'resources_complete',
                  'cleanup_complete', 'negative_controls_rejected'):
        if review.get(field) is not True:
            errors.append('prerequisite_unverified:' + field)
    artifacts = review.get('evidence_artifacts') or {}
    for label in ('cleanup', 'negative_controls', 'resources', 'tls', 'focus'):
        artifact = artifacts.get(label) or {}
        path = Path(artifact.get('path', ''))
        if not path.is_file() or sha256(path) != artifact.get('sha256'):
            errors.append('prerequisite_artifact_unbound:' + label)
    sources = review.get('source_files') or {}
    if sources.get(row['path']) != row['sha256']:
        errors.append('test_source_unbound')
    root = Path(checkout).resolve()
    for name, digest in sources.items():
        path = (root / name).resolve()
        if root not in path.parents or not path.is_file() or sha256(path) != digest:
            errors.append('source_hash_mismatch:' + name)
    errors.extend(navigation_errors(row, result, review))
    observed = result.get('subtests', [])
    if not isinstance(observed, list) or any(not isinstance(item, dict) for item in observed):
        return errors + ['malformed_subtests']
    if row['test_type'] == 'testharness':
        names = [item.get('name') for item in observed]
        expected = review.get('expected_subtests')
        if (not names or any(not isinstance(name, str) for name in names)
                or len(set(names)) != len(names) or expected != names):
            errors.append('subtest_identity_or_order_mismatch')
    if row['test_type'] in {'reftest', 'print-reftest'}:
        if review.get('native_reference_rendering') is not True or review.get('reference_readiness_verified') is not True:
            errors.append('native_reference_readiness_unverified')
        if not review.get('comparison_evidence'):
            errors.append('comparison_evidence_missing')
    for prerequisite in row.get('environment_requirements', []):
        if not (review.get('environment_evidence') or {}).get(prerequisite):
            errors.append('environment_unverified:' + prerequisite)
    return errors


def assess(row, assessment, candidate, checkout, profile_context=None):
    """Independent manual/visual procedure; never converts an automated failure."""
    from wpt_manual_procedures import assessment_errors
    errors = assessment_errors(row, assessment, checkout)
    for key, expected in [('id', row['id']), ('candidate_sha256', candidate),
                          ('source_sha256', row['sha256']), ('upstream_revision', REVISION)]:
        if assessment.get(key) != expected:
            errors.append('assessment_identity_mismatch:' + key)
    if profile_context is not None:
        from wpt_reviewed_dispositions import profile_identity
        if assessment.get('profile_identity') != profile_identity(profile_context, candidate):
            errors.append('assessment_profile_identity_mismatch')
        if assessment.get('platform') != profile_context['platform']:
            errors.append('assessment_profile_platform_mismatch')
    if assessment.get('independent') is not True:
        errors.append('independent_assessment_missing')
    for key in ('reviewer', 'operator', 'platform', 'procedure', 'observations', 'assistive_technology'):
        if not isinstance(assessment.get(key), str) or not assessment[key].strip():
            errors.append('assessment_missing:' + key)
    if assessment.get('reviewer') == assessment.get('operator'):
        errors.append('assessment_reviewer_not_independent')
    source = (Path(checkout).resolve() / row['path']).resolve()
    if Path(checkout).resolve() not in source.parents or not source.is_file() or sha256(source) != row['sha256']:
        errors.append('assessment_source_mismatch')
    artifacts = assessment.get('artifacts') or []
    if not artifacts:
        errors.append('assessment_artifacts_missing')
    for artifact in artifacts:
        path = Path(artifact['path'])
        if not path.is_file() or sha256(path) != artifact.get('sha256'):
            errors.append('assessment_artifact_mismatch')
    if assessment.get('cleanup_complete') is not True:
        errors.append('assessment_cleanup_unverified')
    return {'raw_status': assessment.get('status'), 'record': assessment,
            'qualification_errors': errors,
            'qualified_pass': assessment.get('status') == 'PASS' and not errors}


def reconcile(core, reports, reviews, candidate, checkout, attribution=None, assessments=None, *,
              profile_context=None, disposition_policy=None, disposition_applications=None):
    """reports contains (raw report path, parsed upstream wptreport) pairs.

    Repetitions must be separately reviewed. Every attempt is retained; a failed
    earlier attempt remains open even if a later attempt passes.
    """
    if len(candidate) != 64 or any(c not in '0123456789abcdef' for c in candidate):
        raise ValueError('candidate must be a SHA-256 identity')
    from wpt_reviewed_dispositions import IDENTITY_FIELDS, SATISFIED_STATES, profile_identity
    strict_profile = profile_context is not None
    input_hashes = {}
    input_destinations = {}
    def merge_inputs(additions):
        for name, expected in additions.items():
            if name in input_hashes and input_hashes[name] != expected:
                raise ValueError('profile/disposition input identity changed: ' + name)
            input_hashes[name] = expected
    if strict_profile:
        profile_identity(profile_context, candidate)
        if len(core) != CORE_TOTAL:
            raise ValueError('per-profile Core denominator changed')
        for record in profile_context.get('inputs', []):
            from completion_provenance import read_record
            path = read_record(record)
            original = record['path']
            if original in input_destinations and input_destinations[original] != str(path):
                raise ValueError('profile input destination changed')
            input_destinations[original] = str(path)
            merge_inputs({original: record['sha256'], str(path): record['sha256']})
        if not input_hashes:
            raise ValueError('profile context has no verified input custody')
        identities = profile_context.get('reports') or {}
        if set(identities) != {sha256(path) for path, _ in reports}:
            raise ValueError('profile report population differs from original custody')
        originals = [key for key,value in identities.items() if value.get('run_id') == value.get('original_run_id')]
        if originals != [profile_context.get('original_report_sha256')]:
            raise ValueError('exactly one designated original Core report required')
        for key in ('run_id', 'attempt_id', 'collection_id', 'envelope_sha256'):
            if len({value.get(key) for value in identities.values()}) != len(identities):
                raise ValueError('duplicate profile attempt identity: ' + key)
        if disposition_policy is None:
            raise ValueError('required-profile qualification needs frozen disposition policy')
        if disposition_applications is None:
            raise ValueError('required-profile qualification needs explicit application file, including empty list')
        if disposition_policy.context != profile_identity(profile_context, candidate):
            raise ValueError('disposition policy profile identity mismatch')
        disposition_policy.load_applications(disposition_applications)
    elif disposition_policy is not None or disposition_applications is not None:
        raise ValueError('dispositions cannot be applied without verified profile context')
    by_url = {}
    for row in core.values():
        by_url.setdefault(row['url'], []).append(row)
    attempts = {key: [] for key in core}
    report_hashes = set()
    for path, report in reports:
        if json.loads(Path(path).read_text()) != report:
            raise ValueError('parsed report differs from preserved raw bytes')
        digest = sha256(path)
        if digest in report_hashes:
            raise ValueError('duplicate report input')
        report_hashes.add(digest)
        info = report.get('run_info') or {}
        if not strict_profile and any('oryn_' + key in info for key in ('profile_id', 'registry_sha256', 'envelope_sha256')):
            raise ValueError('profile report cannot downgrade to legacy reconciliation')
        if (info.get('product') != 'oryn' or info.get('oryn_candidate_sha256') != candidate
                or info.get('oryn_wpt_revision') != REVISION
                or info.get('oryn_executor') != 'upstream-wptrunner'):
            raise ValueError('report executor/candidate identity mismatch')
        validate_render_contracts(info.get('oryn_rendering_contracts') or {}, render_contracts())
        repair_manifest = Path(__file__).resolve().parents[1] / 'tools/wptrunner-oryn/oryn_wptrunner/fixture-repairs.json'
        if info.get('oryn_fixture_repairs_sha256') != sha256(repair_manifest):
            raise ValueError('report fixture repair attribution mismatch')
        report_context = None
        if strict_profile:
            report_context = profile_context['reports'][digest]
            identity = profile_identity(report_context, candidate)
            for field in IDENTITY_FIELDS:
                if info.get('oryn_' + field) != identity[field]:
                    raise ValueError('raw report profile/envelope identity mismatch: ' + field)
            for field in ('registry_sha256', 'profile_id', 'profile_sha256', 'platform', 'original_run_id'):
                if identity[field] != profile_context[field]:
                    raise ValueError('mixed profile or original collection: ' + field)
        rows = unique(report.get('results', []), 'test', 'report URL')
        if strict_profile and set(rows) != {row['url'] for row in core.values() if row['test_type'] in EXECUTORS}:
            raise ValueError('profile original/retry lacks complete automatic Core population')
        for url, result in rows.items():
            matches = by_url.get(url, [])
            if len(matches) != 1:
                raise ValueError('unknown or ambiguous Core report URL: ' + url)
            row = matches[0]
            if row['test_type'] not in EXECUTORS:
                raise ValueError('assessment entry in automatic report: ' + row['id'])
            review = reviews.get(row['id'], {}).get(digest)
            errors = review_errors(row, result, review, digest, candidate, checkout)
            if strict_profile and review:
                for field in IDENTITY_FIELDS:
                    if review.get(field) != report_context[field]:
                        errors.append('review_profile_identity_mismatch:' + field)
            status = result.get('status')
            allowed = {'OK', 'ERROR', 'TIMEOUT', 'CRASH', 'PRECONDITION_FAILED', 'SKIP'} if row['test_type'] == 'testharness' else {'PASS', 'FAIL', 'ERROR', 'TIMEOUT', 'CRASH', 'PRECONDITION_FAILED', 'SKIP'}
            if status not in allowed:
                errors.append('malformed_harness_status')
            subtests = result.get('subtests', [])
            if not isinstance(subtests, list) or any(not isinstance(item, dict) or item.get('status') not in {'PASS', 'FAIL', 'TIMEOUT', 'NOTRUN', 'PRECONDITION_FAILED'} for item in subtests):
                errors.append('malformed_subtests')
                subtests = []
            raw_pass = status == ('OK' if row['test_type'] == 'testharness' else 'PASS') and all(item['status'] == 'PASS' for item in subtests)
            dispositions = []
            if disposition_policy is not None:
                dispositions = disposition_policy.apply(row, result, digest, review, errors, report_context)
            satisfied = (raw_pass and not errors) or bool(dispositions) and not errors
            obligation_state = ('satisfied_by_reviewed_disposition' if dispositions and satisfied
                                else 'qualified_pass' if raw_pass and not errors else 'open')
            attempts[row['id']].append({'report': str(Path(path).resolve()), 'report_sha256': digest,
                'raw_status': status, 'raw_subtests': result.get('subtests'),
                'raw_pass': raw_pass, 'qualification_errors': sorted(set(errors)),
                'qualified_pass': raw_pass and not errors, 'obligation_state': obligation_state,
                'satisfied': satisfied, 'reviewed_dispositions': dispositions,
                **({'profile_identity': report_context} if strict_profile else {})})
    if disposition_policy is not None:
        disposition_policy.finish()
        merge_inputs(disposition_policy.inputs)
    attribution = attribution or {}
    assessments = assessments or {}
    if not set(assessments) <= set(core):
        raise ValueError('unknown assessment ID')
    if any(core[key]['test_type'] in EXECUTORS for key in assessments):
        raise ValueError('manual assessment cannot waive an automated obligation')
    if not set(attribution) <= set(core) or not set(reviews) <= set(core):
        raise ValueError('unknown attribution/review ID')
    output = []
    for key, row in sorted(core.items()):
        history = attempts[key]
        state = ('assessment_required' if row['test_type'] not in EXECUTORS else 'not_run')
        if history:
            if all(a['obligation_state'] in SATISFIED_STATES for a in history):
                state = ('satisfied_by_reviewed_disposition' if any(a['reviewed_dispositions'] for a in history)
                         else 'qualified_pass')
            else:
                state = 'open'
        assessment_history = [assess(row, value, candidate, checkout, profile_context) for value in assessments.get(key, [])]
        if assessment_history:
            state = 'qualified_pass' if all(value['qualified_pass'] for value in assessment_history) else 'open'
        note = attribution.get(key, {})
        output.append({'id': key, 'url': row['url'], 'path': row['path'], 'sha256': row['sha256'],
            'test_type': row['test_type'], 'owners': OWNERS[row['root']],
            'prerequisites': row.get('environment_requirements', []),
            'observed_failure': note.get('observed_failure'), 'confirmed_cause': note.get('confirmed_cause'),
            'implementation_change': note.get('implementation_change'),
            'acceptance_evidence': note.get('acceptance_evidence'),
            'state': state, 'obligation_state': state,
            'raw_pass': bool(history) and all(a['raw_pass'] for a in history),
            'qualified_pass': state == 'qualified_pass',
            'attempts': history, 'assessments': assessment_history})
    if disposition_policy is not None:
        disposition_policy.recheck()
    if any(str(Path(path).resolve()) != destination for path, destination in input_destinations.items()):
        raise ValueError('profile input destination changed during reconciliation')
    if any(not Path(path).is_file() or sha256(path) != expected for path, expected in input_hashes.items()):
        raise ValueError('profile/disposition input changed during reconciliation')
    summary = {'schema_version': 2 if strict_profile else 1, 'candidate_sha256': candidate,
               'executor': 'upstream-wptrunner', 'core_total': len(core),
               'counts': dict(Counter(row['state'] for row in output)),
               'complete': bool(output) and all(row['state'] in SATISFIED_STATES for row in output),
               'raw_attempt_counts': dict(Counter('raw_pass' if attempt['raw_pass'] else 'raw_nonpass'
                                                  for history in attempts.values() for attempt in history)),
               'inputs': input_hashes,
               'scope': 'Core execution and assessment qualification; does not replace combined security/performance/product gates'}
    if strict_profile:
        summary.update({key: profile_context[key] for key in IDENTITY_FIELDS})
        summary.update(original_report_sha256=profile_context['original_report_sha256'],
                       automatic_total=sum(row['test_type'] in EXECUTORS for row in core.values()),
                       assessment_total=sum(row['test_type'] not in EXECUTORS for row in core.values()),
                       disposition_policy_sha256=disposition_policy.sha256,
                       disposition_applications_sha256=disposition_policy.applications_sha256,
                       imported_approved_core_dispositions=disposition_policy.imported_core_count,
                       report_identities=profile_context['reports'],
                       raw_pass_count=sum(row['raw_pass'] for row in output),
                       qualified_pass_count=sum(row['state'] == 'qualified_pass' for row in output),
                       reviewed_disposition_count=sum(row['state'] == 'satisfied_by_reviewed_disposition' for row in output),
                       open_count=sum(row['state'] not in SATISFIED_STATES for row in output))
    return output, summary

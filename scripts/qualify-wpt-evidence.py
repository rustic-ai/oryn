#!/usr/bin/env python3
"""Create a conservative, hash-bound qualification overlay; never rewrite raw results."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import tempfile
from pathlib import Path

from wpt_evidence import completion_errors, diagnostics, resource_integrity_errors, prerequisite_review_errors, resource_key, current_harness_errors


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classify(row, review=None, current=False):
    tags = set(row.get('diagnostic_tags', [])) | set(diagnostics(row))
    result = row.get('result') or {}
    reasons = []
    if row['status'] == 'environment_blocked':
        return 'environment_blocked', [row.get('reason') or 'environment_unavailable']
    if current:
        reasons.extend(current_harness_errors(row))
        if review and 'bypass' in (row.get('fixture') or {}).get('tls_validation', 'bypassed'):
            if review.get('tls_validation_independent') is not True:
                reasons.append('tls_validation_dependency_unreviewed')
    execution = row.get('execution') or {}
    if row.get('id') and not str(execution.get('id', '')).startswith(row['id'] + ':'):
        reasons.append('execution_identity_unverified')
    shutdown = execution.get('parent_shutdown') or {}
    if (execution.get('completion_valid') is not True or execution.get('validation_errors')
            or execution.get('error') or execution.get('cleanup_errors')
            or execution.get('replacement_error')
            or type(shutdown.get('returncode')) is not int
            or shutdown.get('returncode') != 0
            or shutdown.get('shutdown_acknowledged') is not True or shutdown.get('error')):
        reasons.append('execution_integrity_unverified')
    if row.get('resource_evidence_complete') is not True or row.get('resource_error'):
        reasons.append('resource_prerequisites_unverified')
    reasons.extend(resource_integrity_errors(row))
    if completion_errors(result) or type(result.get('completion_count')) is not int or result.get('completion_count') != 1:
        reasons.append('completion_unresolved')
    if row.get('fixture_error_evidence') or 'confirmed_upstream_template_failure' in tags:
        reasons.append('confirmed_fixture_error')
    if 'testdriver_operation_not_implemented' in tags:
        reasons.append('unsupported_automation')
    if tags & {'outer_timeout', 'harness_timeout', 'worker_wall_limit',
               'generated_operation_wall_limit', 'reported_worker_crash'}:
        reasons.append('completion_unresolved')
    if result.get('complete') is not True or result.get('harness') != 0:
        reasons.append('completion_unresolved')
    tests = result.get('tests', [])
    if not tests or any(test.get('status') not in (0, 1) for test in tests):
        reasons.append('subtest_completion_unresolved')
    if tests and all(test.get('status') in (0, 1) for test in tests):
        expected = 'passed' if all(test['status'] == 0 for test in tests) else 'failed'
        if row['status'] != expected:
            reasons.append('outcome_inconsistent')
    if reasons:
        return 'unresolved', sorted(set(reasons))
    if not review:
        return 'review_candidate', ['fixture_and_automation_audit_required']
    if len(tests) != review['expected_subtests']:
        return 'unresolved', ['audited_subtest_count_mismatch']
    prerequisites = prerequisite_review_errors(row, review)
    if prerequisites:
        return 'unresolved', prerequisites
    return 'qualified', []


def build(summary_path, ledger_path, checkout, reviews, current=False):
    summary = json.loads(summary_path.read_text())
    if not summary.get('complete') or digest(ledger_path) != summary['ledger_sha256']:
        raise ValueError('incomplete composite or ledger hash mismatch')
    entries = [json.loads(line) for line in ledger_path.read_text().splitlines()]
    if len(entries) != summary['selected'] or len({e['id'] for e in entries}) != len(entries):
        raise ValueError('composite ID count mismatch or duplicate ID')
    if not set(reviews) <= {e['id'] for e in entries}:
        raise ValueError('review ID absent from composite')
    sources = {}
    output = []
    for entry in entries:
        origin = entry.get('replacement') or entry['original']
        key = (origin['path'], origin['sha256'])
        if key not in sources:
            path = Path(origin['path'])
            if digest(path) != origin['sha256']:
                raise ValueError('source hash mismatch: ' + str(path))
            data = json.loads(path.read_text())
            if current and (data.get('schema_version') != 4
                            or data.get('harness', {}).get('server') != 'direct_wptserve_v1'
                            or not data.get('harness', {}).get('capability_manifest_sha256')):
                raise ValueError('current qualification requires direct-fixture schema-v4 evidence')
            if (data['run_fingerprint'] != origin['run_fingerprint'] or
                    data['binary'] != origin['binary'] or
                    data['runtime_metadata']['worker_binary_sha256'] != origin['worker_sha256']):
                raise ValueError('source identity mismatch')
            rows = {}
            for raw in data['assertions']:
                result = raw.get('result') or {}
                rows[raw['id']] = {k: raw[k] for k in
                    ('id', 'path', 'status', 'reason', 'fixture_error_evidence',
                     'execution', 'resource_evidence_complete', 'resource_error', 'resources', 'dependencies',
                     'fixture', 'fixture_error', 'chromium_reference', 'resource_audit') if k in raw}
                rows[raw['id']].update(
                    diagnostic_tags=sorted(set(raw.get('diagnostic_tags', [])) | set(diagnostics(raw))),
                    result={**{k: result.get(k) for k in ('complete', 'harness', 'completion_count', 'testdriver', 'automation_overflow', 'automation_trace')},
                            'tests': [{'name': t.get('name'), 'status': t.get('status')} for t in result.get('tests', [])]})
            if len(rows) != len(data['assertions']):
                raise ValueError('duplicate source ID')
            sources[key] = ({'dependency_preflight': data.get('dependency_preflight', {})}, rows)
        data, rows = sources[key]
        row = rows[entry['id']]
        if row['status'] != entry['status']:
            raise ValueError('ledger/source outcome mismatch')
        review = reviews.get(entry['id'])
        if review:
            if review['evidence_sha256'] != origin['sha256']:
                raise ValueError('stale review evidence hash')
            if not review.get('fixture_audit') or not review.get('automation_audit'):
                raise ValueError('review requires explicit fixture and automation rationale')
            if row['path'] not in review['source_files']:
                raise ValueError('review must bind test source')
            for relative, sha in review['source_files'].items():
                path = (checkout / relative).resolve()
                if checkout.resolve() not in path.parents or digest(path) != sha:
                    raise ValueError('review source hash mismatch')
            for dependency in review['verified_dependencies']:
                for scheme in ('http:', 'https:'):
                    check = data.get('dependency_preflight', {}).get(scheme + dependency, {})
                    if check.get('deferred_to_runtime'):
                        # The per-execution audit below must bind its response;
                        # preflight deliberately did not mutate this fixture.
                        continue
                    if (check.get('status') != 200 or not check.get('sha256') or
                            'javascript' not in (check.get('mime') or '') or
                            check.get('template_failure') or check.get('unresolved_template')):
                        if not any(resource_key(resource['url'], resource.get('query_sha256')) == resource_key(dependency)
                                   for resource in row.get('resources', [])):
                            raise ValueError('review dependency not verified: ' + dependency)
        qualification, reasons = classify(row, review, current=current)
        output.append(dict(entry, qualification=qualification, qualification_reasons=reasons,
                           root=row['path'].split('/')[0], audit=review))
    counts = Counter(e['qualification'] for e in output)
    groups = defaultdict(Counter)
    for entry in output:
        groups[entry['root']][entry['qualification']] += 1
    report = {'schema_version': 2, 'scope': 'Diagnostic composite; strict execution/resource qualification; not release attestation',
              'selected': len(output), 'counts': counts,
              'raw_counts': Counter(e['status'] for e in output),
              'qualified_counts': Counter(e['status'] for e in output if e['qualification'] == 'qualified'),
              'reason_counts': Counter(r for e in output for r in e['qualification_reasons']),
              'groups': dict(sorted(groups.items())),
              'composite_summary_sha256': digest(summary_path),
              'input_ledger_sha256': digest(ledger_path)}
    return output, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--composite', type=Path, required=True)
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--reviews', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--historical', action='store_true', help='Read old diagnostic evidence; cannot establish the current gate')
    args = parser.parse_args()
    if args.output.exists():
        parser.error('choose a new output directory; evidence is immutable')
    summary = json.loads((args.composite / 'summary.json').read_text())
    reviews = json.loads(args.reviews.read_text())
    if summary.get('cohort'):
        if not summary.get('complete') or len(summary.get('evidence_sources', [])) != 1:
            parser.error('qualification requires one complete cohort')
        source = summary['evidence_sources'][0]
        raw_path = Path(source['path'])
        if digest(raw_path) != source['sha256']:
            parser.error('cohort evidence hash mismatch')
        raw = json.loads(raw_path.read_text())
        origin = dict(source, binary=raw['binary'], worker_sha256=raw['runtime_metadata']['worker_binary_sha256'])
        with tempfile.TemporaryDirectory(prefix='oryn-cohort-qualification-') as directory:
            temporary = Path(directory)
            ledger = temporary / 'ledger.jsonl'
            ledger.write_text(''.join(json.dumps({'id': row['id'], 'status': row['status'], 'original': origin}) + '\n'
                                      for row in raw['assertions']))
            temporary_summary = temporary / 'summary.json'
            temporary_summary.write_text(json.dumps({'complete': True, 'selected': len(raw['assertions']),
                                                     'ledger_sha256': digest(ledger)}))
            rows, report = build(temporary_summary, ledger, args.checkout, reviews, current=not args.historical)
        report['cohort_summary_sha256'] = digest(args.composite / 'summary.json')
        report['manifest_coverage'] = summary.get('manifest_coverage')
        report['unsupported'] = summary.get('unsupported')
    else:
        rows, report = build(args.composite / 'summary.json', args.composite / 'ledger.jsonl',
                             args.checkout, reviews, current=not args.historical)
    args.output.mkdir(parents=True)
    ledger = args.output / 'ledger.jsonl'
    ledger.write_text(''.join(json.dumps(row, sort_keys=True) + '\n' for row in rows))
    report.update(ledger_sha256=digest(ledger), reviews_sha256=digest(args.reviews),
                  qualification_gate='historical' if args.historical else 'direct-fixture-v4',
                  classifier_sha256=digest(Path(__file__)),
                  diagnostics_sha256=digest(Path(__file__).with_name('wpt_evidence.py')))
    (args.output / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k != 'groups'}, indent=2))


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Summarize a completed audited cohort without promoting raw outcomes."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def failure_type(row):
    if row['status'] == 'environment_blocked':
        return 'environment_blocked'
    execution = row.get('execution') or {}
    error = str(execution.get('error') or '').lower()
    result = row.get('result') or {}
    if 'pool capacity' in error:
        return 'harness_pool_capacity'
    if 'timeout' in error or 'wall time' in error or row.get('failure_domain') == 'timeout':
        return 'timeout_or_worker_limit'
    if 'worker exceeded' in error and 'limit' in error:
        return 'worker_resource_limit'
    if 'crash' in error:
        return 'worker_crash'
    if execution.get('cleanup_errors') or execution.get('replacement_error'):
        return 'teardown_or_replacement_failure'
    if error:
        return 'runtime_or_transport_error'
    if 'testdriver_operation_not_implemented' in row.get('diagnostic_tags', []):
        return 'unsupported_automation'
    tests = result.get('tests') or []
    if (not execution.get('completion_valid') or result.get('harness') != 0
            or not tests or any(test.get('status') not in (0, 1) for test in tests)):
        return 'incomplete_or_invalid_completion'
    return 'raw_pass' if row['status'] == 'passed' else 'assertion_failure'


def summarize(raw, coverage):
    mapping = {row['runner_id']: row for row in coverage if row.get('runner_id')}
    groups = defaultdict(lambda: {'outcomes': Counter(), 'types': Counter()})
    tiers = defaultdict(lambda: {'outcomes': Counter(), 'types': Counter()})
    tier_roots = defaultdict(lambda: defaultdict(lambda: {'outcomes': Counter(), 'types': Counter()}))
    types = Counter()
    subtests = Counter()
    for row in raw['assertions']:
        entry = mapping[row['id']]
        kind = failure_type(row)
        types[kind] += 1
        for bucket in (groups[entry['root']], tiers[entry['tier']], tier_roots[entry['tier']][entry['root']]):
            bucket['outcomes'][row['status']] += 1
            bucket['types'][kind] += 1
        for test in (row.get('result') or {}).get('tests', []):
            subtests[str(test.get('status'))] += 1
    return {'execution_types': types, 'by_root': dict(sorted(groups.items())),
            'by_tier_root': {tier: dict(sorted(roots.items())) for tier, roots in sorted(tier_roots.items())},
            'by_tier': dict(sorted(tiers.items())), 'subtest_status_counts': subtests,
            'manifest_entries': len(coverage),
            'unsupported': sum(row['status'] == 'unsupported' for row in coverage),
            'executions': len(raw['assertions'])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cohort', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('choose a new output directory; evidence is immutable')
    summary_path = args.cohort / 'summary.json'
    summary = json.loads(summary_path.read_text())
    if not summary.get('complete') or summary.get('parent_integrity_errors'):
        parser.error('a complete cohort with verified parent integrity is required')
    sources = summary['evidence_sources']
    if len(sources) != 1:
        parser.error('expected one raw cohort source')
    source = Path(sources[0]['path'])
    coverage = Path(summary['manifest_coverage']['path'])
    if digest(source) != sources[0]['sha256'] or digest(coverage) != summary['manifest_coverage']['sha256']:
        parser.error('raw or manifest coverage hash mismatch')
    raw = json.loads(source.read_text())
    rows = raw['assertions']
    if len(rows) != summary['selected_executions'] or len({r['id'] for r in rows}) != len(rows):
        parser.error('cohort execution count mismatch or duplicate ID')
    report = summarize(raw, [json.loads(line) for line in coverage.read_text().splitlines()])
    report.update(schema_version=1, scope='Signed-worker dirty-tree diagnostics, not qualified conformance or release attestation',
                  cohort_summary_sha256=digest(summary_path), raw_sha256=digest(source),
                  coverage_sha256=digest(coverage), summarizer_sha256=digest(Path(__file__)),
                  elapsed_seconds=summary['elapsed_seconds'],
                  finalization=summary.get('finalization'),
                  qualification_counts=summary['qualification_counts'])
    args.output.mkdir(parents=True)
    (args.output / 'summary.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    lines = ['# Audited WPT cohort diagnostics', '', report['scope'] + '.', '',
             f"{report['executions']:,} executions; {report['unsupported']:,} unsupported manifest entries retained.", '',
             'Raw passes and assertion failures below still require fixture/resource qualification.', '',
             '| Tier | Raw pass | Raw failed | Environment blocked |', '| --- | ---: | ---: | ---: |']
    for tier, bucket in report['by_tier'].items():
        counts = bucket['outcomes']
        lines.append(f"| {tier} | {counts['passed']:,} | {counts['failed']:,} | {counts['environment_blocked']:,} |")
    lines += ['', '## Execution types', '']
    lines += [f'- {kind}: {count:,}' for kind, count in report['execution_types'].most_common()]
    lines += ['', '## Core groups by raw failure volume', '',
              'This is triage order, not proof that every failure is an engine defect.', '',
              '| Group | Raw pass | Raw failed | Assertion-only failures | Incomplete completion | Timeout/limit |',
              '| --- | ---: | ---: | ---: | ---: | ---: |']
    for root, bucket in sorted(report['by_tier_root'].get('core', {}).items(),
                               key=lambda item: (-item[1]['outcomes']['failed'], item[0])):
        counts, kinds = bucket['outcomes'], bucket['types']
        lines.append(f"| {root} | {counts['passed']:,} | {counts['failed']:,} | {kinds['assertion_failure']:,} | {kinds['incomplete_or_invalid_completion']:,} | {kinds['timeout_or_worker_limit']:,} |")
    if report['finalization']:
        lines += ['', '## Finalization disclosure', '',
                  report['finalization']['fixture_shutdown_intervention'], '',
                  report['finalization']['summary_correction']]
    lines += ['', 'Detailed root/tier/type counts and source hashes are in `summary.json`.', '']
    (args.output / 'README.md').write_text('\n'.join(lines))
    print(json.dumps({k: report[k] for k in ('executions', 'unsupported', 'execution_types')}, indent=2))


if __name__ == '__main__':
    main()

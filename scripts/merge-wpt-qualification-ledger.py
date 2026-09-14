#!/usr/bin/env python3
"""Merge a verified qualification overlay into the complete manifest ledger."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def merge(entries, overlay):
    reviews = {row['id']: row for row in overlay}
    if len(reviews) != len(overlay):
        raise ValueError('duplicate qualification ID')
    expected = {row['runner_id'] for row in entries if row.get('evidence')}
    if set(reviews) != expected:
        raise ValueError('qualification and executed manifest IDs differ')
    output = []
    for entry in entries:
        row = dict(entry)
        review = reviews.get(row.get('runner_id'))
        if review is not None:
            if row['status'] != review['status']:
                raise ValueError('qualification changed a raw outcome')
            origin = review.get('replacement') or review['original']
            if origin['sha256'] != row['evidence']['sha256']:
                raise ValueError('qualification source hash differs from manifest evidence')
            row.update(qualification=review['qualification'],
                       qualification_reasons=review['qualification_reasons'],
                       audit=review.get('audit'))
        output.append(row)
    if len({row['id'] for row in output}) != len(output):
        raise ValueError('duplicate manifest ID')
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cohort', type=Path, required=True)
    parser.add_argument('--qualification', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error('choose a new output directory; evidence is immutable')
    cohort_summary = args.cohort / 'summary.json'
    cohort_ledger = args.cohort / 'ledger.jsonl'
    qualification_summary = args.qualification / 'summary.json'
    qualification_ledger = args.qualification / 'ledger.jsonl'
    source = json.loads(cohort_summary.read_text())
    overlay = json.loads(qualification_summary.read_text())
    if not source.get('complete') or source.get('parent_integrity_errors'):
        parser.error('a complete cohort with verified parent integrity is required')
    if (digest(cohort_ledger) != source['ledger_sha256']
            or digest(qualification_ledger) != overlay['ledger_sha256']
            or digest(cohort_summary) != overlay.get('cohort_summary_sha256')):
        parser.error('ledger or cohort binding hash mismatch')
    entries = [json.loads(line) for line in cohort_ledger.read_text().splitlines()]
    qualified = [json.loads(line) for line in qualification_ledger.read_text().splitlines()]
    if len(entries) != source['manifest_tests'] or len(qualified) != source['selected_executions']:
        parser.error('manifest or execution count mismatch')
    try:
        rows = merge(entries, qualified)
    except ValueError as error:
        parser.error(str(error))
    tiers, roots = defaultdict(Counter), defaultdict(Counter)
    for row in rows:
        tiers[row['tier']][row['qualification']] += 1
        roots[row['root']][row['qualification']] += 1
    args.output.mkdir(parents=True)
    ledger = args.output / 'ledger.jsonl'
    ledger.write_text(''.join(json.dumps(row, sort_keys=True) + '\n' for row in rows))
    report = {
        'schema_version': 1, 'complete': True,
        'scope': 'Signed-worker dirty-tree diagnostics; only explicitly reviewed rows are qualified; not release attestation',
        'manifest_tests': len(rows), 'selected_executions': len(qualified),
        'unsupported': sum(row['status'] == 'unsupported' for row in rows),
        'qualification_counts': Counter(row['qualification'] for row in rows),
        'qualified_outcomes': Counter(row['status'] for row in rows if row['qualification'] == 'qualified'),
        'by_tier': dict(tiers), 'by_root': dict(roots), 'ledger_sha256': digest(ledger),
        'cohort_summary_sha256': digest(cohort_summary),
        'cohort_ledger_sha256': digest(cohort_ledger),
        'qualification_summary_sha256': digest(qualification_summary),
        'qualification_ledger_sha256': digest(qualification_ledger),
        'merger_sha256': digest(Path(__file__)),
    }
    (args.output / 'summary.json').write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')
    print(json.dumps({key: report[key] for key in ('manifest_tests', 'selected_executions', 'unsupported', 'qualification_counts')}, indent=2))


if __name__ == '__main__':
    main()

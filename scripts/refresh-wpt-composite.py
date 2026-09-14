#!/usr/bin/env python3
"""Replace an exact-ID diagnostic slice, retaining prior provenance and all IDs."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def refresh(composite, replacement, inventory):
    summary = json.loads((composite / 'summary.json').read_text())
    ledger = composite / 'ledger.jsonl'
    if not summary['complete'] or digest(ledger) != summary['ledger_sha256']:
        raise ValueError('invalid composite')
    entries = [json.loads(line) for line in ledger.read_text().splitlines()]
    ids = {e['id'] for e in entries}
    if len(ids) != len(entries) or len(ids) != summary['selected']:
        raise ValueError('invalid composite IDs')
    selected = json.loads(inventory.read_text())
    expected = {f"{r['path']}|{g}|{v}" for r in selected['tests']
                for g in r['globals'] for v in r['variants']}
    data = json.loads(replacement.read_text())
    outcome = data.get('outcome', {})
    if (outcome.get('status') not in ('passed', 'failed') or
            outcome.get('required_finished') != len(expected) or
            outcome.get('required_total') != len(expected)):
        raise ValueError('incomplete replacement run')
    rows = {r['id']: r for r in data['assertions']}
    if (data['inventory_sha256'] != digest(inventory) or set(rows) != expected or
            len(rows) != len(data['assertions']) or not expected <= ids):
        raise ValueError('replacement inventory/ID mismatch')
    if any(r['status'] not in ('passed', 'failed', 'environment_blocked') for r in rows.values()):
        raise ValueError('nonterminal replacement')
    origin = {'path': str(replacement.resolve()), 'sha256': digest(replacement),
              'run_fingerprint': data['run_fingerprint'], 'binary': data['binary'],
              'worker_sha256': data['runtime_metadata']['worker_binary_sha256']}
    for entry in entries:
        if entry['id'] not in rows:
            continue
        prior = entry.get('replacement') or entry['original']
        if any(prior[k] != origin[k] for k in ('binary', 'worker_sha256')):
            raise ValueError('browser binary changed')
        row = rows[entry['id']]
        entry['superseded_sources'] = entry.get('superseded_sources', []) + [prior]
        entry.update(replacement=origin, status=row['status'], reason=row.get('reason'),
                     diagnostic_tags=row.get('diagnostic_tags', []))
    return entries, {'complete': True, 'selected': len(entries), 'replaced': len(rows),
                     'counts': Counter(e['status'] for e in entries),
                     'qualification': 'Versioned diagnostic composite; not release evidence',
                     'prior_summary_sha256': digest(composite / 'summary.json'),
                     'replacement_sha256': digest(replacement), 'inventory_sha256': digest(inventory)}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for arg in ('composite', 'replacement', 'inventory', 'output'):
        p.add_argument('--' + arg, type=Path, required=True)
    args = p.parse_args()
    if args.output.exists():
        p.error('output exists; choose an immutable new path')
    entries, summary = refresh(args.composite, args.replacement, args.inventory)
    args.output.mkdir(parents=True)
    ledger = args.output / 'ledger.jsonl'
    ledger.write_text(''.join(json.dumps(e, sort_keys=True) + '\n' for e in entries))
    summary['ledger_sha256'] = digest(ledger)
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Run the fixed focused repair matrix; never claim full conformance."""
import argparse
from collections import Counter
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import threading
import time

from generation_gate import GenerationLease
from suite_input_lease import MetadataSnapshot, source_members, workflow_members
from workflow_contract import planned_attempts
from workflow_execution import cancellation_signals, execute_contract
from workflow_input_lock import snapshot as lock_snapshot, load as lock_load
from workflow_native_adapter import MixedCellAdapter

ROOT = Path(__file__).resolve().parents[1]
MIXED_CELLS = frozenset(('research.frames', 'forms.controls', 'shopping.discovery',
                        'saas.pkce', 'shopping.payment_frame'))


def rescue_plan(contract):
    complete = planned_attempts(contract)
    selected = [item for item in complete if item['mode'] == 'serial' or
                (item['mode'] == 'mixed_family' and item['cell'] in MIXED_CELLS)]
    serial = [item for item in selected if item['mode'] == 'serial']
    mixed = [item for item in selected if item['mode'] == 'mixed_family']
    if (len(complete) != 688 or len(serial) != 87 or len(mixed) != 45
            or len({item['cell'] for item in serial}) != 29):
        raise ValueError('rescue denominator differs from approved 87 + 45 originals')
    for cell in MIXED_CELLS:
        if {(item['context_count'], item['sample']) for item in mixed if item['cell'] == cell} != {
                (count, repetition) for count in (1, 5, 20) for repetition in (1, 2, 3)}:
            raise ValueError('incomplete mixed rescue population: ' + cell)
    return selected


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def source_hash():
    return subprocess.check_output([str(ROOT / 'scripts/source-tree-hash.sh')], cwd=ROOT,
                                   text=True).strip()


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation-gate', type=Path, required=True)
    parser.add_argument('--generation-gate-sha256', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--signed-source-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    policy = parser.add_mutually_exclusive_group()
    policy.add_argument('--deadline', default='2026-09-22T11:11:06Z')
    policy.add_argument('--until-complete', action='store_true',
                        help='explicit acceptance-driven run; retain command limits and cancellation')
    parser.add_argument('--jobs', type=int, default=5)
    args = parser.parse_args(argv)
    if args.jobs < 1:
        parser.error('--jobs must be positive')
    if args.until_complete:
        args.deadline = None
        args.deadline_timestamp = None
    else:
        try:
            instant = datetime.fromisoformat(args.deadline.replace('Z', '+00:00'))
            if instant.tzinfo is None:
                raise ValueError('timezone required')
            args.deadline_timestamp = instant.timestamp()
        except ValueError as error:
            parser.error('invalid deadline: ' + str(error))
        if args.deadline_timestamp <= time.time():
            parser.error('rescue deadline already reached')
    return args


def start_deadline_timer(stop, deadline):
    if deadline is None:
        return None
    timer = threading.Timer(max(0, deadline - time.time()), stop.set)
    timer.daemon = True
    timer.start()
    return timer


def main():
    args = parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)

    def save(name, value):
        path = out / name
        temp = path.with_suffix(path.suffix + '.tmp')
        temp.write_text(json.dumps(value, indent=2) + '\n')
        temp.replace(path)

    stop = threading.Event()
    timer = start_deadline_timer(stop, args.deadline_timestamp)
    started = time.monotonic()
    errors, summary = [], None
    lease = snapshot = source = identity = None
    save('active.json', {'pid': os.getpid(), 'stage': 'admission', 'qualified': False})
    try:
        lease = GenerationLease(args.generation_gate, expected=args.generation_gate_sha256)
        source = source_hash()
        if source != args.signed_source_sha256:
            raise ValueError('current source differs from repaired build source')
        snapshot = MetadataSnapshot([args.binary, args.worker, lease.path],
                                    membership=lambda: sorted(set(source_members(ROOT)) |
                                                              set(workflow_members(ROOT))))
        identity = dict(parent_sha256=digest(args.binary), worker_sha256=digest(args.worker),
                        source_tree_sha256=source)
        snapshot.check()
        lock = lock_snapshot(ROOT, source)
        save('workflow-input-lock.json', lock)
        contract = lock_load(out / 'workflow-input-lock.json', source, ROOT)
        plan = rescue_plan(contract)
        ids = [item['attempt_id'] for item in plan]
        save('selection.json', {'scope': 'focused_rescue', 'qualified': False,
                               'serial_originals': 87, 'mixed_originals': 45,
                               'attempt_ids': ids, 'identity': identity,
                               'forced_failure_validation': 'separate_required_gate'})
        cache = {}
        adapters = {cell: MixedCellAdapter(cell, args.binary.resolve(), args.worker.resolve(),
                                          worker_sha256=identity['worker_sha256'], expected_cache=cache)
                    for cell in {item['cell'] for item in plan}}
        oracles = {item['attempt_id']: adapters[item['cell']].expected(
            item['context_count'], item['mode'], item['fault']) for item in plan}
        save('active.json', {'pid': os.getpid(), 'stage': 'focused_matrix', 'qualified': False})
        with cancellation_signals(stop), lease.suite_checks(snapshot, boundary_only=True):
            os.environ.update(lease.environment())
            def probe():
                lease.check()
                return dict(identity)
            summary = execute_contract(contract, identity, adapters, out / 'matrix', oracles,
                                       workers=args.jobs, stop=stop, candidate_probe=probe,
                                       attempt_ids=ids)
        if stop.is_set():
            errors.append('cancellation reached' if args.until_complete else
                          'rescue deadline or cancellation reached')
        if summary.get('infrastructure_errors'):
            errors.extend(summary['infrastructure_errors'])
    except BaseException as error:
        errors.append(repr(error))
        stop.set()
    finally:
        if timer is not None:
            timer.cancel()
        try:
            if snapshot is not None:
                snapshot.check()
            if lease is not None:
                lease.final_check()
            if identity is not None and (digest(args.binary) != identity['parent_sha256'] or
                                         digest(args.worker) != identity['worker_sha256']):
                raise ValueError('runtime binaries changed during rescue validation')
            if source is not None and source_hash() != source:
                raise ValueError('source changed during rescue validation')
        except BaseException as error:
            errors.append('final custody: ' + repr(error))
        counts = {}
        attempts_path = out / 'matrix/attempts.jsonl'
        try:
            if attempts_path.exists():
                rows = [json.loads(line) for line in attempts_path.read_text().splitlines()]
                for mode in ('serial', 'mixed_family'):
                    counts[mode] = dict(Counter(item.get('disposition', 'unclassified')
                                               for item in rows if item['mode'] == mode))
        except (OSError, ValueError, TypeError, KeyError) as error:
            errors.append('terminal ledger: ' + repr(error))
        result = {'scope': 'focused_rescue', 'qualified': False,
                  'matrix_passed': not errors and summary is not None and
                                   summary.get('structurally_valid') is True and
                                   len(summary.get('validated', [])) == 132,
                  'forced_failure_gate': 'separate_required_evidence',
                  'elapsed_seconds': time.monotonic() - started, 'errors': errors,
                  'counts': counts, 'deadline': args.deadline,
                  'run_policy': 'acceptance_driven' if args.until_complete else 'deadline',
                  'finished_at': datetime.now(timezone.utc).isoformat()}
        save('result.json', result)
        save('active.json', {'pid': os.getpid(), 'stage': 'terminal', 'qualified': False})
        print(json.dumps(result), flush=True)
    return 0 if result['matrix_passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Bounded eight-way direct-fixture churn with one persistent signed parent."""
import argparse
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import time
from wpt_evidence import current_harness_errors

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('runner', ROOT / 'scripts/run-upstream-wpt.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--iterations', type=int, default=1000)
    parser.add_argument('--jobs', type=int, default=8)
    args = parser.parse_args()
    if not 1 <= args.jobs <= 8 or not 1 <= args.iterations <= 1000:
        parser.error('jobs must be 1–8 and iterations 1–1000')
    # Reserve output before starting any processes.
    with args.output.open('x') as output:
        output.write('{"complete":false}\n')
    os.environ['ORYN_WPT_ALLOW_INVALID_CERTS'] = '0'
    os.environ['ORYN_WPT_CA_CERT'] = runner.fixture_certificate(args.checkout)['ca']
    runner.Handler.harness_profile = 'upstream'
    runner.Handler.chromium = None
    runner.Handler.public_https_port = None
    parent = runner.WptSession(args.binary.resolve())
    runner.Handler.session_client = parent
    rows, shutdown = [], None
    started = time.monotonic()
    def run(index):
        row = runner.run_case(args.binary.resolve(), args.checkout.resolve(), 1,
            Path('dom/ranges/Range-stringifier.html'), 'harness_stress', 'window',
            f'?oryn_stress={index}', 30, False, None)
        return row
    try:
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            sequence = iter(range(args.iterations))
            pending = set()
            halted = False
            last_report = started
            while pending or not halted:
                while not halted and len(pending) < args.jobs:
                    index = next(sequence, None)
                    if index is None:
                        halted = True
                        break
                    pending.add(pool.submit(run, index))
                if not pending:
                    break
                done, pending = wait(pending, timeout=1, return_when=FIRST_COMPLETED)
                for future in done:
                    row = future.result()
                    rows.append(row)
                    if row.get('status') != 'passed' or runner.fatal_session_failure(row) or current_harness_errors(row):
                        halted = True
                if time.monotonic() - last_report >= 30:
                    print(json.dumps({'completed':len(rows),'selected':args.iterations,
                        'elapsed_seconds':round(time.monotonic()-started, 1), 'parent':parent.progress()}), flush=True)
                    last_report = time.monotonic()
    finally:
        shutdown = parent.close()
        runner.Handler.session_client = None
        pids = {r['worker_pid'] for row in rows for r in row.get('resources', [])
                if r.get('worker_pid')}
        valid = (len(rows) == args.iterations and len(pids) == args.iterations
                 and all(row.get('status') == 'passed' and not runner.fatal_session_failure(row)
                         and not current_harness_errors(row) and not row.get('execution', {}).get('cleanup_errors')
                         and row.get('resource_evidence_complete') is True for row in rows)
                 and shutdown['returncode'] == 0 and shutdown['shutdown_acknowledged'] and not shutdown['error'])
        runner.atomic_write_json(args.output, {'complete':len(rows) == args.iterations, 'passed':valid,
            'iterations':args.iterations, 'jobs':args.jobs, 'distinct_workers':len(pids),
            'elapsed_seconds':round(time.monotonic()-started, 3), 'shutdown':shutdown,
            'binary_sha256':hashlib.sha256(args.binary.read_bytes()).hexdigest(),
            'scope':'dirty-tree harness stability diagnostic, not conformance', 'assertions':rows})
    return 0 if valid else 1


if __name__ == '__main__':
    raise SystemExit(main())

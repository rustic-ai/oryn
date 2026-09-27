#!/usr/bin/env python3
"""Execute every original full-contract attempt under one admitted suite lease."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import threading

from generation_gate import generation_gate, GenerationLease
from completion_provenance import load_candidate, workflow_identity
from workflow_contract import planned_attempts
from workflow_input_lock import for_candidate
from workflow_execution import execute_contract, cancellation_signals
from workflow_native_adapter import MixedCellAdapter, RESEARCH_RECIPES, ROOT
from workflow_family_recipes import RECIPES
from suite_input_lease import candidate_snapshot


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation-gate', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--worker', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--recovery-workers', type=int, choices=range(1, 5), default=4,
                        help='independent recovery cells in flight; repetitions remain ordered')
    args = parser.parse_args()
    stop = threading.Event()
    with cancellation_signals(stop):
        return run_suite(args, stop)


def run_suite(args, stop):
    # Invalid generation is rejected before source enumeration or output creation.
    admission = GenerationLease(args.generation_gate, validator=generation_gate)
    args.binary = args.binary.resolve()
    args.worker = args.worker.resolve()
    args.output = args.output.resolve()
    snapshot = candidate_snapshot(args.candidate, args.generation_gate, ROOT)
    bound = None
    identity = None
    primary = None

    def validate():
        current = load_candidate(args.candidate)
        contract = for_candidate(current)
        source = subprocess.check_output([str(ROOT / 'scripts/source-tree-hash.sh')],
                                         cwd=ROOT, text=True).strip()
        current_identity = workflow_identity(current, args.binary, args.worker, source)
        return current, contract, current_identity

    try:
        bound, contract, identity = validate()
        # Metadata was captured before validation, closing the admission race.
        snapshot.check()
        with admission.suite_checks(snapshot, boundary_only=True):
            __import__('os').environ.update(admission.environment())

            def probe():
                admission.check()
                return dict(identity)

            expected_cache = {}
            adapters = {cell: MixedCellAdapter(cell, args.binary, args.worker,
                                               worker_sha256=identity['worker_sha256'],
                                               expected_cache=expected_cache)
                        for cell in [*RESEARCH_RECIPES, *RECIPES]}
            oracles = {attempt['attempt_id']: adapters[attempt['cell']].expected(
                attempt['context_count'], attempt['mode'], attempt['fault'])
                for attempt in planned_attempts(contract) if attempt['cell'] in adapters}
            summary = execute_contract(contract, identity, adapters, args.output,
                                       oracles, candidate_probe=probe, stop=stop,
                                       recovery_workers=args.recovery_workers)
    except BaseException as error:
        primary = error
        raise
    finally:
        failures = []
        # Always do the deep final checks, even when metadata or execution failed.
        for check in (snapshot.check, admission.final_check):
            try:
                check()
            except BaseException as error:
                failures.append(error)
        try:
            final, _contract, final_identity = validate()
            if bound is not None and (final['sha256'] != bound['sha256'] or final_identity != identity):
                raise ValueError('suite candidate changed after admission')
        except BaseException as error:
            failures.append(error)
        if failures:
            if primary is not None:
                for error in failures:
                    primary.add_note('suite final validation failed: ' + str(error))
            else:
                first = failures[0]
                for error in failures[1:]:
                    first.add_note('additional final validation failure: ' + str(error))
                raise first
    print(json.dumps(dict(structurally_valid=summary['structurally_valid'],
                          family_qualified=False, planned=summary['planned'],
                          validated=len(summary['validated']))))
    return int(not summary['structurally_valid'])


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Prepare/run locked official WPT only after all source-generation gates close."""
import argparse
import json
from pathlib import Path
import sys
from wpt_official_workflow import prepare, execute, ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=['prepare', 'core', 'canaries', 'placement'])
    parser.add_argument('--generation-gate', type=Path, required=True)
    parser.add_argument('--run-contract', type=Path, required=True)
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--python', type=Path, required=True, help='Pinned upstream dependency runtime')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--prepared', type=Path)
    parser.add_argument('--core-processes', type=int, choices=(1, 2, 4, 6, 8),
                        help='Prepare-only Core manager count (default 1); controls remain serial')
    parser.add_argument('--core-timeout-seconds', type=int,
                        help='Prepare-only Core wall budget, 1..604800 seconds (default 43200)')
    parser.add_argument('--inventory', type=Path)
    parser.add_argument('--manifest', type=Path)
    parser.add_argument('--policy', type=Path, default=ROOT / 'benchmarks/conformance/wpt-capability-tiers-v2.json')
    args = parser.parse_args()
    if args.operation == 'prepare':
        if args.inventory is None or args.manifest is None:
            parser.error('prepare requires --inventory and --manifest')
        result = prepare(args.generation_gate, args.run_contract, args.inventory, args.checkout,
                         args.manifest, args.policy, args.python, args.output,
                         core_processes=1 if args.core_processes is None else args.core_processes,
                         core_timeout_seconds=43200 if args.core_timeout_seconds is None else args.core_timeout_seconds)
    else:
        if args.core_processes is not None or args.core_timeout_seconds is not None:
            parser.error('Core processes and timeout are frozen by prepare; execution cannot override them')
        if args.prepared is None:
            parser.error('execution requires --prepared')
        result = execute(args.generation_gate, args.run_contract, args.checkout, args.prepared,
                         args.python, args.operation, args.output)
    print(json.dumps(result, indent=2))
    return 0 if args.operation == 'prepare' else int(result['returncode'] != 0 or result.get('custody_complete') is not True)


if __name__ == '__main__':
    sys.exit(main())

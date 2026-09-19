#!/usr/bin/env python3
"""Recompute all final component gates on one frozen completion candidate."""
import argparse
from pathlib import Path
from completion_qualification import qualify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'generation-gate', 'plan', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--allow-historical', action='store_true', help='Reproduce a legacy schema1 plan; never current complete-plan acceptance')
    args = parser.parse_args()
    result = qualify(args.candidate, args.generation_gate, args.plan, args.output, allow_historical=args.allow_historical)
    return int(not result['qualified'])


if __name__ == '__main__':
    raise SystemExit(main())

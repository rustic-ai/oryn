#!/usr/bin/env python3
"""Run an existing producer with original-run exact candidate custody."""
import argparse
from pathlib import Path
from completion_execution import KINDS, produce


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kind', choices=sorted(KINDS))
    for name in ('candidate', 'generation-gate', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    result = produce(args.kind, args.candidate, args.generation_gate, args.output)
    return int(not result['producer_completed'])


if __name__ == '__main__':
    raise SystemExit(main())

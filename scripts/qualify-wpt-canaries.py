#!/usr/bin/env python3
"""Record canary verdicts without replacing raw report, logs or process evidence."""
import argparse
import json
from pathlib import Path
from wpt_canary_qualification import qualify
from wpt_execution_evidence import verify as verify_execution


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--execution', type=Path, required=True)
    parser.add_argument('--audit', type=Path, required=True)
    parser.add_argument('--candidate-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    verify_execution(args.report, args.execution, args.candidate_sha256, 'canaries')
    result = qualify(args.report, args.execution, args.audit, args.candidate_sha256)
    with args.output.open('x') as stream:
        stream.write(json.dumps(result, indent=2) + '\n')
    return 0 if result['controls_rejected'] else 1


if __name__ == '__main__':
    raise SystemExit(main())

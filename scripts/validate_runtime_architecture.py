#!/usr/bin/env python3
"""Execute frozen runtime cutover inputs; release attestation joins at qualification."""
import argparse
import json
from pathlib import Path

from architecture_prebuilt import execute
from completion_provenance import load_candidate
from generation_gate import inherited_generation_lease


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--require-cutover', action='store_true', required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--candidate-sha256', required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    lease = inherited_generation_lease()
    candidate = load_candidate(args.candidate)
    if candidate['sha256'] != args.candidate_sha256:
        raise ValueError('architecture candidate digest mismatch')
    result = execute(candidate, args.output, lease)
    if load_candidate(args.candidate)['sha256'] != candidate['sha256']:
        raise ValueError('architecture candidate changed during execution')
    lease.check()
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__': raise SystemExit(main())

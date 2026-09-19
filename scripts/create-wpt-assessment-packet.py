#!/usr/bin/env python3
"""Prepare an unqualified exact-source packet for later human observations."""
import argparse
from pathlib import Path
from completion_execution import persist
from completion_provenance import load_candidate
from generation_gate import generation_gate
from wpt_assessment_packet import create


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'generation-gate', 'checkout', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--id', required=True)
    parser.add_argument('--platform', choices=('macOS', 'Linux', 'Windows'), required=True)
    parser.add_argument('--profile-id')
    parser.add_argument('--report', type=Path, action='append', default=[])
    parser.add_argument('--api', choices=('AXAPI', 'ATK', 'IAccessible2', 'MSAA', 'UIA'))
    args = parser.parse_args(); generation_gate(args.generation_gate)
    candidate = load_candidate(args.candidate)
    context = None
    if args.profile_id is not None:
        from core_profile_contract import load_profile_context
        context = load_profile_context(args.candidate, args.profile_id, args.report)
    elif args.report or 'core_profile_registry' in candidate['artifacts']:
        raise ValueError('profile candidate assessment requires explicit profile and original reports')
    packet = create(args.id, candidate['sha256'], args.checkout, args.platform, args.api, profile_context=context)
    generation_gate(args.generation_gate)
    if load_candidate(args.candidate)['sha256'] != candidate['sha256']: raise ValueError('candidate changed')
    if context is not None:
        from completion_provenance import read_record
        for record in context['inputs']:
            read_record(record)
    persist(args.output, packet)
    return 0


if __name__ == '__main__': raise SystemExit(main())

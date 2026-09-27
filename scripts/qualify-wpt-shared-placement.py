#!/usr/bin/env python3
"""Validate immutable official placement-run artifacts after generation completes."""
import argparse
import json
from pathlib import Path
from wpt_official_workflow import generation_gate, load_contract, sha256
from wpt_shared_placement import qualify
from wpt_execution_evidence import verify as verify_execution


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation-gate', required=True, type=Path)
    parser.add_argument('--run-contract', required=True, type=Path)
    parser.add_argument('--prepared', required=True, type=Path)
    parser.add_argument('--execution', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    generation_gate(args.generation_gate)
    bound = load_contract(args.run_contract)
    root = args.execution
    execution = verify_execution(root / 'wptreport.json', root / 'execution.json', bound['candidate_sha256'], 'placement')
    if (execution.get('collection') != 'placement' or execution.get('returncode') != 0
            or execution.get('launch_error') or execution.get('candidate_sha256') != bound['candidate_sha256']
            or execution.get('workflow_lock_sha256') != sha256(args.prepared / 'workflow-lock.json')):
        raise ValueError('placement execution is incomplete or belongs to another candidate')
    lock = json.loads((args.prepared / 'workflow-lock.json').read_text())
    for path, expected in lock['inputs'].items():
        if sha256(path) != expected:
            raise ValueError('placement source or prepared input changed')
    names = ['placement-context.json', 'placement.json', 'wptreport.json', 'mozlog.jsonl']
    for name in names:
        if execution.get('artifacts', {}).get(name) != sha256(root / name):
            raise ValueError('raw placement evidence changed: ' + name)
    context = json.loads((root / names[0]).read_text())
    if context.get('candidate_sha256') != bound['candidate_sha256'] or context.get('run_contract_sha256') != bound['sha256']:
        raise ValueError('placement context differs from frozen run')
    result = qualify(json.loads((root / names[1]).read_text()), json.loads((root / names[2]).read_text()),
                     context, bound['candidate']['artifacts']['worker']['sha256'])
    result['artifacts'] = {name: sha256(root / name) for name in names}
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')


if __name__ == '__main__':
    main()

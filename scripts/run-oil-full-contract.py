#!/usr/bin/env python3
"""Execute frozen full-contract attempts with real native shared-parent adapters.

Missing behavior/fault recipes stay failed in the complete denominator. This is
not an alias for subset qualification or a replacement for framework/perf gates.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

from generation_gate import generation_gate, GenerationLease
from completion_provenance import load_candidate, workflow_identity
from workflow_contract import planned_attempts
from workflow_input_lock import for_candidate
from workflow_execution import execute_contract
from workflow_native_adapter import MixedCellAdapter, RESEARCH_RECIPES, ROOT
from workflow_family_recipes import RECIPES


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--generation-gate',type=Path,required=True)
    parser.add_argument('--candidate',type=Path,required=True)
    parser.add_argument('--binary',type=Path,required=True,help='oryn-workflow-qualification shared-parent executable')
    parser.add_argument('--worker',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    admission = GenerationLease(args.generation_gate, validator=generation_gate)
    __import__('os').environ.update(admission.environment())
    args.binary=args.binary.resolve();args.worker=args.worker.resolve();args.output=args.output.resolve()
    gate_sha256 = digest(args.generation_gate)
    bound=load_candidate(args.candidate)
    contract=for_candidate(bound)
    def probe():
        admission.check()
        generation_gate(args.generation_gate)
        if digest(args.generation_gate) != gate_sha256:
            raise ValueError("generation declaration changed during full OIL execution")
        current=load_candidate(args.candidate)
        if current['sha256'] != bound['sha256']:
            raise ValueError("combined candidate changed after contract binding")
        return workflow_identity(current, args.binary, args.worker,
            subprocess.check_output([str(ROOT/'scripts/source-tree-hash.sh')],cwd=ROOT,text=True).strip())
    candidate=probe()
    # Snapshot independent expectations before invoking any application/native code.
    adapters={cell:MixedCellAdapter(cell,args.binary,args.worker) for cell in [*RESEARCH_RECIPES,*RECIPES]}
    oracles={attempt['attempt_id']:adapters[attempt['cell']].expected(attempt['context_count'],attempt['mode'],attempt['fault'])
             for attempt in planned_attempts(contract) if attempt['cell'] in adapters}
    summary=execute_contract(contract,candidate,adapters,args.output,oracles,candidate_probe=probe)
    print(json.dumps(dict(structurally_valid=summary['structurally_valid'],family_qualified=False,
                          planned=summary['planned'],validated=len(summary['validated']))))
    return int(not summary['structurally_valid'])


if __name__=='__main__':
    raise SystemExit(main())

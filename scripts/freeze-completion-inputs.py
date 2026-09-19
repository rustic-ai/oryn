#!/usr/bin/env python3
"""Freeze NEW candidate/source/workflow inputs after the complete generation gate.

The artifact map points at already built, signed and independently identified
artifacts. This command neither builds nor signs, and never rewrites v1 evidence.
"""
import argparse
import json
from pathlib import Path
import subprocess
from completion_execution import reference, persist, source_hash
from completion_provenance import ROOT, REQUIRED_ARTIFACTS, read_record, load_candidate
from completion_source_snapshot import inventory, source_digest, write, validate
from generation_gate import generation_gate, GenerationLease
from workflow_input_lock import snapshot as workflow_lock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('generation-gate', 'artifacts', 'output'): parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args(); admission = GenerationLease(args.generation_gate, validator=generation_gate)
    gate = dict(path=str(admission.path), sha256=admission.sha256)
    output = args.output.resolve()
    if output == ROOT or ROOT in output.parents:
        raise ValueError('candidate output must be outside the source tree to preserve its identity')
    records = json.loads(args.artifacts.read_text())
    supplied = REQUIRED_ARTIFACTS - {'source_snapshot', 'workflow_contract'}
    if not isinstance(records, dict) or not supplied <= records.keys(): raise ValueError('compiled/signed candidate artifact map incomplete')
    for name, record in records.items():
        if name not in ('source_snapshot', 'workflow_contract'): read_record(record)
    raw = subprocess.check_output(['git', '-C', str(ROOT), 'ls-files', '--cached', '--others', '--exclude-standard', '-z'])
    names = [name.decode() for name in raw.split(b'\0') if name]
    sources = inventory(ROOT, names); identity = source_digest(sources)
    if source_hash() != identity: raise ValueError('source inventory differs from authoritative source-tree hash')
    admission.check()
    output.mkdir(parents=True, exist_ok=False)
    archive = output / 'source.tar'; write(archive, ROOT, sources); validate(archive, identity)
    lock_path = output / 'workflow-input-lock-v2.json'; persist(lock_path, workflow_lock(ROOT, identity))
    records['source_snapshot'] = reference(archive); records['workflow_contract'] = reference(lock_path)
    package = ROOT / 'tools/wptrunner-oryn/oryn_wptrunner'
    candidate = {'schema_version': 1, 'source_tree_sha256': identity, 'artifacts': records,
                 'plugin_sources': {path.relative_to(package).as_posix(): reference(path)['sha256'] for path in sorted(package.rglob('*.py'))}}
    admission.check()
    candidate_path = output / 'candidate.json'; persist(candidate_path, candidate)
    load_candidate(candidate_path)
    admission.check(); read_record(gate)
    if inventory(ROOT, names) != sources or source_hash() != identity:
        raise ValueError('source changed during candidate freeze')
    persist(output / 'freeze-receipt.json', {'schema_version': 1, 'candidate': reference(candidate_path),
            'generation_gate': gate, 'source_tree_sha256': identity, 'qualified': False})
    print(str(candidate_path))
    return 0


if __name__ == '__main__': raise SystemExit(main())

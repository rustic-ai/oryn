#!/usr/bin/env python3
"""Observe provisioned fixture routes; never provision hosts, trust or public IPs."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from completion_execution import persist, reference, verify_receipt, source_hash
from completion_provenance import ROOT, load_candidate, read_record
from generation_gate import generation_gate, GenerationLease
from wpt_full_qualification import REVISION
from wpt_fixture_contract import validate
from wpt_topology_probe import probe, validate_observations


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('candidate', 'generation-gate', 'output'):
        parser.add_argument('--' + field, type=Path, required=True)
    args = parser.parse_args(); admission = GenerationLease(args.generation_gate, validator=generation_gate)
    candidate = load_candidate(args.candidate)
    if source_hash() != candidate['record']['source_tree_sha256']: raise ValueError('candidate source drift')
    fixture_path = candidate['artifacts']['fixture_contract']; fixture = json.loads(fixture_path.read_text())
    validate(fixture, fixture_path.parent)
    controls = json.loads(candidate['artifacts']['native_controls_contract'].read_text())
    checkout = Path(controls['upstream_checkout']).resolve()
    if subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip() != REVISION:
        raise ValueError('topology checkout is not pinned')
    sources = []
    for name in ('common/square.png', 'websockets/handlers/echo_wsh.py'):
        path = checkout / name
        original = subprocess.check_output(['git', '-C', str(checkout), 'show', REVISION + ':' + name])
        if path.read_bytes() != original: raise ValueError('modified upstream probe helper')
        sources.append(reference(path))
    # Use the same pinned dependency registration as the official runner.
    if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain', '--untracked-files=all', '--', 'tools'], text=True).strip():
        raise ValueError('modified pinned topology dependencies')
    sys.path.insert(0, str(checkout / 'tools'))
    import localpaths  # noqa: F401
    plan = json.loads(candidate['artifacts']['qualification_plan'].read_text())
    negative_receipt = Path(plan['receipts']['native-prerequisites']).resolve()
    verify_receipt(negative_receipt, 'native-prerequisites', candidate)
    from completion_qualification import native_prerequisites
    native_prerequisites(negative_receipt, candidate)
    args.output = args.output.resolve(); admission.check(); args.output.mkdir(parents=True, exist_ok=False)
    before = {'candidate': reference(args.candidate), 'fixture': reference(fixture_path),
              'negative_receipt': reference(negative_receipt), 'sources': sources,
              'generation_gate': reference(args.generation_gate),
              'producer': reference(Path(__file__)), 'wire_source': reference(ROOT / 'scripts/wpt_topology_probe.py')}
    persist(args.output / 'preflight.json', before)
    result = {'schema_version': 1, 'candidate_manifest_sha256': candidate['sha256'], 'errors': [],
              'contract_sha256': hashlib.sha256(json.dumps(fixture, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
              'endpoints': [], 'untrusted_certificate_rejected': True, 'wrong_hostname_rejected': True,
              'negative_receipt': before['negative_receipt'], 'qualified': False}
    ca = fixture_path.parent / fixture['tls']['ca']['path']
    for route in fixture['endpoint_routes']:
        try:
            admission.check()
            result['endpoints'].append(probe(route, ca, (checkout / 'common/square.png').read_bytes()))
        except Exception as error:
            result['errors'].append({'protocol': route['protocol'], 'port': route['port'], 'error': type(error).__name__ + ': ' + str(error)})
    try:
        generation_gate(args.generation_gate)
        if load_candidate(args.candidate)['sha256'] != candidate['sha256'] or source_hash() != candidate['record']['source_tree_sha256']:
            raise ValueError('topology candidate changed')
        for key, value in before.items():
            for item in value if key == 'sources' else [value]: read_record(item)
        verify_receipt(negative_receipt, 'native-prerequisites', candidate)
        if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain', '--untracked-files=all', '--', 'tools'], text=True).strip():
            raise ValueError('pinned topology dependencies changed during probe')
        validate_observations(result, sources[0]["sha256"])
        validate(fixture, fixture_path.parent, result)
    except Exception as error: result['errors'].append({'error': str(error)})
    result['topology_passed'] = not result['errors']
    persist(args.output / 'result.json', result)
    return int(not result['topology_passed'])


if __name__ == '__main__': sys.exit(main())

#!/usr/bin/env python3
"""Collect real native/reference prerequisites only after all generation closes."""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import sys
from completion_execution import persist, reference, run_owned, source_hash
from completion_provenance import ROOT, load_candidate, read_record
from generation_gate import generation_gate, GenerationLease
from wpt_fixture_contract import validate as validate_fixture
from wpt_native_controls import Fixture, Driver, tls_probe, browser_controls, validate_contract, validate_results, focus_lease, reference_capabilities, new_controls


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('generation-gate', 'candidate', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    admission = GenerationLease(args.generation_gate, validator=generation_gate)
    os.environ.update(admission.environment())
    bound = load_candidate(args.candidate)
    if source_hash() != bound['record']['source_tree_sha256']:
        raise ValueError('native prerequisite source differs from candidate')
    artifacts = bound['artifacts']
    fixture_path = artifacts['fixture_contract']; fixture = json.loads(fixture_path.read_text())
    validate_fixture(fixture, fixture_path.parent)
    contract_path = artifacts['native_controls_contract']; contract = json.loads(contract_path.read_text())
    validate_contract(contract, fixture)
    if read_record(contract['fixture_contract']) != fixture_path:
        raise ValueError('control fixture differs from candidate fixture')
    python = read_record(contract['upstream_python'])
    checkout = Path(contract['upstream_checkout']).resolve()
    ca = fixture_path.parent / fixture['tls']['ca']['path']
    args.output = args.output.resolve(); admission.check(); args.output.mkdir(parents=True, exist_ok=False)
    before = {'candidate': reference(args.candidate), 'generation_gate': reference(args.generation_gate),
              'producer': reference(Path(__file__)), 'controls_source': reference(ROOT / 'scripts/wpt_native_controls.py'),
              'contract': reference(contract_path), 'source_tree_sha256': bound['record']['source_tree_sha256']}
    before['control_inputs'] = [reference(fixture_path.parent / entry['path']) for entry in fixture['tls'].values()]
    before['control_inputs'] += [entry[field] for entry in contract['tls'].values() for field in ('certificate', 'key')]
    before['control_inputs'].append(contract['upstream_python'])
    persist(args.output / 'preflight.json', before)
    record = {'schema_version': 1, 'candidate_manifest_sha256': bound['sha256'], 'errors': [],
              'native': {'errors': [], 'controls': new_controls(True)},
              'reference': {'errors': [], 'controls': new_controls(False)}}
    token = secrets.token_hex(32); token_path = args.output / 'native.token'
    descriptor = os.open(token_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, 'w') as stream: stream.write(token)
    native_args = ['--host', '127.0.0.1', '--port', '{port}', '--browser-binary', str(artifacts['runtime']),
                   '--auth-token-file', str(token_path), '--configuration', str(fixture_path),
                   '--configuration-sha256', bound['record']['artifacts']['fixture_contract']['sha256']]
    specs = [('native', artifacts['driver'], native_args, token,
              {'browserName': 'oryn', 'acceptInsecureCerts': False}, (0,)),
             ('reference', artifacts['reference_driver'], ['--port={port}'], None,
              reference_capabilities(artifacts['reference_browser']), (0, -signal.SIGTERM))]
    try:
        admission.check()
        with focus_lease(), Fixture(contract):
            record['focus_isolation'] = 'host_serialized'
            record['tls_probes'] = {kind: tls_probe(entry, ca) for kind, entry in contract['tls'].items()}
            for name, binary, arguments, credential, capabilities, allowed_exit in specs:
                admission.check()
                driver = Driver(binary, arguments, args.output / name, credential, allowed_exit)
                data = record[name]
                try:
                    with driver:
                        data['controls'] = browser_controls(driver.wire, contract, capabilities, name == 'native', results=data['controls'])
                except Exception as error:
                    data['errors'].append(type(error).__name__ + ': ' + str(error))
                data['cleanup'] = driver.cleanup
                if hasattr(driver, 'wire'): data['commands'] = driver.wire.log
                record[name] = data
        generation_gate(args.generation_gate)
        fault_directory = args.output / 'protocol-faults'; fault_directory.mkdir()
        command = [str(python), str(ROOT / 'scripts/wpt-upstream-entry.py'), '--checkout', str(checkout),
                   '--generation-gate', str(args.generation_gate.resolve()), 'protocol-controls',
                   '--output', str(fault_directory / 'result.json')]
        admission.check()
        execution = run_owned(command, fault_directory, timeout=120, environment=admission.environment())
        persist(fault_directory / 'execution.json', execution)
        if execution['returncode'] != 0 or execution['timed_out'] or execution['error'] or not execution['cleanup_complete']:
            record['errors'].append('protocol fault control execution failed')
        if (fault_directory / 'result.json').is_file(): record['protocol_faults'] = json.loads((fault_directory / 'result.json').read_text())
    except Exception as error:
        record['errors'].append(type(error).__name__ + ': ' + str(error))
    finally:
        token_path.unlink(missing_ok=True)
    try:
        generation_gate(args.generation_gate)
        if load_candidate(args.candidate)['sha256'] != bound['sha256'] or source_hash() != before['source_tree_sha256']:
            raise ValueError('prerequisite candidate changed during execution')
        for field in ('candidate', 'generation_gate', 'producer', 'controls_source', 'contract'): read_record(before[field])
        for item in before['control_inputs']: read_record(item)
    except Exception as error: record['errors'].append(str(error))
    record['errors'].extend(validate_results(record))
    record['prerequisites_passed'] = not record['errors']
    record['qualified'] = False
    record['scope'] = 'Native/reference TLS, actual focus transitions and native/protocol fault controls only'
    record['artifacts'] = [reference(path) for path in sorted(args.output.rglob('*')) if path.is_file()]
    persist(args.output / 'result.json', record)
    print(json.dumps({'prerequisites_passed': record['prerequisites_passed'], 'errors': record['errors']}))
    return int(not record['prerequisites_passed'])


if __name__ == '__main__':
    sys.exit(main())

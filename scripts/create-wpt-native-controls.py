#!/usr/bin/env python3
"""Create NEW control TLS inputs after generation; never install a CA or bypass it."""
import argparse
import json
import os
from pathlib import Path
import secrets
import subprocess
from completion_execution import persist, reference
from completion_provenance import read_record
from generation_gate import generation_gate
from wpt_fixture_contract import validate as validate_fixture
from wpt_native_controls import validate_contract


def openssl(*arguments):
    return subprocess.check_output(['openssl', *map(str, arguments)], stderr=subprocess.PIPE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('generation-gate', 'fixture-contract', 'ca-key', 'upstream-python', 'upstream-checkout', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args(); generation_gate(args.generation_gate)
    gate = reference(args.generation_gate); fixture_ref = reference(args.fixture_contract)
    fixture = json.loads(args.fixture_contract.read_text()); validate_fixture(fixture, args.fixture_contract.parent)
    ca = args.fixture_contract.parent / fixture['tls']['ca']['path']
    # Check only public key outputs; private key bytes never enter an evidence log.
    if openssl('x509', '-in', ca, '-pubkey', '-noout') != openssl('pkey', '-in', args.ca_key, '-pubout'):
        raise ValueError('control signing key does not match the selected test CA')
    root = args.output.resolve()
    for protected in (Path(__file__).resolve().parents[1], args.upstream_checkout.resolve()):
        if root == protected or protected in root.parents:
            raise ValueError("control outputs must be outside source and pinned checkout")
    root.mkdir(parents=True, exist_ok=False)
    contract = {'schema_version': 1, 'upstream_python': reference(args.upstream_python),
                'upstream_checkout': str(args.upstream_checkout.resolve()), 'fixture_contract': fixture_ref, 'tls': {}}
    trusted = {'certificate': reference(args.fixture_contract.parent / fixture['tls']['certificate']['path']),
               'key': reference(args.fixture_contract.parent / fixture['tls']['key']['path'])}
    ports = [*fixture['ports']['https'], *fixture['ports']['h2']]
    contract['tls']['trusted'] = dict(trusted, url=f'https://web-platform.test:{ports[0]}/native-prerequisite')
    for index, kind in enumerate(('untrusted', 'wrong_hostname'), 1):
        key = root / (kind + '.key'); certificate = root / (kind + '.pem')
        name = 'not-web-platform.test' if kind == 'wrong_hostname' else 'web-platform.test'
        request = root / (kind + '.csr')
        openssl('req', '-new', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:P-256', '-nodes',
                '-keyout', key, '-out', request, '-subj', '/CN=' + name)
        key.chmod(0o600)
        extension = root / (kind + '.extensions')
        extension.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:' + name + '\n')
        authority = ['-CA', ca, '-CAkey', args.ca_key] if kind == 'wrong_hostname' else ['-signkey', key]
        openssl('x509', '-req', '-in', request, *authority, '-set_serial', str(secrets.randbits(127) + 1),
                '-days', '30', '-extfile', extension, '-out', certificate)
        contract['tls'][kind] = {'url': f'https://web-platform.test:{ports[index]}/native-prerequisite',
                                 'certificate': reference(certificate), 'key': reference(key)}
    validate_contract(contract, fixture)
    generation_gate(args.generation_gate); read_record(gate); read_record(fixture_ref)
    persist(root / 'native-controls-contract-v1.json', contract)
    print(str(root / 'native-controls-contract-v1.json'))
    return 0


if __name__ == '__main__': raise SystemExit(main())

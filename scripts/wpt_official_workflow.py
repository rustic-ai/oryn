"""Source-only workflow definition for running the actual pinned WPT executors.

Every side-effecting entry point requires a complete source-generation gate.
Generation of this module does not execute the workflow.
"""
import configparser
import hashlib
import json
import os
import secrets
from pathlib import Path
import subprocess
import sys

from wpt_full_qualification import REVISION, EXECUTORS, sha256, bind_inventory, validate_inventory
from wpt_fixture_contract import validate as validate_fixture
from completion_execution import run_owned, reference, persist
from wpt_shared_placement import TEST as PLACEMENT_TEST
from wpt_execution_evidence import execution_metadata, verify_execution_policy

ROOT = Path(__file__).resolve().parents[1]
PLUGIN = ROOT / 'tools/wptrunner-oryn'
sys.path.insert(0, str(PLUGIN))
from oryn_wptrunner.provenance import load_contract
from oryn_wptrunner import feature_profile
from core_profile_contract import BASE_FIELDS, identity as profile_identity, validate_inventory_binding

from generation_gate import REQUIREMENTS, GenerationLease, generation_gate as _generation_gate


def generation_gate(path):
    # Retain the established public entry point and caller-selected source root.
    return _generation_gate(path, ROOT)


def read_inventory(folder, checkout, manifest, policy):
    folder = Path(folder)
    rows = [json.loads(line) for line in (folder / 'coverage.jsonl').read_text().splitlines()]
    core = validate_inventory(rows, json.loads((folder / 'plan.json').read_text()))
    bind_inventory(core, manifest, policy, checkout)
    return core


def runner_command(python, checkout, contract, prepared, kind, output, gate_path, *, core_processes=1):
    processes = execution_metadata(kind, core_processes)['processes']
    bound = load_contract(contract)
    candidate = bound['candidate']['artifacts']
    fixture = bound['fixture']
    root = Path(bound['fixture_path']).parent
    args = [str(python), str(ROOT / 'scripts/wpt-upstream-entry.py'), '--checkout', str(checkout), '--generation-gate', str(gate_path), 'run',
            '--product', 'oryn', '--oryn-run-contract', str(Path(contract).resolve()),
            '--binary', candidate['runtime']['path'], '--webdriver-binary', candidate['driver']['path'],
            '--config', str(Path(prepared) / 'wptrunner.ini'), '--no-manifest-update', '--no-manifest-download',
            '--ssl-type', 'pregenerated', '--ca-cert-path', str(root / fixture['tls']['ca']['path']),
            '--host-cert-path', str(root / fixture['tls']['certificate']['path']),
            '--host-key-path', str(root / fixture['tls']['key']['path']),
            '--include-file', str(Path(prepared) / (kind + '.include')),
            '--processes', str(processes), '--repeat', '1', '--rerun', '1', '--retry-unexpected', '0',
            '--no-pause-after-test', '--no-install-fonts',
            '--log-wptreport', str(Path(output) / 'wptreport.json'),
            '--log-raw', str(Path(output) / 'mozlog.jsonl'),
            '--test-types', *sorted(EXECUTORS if kind == 'core' else {'testharness'} if kind == 'placement' else {'testharness', 'reftest'})]
    return args


def prepare(gate_path, contract_path, inventory, checkout, manifest, policy, python, output, *, core_processes=1, core_timeout_seconds=43200):
    execution_metadata('core', core_processes, core_timeout_seconds)
    admission = GenerationLease(gate_path, validator=generation_gate)
    bound = load_contract(contract_path)
    validate_fixture(bound['fixture'], Path(bound['fixture_path']).parent)
    checkout = Path(checkout).resolve()
    revision = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != REVISION:
        raise ValueError('upstream checkout revision mismatch')
    core = read_inventory(inventory, checkout, manifest, policy)
    output = Path(output).resolve()
    envelope = bound.get('profile_envelope')
    if envelope:
        if output != Path(envelope['envelope']['prepared']):
            raise ValueError('prepared output differs from frozen profile envelope')
        validate_inventory_binding(envelope['registry'], manifest, policy, Path(inventory)/'coverage.jsonl')
    def check_preparation_inputs():
        admission.check()
        after = load_contract(contract_path)
        if after['sha256'] != bound['sha256'] or after['candidate_sha256'] != bound['candidate_sha256']:
            raise ValueError('profile candidate/envelope changed during preparation')
    check_preparation_inputs()
    output.mkdir(parents=True, exist_ok=False)
    metadata = output / 'metadata'
    metadata.mkdir()
    canary_metadata = output / 'canary-metadata'
    canary_metadata.mkdir()
    canary_manifest = output / 'canary-manifest.json'
    # Preserve the fixture files and upstream classification: each collection
    # is a selected child directory of the same root mounted by wptrunner.
    controls = json.loads((PLUGIN / 'canaries/controls.json').read_text())
    canary_requirements = [argument for row in controls['controls']
                           for argument in ('--require-test', row['type'],
                                            controls['mount'] + row['path'])]
    check_preparation_inputs()
    subprocess.run([str(python), str(ROOT / 'scripts/wpt-upstream-entry.py'), '--checkout', str(checkout), '--generation-gate', str(gate_path),
                    'manifest', '--tests', str(PLUGIN), '--subdir', 'canaries', '--output', str(canary_manifest),
                    '--url-base', '/__oryn_qualification__/', *canary_requirements], check=True, env=admission.environment())
    check_preparation_inputs()
    placement_root = ROOT / 'crates/oryn-native/tests/fixtures'
    placement_metadata = output / 'placement-metadata'
    placement_metadata.mkdir()
    placement_manifest = output / 'placement-manifest.json'
    admission.check()
    subprocess.run([str(python), str(ROOT / 'scripts/wpt-upstream-entry.py'), '--checkout', str(checkout), '--generation-gate', str(gate_path),
                    'manifest', '--tests', str(placement_root), '--subdir', 'shared-memory', '--output', str(placement_manifest),
                    '--url-base', '/__oryn_shared_memory__/', '--require-test', 'testharness',
                    PLACEMENT_TEST], check=True, env=admission.environment())
    check_preparation_inputs()
    config = configparser.ConfigParser(interpolation=None)
    config['manifest:official'] = {'tests': str(checkout), 'metadata': str(metadata),
                                    'manifest': str(Path(manifest).resolve()), 'url_base': '/'}
    config['manifest:oryn-qualification'] = {'tests': str(PLUGIN),
        'metadata': str(canary_metadata), 'manifest': str(canary_manifest), 'url_base': '/__oryn_qualification__/'}
    config['manifest:oryn-shared-memory'] = {'tests': str(placement_root), 'metadata': str(placement_metadata),
        'manifest': str(placement_manifest), 'url_base': '/__oryn_shared_memory__/'}
    (output / 'placement.include').write_text(PLACEMENT_TEST + '\n')
    with (output / 'wptrunner.ini').open('w') as stream:
        config.write(stream)
    (output / 'core.include').write_text(''.join(row['url'] + '\n' for _, row in sorted(core.items()) if row['test_type'] in EXECUTORS))
    (output / 'canaries.include').write_text(''.join(controls['mount'] + row['path'] + '\n' for row in controls['controls']))
    inputs = [Path(gate_path), Path(contract_path), Path(manifest), Path(policy), Path(python).resolve(),
              Path(__file__), ROOT / 'scripts/completion_execution.py', ROOT / 'scripts/completion_provenance.py',
              ROOT / 'scripts/wpt_execution_evidence.py', ROOT / 'scripts/generation_gate.py', ROOT / 'scripts/wpt-upstream-entry.py', ROOT / 'scripts/run-official-wpt.py',
              *sorted((PLUGIN / 'oryn_wptrunner').rglob('*.py')),
              Path(inventory) / 'plan.json', Path(inventory) / 'coverage.jsonl',
              *sorted((PLUGIN / 'canaries').rglob('*')), *sorted(output.glob('*.ini')), *sorted(output.glob('*.include')), canary_manifest, placement_manifest, *sorted((placement_root / 'shared-memory').rglob('*')), ROOT / 'scripts/wpt_shared_placement.py', ROOT / 'scripts/wpt_canary_qualification.py']
    if envelope:
        inputs += [Path(record['path']) for record in envelope['registry']['inputs']]
        inputs.append(ROOT / 'scripts/core_profile_contract.py')
    lock = {'schema_version': 2 if envelope else 1, 'core_processes': core_processes,
            'core_timeout_seconds': core_timeout_seconds,
            'candidate_sha256': bound['candidate_sha256'],
            'run_contract_sha256': bound['sha256'], 'upstream_revision': REVISION,
            'feature_environment_sha256': bound['feature_environment_sha256'],
            'feature_profile': bound['feature_profile'], 'feature_profile_sha256': bound['feature_profile_sha256'],
            'cookie_profile': bound['cookie_profile'], 'cookie_profile_sha256': bound['cookie_profile_sha256'],
            'fixture_environment_sha256': bound['fixture_environment_sha256'],
            'manifest_sha256': sha256(manifest), 'coverage_sha256': sha256(Path(inventory) / 'coverage.jsonl'),
            'policy_sha256': sha256(policy), 'core_include_sha256': sha256(output / 'core.include'),
            'core_total': len(core), 'inventory': str(Path(inventory).resolve()),
            'manifest': str(Path(manifest).resolve()), 'policy': str(Path(policy).resolve()), 'checkout': str(checkout), 'python': str(Path(python).resolve()), 'inputs': {str(path.resolve()): sha256(path) for path in inputs if path.is_file()}}
    if envelope:
        lock.update({key: envelope[key] for key in BASE_FIELDS})
        for record in envelope['registry']['inputs']:
            name = str(Path(record['path']).resolve())
            if lock['inputs'].get(name) != record['sha256']:
                raise ValueError('profile prerequisite changed while preparing lock')
            lock['inputs'][name] = record['sha256']
    check_preparation_inputs()
    for name, expected in lock['inputs'].items():
        if sha256(name) != expected:
            raise ValueError('prepared input changed before lock publication: ' + name)
    (output / 'workflow-lock.json').write_text(json.dumps(lock, indent=2, sort_keys=True) + '\n')
    return lock


def execute(gate_path, contract_path, checkout, prepared, python, kind, output):
    admission = GenerationLease(gate_path, validator=generation_gate)
    frozen_gate = reference(gate_path)
    bound = load_contract(contract_path)
    lock = json.loads((Path(prepared) / 'workflow-lock.json').read_text())
    process_policy = execution_metadata(kind, lock.get('core_processes', 1),
                                        lock.get('core_timeout_seconds', 43200))
    if str(Path(checkout).resolve()) != lock.get('checkout') or str(Path(python).resolve()) != lock.get('python'):
        raise ValueError('prepared checkout/runtime path changed')
    if lock['candidate_sha256'] != bound['candidate_sha256'] or lock['run_contract_sha256'] != bound['sha256']:
        raise ValueError('prepared workflow candidate changed')
    envelope = bound.get('profile_envelope')
    if envelope:
        if (lock.get('schema_version') != 2 or any(lock.get(key) != envelope[key] for key in BASE_FIELDS)
                or Path(prepared).resolve() != Path(envelope['envelope']['prepared'])
                or Path(output).resolve() != Path(envelope['envelope']['outputs'][kind])):
            raise ValueError('prepared/collection profile envelope changed')
    elif lock.get('schema_version') != 1:
        raise ValueError('prepared profile envelope cannot downgrade')
    if envelope:
        from completion_qualification import profile_prerequisites, profile_original_controls, read
        candidate = envelope['candidate']
        profile_prerequisites(candidate)
        plan = read(candidate['artifacts']['qualification_plan'])
        if kind == 'core':
            profile_original_controls(plan['core']['profiles'][envelope['profile_id']], envelope, candidate)
    feature_profile.verify_record(lock, bound['feature_profile'])
    if lock.get('cookie_profile') != bound['cookie_profile'] or lock.get('cookie_profile_sha256') != bound['cookie_profile_sha256']:
        raise ValueError('prepared native cookie profile changed')
    for path, expected in lock['inputs'].items():
        if sha256(path) != expected:
            raise ValueError('prepared input changed: ' + path)
    read_inventory(lock['inventory'], checkout, lock['manifest'], lock['policy'])
    output = Path(output).resolve()
    admission.check()
    output.mkdir(parents=True, exist_ok=False)
    command = runner_command(python, checkout, contract_path, prepared, kind, output, gate_path,
                             core_processes=process_policy['core_processes'])
    verify_execution_policy(lock, kind, {**process_policy, 'argv': command})
    environment = admission.environment()
    # Native configuration is immutable; remove legacy diagnostics from the child.
    for name in list(environment):
        if name.startswith('ORYN_WPT_'):
            del environment[name]
    custody_path = output / 'protocol-custody.jsonl'
    descriptor = os.open(custody_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    if envelope:
        environment['ORYN_CORE_COLLECTION'] = kind
    environment['ORYN_WPT_PROTOCOL_CUSTODY'] = str(custody_path)
    environment['ORYN_WPT_CANDIDATE_SHA256'] = bound['candidate_sha256']
    if kind == 'canaries':
        environment['ORYN_WPT_CANARY_RESOURCE_AUDIT'] = '1'
    if kind == 'placement':
        placement = {'schema': 1, 'test': PLACEMENT_TEST,
                     'nonce': secrets.token_hex(32), 'run_contract': str(Path(contract_path).resolve()),
                     'candidate_sha256': bound['candidate_sha256'], 'run_contract_sha256': bound['sha256'],
                     'output': str(output / 'placement.json')}
        context_path = output / 'placement-context.json'
        context_path.write_text(json.dumps(placement, indent=2) + '\n')
        environment['ORYN_WPT_PLACEMENT_CONTEXT'] = str(context_path)
    profile_context = profile_identity(envelope, kind) if envelope else {}
    preflight = {**profile_context, **process_policy, 'schema_version': 2 if envelope else 1, 'candidate_sha256': bound['candidate_sha256'],
                 'collection': kind, 'argv': command,
                 'generation_gate': frozen_gate, 'run_contract': reference(contract_path),
                 'workflow_lock': reference(Path(prepared) / 'workflow-lock.json'),
                 'producer_source': reference(Path(__file__)),
                 'supervisor_source': reference(ROOT / 'scripts/completion_execution.py')}
    persist(output / 'preflight.json', preflight)
    revoked = False
    def verify_inputs():
        nonlocal revoked
        if revoked: raise ValueError('official input custody was already revoked')
        try:
            _verify_inputs()
        except Exception:
            revoked = True
            raise
    def _verify_inputs():
        admission.check()
        generation_gate(gate_path)
        after = load_contract(contract_path)
        if after['candidate_sha256'] != bound['candidate_sha256'] or after['sha256'] != bound['sha256']:
            raise ValueError('official candidate or run contract changed')
        for field in ('generation_gate', 'run_contract', 'workflow_lock', 'producer_source', 'supervisor_source'):
            if sha256(preflight[field]['path']) != preflight[field]['sha256']:
                raise ValueError('official preflight input changed: ' + field)
        feature_profile.verify_record(lock, bound['feature_profile'])
        if lock.get('cookie_profile') != bound['cookie_profile'] or lock.get('cookie_profile_sha256') != bound['cookie_profile_sha256']:
            raise ValueError('prepared native cookie profile changed')
        for path, expected in lock['inputs'].items():
            if sha256(path) != expected:
                raise ValueError('official prepared input changed: ' + path)
        if envelope:
            profile_prerequisites(candidate)
            if kind == 'core':
                profile_original_controls(plan['core']['profiles'][envelope['profile_id']], envelope, candidate)
    # Repeat admission after preflight work, before any upstream executor runs.
    # Keep the same check after execution to retain failures in the run receipt.
    verify_inputs()
    execution = run_owned(command, output, timeout=process_policy['timeout_seconds'],
                          environment=environment, cwd=checkout)
    errors = []
    if execution['returncode'] == 65:
        errors.append('official run admission or final integrity was rejected')
    if execution['timed_out'] or execution['error'] or not execution['cleanup_complete']:
        errors.append('official executor failed custody or cleanup')
    try:
        verify_inputs()
    except Exception as error:
        errors.append(str(error))
    record = {'upstream_revision': REVISION, **{key: lock[key] for key in ('feature_environment_sha256', 'feature_profile', 'feature_profile_sha256', 'cookie_profile', 'cookie_profile_sha256', 'fixture_environment_sha256', 'manifest_sha256', 'coverage_sha256', 'policy_sha256', 'core_include_sha256', 'core_total')},
              **{name + '_sha256': bound['candidate']['artifacts'][name]['sha256'] for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')},
              **profile_context, **process_policy, 'schema_version': 2 if envelope else 1, 'candidate_sha256': bound['candidate_sha256'], 'collection': kind,
              'argv': command, 'returncode': execution['returncode'], 'launch_error': execution['error'],
              'timed_out': execution['timed_out'], 'cleanup_complete': execution['cleanup_complete'],
              'elapsed_ns': execution['elapsed_ns'], 'errors': errors, 'custody_complete': not errors,
              'preflight': reference(output / 'preflight.json'),
              'workflow_lock_sha256': preflight['workflow_lock']['sha256'],
              'artifacts': {path.name: sha256(path) for path in output.iterdir() if path.is_file()}}
    (output / 'execution.json').write_text(json.dumps(record, indent=2) + '\n')
    return record

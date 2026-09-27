"""Real native/reference and topology collectors under explicit input custody.

These are prerequisites, never part of the 15,985 URL denominator. Existing
browser controls, transport probes and descendant supervision remain canonical.
The caller provisions fixture routes and trust; this module does not alter them.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import secrets
import signal
import sys
import unittest

from .admission import SOURCE_ROOT, legacy_algorithms, object_digest, validate_registry
from .inputs import InputLease, resolve_input, snapshot_input, verify_inputs
from .manifests import load_manifest, manifest_digest
from .production import _hash, _read, _require, _object, validate_production, validate_source_export
from .processes import run_owned, ResourceLease
from .worker_environment import (worker_environment, assert_worker_environment,
                                 selected_worker, verify_worker_binding)

STAGES = {'native': 'native-prerequisites', 'topology': 'fixture-topology', 'runtime': 'native-runtime-readiness'}
RESOURCE_KEYS = ['native-focus', 'wpt-fixed-ports']


def _write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')


def load_inputs(path, *, store_root, bundle_root):
    path = Path(path).absolute()
    run = load_manifest(path, 'run')
    store_root = Path(store_root).absolute()
    _require(path.is_relative_to(store_root), 'prerequisite run manifest must be inside the configured store')
    manifest_record = snapshot_input(store_root, path.relative_to(store_root).as_posix())
    records = [*run['immutable_inputs'], manifest_record]
    assets = {name: resolve_input(bundle_root if record['scope'] == 'bundle' else store_root,
                                  record['path']) for name, record in run.get('assets', {}).items()}
    verify_inputs(records, store_root=store_root, bundle_root=bundle_root)
    bound = {'run': run, 'path': path, 'sha256': _hash(path), 'records': records,
            'assets': assets, 'store_root': Path(store_root).absolute(),
            'bundle_root': Path(bundle_root).absolute()}
    validate_runner_sources(bound)
    return bound


def validate_runner_sources(bound):
    """Executing collector/helper bytes must belong to the committed runner."""
    assets, run = bound['assets'], bound['run']
    _require({'runner_source', 'runner_identity'} <= assets.keys(), 'committed runner export and identity required')
    record = run['assets']['runner_source']
    _require(record['kind'] == 'directory', 'runner source must have exact directory membership')
    identity = validate_source_export(_read(assets['runner_identity']), assets['runner_source'], record)
    _require(run['runner'] == {'commit': identity['commit'], 'tree_sha256': identity['tree_sha256']},
             'runner commit/export identity differs')
    selected = [p for directory in ('tools/validation/oryn_validation', 'tools/wptrunner-oryn', 'scripts')
                for p in (SOURCE_ROOT / directory).rglob('*.py') if '__pycache__' not in p.parts]
    for path in selected:
        relative = str(path.relative_to(SOURCE_ROOT))
        _require(relative in identity['tracked'] and _hash(path) == identity['tracked'][relative]['sha256'],
                 'executing runner/helper differs from committed export: ' + relative)
    return identity


def _selected(bound):
    assets, run = bound['assets'], bound['run']
    registry = _read(assets['core_profile_registry'])
    fixture = validate_registry(registry, assets, selected_profile=run['profile'])
    fixture_asset = registry['profiles'][run['profile']]['fixture']
    return fixture_asset, assets[fixture_asset], fixture


def reference_environment(bound, *, verify=True):
    """Bind the complete signed reference app; two executable hashes do not suffice."""
    from .reference_environment import physical_inputs, verify_reference_environment
    names = {'reference_environment_identity', 'reference_browser', 'reference_driver'}
    _require(names <= bound['assets'].keys(), 'complete reference environment assets required')
    run = bound['run']
    _require(names <= run.get('assets', {}).keys(), 'reference typed input roles missing')
    identity = _read(bound['assets']['reference_environment_identity'])
    records = physical_inputs(identity)
    _require(all(record in run['immutable_inputs'] for record in records),
             'reference physical input absent from declared immutable closure')
    _require(run['assets']['reference_environment_identity'] in run['immutable_inputs'],
             'reference environment identity absent from immutable closure')
    for role, field in (('reference_browser', 'browser'), ('reference_driver', 'driver')):
        _require(run['assets'][role] == identity[field] and
                 bound['assets'][role] == resolve_input(bound['store_root'], identity[field]['path']),
                 'reference executable role differs from complete environment: ' + role)
    receipt = verify_reference_environment(identity, bound['store_root']) if verify else None
    return identity, receipt


def _reference_receipts(value, identity):
    from .reference_environment import physical_inputs
    _require(isinstance(value, dict) and value.get('errors') == [], 'reference custody errors or missing proof')
    expected = {'identity_sha256': manifest_digest(identity),
                'inputs_sha256': manifest_digest(physical_inputs(identity)),
                'signatures': identity['signatures'], 'system': identity['system']}
    for stage in ('admission', 'finalization'):
        receipt = value.get(stage)
        _require(isinstance(receipt, dict) and all(receipt.get(key) == item for key, item in expected.items())
                 and receipt.get('inputs', {}).get('inputs_sha256') == expected['inputs_sha256'],
                 'reference environment ' + stage + ' proof differs')


def validate_reference_evidence(bound, custody, execution):
    # Current physical files, signed layout and system dependencies are verified
    # anew; saved successful booleans are never an environment admission.
    identity, _ = reference_environment(bound)
    for record in (custody, execution):
        _reference_receipts(record.get('reference_environment'), identity)


class _NativeInputBoundary:
    def __init__(self, inputs, reference):
        self.inputs, self.reference = inputs, reference

    def checkpoint(self):
        self.inputs.checkpoint()
        self.reference.checkpoint()


def native_contract(bound):
    """Translate only the documented typed controls schema, not arbitrary JSON."""
    legacy_algorithms()
    from wpt_native_controls import validate_contract, TLS_KINDS
    fixture_asset, fixture_path, fixture = _selected(bound)
    assets = bound['assets']
    value = _read(assets['native_controls_contract'])
    _object(value, {'schema_version', 'kind', 'fixture', 'python', 'wpt_checkout', 'tls'}, 'typed native controls')
    _require(type(value['schema_version']) is int and value['schema_version'] == 2
             and value['kind'] == 'native-controls', 'typed native controls schema 2 required')
    _require(value['fixture'] == fixture_asset and value['python'] == 'python'
             and value['wpt_checkout'] == 'wpt_checkout', 'native control input roles differ')
    _object(value['tls'], TLS_KINDS, 'TLS negative controls')
    tls = {}
    for kind, entry in value['tls'].items():
        _object(entry, {'url', 'certificate', 'key'}, 'TLS control')
        tls[kind] = {'url': entry['url']}
        for field in ('certificate', 'key'):
            name = entry[field]
            _require(isinstance(name, str) and name in assets, 'undeclared TLS control asset')
            record = bound['run']['assets'][name]
            _require(record['kind'] == 'file', 'TLS control must be a pinned file')
            tls[kind][field] = {'path': str(assets[name]), 'sha256': record['sha256']}
    selected = {'schema_version': 1, 'tls': tls}
    validate_contract(selected, fixture)
    return selected, fixture_path, fixture


def validate_native_observations(record):
    """Recompute every current TLS/focus/fault verdict and raw transcript join."""
    legacy_algorithms()
    from wpt_native_controls import validate_results, CONTROL_STAGES
    _require(isinstance(record, dict) and not record.get('errors'), 'native prerequisite execution errors')
    errors = validate_results(record)
    _require(not errors, 'native/reference prerequisites failed: ' + '; '.join(errors))
    for name in ('native', 'reference'):
        browser = record[name]
        controls = browser['controls']
        required = CONTROL_STAGES if name == 'native' else CONTROL_STAGES[:5]
        _require(set(controls.get('control_execution', {})) == set(required)
                 and all(controls['control_execution'][stage] == {'status': 'completed'} for stage in required),
                 'current native prerequisite stages incomplete: ' + name)
        commands = browser.get('commands')
        _require(isinstance(commands, list) and commands, 'native prerequisite raw commands missing')
        for value in controls.values():
            if isinstance(value, dict) and 'response' in value:
                _require(value in commands, 'native verdict differs from raw command transcript')
        _require(controls.get('capabilities', {}).get('acceptInsecureCerts') is False,
                 'native prerequisite bypassed certificate validation')
    return record


def _success(process):
    _require(type(process.get('returncode')) is int and process['returncode'] == 0
             and process.get('timed_out') is False and process.get('error') is None
             and process.get('cleanup_complete') is True and not process.get('cleanup_errors')
             and not process.get('forced_cleanup') and not process.get('cancellation_signal'),
             'prerequisite process failed, timed out, or lost cleanup custody')


def _stage_evidence(bound, prefix, stage):
    """Resolve exact current stage evidence from declared named assets only."""
    assets, run = bound['assets'], bound['run']
    names = {kind: prefix + '_' + kind for kind in ('run', 'execution', 'result', 'process', 'preflight', 'custody', 'output')}
    _require(set(names.values()) <= assets.keys(), 'current prerequisite evidence assets missing: ' + prefix)
    stage_run = load_manifest(assets[names['run']], 'run')
    _require(stage_run['suite']['name'] == STAGES[stage] and stage_run['attempt'] == 1
             and stage_run['original_run_id'] == stage_run['run_id'], 'prerequisite must be its own original stage run')
    for field in ('build', 'runner'):
        _require(stage_run[field] == run[field], 'prerequisite uses different ' + field)
    # Every retained prerequisite input must still be in current custody. The
    # current full run may additionally declare these output evidence records.
    _require(all(record in bound['records'] for record in stage_run['immutable_inputs']),
             'prerequisite input is absent from current immutable closure')
    _require(all(run['assets'].get(name) == record for name, record in stage_run['assets'].items()),
             'prerequisite named input role was substituted')
    registry = _read(assets['core_profile_registry'])
    _require(stage_run['assets']['core_profile_registry'] == run['assets']['core_profile_registry'],
             'prerequisite registry differs')
    _require(stage_run['profile'] in registry['profiles'], 'prerequisite profile missing')
    previous = registry['profiles'][stage_run['profile']]
    current = registry['profiles'][run['profile']]
    _require({k: v for k, v in previous.items() if k != 'fixture'} ==
             {k: v for k, v in current.items() if k != 'fixture'}, 'prerequisite platform/execution policy differs')
    def relevant(fixture):
        return {k: v for k, v in fixture.items() if k not in {'web_features', 'third_party_cookies'}}
    _require(relevant(_read(assets[previous['fixture']])) == relevant(_read(assets[current['fixture']])),
             'profile needs separately admitted native/topology prerequisites')
    execution = _read(assets[names['execution']])
    _require(execution.get('schema_version') == 1 and execution.get('kind') == 'typed-prerequisite-execution'
             and execution.get('stage') == stage and execution.get('run_id') == stage_run['run_id']
             and execution.get('run_manifest_sha256') == _hash(assets[names['run']])
             and execution.get('build') == run['build'] and execution.get('errors') == [],
             'prerequisite execution identity or integrity differs')
    fixed = {'result': 'result.json', 'process': 'process/process-result.json',
             'preflight': 'process/preflight.json', 'custody': 'custody.json'}
    for field, relative in fixed.items():
        _require(execution.get('artifacts', {}).get(relative) == _hash(assets[names[field]]),
                 'prerequisite execution artifact differs: ' + field)
    output_record = run['assets'][names['output']]
    _require(output_record['kind'] == 'directory', 'complete prerequisite output membership missing')
    expected_artifacts = {item['path']: item['sha256'] for item in output_record['members']
                          if item['path'] != 'execution.json'}
    _require(execution.get('artifacts') == expected_artifacts, 'prerequisite output evidence membership differs')
    output_root = assets[names['output']]
    for field, relative in {**fixed, 'execution': 'execution.json'}.items():
        _require(resolve_input(output_root, relative) == assets[names[field]],
                 'prerequisite evidence file lies outside its completed output')
    process = _read(assets[names['process']]); preflight = _read(assets[names['preflight']])
    _success(process)
    argv = preflight.get('argv', [])
    birth = preflight.get('child_identity')
    _require(isinstance(birth, list) and len(birth) == 2 and isinstance(birth[0], list)
             and len(birth[0]) in {1, 2} and type(birth[1]) is int and birth[1] == preflight.get('owner_pid'),
             'prerequisite kernel process birth identity missing')
    _require(argv == process.get('argv') and len(argv) == 15
             and argv[1:5] == ['-B', '-m', 'oryn_validation.prerequisites', 'collect']
             and type(preflight.get('child_pid')) is int and preflight['child_pid'] > 0,
             'prerequisite collector launch evidence missing')
    _require(argv[5::2] == ['--stage', '--run', '--store-root', '--bundle-root', '--output'] and argv[6] == stage,
             'prerequisite collector arguments substituted')
    original_store, original_bundle = Path(argv[10]), Path(argv[12])
    _require(original_store.is_absolute() and original_bundle.is_absolute(), 'prerequisite original roots missing')
    python = stage_run['assets']['python']
    _require(Path(argv[0]) == (original_bundle if python['scope'] == 'bundle' else original_store) / python['path']
             and run['assets'][names['run']]['scope'] == 'store'
             and Path(argv[8]) == original_store / run['assets'][names['run']]['path']
             and Path(argv[14]) == original_store / stage_run['outputs']['results'],
             'prerequisite collector input/output identities differ')
    if stage == 'topology':
        from .fixture_startup import verify_collector_budget
        verify_collector_budget(output_root, preflight, process)
    if stage == 'native':
        verify_worker_binding(execution.get('worker_binding'), stage_run, original_bundle)
    custody = _read(assets[names['custody']])
    expected_digest = manifest_digest([*stage_run['immutable_inputs'], run['assets'][names['run']]])
    _require(custody.get('run_manifest_sha256') == execution['run_manifest_sha256']
             and custody.get('errors') == [] and custody.get('admission', {}).get('inputs_sha256') == expected_digest
             and custody.get('finalization', {}).get('inputs_sha256') == expected_digest,
             'prerequisite immutable-input finalization missing')
    if stage == 'native':
        verify_worker_binding(custody.get('worker_binding'), stage_run, original_bundle)
        verify_worker_binding(_read(assets[names['result']]).get('worker_binding'), stage_run, original_bundle)
        validate_reference_evidence(bound, custody, execution)
    return _read(assets[names['result']])


def validate_native_prerequisites(bound):
    validate_runtime_readiness(bound)
    record = _stage_evidence(bound, 'native_prerequisites', 'native')
    validate_native_observations(record)
    stage_run = load_manifest(bound['assets']['native_prerequisites_run'], 'run')
    contract, _, _ = native_contract(dict(bound, run=stage_run))
    expected_urls = {name: value['url'] for name, value in contract['tls'].items()}
    validate_native_urls(record, expected_urls)
    root = bound['assets']['native_prerequisites_output']
    _success(_read(resolve_input(root, 'protocol-faults/process/process-result.json')))
    _require(record['protocol_faults'] == _read(resolve_input(root, 'protocol-faults/result.json')),
             'native protocol fault result differs from actual child output')
    return record


def validate_runtime_readiness(bound):
    from .runtime_readiness import (validate_descriptors, validate_parent_purity, validate_profile_equivalence,
                                    validate_toolchain, metadata_arguments, PURITY_ARGUMENTS)
    from .cargo_custody import validate_observations
    result = _stage_evidence(bound, 'runtime_readiness', 'runtime')
    _require(not result.get('errors'), 'runtime readiness errors')
    output = bound['assets']['runtime_readiness_output']
    outer = _read(bound['assets']['runtime_readiness_preflight'])['argv']
    def original(name):
        entry = bound['run']['assets'][name]
        return str(Path(outer[12] if entry['scope'] == 'bundle' else outer[10]) / entry['path'])
    _require(result['toolchain_before'] == result['toolchain_after'], 'runtime host toolchain final observation differs')
    validate_toolchain(bound, result['toolchain_before'], result['toolchain_version'])
    cargo = result['toolchain_before']['cargo']['path']
    commands = {'toolchain_version': [result['toolchain_before']['rustc']['path'], '-vV'],
                **{'metadata_' + name: [cargo, *args] for name, args in metadata_arguments(Path(original('source_export'))).items()},
                'parent': [original('runtime'), 'native', '--runtime-info'],
                'worker': [original('engine_worker'), '--runtime-info'],
                **{name: [cargo, *args] for name, args in PURITY_ARGUMENTS.items()}}
    for name in commands:
        process = _read(resolve_input(output, name + '/process-result.json')); _success(process)
        _require(result['processes'][name] == process and process['argv'] == commands[name],
                 'runtime readiness process substitution')
        raw = resolve_input(output, name + '/stdout.log').read_text()
        is_json = name in {'parent', 'worker'} or name.startswith('metadata_')
        expected = (result['descriptors'][name] if name in {'parent', 'worker'} else
                    result['metadata'][name.removeprefix('metadata_')] if name.startswith('metadata_') else
                    result['toolchain_version'] if name == 'toolchain_version' else result['trees'][name])
        _require((_read(resolve_input(output, name + '/stdout.log')) if is_json else raw) == expected,
                 'runtime readiness output substitution')
    _require(set(result['metadata']) == set(PURITY_ARGUMENTS)
             and result['packages_before'] == result['packages_after'], 'Cargo package final integrity differs')
    for name, metadata in result['metadata'].items():
        validate_observations(metadata, result['packages_before'][name], bound['assets']['source_export'])
    validate_descriptors(bound, **result['descriptors'])
    validate_parent_purity(result['trees'])
    equivalence = validate_profile_equivalence(bound)
    _require(result['profile_equivalence'] == equivalence, 'runtime profile equivalence changed')
    return result


def validate_native_urls(record, expected_urls):
    for browser in ('native', 'reference'):
        controls = record[browser]['controls']
        _require(controls.get('tls_urls') == expected_urls, 'native TLS controls used different fixture URLs')
        for name, url in expected_urls.items():
            navigation = controls['tls_' + name]
            _require(navigation.get('method') == 'POST'
                     and navigation.get('path') == '/session/' + controls['session_id'] + '/url'
                     and navigation.get('body') == {'url': url},
                     'native TLS control navigation differs from declared fixture')


def validate_prerequisites(bound):
    """Current native and topology gate; policy axes may share only exact peers."""
    native = validate_native_prerequisites(bound)
    record = _stage_evidence(bound, 'fixture_topology', 'topology')
    from .wpt import relay_environment
    relay_environment(bound)
    _require(not record.get('errors'), 'fixture topology errors')
    _require(record.get('native_result_sha256') == _hash(bound['assets']['native_prerequisites_result']),
             'topology used different native negative controls')
    legacy_algorithms()
    from wpt_topology_probe import validate_observations
    from wpt_fixture_contract import validate
    registry = _read(bound['assets']['core_profile_registry'])
    topology_run = load_manifest(bound['assets']['fixture_topology_run'], 'run')
    fixture_path = bound['assets'][registry['profiles'][topology_run['profile']]['fixture']]
    fixture = _read(fixture_path)
    helper = resolve_input(bound['assets']['wpt_checkout'], 'common/square.png')
    validate_observations(record, _hash(helper))
    validate(fixture, fixture_path.parent, record)
    from .wpt import verify_fixture_services
    from .fixture_tls import verify_receipts
    stage_bound = dict(bound, run=topology_run, path=bound['assets']['fixture_topology_run'])
    outer = _read(bound['assets']['fixture_topology_preflight'])['argv']
    output = bound['assets']['fixture_topology_output']
    verify_fixture_services(stage_bound, {'fixture': fixture}, output, upstream=True, original_roots=(outer[10], outer[12]))
    ports = [p for name, values in fixture['ports'].items() if name.startswith('https') for p in values]
    verify_receipts(output / 'tls-activation', bound['assets']['fixture_tls_policy'],
                    _hash(bound['assets']['fixture_tls_policy']), ports, require_live=False, bound=stage_bound)
    _require(native['tls_probes']['untrusted']['verified'] is False
             and native['tls_probes']['wrong_hostname']['verified'] is False, 'topology negative control linkage failed')
    return {'native': native, 'topology': record}


def _upstream(bound):
    legacy_algorithms()
    from .datasets import verify_dataset
    checkout = bound['assets']['wpt_checkout']
    verify_dataset(_read(bound['assets']['upstream_identity']), checkout, bound['run']['assets']['wpt_checkout'])
    sys.path.insert(0, str(checkout / 'tools'))
    import localpaths  # noqa: F401 -- official upstream dependency registration


def _protocol_faults(bound, lease, output):
    assert_worker_environment(bound)
    _upstream(bound)
    suite = unittest.defaultTestLoader.discover(str(SOURCE_ROOT / 'tools/wptrunner-oryn/tests'),
                                               pattern='test_protocol_faults.py')
    def names(item):
        return [name for child in item for name in names(child)] if isinstance(item, unittest.TestSuite) else [item.id()]
    identities = names(suite)
    lease.checkpoint()
    result = unittest.TestResult(); suite.run(result)
    lease.checkpoint()
    value = {'schema_version': 1, 'tests': identities, 'tests_run': result.testsRun,
             'successful': result.wasSuccessful(), 'failures': [(test.id(), error) for test, error in result.failures],
             'errors': [(test.id(), error) for test, error in result.errors],
             'skipped': [(test.id(), reason) for test, reason in result.skipped]}
    _write(output, value)
    return value


def _collect_native(bound, lease, output):
    legacy_algorithms()
    from wpt_native_controls import (Fixture, Driver, tls_probe, browser_controls,
        focus_lease, reference_capabilities, new_controls)
    contract, fixture_path, fixture = native_contract(bound)
    assets = bound['assets']; ca = fixture_path.parent / fixture['tls']['ca']['path']
    binding = assert_worker_environment(bound)
    def check_inputs():
        lease.checkpoint()
        assert_worker_environment(bound)
    record = {'schema_version': 2, 'errors': [], 'worker_binding': binding, 'native': {'errors': [], 'controls': new_controls(True)},
              'reference': {'errors': [], 'controls': new_controls(False)}}
    token = secrets.token_hex(32); token_path = output / 'native.token'
    fd = os.open(token_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as stream:
        stream.write(token)
    native_args = ['--host', '127.0.0.1', '--port', '{port}', '--browser-binary', str(assets['runtime']),
                   '--auth-token-file', str(token_path), '--configuration', str(fixture_path),
                   '--configuration-sha256', _hash(fixture_path)]
    specs = [('native', assets['driver'], native_args, token,
              {'browserName': 'oryn', 'acceptInsecureCerts': False}, (0,)),
             ('reference', assets['reference_driver'], ['--port={port}'], None,
              reference_capabilities(assets['reference_browser']), (0, -signal.SIGTERM))]
    try:
        with focus_lease(), Fixture(contract, check_inputs=check_inputs):
            record['focus_isolation'] = 'host_serialized'
            record['tls_probes'] = {kind: tls_probe(entry, ca) for kind, entry in contract['tls'].items()}
            for name, binary, arguments, credential, capabilities, allowed in specs:
                driver = Driver(binary, arguments, output / name, credential, allowed, check_inputs=check_inputs)
                data = record[name]
                try:
                    with driver:
                        check_inputs()
                        data['controls'] = browser_controls(driver.wire, contract, capabilities,
                                                            name == 'native', results=data['controls'])
                        check_inputs()
                except Exception as error:
                    data['errors'].append(type(error).__name__ + ': ' + str(error))
                data['cleanup'] = driver.cleanup
                if hasattr(driver, 'wire'):
                    data['commands'] = driver.wire.log
        fault_directory = output / 'protocol-faults'
        fault_directory.mkdir()
        command = [str(assets['python']), '-B', '-m', 'oryn_validation.prerequisites', 'protocol-faults',
                   '--stage', 'native', '--run', str(bound['path']), '--store-root', str(bound['store_root']),
                   '--bundle-root', str(bound['bundle_root']), '--output', str(fault_directory / 'result.json')]
        check_inputs()
        execution = run_owned(command, cwd=SOURCE_ROOT, output=fault_directory / 'process',
                              env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1',
                                       PYTHONPATH=str(SOURCE_ROOT / 'tools/validation')), timeout_seconds=120)
        _success(execution)
        record['protocol_faults'] = _read(fault_directory / 'result.json')
    except Exception as error:
        record['errors'].append(type(error).__name__ + ': ' + str(error))
    finally:
        token_path.unlink(missing_ok=True)
    try:
        validate_native_observations(record)
    except Exception as error:
        record['errors'].append(str(error))
    return record


def _collect_topology(bound, lease, output):
    del output
    legacy_algorithms()
    from wpt_fixture_contract import validate
    from wpt_topology_probe import probe, validate_observations
    native = validate_native_prerequisites(bound)
    _upstream(bound)
    _, fixture_path, fixture = _selected(bound)
    helper = resolve_input(bound['assets']['wpt_checkout'], 'common/square.png')
    ca = fixture_path.parent / fixture['tls']['ca']['path']
    result = {'schema_version': 2, 'errors': [], 'contract_sha256': object_digest(fixture),
              'endpoints': [], 'untrusted_certificate_rejected': native['tls_probes']['untrusted']['verified'] is False,
              'wrong_hostname_rejected': native['tls_probes']['wrong_hostname']['verified'] is False,
              'native_result_sha256': _hash(bound['assets']['native_prerequisites_result'])}
    for route in fixture['endpoint_routes']:
        try:
            lease.checkpoint()
            result['endpoints'].append(probe(route, ca, helper.read_bytes()))
        except Exception as error:
            result['errors'].append({'protocol': route['protocol'], 'port': route['port'], 'error': str(error)})
    try:
        validate_observations(result, _hash(helper))
        validate(fixture, fixture_path.parent, result)
    except Exception as error:
        result['errors'].append({'error': str(error)})
    return result


def _collect_runtime(bound, lease, output):
    from .runtime_readiness import (validate_descriptors, validate_parent_purity, validate_profile_equivalence,
                                    validate_toolchain, metadata_arguments, PURITY_ARGUMENTS)
    from .build_evidence import _toolchain, _reject_implicit_cargo_configuration
    from .cargo_custody import observe_packages
    assets, records = bound['assets'], bound['run']['assets']
    _require(records['engine_worker']['executable'], 'unsigned worker runtime-info needs executable input')
    environment = {key: value for key, value in os.environ.items()
                   if key in {'PATH', 'HOME', 'USER', 'LOGNAME', 'TMPDIR', 'LANG', 'LC_ALL'}}
    environment.update(worker_environment(bound))
    environment.update(CARGO_NET_OFFLINE='true', GIT_OPTIONAL_LOCKS='0')
    result = {'errors': [], 'descriptors': {}, 'trees': {}, 'processes': {}, 'metadata': {}, 'packages_before': {}, 'packages_after': {}}
    try:
        _reject_implicit_cargo_configuration(assets['source_export'], environment['HOME'])
        toolchain = _toolchain(environment, assets['source_export']); result['toolchain_before'] = toolchain
        environment['RUSTC'] = toolchain['rustc']['path']
        environment['PATH'] = str(Path(toolchain['cargo']['path']).parent) + os.pathsep + environment.get('PATH', '')
        commands = {'toolchain_version': [toolchain['rustc']['path'], '-vV'],
                    **{'metadata_' + name: [toolchain['cargo']['path'], *args]
                       for name, args in metadata_arguments(assets['source_export']).items()},
                    'parent': [str(assets['runtime']), 'native', '--runtime-info'],
                    'worker': [str(assets['engine_worker']), '--runtime-info'],
                    **{name: [toolchain['cargo']['path'], *args] for name, args in PURITY_ARGUMENTS.items()}}
        for name, command in commands.items():
            lease.checkpoint()
            process = run_owned(command, cwd=assets['source_export'], output=output / name,
                                env=environment, timeout_seconds=120)
            result['processes'][name] = process; _success(process)
            if name in {'parent', 'worker'}:
                result['descriptors'][name] = _read(output / name / 'stdout.log')
            elif name == 'toolchain_version':
                result['toolchain_version'] = (output / name / 'stdout.log').read_text()
                validate_toolchain(bound, toolchain, result['toolchain_version'])
            elif name.startswith('metadata_'):
                graph = name.removeprefix('metadata_')
                result['metadata'][graph] = _read(output / name / 'stdout.log')
                result['packages_before'][graph] = observe_packages(result['metadata'][graph], assets['source_export'],
                                                                   Path(environment['HOME']) / '.cargo')
            else:
                result['trees'][name] = (output / name / 'stdout.log').read_text()
        for graph, metadata in result['metadata'].items():
            result['packages_after'][graph] = observe_packages(metadata, assets['source_export'], Path(environment['HOME']) / '.cargo')
        _require(result['packages_before'] == result['packages_after'], 'offline Cargo package sources changed during graph probes')
        _reject_implicit_cargo_configuration(assets['source_export'], environment['HOME'])
        result['toolchain_after'] = _toolchain(environment, assets['source_export'])
        _require(result['toolchain_before'] == result['toolchain_after'], 'host Rust toolchain changed during readiness')
        validate_descriptors(bound, **result['descriptors'])
        validate_parent_purity(result['trees'])
        result['profile_equivalence'] = validate_profile_equivalence(bound)
    except Exception as error:
        result['errors'].append(type(error).__name__ + ': ' + str(error))
    return result


def collect(bound, stage, output):
    """Child collector; the outer owned process provides descendant settlement."""
    output = Path(output)
    _require(bound['run']['suite']['name'] == STAGES[stage], 'collector run suite differs')
    _require(output == bound['store_root'] / bound['run']['outputs']['results'], 'collector output differs')
    errors, lease = [], InputLease(bound['records'], store_root=bound['store_root'], bundle_root=bound['bundle_root'])
    reference = None
    binding = None
    try:
        with ExitStack() as stack:
            stack.enter_context(lease)
            boundary = lease
            if stage == 'native':
                from .reference_environment import ReferenceLease
                identity, _ = reference_environment(bound, verify=False)
                reference = ReferenceLease(identity, bound['store_root'])
                stack.enter_context(reference)
                boundary = _NativeInputBoundary(lease, reference)
            validate_production(bound)
            binding = assert_worker_environment(bound)
            _selected(bound)
            if stage == 'native':
                validate_runtime_readiness(bound)
            result = {'native': _collect_native, 'topology': _collect_topology, 'runtime': _collect_runtime}[stage](bound, boundary, output)
            _write(output / 'result.json', result)
            _require(not result['errors'], 'prerequisite observations failed')
    except BaseException as error:
        errors.append(type(error).__name__ + ': ' + str(error))
        raise
    finally:
        primary_failure = bool(errors)
        final_error = None
        try:
            _require(_hash(bound['path']) == bound['sha256'], 'prerequisite run manifest changed')
            if binding is not None:
                assert_worker_environment(bound)
        except BaseException as error:
            final_error = error
            errors.append(type(error).__name__ + ': ' + str(error))
        receipt = {'run_manifest_sha256': bound['sha256'], 'errors': errors,
                   'admission': lease.admission, 'finalization': lease.finalization,
                   'worker_binding': binding}
        if stage == 'native':
            reference_errors = ([str(reference.final_error)] if reference is not None and reference.final_error else [])
            receipt['reference_environment'] = {
                'admission': reference.admission if reference is not None else None,
                'finalization': reference.finalization if reference is not None else None,
                'errors': reference_errors}
        _write(output / 'custody.json', receipt)
        if final_error is not None and not primary_failure:
            raise final_error


def execute(path, *, store_root, bundle_root, stage, timeout_seconds=600):
    """One serialized current prerequisite attempt; no hidden restart or retry."""
    bound = load_inputs(path, store_root=store_root, bundle_root=bundle_root)
    validate_production(bound)
    _, fixture_path, fixture = _selected(bound)
    _require(stage in STAGES and bound['run']['suite']['name'] == STAGES[stage], 'stage suite differs')
    from .bundle_ownership import registered_bundle_owner
    from .wpt import python_environment, relay_environment
    build_objects = [registered_bundle_owner(bound)]
    staged_environment = python_environment(bound)
    binding = selected_worker(bound)
    reference_before = reference_environment(bound)[1] if stage == 'native' else None
    if stage == 'topology':
        relay_environment(bound)
    output = bound['store_root'] / bound['run']['outputs']['results']
    for parent in (output, *output.parents):
        _require(not parent.is_symlink(), 'prerequisite output traverses a symlink')
    output.mkdir(parents=True, exist_ok=False)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(('ORYN_WPT_', 'ORYN_GENERATION_', 'ORYN_FIXTURE_TLS_', 'PYTHON', 'DYLD_'))}
    environment.update(PYTHONDONTWRITEBYTECODE='1', PYTHONPATH=str(SOURCE_ROOT / 'tools/validation'))
    if stage == 'topology':
        environment.update(ORYN_VALIDATION_STORE_ROOT=str(bound['store_root']),
                           ORYN_VALIDATION_BUNDLE_ROOT=str(bound['bundle_root']), ORYN_WPT_RUN_CONTRACT=str(bound['path']),
                           ORYN_WPT_RUN_CONTRACT_SHA256=bound['sha256'],
                           ORYN_FIXTURE_TLS_CONTRACT=str(bound['assets']['fixture_tls_policy']),
                           ORYN_FIXTURE_TLS_CONTRACT_SHA256=_hash(bound['assets']['fixture_tls_policy']),
                           ORYN_FIXTURE_TLS_RECEIPTS=str(output / 'tls-activation'),
                           PYTHONPATH=os.pathsep.join(str(SOURCE_ROOT / part) for part in
                              ('tools/validation/bootstrap', 'tools/validation', 'tools/wptrunner-oryn')))
        (output / 'tls-activation').mkdir()
    environment.update(staged_environment)
    environment.update(worker_environment(bound))
    assert_worker_environment(bound, environment)
    command = [str(bound['assets']['python']), '-B', '-m', 'oryn_validation.prerequisites', 'collect',
               '--stage', stage, '--run', str(bound['path']), '--store-root', str(bound['store_root']),
               '--bundle-root', str(bound['bundle_root']), '--output', str(output)]
    errors, process = [], None
    resource_keys = list(RESOURCE_KEYS)
    if stage == 'runtime':
        from .builds import cargo_cache_key
        resource_keys.append('cargo-cache:' + cargo_cache_key(_read(bound['assets']['build_manifest'])['configuration']))
    try:
        with InputLease(bound['records'], store_root=store_root, bundle_root=bundle_root) as lease:
            with ResourceLease(store_root, resource_keys, objects=build_objects) as resource:
                services = []
                process_attempted = False
                try:
                    if stage == 'topology':
                        from .wpt import fixture_services
                        from .fixture_tls import verify_receipts
                        selected = {'fixture': fixture, 'fixture_path': fixture_path}
                        with fixture_services(bound, selected, output=output, environment=environment,
                                              lease=lease, timeout_seconds=timeout_seconds, upstream=True, services=services):
                            from .fixture_startup import collector_budget
                            budget = collector_budget(services)
                            _write(output / 'collector-budget.json', budget)
                            process_attempted = True
                            process = run_owned(command, cwd=SOURCE_ROOT, output=output / 'process', env=environment,
                                                timeout_seconds=budget['timeout_seconds'])
                            _success(process)
                            from .fixture_startup import verify_collector_budget
                            verify_collector_budget(output, _read(output / 'process/preflight.json'), process)
                            ports = [p for name, values in fixture['ports'].items() if name.startswith('https') for p in values]
                            verify_receipts(output / 'tls-activation', bound['assets']['fixture_tls_policy'],
                                            _hash(bound['assets']['fixture_tls_policy']), ports, bound=bound)
                    else:
                        process_attempted = True
                        process = run_owned(command, cwd=SOURCE_ROOT, output=output / 'process', env=environment,
                                            timeout_seconds=timeout_seconds)
                        _success(process)
                finally:
                    from .fixture_startup import settlement, require_settlement
                    settled = settlement(process, services, process_attempted=process_attempted)
                    _write(output / 'resource-settlement.json', settled)
                    resource.mark_settled(settled)
                    require_settlement(settled, sys.exc_info()[1])
    except BaseException as error:
        errors.append(type(error).__name__ + ': ' + str(error))
        raise
    finally:
        # Recheck external observations only after owned descendants settle.
        primary_failure = bool(errors)
        final_errors, reference_errors = [], []
        reference_after = None
        for label, check in (
                ('Python environment', lambda: python_environment(bound)),
                ('run manifest', lambda: _require(_hash(bound['path']) == bound['sha256'],
                                                  'prerequisite run manifest changed'))):
            try:
                check()
            except BaseException as error:
                final_errors.append(error)
                errors.append(label + ' finalization: ' + type(error).__name__ + ': ' + str(error))
        if stage == 'topology':
            try:
                relay_environment(bound)
            except BaseException as error:
                final_errors.append(error)
                errors.append('Fixture relay finalization: ' + type(error).__name__ + ': ' + str(error))
        if stage == 'native':
            try:
                reference_after = reference_environment(bound)[1]
            except BaseException as error:
                final_errors.append(error)
                reference_errors.append(type(error).__name__ + ': ' + str(error))
                errors.append('Reference environment finalization: ' + reference_errors[-1])
        record = {'schema_version': 1, 'kind': 'typed-prerequisite-execution',
              'stage': stage, 'run_id': bound['run']['run_id'], 'run_manifest_sha256': bound['sha256'],
              'build': bound['run']['build'], 'errors': errors, 'worker_binding': binding,
              'artifacts': {str(p.relative_to(output)): _hash(p) for p in output.rglob('*') if p.is_file()}}
        if stage == 'native':
            record['reference_environment'] = {'admission': reference_before,
                'finalization': reference_after, 'errors': reference_errors}
        _write(output / 'execution.json', record)
        if final_errors and not primary_failure:
            raise final_errors[0]
    return _read(output / 'result.json')


def main(argv=None):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('mode', choices=['execute', 'collect', 'protocol-faults'])
    parser.add_argument('--stage', choices=STAGES, required=True)
    for name in ('run', 'store-root', 'bundle-root'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args(argv)
    if args.mode == 'execute':
        _require(args.output is None, 'outer output comes only from the run manifest')
        execute(args.run, store_root=args.store_root, bundle_root=args.bundle_root, stage=args.stage)
    elif args.mode == 'collect':
        _require(args.output is not None, 'collector requires exact output')
        collect(load_inputs(args.run, store_root=args.store_root, bundle_root=args.bundle_root), args.stage, args.output)
    else:
        bound = load_inputs(args.run, store_root=args.store_root, bundle_root=args.bundle_root)
        _require(args.stage == 'native' and args.output == bound['store_root'] / bound['run']['outputs']['results'] / 'protocol-faults/result.json',
                 'protocol control output differs')
        with InputLease(bound['records'], store_root=args.store_root, bundle_root=args.bundle_root) as lease:
            result = _protocol_faults(bound, lease, args.output)
            assert_worker_environment(bound)
            _require(result['successful'] and not result['skipped'], 'protocol controls failed')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

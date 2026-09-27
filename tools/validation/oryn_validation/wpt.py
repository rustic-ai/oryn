"""Typed WPT planning and execution through reviewed current-input validators.

No direct browser loop or substitute executor is provided. The adapter and
upstream wptrunner continue to own WebDriver, fixtures, and test semantics.
"""
import argparse
from contextlib import ExitStack, contextmanager
import configparser
import hashlib
import json
import os
import secrets
import socket
from pathlib import Path
import sys
import time

from .admission import (SOURCE_ROOT, legacy_algorithms, read_object, require_migrated_gates,
                        validate_inventory, validate_registry)
from .inputs import InputLease, resolve_input, snapshot_input, verify_inputs
from .manifests import load_manifest

REQUIRED_ASSETS = frozenset({'runtime', 'driver', 'worker_manifest', 'fixture_repairs',
                           'core_profile_registry', 'coverage', 'inventory_plan', 'core_manifest',
                           'policy', 'include', 'runner_config', 'wpt_checkout', 'python',
                           'runner_source', 'runner_identity', 'core_include', 'fixture_relay',
                           'fixture_tls_policy', 'fixture_tls_module', 'fixture_tls_bootstrap',
                           'execution_policy', 'upstream_identity', 'python_environment',
                           'python_environment_identity', 'python_dependency_lock', 'probe'})
SUITES = {'wpt-core': 'core', 'wpt-canaries': 'canaries', 'wpt-placement': 'placement'}

# These are the reviewed original process budgets. Consolidation removes the
# redundant outer runner, not the original test budget or serial controls.
EXECUTION_POLICY = {
    'schema_version': 1, 'kind': 'wpt-original-execution-policy',
    'core_processes': 6, 'core_timeout_seconds': 345600,
    'control_timeout_seconds': 43200, 'relay_timeout_seconds': 1296000,
}


def execution_policy(bound, collection, override=None):
    policy = read_object(bound['assets']['execution_policy'])
    if (set(policy) != set(EXECUTION_POLICY)
            or any(type(policy[key]) is not type(value) or policy[key] != value
                   for key, value in EXECUTION_POLICY.items())):
        raise ValueError('execution policy differs from the reviewed original budgets')
    if collection not in ('core', 'canaries', 'placement'):
        raise ValueError('unknown original collection')
    timeout = policy['core_timeout_seconds' if collection == 'core' else 'control_timeout_seconds']
    if override is not None and (type(override) is not int or override != timeout):
        raise ValueError('timeout override differs from immutable execution policy')
    return dict(policy, timeout_seconds=timeout)



def _digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def load_inputs(path, *, store_root, bundle_root):
    """Resolve only schema-declared input records; history and outputs stay inert."""
    path = Path(path).absolute()
    run = load_manifest(path, expected_kind='run')
    assets = run.get('assets', {})
    missing = REQUIRED_ASSETS - assets.keys()
    if missing:
        raise ValueError('WPT run lacks typed assets: ' + ', '.join(sorted(missing)))
    records = {}
    for item in [*run['immutable_inputs'], *assets.values()]:
        key = item['scope'], item['path']
        if key in records and records[key] != item:
            raise ValueError('conflicting current WPT input identities')
        records[key] = item
    manifest_record = snapshot_input(path.parent, path.name, scope='store', kind='file')
    # The run manifest itself is protected separately because it may reside in
    # runs/<id> while immutable assets resolve relative to the configured store.
    paths = {name: resolve_input(bundle_root if item['scope'] == 'bundle' else store_root,
                                 item['path']) for name, item in assets.items()}
    verify_inputs(records.values(), store_root=store_root, bundle_root=bundle_root)
    return dict(run=run, path=path, sha256=manifest_record['sha256'], assets=paths,
                records=list(records.values()), store_root=Path(store_root).absolute(), bundle_root=Path(bundle_root).absolute())


def _check_config(path, assets, collection='core'):
    """INI inputs must be relative and resolve to explicit typed assets."""
    config = configparser.ConfigParser(interpolation=None)
    with Path(path).open() as stream:
        config.read_file(stream)
    section, source, manifest, metadata, mount = {
        'core': ('manifest:official', 'wpt_checkout', 'core_manifest', 'core_metadata', '/'),
        'canaries': ('manifest:oryn-qualification', 'canary_source', 'canary_manifest', 'canary_metadata', '/__oryn_qualification__/'),
        'placement': ('manifest:oryn-shared-memory', 'placement_source', 'placement_manifest', 'placement_metadata', '/__oryn_shared_memory__/'),
    }[collection]
    if config.defaults() or set(config.sections()) != {section}:
        raise ValueError('runner config must declare only its selected collection manifest')
    values = dict(config[section])
    if set(values) != {'tests', 'metadata', 'manifest', 'url_base'} or values['url_base'] != mount:
        raise ValueError('runner manifest configuration differs')
    allowed = {Path(value).resolve() for value in assets.values()}
    for name in ('tests', 'metadata', 'manifest'):
        value = Path(values[name])
        if value.is_absolute():
            raise ValueError('runner config must be data-root relocatable')
        target = (Path(path).parent / value).resolve()
        if target not in allowed:
            raise ValueError('runner config input absent from typed closure: ' + name)
        if name == 'tests' and target != assets[source]:
            raise ValueError('runner config substitutes collection source')
        if name == 'manifest' and target != assets[manifest]:
            raise ValueError('runner config substitutes collection manifest')
        if name == 'metadata' and target != assets.get(metadata):
            raise ValueError('runner config substitutes collection metadata')


def python_environment(bound):
    """Join the verified Python environment to this run's exact named inputs."""
    from .environments import runtime_environment
    identity = read_object(bound['assets']['python_environment_identity'])
    for field, role in {'environment': 'python_environment', 'python': 'python',
                        'dependency_lock': 'python_dependency_lock', 'probe': 'probe'}.items():
        if identity.get(field) != bound['run']['assets'].get(role):
            raise ValueError('Python environment substitutes declared run input: ' + role)
    return runtime_environment(identity, bound['store_root'])


def inspect_admission(bound):
    """Run currently migrated semantic checks without pretending all gates exist."""
    run, assets = bound['run'], bound['assets']
    if assets['runner_source'] != SOURCE_ROOT:
        raise ValueError('official execution must use the declared committed runner export')
    from .production import validate_source_export
    runner = validate_source_export(read_object(assets['runner_identity']), assets['runner_source'],
                                    run['assets']['runner_source'])
    if runner['commit'] != run['runner']['commit'] or runner['tree_sha256'] != run['runner']['tree_sha256']:
        raise ValueError('runner source does not prove the declared commit/tree identity')
    from .datasets import verify_dataset
    dataset = verify_dataset(read_object(assets['upstream_identity']), assets['wpt_checkout'], run['assets']['wpt_checkout'])
    legacy_algorithms()
    from oryn_wptrunner import WPT_REVISION
    if dataset['commit'] != WPT_REVISION:
        raise ValueError('upstream dataset commit differs from pinned WPT')
    if run['suite']['upstream_commit'] != WPT_REVISION or run['suite']['name'] not in SUITES:
        raise ValueError('typed launch requires a pinned WPT collection')
    collection = SUITES[run['suite']['name']]
    execution_policy(bound, collection)
    environment = python_environment(bound)
    if run['suite']['sha256'] != _digest(assets['core_manifest']):
        raise ValueError('suite identity differs from pinned full WPT manifest')
    if run['workers'] != (6 if collection == 'core' else 1):
        raise ValueError('current policy requires six Core workers and serial controls')
    if run['attempt'] != 1 or run['original_run_id'] != run['run_id']:
        raise ValueError('new full-original path cannot substitute or append a retry')
    registry = read_object(assets['core_profile_registry'])
    fixture = validate_registry(registry, assets, selected_profile=run['profile'])
    if registry['coverage_sha256'] != _digest(assets['coverage']):
        raise ValueError('registry coverage differs from selected inventory')
    urls = validate_inventory(assets, expected_automated=15985, expected_assessments=752)
    if collection == 'canaries':
        from wpt_canary_qualification import MOUNT, EXPECTED
        urls = sorted(MOUNT + name for name in EXPECTED)
        if assets.get('canary_source') != SOURCE_ROOT / 'tools/wptrunner-oryn':
            raise ValueError('canary source differs from committed runner fixtures')
    elif collection == 'placement':
        from wpt_shared_placement import TEST
        urls = [TEST]
        if assets.get('placement_source') != SOURCE_ROOT / 'crates/oryn-native/tests/fixtures':
            raise ValueError('placement source differs from committed native fixtures')
    if run['expected'] != {'automated': len(urls), 'assessments': 752 if collection == 'core' else 0}:
        raise ValueError('selected collection population differs')
    included = Path(assets['include']).read_text().splitlines()
    if len(included) != len(set(included)) or set(included) != set(urls):
        raise ValueError('selected include list differs from exact collection')
    _check_config(assets['runner_config'], assets, collection)
    fixture_path = assets[registry['profiles'][run['profile']]['fixture']]
    from .fixture_tls import load_contract as tls_contract
    tls_contract(assets['fixture_tls_policy'], _digest(assets['fixture_tls_policy']), bound=bound)
    return dict(fixture=fixture, fixture_path=fixture_path, urls=urls, collection=collection,
                python_environment=environment)


def official_admission(bound):
    selected = inspect_admission(bound)
    from .production import validate_production
    selected['production'] = validate_production(bound)
    from .prerequisites import validate_prerequisites
    selected['prerequisites'] = validate_prerequisites(bound)
    # Port review never substitutes for current native, topology, and control
    # evidence. Every semantic validator above and below remains mandatory.
    require_migrated_gates(selected['collection'])
    if selected['collection'] == 'core':
        from .controls import verify_controls
        selected['controls'] = verify_controls(bound)
    return selected


def plan_command(bound, selected, output):
    """Construct the one original's fixed upstream command, not a shell string."""
    assets, run = bound['assets'], bound['run']
    fixture, root = selected['fixture'], selected['fixture_path'].parent
    command = [str(assets['python']), '-B', '-m', 'oryn_validation.wpt', 'upstream',
               '--run', str(bound['path']), '--store-root', str(bound['store_root']),
               '--bundle-root', str(bound['bundle_root']), '--',
               '--product', 'oryn', '--oryn-run-contract', str(bound['path']),
               '--binary', str(assets['runtime']), '--webdriver-binary', str(assets['driver']),
               '--config', str(assets['runner_config']), '--no-manifest-update', '--no-manifest-download',
               '--ssl-type', 'pregenerated']
    for option, name in (('ca-cert-path', 'ca'), ('host-cert-path', 'certificate'), ('host-key-path', 'key')):
        command += ['--' + option, str(root / fixture['tls'][name]['path'])]
    command += ['--include-file', str(assets['include']), '--processes', str(run['workers']),
                '--repeat', '1', '--rerun', '1', '--retry-unexpected', '0',
                '--no-pause-after-test', '--no-install-fonts',
                '--log-wptreport', str(Path(output) / 'wptreport.json'),
                '--log-raw', str(Path(output) / 'mozlog.jsonl'),
                '--test-types']
    command += {'core': ['crashtest', 'print-reftest', 'reftest', 'testharness'],
                'canaries': ['reftest', 'testharness'], 'placement': ['testharness']}[selected.get('collection', 'core')]
    return command


def prepare_controls(records, *, store_root, runner_source, wpt_checkout, upstream_identity, output):
    """Generate genuine upstream local manifests before sealing run inputs.

    This is preparation only: no browser, native test, or qualification runs.
    Caller later records every generated input with snapshot_input and builds a
    reviewed run manifest. Relative INI references survive data-root relocation.
    """
    legacy_algorithms()
    from oryn_wptrunner import WPT_REVISION
    runner_source, wpt_checkout = Path(runner_source).absolute(), Path(wpt_checkout).absolute()
    declared = {resolve_input(store_root, record['path']) for record in records
                if record['scope'] == 'store' and record['kind'] == 'directory'}
    if runner_source not in declared or wpt_checkout not in declared:
        raise ValueError('preparation requires explicit runner and upstream directory inputs')
    output = Path(output).absolute()
    if not output.is_relative_to(Path(store_root).absolute()) or output.exists():
        raise ValueError('preparation requires a fresh output inside the configured store')
    for path in (output, *output.parents):
        if path.is_symlink():
            raise ValueError('preparation output traverses a symlink')
    if any(output == path or output.is_relative_to(path) for path in declared):
        raise ValueError('preparation output overlaps immutable source')
    identities = {resolve_input(store_root, item['path']): item for item in records if item['scope'] == 'store'}
    upstream_identity = Path(upstream_identity).absolute()
    if upstream_identity not in identities:
        raise ValueError('preparation requires declared upstream commit identity')
    with InputLease(records, store_root=store_root):
        from .datasets import verify_dataset
        revision = verify_dataset(read_object(upstream_identity), wpt_checkout, identities[wpt_checkout])['commit']
        if revision != WPT_REVISION:
            raise ValueError('upstream preparation revision differs')
        sys.path.insert(0, str(wpt_checkout / 'tools'))
        import localpaths  # noqa: F401
        from manifest.manifest import load_and_update
        from wpt_canary_qualification import MOUNT, EXPECTED
        from wpt_shared_placement import TEST
        output.mkdir(parents=True, exist_ok=False)
        declarations = {
            'canaries': (runner_source / 'tools/wptrunner-oryn', 'canaries', '/__oryn_qualification__/',
                         'manifest:oryn-qualification', {MOUNT + name for name in EXPECTED}),
            'placement': (runner_source / 'crates/oryn-native/tests/fixtures', 'shared-memory', '/__oryn_shared_memory__/',
                          'manifest:oryn-shared-memory', {TEST}),
        }
        generated = {}
        for kind, (source, subdir, mount, section, expected) in declarations.items():
            target = output / kind
            target.mkdir()
            (target / 'metadata').mkdir()
            manifest_path = target / 'manifest.json'
            manifest = load_and_update(str(source), str(manifest_path), mount, update=True,
                                       rebuild=True, parallel=False, paths_to_update=[subdir],
                                       cache_root=str(target / 'manifest-cache'))
            discovered = {(test_type, test.id) for test_type, _, items in manifest.itertypes('testharness', 'reftest')
                          for test in items}
            expected_types = {('reftest' if url.endswith(('reference-mismatch.html', 'reference-never-ready.html')) else 'testharness', url)
                              for url in expected}
            if not expected_types <= discovered:
                raise ValueError('upstream omitted or misclassified required ' + kind + ' controls')
            (target / 'include').write_text(''.join(url + '\n' for url in sorted(expected)))
            config = configparser.ConfigParser(interpolation=None)
            config[section] = {'tests': os.path.relpath(source, target), 'metadata': 'metadata',
                               'manifest': 'manifest.json', 'url_base': mount}
            with (target / 'runner.ini').open('x') as stream:
                config.write(stream)
            generated[kind] = {name: str((target / name).relative_to(Path(store_root).absolute()))
                               for name in ('manifest.json', 'include', 'runner.ini', 'metadata')}
        with (output / 'preparation.json').open('x') as stream:
            json.dump({'schema_version': 1, 'kind': 'control-preparation', 'upstream_revision': revision,
                       'qualification': 'not-evaluated', 'collections': generated}, stream, indent=2)
        return generated


def execute(path, *, store_root, bundle_root, output, timeout_seconds=None):
    """Owned upstream invocation after all typed admission gates have passed."""
    from .processes import run_owned, ResourceLease
    from .results import read_mozlog, reconcile_wpt
    bound = load_inputs(path, store_root=store_root, bundle_root=bundle_root)
    selected = official_admission(bound)
    policy = execution_policy(bound, selected['collection'], timeout_seconds)
    timeout_seconds = policy['timeout_seconds']
    from .bundle_ownership import registered_bundle_owner
    build_objects = [registered_bundle_owner(bound)]
    output = Path(output).absolute()
    expected_output = Path(store_root).absolute() / bound['run']['outputs']['results']
    if output != expected_output:
        raise ValueError('output differs from declared original destination')
    if bound['run']['outputs']['events'] != bound['run']['outputs']['results'] + '/mozlog.jsonl':
        raise ValueError('event destination differs from the declared original')
    for parent in (output, *output.parents):
        if parent.is_symlink():
            raise ValueError('output traverses a symlink')
    output.mkdir(parents=True, exist_ok=False)
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith(('ORYN_WPT_', 'ORYN_GENERATION_', 'ORYN_FIXTURE_TLS_', 'PYTHON', 'DYLD_'))}
    environment.update(ORYN_VALIDATION_STORE_ROOT=str(store_root),
                       ORYN_VALIDATION_BUNDLE_ROOT=str(bundle_root),
                       ORYN_WPT_RUN_CONTRACT=str(bound['path']),
                       ORYN_WPT_RUN_CONTRACT_SHA256=bound['sha256'],
                       ORYN_FIXTURE_TLS_CONTRACT=str(bound['assets']['fixture_tls_policy']),
                       ORYN_FIXTURE_TLS_CONTRACT_SHA256=_digest(bound['assets']['fixture_tls_policy']),
                       ORYN_FIXTURE_TLS_RECEIPTS=str(output / 'tls-activation'),
                       ORYN_CORE_COLLECTION=selected['collection'], ORYN_WPT_CANDIDATE_SHA256=bound['run']['build']['manifest_sha256'],
                       ORYN_WPT_PROTOCOL_CUSTODY=str(output / 'protocol-custody.jsonl'),
                       PYTHONDONTWRITEBYTECODE='1',
                       PYTHONPATH=os.pathsep.join((str(SOURCE_ROOT / 'tools/validation/bootstrap'),
                                                 str(SOURCE_ROOT / 'tools/validation'),
                                                 str(SOURCE_ROOT / 'tools/wptrunner-oryn'))))
    environment.update(selected['python_environment'])
    (output / 'tls-activation').mkdir()
    with (output / 'protocol-custody.jsonl').open('x'):
        pass
    if selected['collection'] == 'canaries':
        environment['ORYN_WPT_CANARY_RESOURCE_AUDIT'] = '1'
    elif selected['collection'] == 'placement':
        from wpt_shared_placement import TEST
        context = {'schema': 1, 'test': TEST, 'nonce': secrets.token_hex(32),
                   'run_contract': str(bound['path']), 'candidate_sha256': bound['run']['build']['manifest_sha256'],
                   'run_contract_sha256': bound['sha256'], 'output': str(output / 'placement.json')}
        with (output / 'placement-context.json').open('x') as stream:
            json.dump(context, stream)
        environment['ORYN_WPT_PLACEMENT_CONTEXT'] = str(output / 'placement-context.json')
    lease = InputLease(bound['records'], store_root=store_root, bundle_root=bundle_root)
    execution = None
    integrity_errors = []
    services = []
    try:
        with lease:
            with ResourceLease(store_root, ['wpt-fixed-ports', 'native-focus'], objects=build_objects) as resource:
                with fixture_services(bound, selected, output=output, environment=environment,
                                      lease=lease, timeout_seconds=policy['relay_timeout_seconds']) as services:
                    execution = run_owned(plan_command(bound, selected, output), cwd=bound['assets']['wpt_checkout'],
                                          output=output / 'process', env=environment, timeout_seconds=timeout_seconds)
                    for service in services:
                        service.check()
                    from .fixture_tls import verify_receipts
                    tls_ports = [port for name, ports in selected['fixture']['ports'].items()
                                 if name.startswith('https') for port in ports]
                    verify_receipts(output / 'tls-activation', bound['assets']['fixture_tls_policy'],
                                    _digest(bound['assets']['fixture_tls_policy']), tls_ports,
                                    require_live=False, bound=bound)
                fixture_cleanup = [service.result for service in services]
                execution['fixture_services'] = fixture_cleanup
                for service in fixture_cleanup:
                    if service.get('cleanup_complete') is not True or service.get('cleanup_errors'):
                        execution['cleanup_complete'] = False
                        execution['cleanup_errors'].append('fixture service cleanup incomplete')
                resource.mark_settled(execution)
    except BaseException as error:
        integrity_errors.append(type(error).__name__ + ': ' + str(error))
        raise
    finally:
        if _digest(bound['path']) != bound['sha256']:
            integrity_errors.append('run manifest changed during execution')
        if execution is not None:
            record = {'schema_version': 1, 'kind': 'typed-wpt-execution',
                      'run_id': bound['run']['run_id'], 'run_manifest_sha256': bound['sha256'],
                      'candidate_sha256': bound['run']['build']['manifest_sha256'],
                      'profile': bound['run']['profile'], 'collection': selected['collection'],
                      'execution_policy': policy,
                      'returncode': execution['returncode'], 'timed_out': execution['timed_out'],
                      'cleanup_complete': execution['cleanup_complete'], 'launch_error': execution['error'],
                      'errors': integrity_errors, 'custody_complete': not integrity_errors,
                      'artifacts': {str(path.relative_to(output)): _digest(path)
                                    for path in output.rglob('*') if path.is_file()}}
            with (output / 'execution.json').open('x') as stream:
                json.dump(record, stream, indent=2)
    if integrity_errors:
        raise ValueError('; '.join(integrity_errors))
    result = reconcile_wpt(selected['urls'], read_mozlog(output / 'mozlog.jsonl'),
                           execution=execution, assessment_total=bound['run']['expected']['assessments'])
    from wpt_execution_evidence import protocol_custody
    protocol_custody(output / 'protocol-custody.jsonl', bound['run']['build']['manifest_sha256'])
    with (output / 'reconciliation.json').open('x') as stream:
        json.dump(result, stream, indent=2)
    if selected['collection'] == 'canaries':
        write_canary_audit(bound, output, execution)
    return result


def write_canary_audit(bound, output, process):
    """Derive the canary audit from actual raw traffic and process settlement."""
    from .results import read_mozlog
    from oryn_wptrunner.canary_resource_audit import response_status, MISSING_HELPER
    from wpt_canary_qualification import MOUNT
    output = Path(output)
    report = read_object(output / 'wptreport.json')
    worker = read_object(bound['assets']['worker_manifest'])
    contracts = {name: worker[name] for name in ('render_contract_version', 'local_effects_contract_version')}
    info = report.get('run_info', {})
    native = (info.get('oryn_rendering_contracts') == contracts
              and info.get('oryn_executor') == 'upstream-wptrunner'
              and info.get('oryn_candidate_sha256') == bound['run']['build']['manifest_sha256'])
    clean = (process.get('cleanup_complete') is True and process.get('cleanup_errors') == []
             and process.get('forced_cleanup') is False and process.get('timed_out') is False
             and process.get('error') is None)
    resources = [{'path': MISSING_HELPER, 'status': status} for event in read_mozlog(output / 'mozlog.jsonl')
                 if (status := response_status(event)) is not None]
    audit = {'candidate_sha256': bound['run']['build']['manifest_sha256'],
             'report_sha256': _digest(output / 'wptreport.json'), 'cleanup_complete': clean,
             'all_workers_reaped': clean, 'native_reference_rendering': native,
             'resources': {MOUNT + 'missing-helper.html': resources}}
    with (output / 'audit.json').open('x') as stream:
        json.dump(audit, stream, indent=2)
    return audit


@contextmanager
def fixture_services(bound, selected, *, output, environment, lease, timeout_seconds, upstream=False):
    """Own relays inside the caller's single resource lease, including teardown.

    Each service has continuous canonical process supervision. This context is
    also usable by the topology collector; it does not acquire another lock.
    """
    from .processes import OwnedService
    services = []
    with ExitStack() as stack:
        for name, argv, address in relay_commands(bound['assets']['fixture_relay'], selected['fixture']):
            service = stack.enter_context(OwnedService(argv, cwd=bound['assets']['wpt_checkout'],
                output=Path(output) / 'services' / name, env=environment,
                timeout_seconds=timeout_seconds, grace_seconds=60))
            services.append(service)
            deadline = time.monotonic() + 10
            while True:
                lease.checkpoint()
                service.check()
                try:
                    with socket.create_connection(address, timeout=.25):
                        break
                except OSError:
                    if time.monotonic() >= deadline:
                        raise TimeoutError('fixture relay did not bind: ' + name)
                    time.sleep(.05)
        if upstream:
            argv = [str(bound['assets']['python']), '-B', '-m', 'oryn_validation.wpt', 'serve',
                    '--run', str(bound['path']), '--store-root', str(bound['store_root']),
                    '--bundle-root', str(bound['bundle_root'])]
            service = stack.enter_context(OwnedService(argv, cwd=bound['assets']['wpt_checkout'],
                output=Path(output) / 'services/upstream', env=environment,
                timeout_seconds=timeout_seconds, grace_seconds=60))
            services.append(service)
            deadline = time.monotonic() + 30
            for ports in selected['fixture']['ports'].values():
                for port in ports:
                    while True:
                        lease.checkpoint()
                        service.check()
                        try:
                            with socket.create_connection(('127.0.0.1', port), timeout=.25):
                                break
                        except OSError:
                            if time.monotonic() >= deadline:
                                raise TimeoutError('upstream fixture did not bind: ' + str(port))
                            time.sleep(.05)
        yield services
        for service in services:
            service.check()


def relay_commands(binary, fixture):
    """Retain reviewed raw TCP relays; TLS remains end to end with upstream."""
    import ipaddress
    selected = [row for row in fixture['endpoint_routes'] if row['protocol'].endswith(('-local', '-public'))]
    if {row['protocol'] for row in selected} != {'http-local', 'https-local', 'http-public', 'https-public'} or len(selected) != 4:
        raise ValueError('exact four local/public fixture relays required')
    result = []
    for row in selected:
        ip = ipaddress.ip_address(row['connect_ip'])
        port = row['port']
        listen = (f'TCP4-LISTEN:{port},bind={ip},reuseaddr,fork,max-children=256'
                  if ip.version == 4 else f'TCP6-LISTEN:{port},bind=[{ip}],ipv6only=1,reuseaddr,fork,max-children=256')
        result.append((row['protocol'], [str(binary), '-d', '-d', '-t', '3600', listen,
                                        f'TCP4:127.0.0.1:{port}'], (str(ip), port)))
    return result


def verify_fixture_services(bound, selected, output, *, upstream=False, original_roots=None):
    """Recheck completed sibling-service custody using explicit original roots.

    Output must already be a declared whole-directory input to the caller's
    current lease. Original absolute command paths are evidence, never read.
    """
    current_store, current_bundle = bound['store_root'], bound['bundle_root']
    original_store, original_bundle = (map(Path, original_roots) if original_roots is not None
                                        else (Path(current_store), Path(current_bundle)))
    if not original_store.is_absolute() or not original_bundle.is_absolute():
        raise ValueError('fixture service original roots must be absolute')
    assets = {name: (original_bundle if item['scope'] == 'bundle' else original_store) / item['path']
              for name, item in bound['run']['assets'].items()}
    run_path = original_store / Path(bound['path']).relative_to(current_store)
    expected = [(name, argv) for name, argv, _ in relay_commands(assets['fixture_relay'], selected['fixture'])]
    if upstream:
        expected.append(('upstream', [str(assets['python']), '-B', '-m', 'oryn_validation.wpt', 'serve',
            '--run', str(run_path), '--store-root', str(original_store), '--bundle-root', str(original_bundle)]))
    root = Path(output) / 'services'
    if {path.name for path in root.iterdir()} != {name for name, _ in expected}:
        raise ValueError('fixture service population differs')
    result = []
    for name, argv in expected:
        process = read_object(root / name / 'process-result.json')
        service = read_object(root / name / 'service-result.json')
        preflight = read_object(root / name / 'preflight.json')
        birth = preflight.get('child_identity')
        if (service != dict(process, service_stop_requested=True)
                or process.get('argv') != argv or preflight.get('argv') != argv
                or preflight.get('cwd') != str(assets['wpt_checkout'])
                or not isinstance(birth, list) or len(birth) != 2 or not isinstance(birth[0], list)
                or len(birth[0]) not in (1, 2) or birth[1] != preflight.get('owner_pid')
                or type(preflight.get('child_pid')) is not int or preflight['child_pid'] <= 0
                or process.get('cleanup_complete') is not True or process.get('cleanup_errors') != []
                or process.get('timed_out') is not False or process.get('forced_cleanup') is not False
                or process.get('error') != 'owned execution cancelled by signal 15'
                or process.get('cancellation_signal') != 15):
            raise ValueError('fixture service original custody/cleanup differs: ' + name)
        result.append(service)
    return result


def topology_server_config(bound, selected):
    """Resolve the reviewed upstream serve config from current typed fixtures."""
    assets, fixture = bound['assets'], selected['fixture']
    base = Path(selected['fixture_path']).parent
    return {'alternate_hosts': {'alt': 'not-web-platform.test'}, 'bind_address': True,
            'browser_host': 'web-platform.test', 'check_subdomains': False,
            'doc_root': str(assets['wpt_checkout']), 'ports': fixture['ports'],
            'server_host': '127.0.0.1', 'ssl': {'encrypt_after_connect': False,
                'pregenerated': {'host_cert_path': str(base / fixture['tls']['certificate']['path']),
                                 'host_key_path': str(base / fixture['tls']['key']['path'])},
                'type': 'pregenerated'},
            'ws_doc_root': str(assets['wpt_checkout'] / 'websockets/handlers')}


def serve_topology(args):
    """Canonical upstream wpt serve entry, exclusively for the topology stage."""
    from .prerequisites import load_inputs as prerequisite_inputs, _selected, validate_native_prerequisites
    from .production import validate_production
    bound = prerequisite_inputs(args.run, store_root=args.store_root, bundle_root=args.bundle_root)
    if bound['run']['suite']['name'] != 'fixture-topology':
        raise ValueError('fixture server requires the typed topology collection')
    validate_production(bound)
    validate_native_prerequisites(bound)
    _, fixture_path, fixture = _selected(bound)
    config = topology_server_config(bound, {'fixture': fixture, 'fixture_path': fixture_path})
    sys.path.insert(0, str(bound['assets']['wpt_checkout']))
    sys.path.insert(0, str(bound['assets']['wpt_checkout'] / 'tools'))
    import localpaths  # noqa: F401
    from tools.serve import serve
    class SelectedConfig(serve.ConfigBuilder):
        def __enter__(self):
            self.update(config)
            return super().__enter__()
    with InputLease(bound['records'], store_root=args.store_root, bundle_root=args.bundle_root):
        return serve.run(config_cls=SelectedConfig)


def upstream(argv=None):
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('mode', choices=['upstream', 'serve'])
    for option in ('run', 'store-root', 'bundle-root'):
        parser.add_argument('--' + option, type=Path, required=True)
    args, remaining = parser.parse_known_args(argv)
    if args.mode == 'serve':
        if remaining:
            raise ValueError('topology serve has no caller-selected upstream overrides')
        return serve_topology(args)
    remaining = remaining[1:] if remaining and remaining[0] == '--' else remaining
    bound = load_inputs(args.run, store_root=args.store_root, bundle_root=args.bundle_root)
    selected = official_admission(bound)
    output = args.store_root.absolute() / bound['run']['outputs']['results']
    command = plan_command(bound, selected, output)
    if remaining != command[command.index('--') + 1:]:
        raise ValueError('upstream arguments differ from the frozen original execution policy')
    os.environ['ORYN_VALIDATION_STORE_ROOT'] = str(args.store_root.absolute())
    os.environ['ORYN_VALIDATION_BUNDLE_ROOT'] = str(args.bundle_root.absolute())
    sys.path.insert(0, str(bound['assets']['wpt_checkout'] / 'tools'))
    import localpaths  # noqa: F401 -- pinned upstream path registration
    from wptrunner import wptrunner
    from oryn_wptrunner.provenance import run_admission
    sys.argv = [sys.argv[0], *remaining]
    with run_admission(args.run, None, bound['assets']['runtime'], bound['assets']['driver']):
        return wptrunner.main()


if __name__ == '__main__':
    sys.exit(upstream())

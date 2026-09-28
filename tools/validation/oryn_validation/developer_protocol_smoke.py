"""Coordinator-only native adapter smoke, explicitly outside qualification.

The parent owns resources/processes and verifies retained immutable inputs. The
child uses the current adapter with an explicitly synthetic qualification lookup
only; browser startup, protocol parts, harness/window readiness, one test and
teardown are real. This HTTP probe omits the official TLS activation bootstrap
and address-class relays; it cannot establish fixture/TLS qualification. This
module is never called by official WPT admission.
"""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import sys
import time

from .admission import SOURCE_ROOT, legacy_algorithms, read_object
from .inputs import InputLease, snapshot_input
from .wpt import _digest, load_inputs, plan_command, python_environment
from .worker_environment import worker_environment, assert_worker_environment

TEST = '/dom/nodes/Node-isConnected.html'
DEADLINE = 600
SCOPE = 'HTTP adapter smoke only; no official TLS activation or address-class relay coverage'


def _write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2)


def _cleanup(label, operation, result, primary):
    try:
        operation()
    except BaseException as error:
        message = label + ': ' + type(error).__name__ + ': ' + str(error)
        result['errors'].append(message)
        if primary is None:
            raise
        primary.add_note(message)


def _contract(bound, fixture_path, fixture):
    """Developer projection, not production/current-runner admission."""
    legacy_algorithms()
    from oryn_wptrunner import cookie_profile, feature_profile
    from completion_provenance import render_contracts, validate_render_contracts
    render = render_contracts(SOURCE_ROOT)
    validate_render_contracts(read_object(bound['assets']['worker_manifest']), render)
    features, cookies = feature_profile.from_fixture(fixture), cookie_profile.from_fixture(fixture)
    return dict(path=str(bound['path']), sha256=bound['sha256'], typed_run=bound['run'],
                worker_binding=assert_worker_environment(bound),
                candidate_sha256=bound['run']['build']['manifest_sha256'],
                fixture_path=str(fixture_path), fixture_sha256=_digest(fixture_path), fixture=fixture,
                fixture_repairs=str(bound['assets']['fixture_repairs']),
                fixture_repairs_sha256=_digest(bound['assets']['fixture_repairs']),
                rendering_contracts=render, feature_profile=features,
                feature_profile_sha256=feature_profile.digest(features),
                cookie_profile=cookies, cookie_profile_sha256=cookie_profile.digest(cookies),
                developer_only=True, qualification='not-evaluated')


def child(config_path, expected_sha256):
    """One actual adapter session under the parent canonical process supervisor."""
    if _digest(config_path) != expected_sha256:
        raise ValueError('developer smoke configuration changed')
    config = read_object(config_path)
    if config.get('kind') != 'developer-protocol-smoke' or config.get('qualification') != 'not-evaluated':
        raise ValueError('developer smoke requires explicitly non-qualification configuration')
    window_samples = config.get('window_profile_samples', 0)
    if type(window_samples) is not int or not 0 <= window_samples <= 20:
        raise ValueError('window profiling requires 0 to 20 samples')
    path, store, bundle = (Path(config[key]) for key in ('run', 'store_root', 'bundle_root'))
    run = read_object(path)
    if _digest(path) != config['run_sha256']:
        raise ValueError('retained run changed after parent admission')
    assets = {name: (bundle if record['scope'] == 'bundle' else store) / record['path']
              for name, record in run['assets'].items()}
    bound = dict(run=run, path=path, sha256=config['run_sha256'], assets=assets,
                 store_root=store, bundle_root=bundle)
    assert_worker_environment(bound)
    output = Path(config_path).parent
    fixture_path = assets[config['fixture_role']]; fixture = read_object(fixture_path)
    sys.path.insert(0, str(assets['wpt_checkout'] / 'tools'))
    import localpaths  # noqa: F401
    from oryn_wptrunner.registration import verify_registration
    verify_registration(SOURCE_ROOT)
    from oryn_wptrunner import product as integration, provenance
    from wptrunner import wptcommandline, wptrunner, environment
    from unittest.mock import patch
    from types import SimpleNamespace
    # Reuse exact normal parsing/configuration, overriding only the developer
    # selection to one existing pinned Core URL (not an official original).
    smoke_assets = dict(assets, runner_config=assets['core_runner_config'], include=assets['core_include'])
    argv = plan_command(dict(bound, assets=smoke_assets, run=dict(run, workers=1)),
                        {'collection': 'core', 'fixture': fixture, 'fixture_path': fixture_path}, output)
    args = vars(wptcommandline.create_parser().parse_args(argv[argv.index('--') + 1:]))
    args.update(include=[TEST], include_file=None, test_types=['testharness'],
                log_wptreport=None, log_raw=None, no_capture_stdio=True)
    wptcommandline.check_args(args)
    args['prefs_root'] = str(Path(wptrunner.__file__).parent / 'prefs')
    contract = _contract(bound, fixture_path, fixture)
    def lookup(requested=None, binary=None, webdriver_binary=None):
        if requested is not None and Path(requested).absolute() != path:
            raise ValueError('developer lookup cannot substitute run')
        for value, role in ((binary, 'runtime'), (webdriver_binary, 'driver')):
            if value is not None and Path(value).absolute() != assets[role]:
                raise ValueError('developer lookup cannot substitute ' + role)
        assert_worker_environment(bound)
        return contract
    result = {'kind': 'developer-protocol-smoke', 'qualification': 'not-evaluated', 'scope': SCOPE,
              'test': TEST, 'setup_complete': False, 'teardown_complete': False,
              'errors': [], 'selected_worker': contract['worker_binding']}
    browser = executor = None
    try:
        with ExitStack() as stack:
            # These are the only replaced product functions. The developer
            # environment also omits official TLS bootstrap/relay qualification.
            stack.enter_context(patch.object(integration, 'load_contract', side_effect=lookup))
            stack.enter_context(patch.object(provenance, 'load_run_contract', side_effect=lookup))
            logger = stack.enter_context(wptrunner.GlobalLogger(args, {'raw': sys.stdout}))
            options = dict(args); product = options.pop('product'); paths = options.pop('test_paths')
            options.pop('config')
            _, loader = wptrunner.get_loader(paths, product, **options)
            if set(loader.test_ids) != {TEST}:
                raise ValueError('developer smoke selected another test population')
            tests = [(name, test) for name, types in loader.tests.items()
                     for test in types.get('testharness', [])]
            if len(tests) != 1:
                raise ValueError('developer smoke test is disabled or missing')
            name, test = tests[0]; subsuite = loader.subsuites[name]
            product.check_args(**options)
            environment.do_delayed_imports(logger, paths)
            ssl = {'type': 'pregenerated', 'pregenerated': {key: options[key] for key in
                   ('host_key_path', 'host_cert_path', 'ca_cert_path')}}
            env = stack.enter_context(environment.TestEnvironment(paths, 1, False, False, None,
                dict(product.env_options), ssl, product.get_env_extras(**options)))
            env.ensure_started()
            browser_args = product.get_browser_kwargs(logger, 'testharness', subsuite.run_info,
                config=env.config, subsuite=subsuite, **options)
            browser = product.get_browser_cls('testharness')(logger, manager_number=0, **browser_args)
            try:
                browser.start({})
                cls, browser_kwargs = browser.executor_browser()
                executor_args = product.get_executor_kwargs(logger, 'testharness', env,
                    subsuite.run_info, subsuite=subsuite, **options)
                executor = product.executor_classes['testharness'](logger, cls(**browser_kwargs), **executor_args)
                try:
                    executor.setup(SimpleNamespace(logger=logger))
                    result['setup_complete'] = True
                    session = executor.protocol.webdriver.session_id
                    if not isinstance(session, str) or not session:
                        raise ValueError('developer protocol did not establish a session')
                    result['session_id'] = session
                    result['window_profile'] = []
                    for sample in range(window_samples):
                        executor.protocol.testharness.close_old_windows()
                        started = time.monotonic_ns()
                        executor.create_test_window(executor.protocol)
                        result['window_profile'].append({'sample': sample + 1,
                            'elapsed_ns': time.monotonic_ns() - started})
                        executor.protocol.testharness.close_old_windows()
                    observed, subtests = executor.do_test(test)
                    result['outcome'] = {'status': observed.status, 'message': observed.message,
                        'subtests': [{'name': item.name, 'status': item.status, 'message': item.message}
                                     for item in subtests]}
                finally:
                    def teardown():
                        executor.teardown()
                        result['teardown_complete'] = True
                    _cleanup('protocol teardown', teardown, result, sys.exc_info()[1])
            finally:
                def stop():
                    try:
                        if not browser.stop():
                            raise RuntimeError('developer native driver did not stop')
                    finally:
                        _cleanup('browser cleanup', browser.cleanup, result, sys.exc_info()[1])
                _cleanup('driver stop', stop, result, sys.exc_info()[1])
        from wpt_execution_evidence import protocol_custody
        result['completed_sessions'] = protocol_custody(output / 'protocol-custody.jsonl',
                                                       run['build']['manifest_sha256'])
        if result.get('outcome', {}).get('status') != 'OK' or any(
                row['status'] != 'PASS' for row in result.get('outcome', {}).get('subtests', [])):
            raise RuntimeError('developer smoke observed a test failure; see preserved outcome')
    except BaseException as error:
        result['errors'].append(type(error).__name__ + ': ' + str(error))
        raise
    finally:
        _write(output / 'developer-result.json', result)


def execute(run_path, *, store_root, bundle_root, output, window_profile_samples=0):
    from .processes import run_owned, ResourceLease
    from .bundle_ownership import registered_bundle_owner
    from .production import validate_production
    legacy_algorithms()
    from oryn_wptrunner.custody import create
    if type(window_profile_samples) is not int or not 0 <= window_profile_samples <= 20:
        raise ValueError('window profiling requires 0 to 20 samples')
    if '..' in Path(output).parts:
        raise ValueError('developer smoke output cannot traverse parent directories')
    store_root, bundle_root, output = map(lambda p: Path(p).absolute(), (store_root, bundle_root, output))
    if not output.is_relative_to(store_root / 'migration/verification'):
        raise ValueError('developer smoke output must be under migration/verification, never official runs')
    for item in (output, *output.parents):
        if item.is_symlink(): raise ValueError('developer output traverses a symlink')
    bound = load_inputs(run_path, store_root=store_root, bundle_root=bundle_root)
    validate_production(bound)
    registry = read_object(bound['assets']['core_profile_registry'])
    fixture_role = registry['profiles'][bound['run']['profile']]['fixture']
    output.mkdir(parents=True, exist_ok=False)
    create(output / 'protocol-custody.jsonl')
    config = {'kind': 'developer-protocol-smoke', 'qualification': 'not-evaluated', 'scope': SCOPE,
              'run': str(bound['path']), 'run_sha256': bound['sha256'], 'store_root': str(store_root),
              'bundle_root': str(bundle_root), 'fixture_role': fixture_role,
              'window_profile_samples': window_profile_samples,
              'qualification_lookup': 'synthetic developer projection; no official admission'}
    config_path = output / 'developer-config.json'; _write(config_path, config)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(('ORYN_', 'PYTHON', 'DYLD_'))}
    env.update(python_environment(bound)); env.update(worker_environment(bound))
    env.update(PYTHONPATH=os.pathsep.join((str(bound['assets']['wpt_checkout']),
                                        str(SOURCE_ROOT / 'tools/validation'),
                                        str(SOURCE_ROOT / 'tools/wptrunner-oryn'))),
               ORYN_WPT_PROTOCOL_CUSTODY=str(output / 'protocol-custody.jsonl'),
               ORYN_WPT_CANDIDATE_SHA256=bound['run']['build']['manifest_sha256'])
    current_records = [snapshot_input(SOURCE_ROOT, item, kind='directory') for item in
                       ('tools/validation', 'tools/wptrunner-oryn', 'scripts')]
    current_records.append(snapshot_input(SOURCE_ROOT, 'crates/oryn-common/src/v4.rs'))
    _write(output / 'developer-source-inputs.json', current_records)
    records = [*bound['records'], snapshot_input(store_root, str(config_path.relative_to(store_root))),
               snapshot_input(store_root, str(bound['path'].relative_to(store_root)))]
    with InputLease(records, store_root=store_root, bundle_root=bundle_root) as inputs, \
         InputLease(current_records, store_root=SOURCE_ROOT) as source:
        with ResourceLease(store_root, ['native-focus', 'wpt-fixed-ports'],
                           objects=[registered_bundle_owner(bound)]) as resource:
            process = run_owned([str(bound['assets']['python']), '-B', '-m',
                'oryn_validation.developer_protocol_smoke', 'child', '--config', str(config_path),
                '--config-sha256', _digest(config_path)], cwd=bound['assets']['wpt_checkout'],
                output=output / 'process', env=env, timeout_seconds=DEADLINE)
            resource.mark_settled(process)
        inputs.finalize(); source.finalize(); python_environment(bound)
        _write(output / 'developer-custody.json', {'qualification': 'not-evaluated',
               'inputs': inputs.finalization, 'current_source': source.finalization})
    if process['returncode'] != 0 or not process['cleanup_complete'] or process['forced_cleanup']:
        raise RuntimeError('developer protocol smoke failed; inspect ' + str(output))
    return read_object(output / 'developer-result.json')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['execute', 'child'])
    for name in ('run', 'store-root', 'bundle-root', 'output', 'config'):
        parser.add_argument('--' + name, type=Path)
    parser.add_argument('--config-sha256')
    parser.add_argument('--window-profile-samples', type=int, default=0)
    args = parser.parse_args()
    if args.mode == 'child':
        child(args.config, args.config_sha256)
    else:
        if any(getattr(args, key) is None for key in ('run', 'store_root', 'bundle_root', 'output')):
            parser.error('execute requires explicit run, store-root, bundle-root and developer output')
        execute(args.run, store_root=args.store_root, bundle_root=args.bundle_root, output=args.output,
                window_profile_samples=args.window_profile_samples)


if __name__ == '__main__':
    main()

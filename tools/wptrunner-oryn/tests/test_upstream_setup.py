"""Actual pinned parser/loader/manager setup; never launch a browser or server.

Set ORYN_TEST_RUN_MANIFEST to a retained typed run to exercise its real frozen
collections read-only. Native admission is represented by a synthetic contract;
this developer check is not a qualification original or a custody receipt.
"""
from contextlib import ExitStack
import copy
import importlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

from oryn_validation.admission import SOURCE_ROOT
from oryn_validation.wpt import plan_command
from oryn_wptrunner import product as integration, provenance
from oryn_wptrunner.registration import verify_registration
from wptrunner import products, wptcommandline, wptrunner, environment


class Discovery(unittest.TestCase):
    def test_real_entrypoint_and_parser_register_custom_argument(self):
        product = verify_registration(SOURCE_ROOT)
        self.assertIs(product.get_browser_cls('testharness'), integration.OrynBrowser)
        args = wptcommandline.create_parser().parse_args(['--product', 'oryn', '--oryn-run-contract', 'run.json'])
        self.assertEqual(args.product, 'oryn')
        self.assertEqual(args.oryn_run_contract, 'run.json')

    def test_duplicate_distribution_and_foreign_entrypoint_fail_even_if_upstream_masks_them(self):
        for name, entry in (('oryn-wptrunner', False), ('foreign-runner', True)):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temporary:
                info = Path(temporary) / (name.replace('-', '_') + '-0.1.0.dist-info')
                info.mkdir()
                (info / 'METADATA').write_text('Metadata-Version: 2.1\nName: ' + name + '\nVersion: 0.1.0\n')
                if entry:
                    (info / 'entry_points.txt').write_text('[wptrunner.products]\noryn = oryn_wptrunner:load_product\n')
                with patch.object(sys, 'path', [*sys.path, temporary]):
                    importlib.invalidate_caches()
                    with self.assertRaisesRegex(ValueError, 'exactly one'):
                        verify_registration(SOURCE_ROOT)

    def test_loaded_module_must_come_from_selected_source(self):
        with patch.object(integration, '__file__', '/foreign/oryn_wptrunner/product.py'):
            with self.assertRaisesRegex(ValueError, 'foreign Oryn browser'):
                verify_registration(SOURCE_ROOT)

    def test_fresh_interpreter_discovers_source_plugin_without_installed_copy(self):
        program = ('import localpaths; from oryn_wptrunner.registration import verify_registration; '
                   'from wptrunner import wptcommandline; '
                   'verify_registration(' + repr(str(SOURCE_ROOT)) + '); '
                   'p=wptcommandline.create_parser(); '
                   'assert p.parse_args(["--product","oryn","--oryn-run-contract","run"]).oryn_run_contract=="run"')
        child = subprocess.run([sys.executable, '-B', '-c', program], env=os.environ.copy(),
                               text=True, capture_output=True, timeout=60)
        self.assertEqual(child.returncode, 0, child.stderr)

    def test_control_evidence_and_workflow_imports_preserve_fresh_registration(self):
        program = '''
import importlib
import sys
import localpaths
from oryn_validation.admission import SOURCE_ROOT, legacy_algorithms
from oryn_wptrunner.registration import verify_registration
from wptrunner import wptcommandline
legacy_algorithms()
plugin_path = str(SOURCE_ROOT / 'tools/wptrunner-oryn')
assert sys.path.count(plugin_path) == 1
for name in ('wpt_execution_evidence', 'wpt_official_workflow'):
    module = importlib.import_module(name)
    for action in ('import', 'reload'):
        if action == 'reload':
            importlib.reload(module)
        verify_registration(SOURCE_ROOT)
        assert sys.path.count(plugin_path) == 1, (name, action, sys.path)
        args = wptcommandline.create_parser().parse_args(
            ['--product', 'oryn', '--oryn-run-contract', 'run.json'])
        assert args.oryn_run_contract == 'run.json'
sys.path.append(plugin_path)
for name in ('wpt_execution_evidence', 'wpt_official_workflow'):
    importlib.reload(sys.modules[name])
assert sys.path.count(plugin_path) == 2
try:
    verify_registration(SOURCE_ROOT)
except ValueError as error:
    assert 'exactly one' in str(error)
else:
    raise AssertionError('pre-existing duplicate registration was hidden')
'''
        child = subprocess.run([sys.executable, '-B', '-c', program], env=os.environ.copy(),
                               text=True, capture_output=True, timeout=60)
        self.assertEqual(child.returncode, 0, child.stderr)


@unittest.skipUnless(os.environ.get('ORYN_TEST_RUN_MANIFEST'), 'requires retained typed inputs for read-only setup')
class RetainedCollectionSetup(unittest.TestCase):
    def setUp(self):
        self.path = Path(os.environ['ORYN_TEST_RUN_MANIFEST']).absolute()
        self.store = self.path.parents[2]
        self.run = json.loads(self.path.read_text())
        build_record = self.run['assets']['build_manifest']
        build = json.loads((self.store / build_record['path']).read_text())
        # The build manifest and bundle are siblings in the published layout.
        self.bundle = (self.store / build_record['path']).parent / 'runtime'
        self.assets = {name: (self.bundle if record['scope'] == 'bundle' else self.store) / record['path']
                       for name, record in self.run['assets'].items()}
        self.fixture_path = self.assets['fixture_standard_block']
        self.fixture = json.loads(self.fixture_path.read_text())
        self.assertEqual(build['build_id'], self.run['build']['build_id'])
        verify_registration(SOURCE_ROOT)

    def kwargs(self, collection, output):
        assets = dict(self.assets)
        prefix = 'canary' if collection == 'canaries' else collection
        assets.update(include=assets[prefix + '_include'], runner_config=assets[prefix + '_runner_config'])
        bound = dict(assets=assets, run=dict(self.run, workers=6 if collection == 'core' else 1),
                     path=self.path, store_root=self.store, bundle_root=self.bundle)
        argv = plan_command(bound, {'collection': collection, 'fixture': self.fixture,
                                    'fixture_path': self.fixture_path}, output)
        args = vars(wptcommandline.create_parser().parse_args(argv[argv.index('--') + 1:]))
        wptcommandline.check_args(args)
        args['prefs_root'] = str(Path(wptrunner.__file__).parent / 'prefs')
        return args, assets

    def contract(self):
        return dict(path=str(self.path), fixture_path=str(self.fixture_path), fixture=self.fixture,
                    fixture_sha256='f' * 64, fixture_repairs=None, fixture_repairs_sha256=None,
                    candidate_sha256='a' * 64, sha256='b' * 64, feature_profile={},
                    feature_profile_sha256='c' * 64, cookie_profile={}, cookie_profile_sha256='d' * 64,
                    rendering_contracts={})

    def test_actual_loader_populations_and_manager_setup_for_all_collections(self):
        for collection, count in (('canaries', 5), ('placement', 1), ('core', 15985)):
            with self.subTest(collection=collection), tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
                args, assets = self.kwargs(collection, Path(temporary))
                self.assertEqual(Path(args['test_paths']['/'].tests_path), self.assets['wpt_checkout'])
                self.assertEqual(Path(args['test_paths']['/'].metadata_path), self.assets['core_metadata'])
                self.assertEqual(Path(args['test_paths']['/'].manifest_path), self.assets['core_manifest'])
                self.assertFalse(args['manifest_update']); self.assertFalse(args['manifest_download'])
                contract = self.contract()
                stack.enter_context(patch.object(integration, 'load_contract', return_value=contract))
                stack.enter_context(patch.object(provenance, 'load_run_contract', return_value=contract))
                logger = stack.enter_context(wptrunner.GlobalLogger(args, {}))
                options = dict(args); product = options.pop('product'); paths = options.pop('test_paths')
                options.pop('config')
                queue, loader = wptrunner.get_loader(paths, product, **options)
                expected = {'/' + line.lstrip('/') for line in assets['include'].read_text().splitlines()}
                self.assertEqual(len(expected), count)
                observed = set(loader.test_ids)
                self.assertEqual((len(observed), sorted(observed - expected)[:5], sorted(expected - observed)[:5]),
                                 (count, [], []))
                self.assertEqual(sum(len(tests) for types in loader.disabled_tests.values() for tests in types.values()), 0)
                product.check_args(**options)
                extras = product.get_env_extras(**options)
                environment.do_delayed_imports(logger, paths)
                # Construct the real upstream environment/config and run its
                # Oryn extension before stopping exactly at manager creation.
                # Managers, socket servers and native processes are forbidden.
                stack.enter_context(patch.object(environment.mpcontext, 'get_context', return_value=MagicMock()))
                stack.enter_context(patch.object(environment, 'ProxyLoggingContext'))
                stack.enter_context(patch('wptserve.config.get_port', return_value=49152))
                ssl = {'type': 'pregenerated', 'pregenerated': {key: options[key] for key in
                       ('host_key_path', 'host_cert_path', 'ca_cert_path')}}
                env = environment.TestEnvironment(paths, 1, False, False, None,
                    dict(product.env_options), ssl, extras)
                config = stack.enter_context(env.build_config())
                env.config = config
                for extra in extras:
                    stack.enter_context(extra(env.options, config))
                self.assertEqual({name: list(config.ports[name]) for name in self.fixture['ports']},
                                 self.fixture['ports'])
                self.assertEqual(sum(len(config.ports[name]) for name in self.fixture['ports']), 11)
                self.assertEqual(config.ports['h2'], [9000])
                self.assertEqual(config.ssl_config['cert_path'], options['host_cert_path'])
                self.assertEqual(config.ssl_config['key_path'], options['host_key_path'])
                self.assertEqual(config.doc_root, str(self.assets['wpt_checkout']))
                mount = {'canaries': '/__oryn_qualification__/',
                         'placement': '/__oryn_shared_memory__/', 'core': '/'}[collection]
                self.assertEqual(set(paths), {'/', mount})
                selected_source = self.assets[{'canaries': 'canary_source',
                                               'placement': 'placement_source', 'core': 'wpt_checkout'}[collection]]
                self.assertEqual(Path(paths[mount].tests_path), selected_source)
                env.reset = lambda: None
                class StopBeforeManagers(Exception): pass
                manager = stack.enter_context(patch.object(wptrunner, 'ManagerGroup', side_effect=StopBeforeManagers))
                with self.assertRaises(StopBeforeManagers):
                    wptrunner.run_test_iteration(wptrunner.TestStatus(), loader, queue, MagicMock(), env, product, options)
                implementations = manager.call_args.args[2]
                self.assertTrue(implementations)
                for (_, kind), item in implementations.items():
                    self.assertIs(item.browser_cls, integration.OrynBrowser)
                    self.assertEqual(item.browser_kwargs['binary'], str(self.assets['runtime']))
                    self.assertEqual(item.executor_kwargs['capabilities']['browserName'], 'oryn')
                    self.assertFalse(item.executor_kwargs['capabilities']['acceptInsecureCerts'])

    def test_developer_smoke_actual_child_setup_stops_before_native_start(self):
        from oryn_validation import developer_protocol_smoke as smoke
        from oryn_validation.worker_environment import worker_environment
        from oryn_wptrunner import custody
        class StopBeforeNative(Exception): pass
        stop = StopBeforeNative('developer setup reached native boundary')
        bound = dict(run=self.run, assets=self.assets, store_root=self.store, bundle_root=self.bundle)
        with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            output = Path(temporary).resolve(); config = output / 'developer-config.json'
            config.write_text(json.dumps({'kind': 'developer-protocol-smoke', 'qualification': 'not-evaluated',
                'run': str(self.path), 'run_sha256': smoke._digest(self.path),
                'store_root': str(self.store), 'bundle_root': str(self.bundle),
                'fixture_role': 'fixture_standard_block'}))
            custody.create(output / 'protocol-custody.jsonl')
            stack.enter_context(patch.dict(os.environ, {
                **worker_environment(bound), 'ORYN_WPT_PROTOCOL_CUSTODY': str(output / 'protocol-custody.jsonl'),
                'ORYN_WPT_CANDIDATE_SHA256': self.run['build']['manifest_sha256']}))
            stack.enter_context(patch.object(environment.mpcontext, 'get_context', return_value=MagicMock()))
            stack.enter_context(patch.object(environment, 'ProxyLoggingContext'))
            stack.enter_context(patch('wptserve.config.get_port', return_value=49152))
            def enter(env):
                env.config = stack.enter_context(env.build_config())
                for extra in env.env_extras:
                    stack.enter_context(extra(env.options, env.config))
                self.assertEqual({name: list(env.config.ports[name]) for name in self.fixture['ports']},
                                 self.fixture['ports'])
                return env
            stack.enter_context(patch.object(environment.TestEnvironment, '__enter__', enter))
            stack.enter_context(patch.object(environment.TestEnvironment, '__exit__', return_value=False))
            stack.enter_context(patch.object(environment.TestEnvironment, 'ensure_started'))
            start = stack.enter_context(patch.object(integration.OrynBrowser, 'start', side_effect=stop))
            with self.assertRaises(StopBeforeNative) as caught:
                smoke.child(config, smoke._digest(config))
            self.assertIs(caught.exception, stop)
            start.assert_called_once_with({})
            result = json.loads((output / 'developer-result.json').read_text())
            self.assertFalse(result['setup_complete'])
            self.assertFalse(result['teardown_complete'])
            self.assertEqual(result['qualification'], 'not-evaluated')
            self.assertIn('HTTP adapter smoke only', result['scope'])
            self.assertIn('developer setup reached native boundary', result['errors'][0])


if __name__ == '__main__':
    unittest.main()

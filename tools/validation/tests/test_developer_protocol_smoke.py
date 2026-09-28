"""Pure developer harness guards; these tests never start its native child."""
from pathlib import Path
from contextlib import ExitStack
import json
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import developer_protocol_smoke as smoke


class DeveloperSmokeGuards(unittest.TestCase):
    def test_parent_persists_exact_mode_and_rejects_mismatched_child_result(self):
        # Only in-memory supervisors/leases are used; no child process launches.
        class Lease:
            finalization = {'verified': 'synthetic'}
            def __init__(self, *args, **kwargs): pass
            def __enter__(self): return self
            def __exit__(self, *args): return False
            def finalize(self): return self.finalization
            def mark_settled(self, process): pass

        for enabled, reported in ((True, True), (False, False), (False, True)):
            with self.subTest(enabled=enabled, reported=reported), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                run = root / 'run.json'; run.write_text('{}')
                registry = root / 'registry.json'
                registry.write_text(json.dumps({'profiles': {'standard-block': {'fixture': 'fixture_contract'}}}))
                output = root / 'migration/verification/probe'
                bound = {'path': run, 'sha256': smoke._digest(run), 'records': [],
                    'assets': {'core_profile_registry': registry, 'python': root / 'python',
                               'wpt_checkout': root / 'wpt'},
                    'run': {'profile': 'standard-block', 'build': {'manifest_sha256': 'a' * 64}}}

                def completed(argv, *, output, **kwargs):
                    output.mkdir()
                    config = json.loads((output.parent / 'developer-config.json').read_text())
                    self.assertIs(config['native_timing'], enabled)
                    self.assertEqual(config['native_timing_scope'], smoke.NATIVE_TIMING_SCOPE)
                    self.assertNotIn('--no-native-timing', argv)  # child uses hashed config only
                    smoke._write(output.parent / 'developer-result.json', {'native_timing': reported})
                    return {'returncode': 0, 'cleanup_complete': True, 'forced_cleanup': False}

                with ExitStack() as patches:
                    for target, replacement in (
                        ('oryn_validation.developer_protocol_smoke.load_inputs', lambda *a, **k: bound),
                        ('oryn_validation.production.validate_production', lambda *a: None),
                        ('oryn_validation.bundle_ownership.registered_bundle_owner', lambda *a: 'build:synthetic'),
                        ('oryn_validation.developer_protocol_smoke.python_environment', lambda *a: {}),
                        ('oryn_validation.developer_protocol_smoke.worker_environment', lambda *a: {}),
                        ('oryn_validation.developer_protocol_smoke.snapshot_input', lambda *a, **k: {'synthetic': True}),
                        ('oryn_validation.developer_protocol_smoke.InputLease', Lease),
                        ('oryn_validation.processes.ResourceLease', Lease),
                        ('oryn_validation.processes.run_owned', completed),
                    ):
                        patches.enter_context(patch(target, replacement))
                    if enabled is reported:
                        result = smoke.execute(run, store_root=root, bundle_root=root / 'bundle',
                            output=output, native_timing=enabled)
                        self.assertIs(result['native_timing'], enabled)
                    else:
                        with self.assertRaisesRegex(ValueError, 'result differs'):
                            smoke.execute(run, store_root=root, bundle_root=root / 'bundle',
                                output=output, native_timing=enabled)

    def test_native_timing_selection_is_strict_before_loading_or_child_imports(self):
        for value in (0, 1, None, 'false', []):
            with self.subTest(value=value), patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, 'must be a boolean'):
                    smoke.execute('/run.json', store_root='/store', bundle_root='/bundle',
                        output='/store/migration/verification/profile', native_timing=value)
                load.assert_not_called()
            with tempfile.TemporaryDirectory() as temporary:
                config = Path(temporary) / 'config.json'
                config.write_text(json.dumps({'kind': 'developer-protocol-smoke',
                    'qualification': 'not-evaluated', 'native_timing': value}))
                with self.assertRaisesRegex(ValueError, 'must be a boolean'):
                    smoke.child(config, smoke._digest(config))

    def test_native_timing_cli_defaults_on_and_child_cannot_override_hashed_choice(self):
        base = ['developer-protocol-smoke', 'execute', '--run', '/run', '--store-root', '/store',
                '--bundle-root', '/bundle', '--output', '/store/migration/verification/probe']
        for extra, expected in (([], True), (['--no-native-timing'], False)):
            with self.subTest(expected=expected), patch.object(smoke.sys, 'argv', base + extra), \
                 patch.object(smoke, 'execute') as execute:
                smoke.main()
                self.assertIs(execute.call_args.kwargs['native_timing'], expected)
        with patch.object(smoke.sys, 'argv', ['probe', 'child', '--no-native-timing']), \
             patch.object(smoke, 'child') as child, self.assertRaises(SystemExit):
            smoke.main()
        child.assert_not_called()

    def test_ordinary_run_output_cannot_be_used_for_developer_smoke(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, 'never official runs'):
                    smoke.execute(root / 'run.json', store_root=root, bundle_root=root / 'build',
                                  output=root / 'runs/fake-original')
                load.assert_not_called()


    def test_parent_traversal_cannot_escape_into_official_run_namespace(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output = root / 'migration/verification/../../runs/fake-original'
            with patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, 'parent directories'):
                    smoke.execute(root / 'run.json', store_root=root, bundle_root=root / 'build', output=output)
                load.assert_not_called()
                self.assertFalse((root / 'runs').exists())

    def test_child_refuses_unbound_configuration_before_protocol_import_or_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / 'config.json'; config.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'configuration changed'):
                smoke.child(config, 'f' * 64)
            with self.assertRaisesRegex(ValueError, 'non-qualification'):
                smoke.child(config, smoke._digest(config))

    def test_cleanup_failure_cannot_hide_primary_initialization_error(self):
        primary = ValueError('actual initialization failed')
        secondary = RuntimeError('actual delete failed')
        result = {'errors': []}
        def fail(): raise secondary
        smoke._cleanup('protocol teardown', fail, result, primary)
        self.assertIn('actual delete failed', primary.__notes__[0])
        self.assertIn('actual delete failed', result['errors'][0])
        with self.assertRaises(RuntimeError) as caught:
            smoke._cleanup('protocol teardown', fail, result, None)
        self.assertIs(caught.exception, secondary)

    def test_window_profile_sample_count_is_bounded_before_loading(self):
        for count in (-1, 21, True):
            with self.subTest(count=count), patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, '0 to 20 samples'):
                    smoke.execute('/run.json', store_root='/store', bundle_root='/bundle',
                        output='/store/migration/verification/profile', window_profile_samples=count)
                load.assert_not_called()


class DeveloperNativeTimingCommand(unittest.TestCase):
    def browser(self):
        contract = {'fixture_path': '/fixture/current.json', 'fixture_sha256': 'a' * 64}
        browser = SimpleNamespace(webdriver_binary='/bundle/driver', host='127.0.0.1', port=8888,
            binary='/bundle/runtime', _token_path='/tmp/private-token',
            _native_timing=SimpleNamespace(fd=71, closed=False), calls=0)

        def normal():
            browser.calls += 1
            # A normal restart creates a fresh collector/descriptor.
            browser._native_timing = SimpleNamespace(fd=70 + browser.calls, closed=False)
            return [browser.webdriver_binary, '--host', browser.host, '--port', str(browser.port),
                '--browser-binary', browser.binary, '--auth-token-file', browser._token_path,
                '--configuration', contract['fixture_path'],
                '--configuration-sha256', contract['fixture_sha256'],
                '--diagnostic-fd', str(browser._native_timing.fd)]
        browser.make_command = normal
        return browser, contract

    def test_default_is_unwrapped_and_off_calls_normal_method_on_each_restart(self):
        browser, contract = self.browser()
        original = browser.make_command
        with smoke._browser_native_timing(browser, contract, True):
            self.assertIs(browser.make_command, original)
            baseline = browser.make_command()
        with smoke._browser_native_timing(browser, contract, False):
            first = browser.make_command()
            self.assertEqual(first, baseline[:-2])
            self.assertEqual(browser._native_timing.fd, 72)
            self.assertFalse(browser._native_timing.closed)
            second = browser.make_command()
            self.assertEqual(second, baseline[:-2])
            self.assertEqual(browser._native_timing.fd, 73)
        self.assertIs(browser.make_command, original)
        self.assertEqual(browser.calls, 3)
        self.assertEqual(first[7:9], ['--auth-token-file', '/tmp/private-token'])

    def test_off_rejects_changed_tokens_fixture_duplicate_flags_and_substituted_fd(self):
        browser, contract = self.browser()
        command = browser.make_command()
        cases = [command[:-2], command + ['--diagnostic-fd', '71'],
                 command[:-1] + ['72'], command[:-1] + ['071'],
                 command[:-2] + ['--diagnostic-fd=71'], tuple(command)]
        for index, replacement in ((0, '/other/driver'), (6, '/other/runtime'),
                                   (8, '/other/token'), (10, '/other/fixture'), (12, 'b' * 64)):
            changed = list(command); changed[index] = replacement; cases.append(changed)
        for changed in cases:
            with self.subTest(command=changed), self.assertRaisesRegex(ValueError, 'unexpected normal'):
                smoke._without_native_timing(changed, browser, contract)
        for descriptor, closed in ((True, False), (2, False), (71, True), (71, None)):
            browser._native_timing = SimpleNamespace(fd=descriptor, closed=closed)
            with self.subTest(descriptor=descriptor, closed=closed), \
                 self.assertRaisesRegex(ValueError, 'active normal collector'):
                smoke._without_native_timing(command, browser, contract)

    def test_normal_admission_error_is_not_hidden_by_toggle(self):
        browser, contract = self.browser()
        primary = ValueError('typed sealed worker admission failed')
        def rejected(): raise primary
        browser.make_command = rejected
        with smoke._browser_native_timing(browser, contract, False):
            with self.assertRaises(ValueError) as caught:
                browser.make_command()
            self.assertIs(caught.exception, primary)
        self.assertIs(browser.make_command, rejected)

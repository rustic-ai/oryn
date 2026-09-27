"""Preparation/custody regressions; subprocesses and browsers are never launched."""
import copy
import importlib.util
from contextlib import contextmanager
from types import SimpleNamespace
import json
import os
import shutil
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import urljoin, urlsplit
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_official_workflow as workflow
from oryn_wptrunner import cookie_profile, feature_profile


def profile_metadata():
    profile = cookie_profile.from_fixture({'third_party_cookies': 'block'})
    features = feature_profile.from_fixture({'web_features': {'usermedia_element': 'standard'}})
    return {'feature_environment_sha256': '9' * 64, 'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features), 'cookie_profile': profile, 'cookie_profile_sha256': cookie_profile.digest(profile),
            'fixture_environment_sha256': 'f' * 64, 'manifest_sha256': 'b' * 64,
            'coverage_sha256': 'c' * 64, 'policy_sha256': 'd' * 64,
            'core_include_sha256': 'e' * 64, 'core_total': 42}



class OfficialWorkflowTests(unittest.TestCase):
    def test_feature_profile_drift_rejects_before_output_or_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **copy.deepcopy(bound), 'checkout': str(root), 'python': str(python),
                    'run_contract_sha256': bound['sha256'], 'inputs': {}}
            bound['feature_profile'] = feature_profile.from_fixture({'web_features': {'usermedia_element': 'legacy'}})
            bound['feature_profile_sha256'] = feature_profile.digest(bound['feature_profile'])
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), patch.object(workflow, 'run_owned') as launch:
                with self.assertRaisesRegex(ValueError, 'feature profile changed'):
                    workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
                launch.assert_not_called()
            self.assertFalse((root / 'output').exists())

    def test_preflight_gate_drift_prevents_upstream_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned executable fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {name: {'sha256': 'a' * 64} for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}}, 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **profile_metadata(), 'checkout': str(root), 'python': str(python), 'candidate_sha256': 'a' * 64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {}, 'inventory': 'inventory',
                    'manifest': 'manifest', 'policy': 'policy'}
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            original_persist = workflow.persist
            def persist(path, value):
                original_persist(path, value)
                if path.name == 'preflight.json':
                    gate.write_text('{"replacement":true}')
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), \
                 patch.object(workflow, 'read_inventory'), patch.object(workflow, 'runner_command', return_value=['pinned', 'wpt', '--processes', '1']), \
                 patch.object(workflow, 'persist', side_effect=persist), patch.object(workflow, 'run_owned') as launch:
                with self.assertRaisesRegex(ValueError, 'generation declaration changed'):
                    workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
                launch.assert_not_called()
            self.assertFalse((root / 'output/execution.json').exists())

    def test_no_preparation_side_effect_before_complete_generation(self):
        with patch.object(workflow, 'generation_gate', side_effect=ValueError('generation incomplete')), patch.object(workflow.subprocess, 'run') as spawn:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                workflow.prepare('gate', 'contract', 'inventory', 'checkout', 'manifest', 'policy', 'python', 'output')
            spawn.assert_not_called()

    def test_no_execution_before_complete_generation(self):
        with patch.object(workflow, 'generation_gate', side_effect=ValueError('generation incomplete')), patch.object(workflow, 'run_owned') as spawn:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                workflow.execute('gate', 'contract', 'checkout', 'prepared', 'python', 'core', 'output')
            spawn.assert_not_called()

    def test_command_uses_actual_upstream_and_exact_frozen_tls(self):
        bound = {'candidate': {'artifacts': {'runtime': {'path': '/candidate/runtime'}, 'driver': {'path': '/candidate/driver'}}},
                 'fixture_path': '/candidate/config/fixture.json',
                 'fixture': {'tls': {name: {'path': name + '.pem'} for name in ('ca', 'certificate', 'key')}}}
        with patch.object(workflow, 'load_contract', return_value=bound):
            command = workflow.runner_command('/pinned/python', '/pinned/wpt', '/candidate/run.json', '/prepared', 'canaries', '/new/output', '/gate.json')
        self.assertEqual(command[0], '/pinned/python')
        self.assertIn('wpt-upstream-entry.py', command[1])
        self.assertIn('--no-manifest-update', command)
        self.assertIn('--no-manifest-download', command)
        self.assertEqual(command[command.index('--retry-unexpected') + 1], '0')
        self.assertEqual(command[command.index('--processes') + 1], '1')
        self.assertEqual(command[command.index('--include-file') + 1], '/prepared/canaries.include')
        self.assertEqual(command[command.index('--ca-cert-path') + 1], '/candidate/config/ca.pem')
        self.assertNotIn('--reuse-window', command)
        self.assertNotIn('--binary-arg', command)

    def test_core_manager_counts_preserve_serial_controls_and_original_run_flags(self):
        bound = {'candidate': {'artifacts': {'runtime': {'path': '/candidate/runtime'},
                                          'driver': {'path': '/candidate/driver'}}},
                 'fixture_path': '/candidate/config/fixture.json',
                 'fixture': {'tls': {name: {'path': name + '.pem'}
                                    for name in ('ca', 'certificate', 'key')}}}
        with patch.object(workflow, 'load_contract', return_value=bound):
            for count in (1, 2, 4, 6, 8):
                for kind in ('core', 'canaries', 'placement'):
                    with self.subTest(count=count, collection=kind):
                        command = workflow.runner_command('python', 'checkout', 'contract',
                            '/prepared', kind, '/output', 'gate', core_processes=count)
                        self.assertEqual(command[command.index('--processes') + 1],
                                         str(count if kind == 'core' else 1))
                        self.assertEqual(command[command.index('--include-file') + 1],
                                         '/prepared/' + kind + '.include')
                        for flag, expected in (('--repeat', '1'), ('--rerun', '1'),
                                               ('--retry-unexpected', '0')):
                            self.assertEqual(command[command.index(flag) + 1], expected)
                        self.assertFalse(any(arg.startswith('--exclude') for arg in command))

    def test_invalid_core_manager_count_fails_before_preparation_or_contract_access(self):
        for count in (True, False, 0, -1, 3, 5, 7, 9, '2', 2.0, None):
            with self.subTest(count=count), patch.object(workflow, 'GenerationLease') as lease, \
                    patch.object(workflow, 'load_contract') as contract:
                with self.assertRaisesRegex(ValueError, 'Core processes'):
                    workflow.prepare('gate', 'contract', 'inventory', 'checkout',
                                     'manifest', 'policy', 'python', 'output', core_processes=count)
                with self.assertRaisesRegex(ValueError, 'Core processes'):
                    workflow.runner_command('python', 'checkout', 'contract', 'prepared',
                                            'core', 'output', 'gate', core_processes=count)
                lease.assert_not_called()
                contract.assert_not_called()

    def test_invalid_core_timeout_fails_before_generation_or_output(self):
        for timeout in (True, False, 0, -1, 604801, '345600', 1.0, None):
            with self.subTest(timeout=timeout), patch.object(workflow, 'GenerationLease') as lease:
                with self.assertRaisesRegex(ValueError, 'Core timeout'):
                    workflow.prepare('gate', 'contract', 'inventory', 'checkout', 'manifest',
                                     'policy', 'python', 'output', core_timeout_seconds=timeout)
                lease.assert_not_called()

    def test_cli_only_freezes_process_count_during_prepare(self):
        spec = importlib.util.spec_from_file_location('official_cli', workflow.ROOT / 'scripts/run-official-wpt.py')
        cli = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cli)
        base = ['--generation-gate', 'gate', '--run-contract', 'contract', '--checkout',
                'checkout', '--python', 'python', '--output', 'output']
        for count in (None, 2, 6, 8):
            option = [] if count is None else ['--core-processes', str(count)]
            with self.subTest(count=count), patch.object(sys, 'argv',
                        ['run-official-wpt.py', 'prepare', *base, '--inventory', 'inventory',
                         '--manifest', 'manifest', *option]), \
                    patch.object(cli, 'prepare', return_value={}) as prepare, patch('builtins.print'):
                self.assertEqual(cli.main(), 0)
                self.assertEqual(prepare.call_args.kwargs,
                                 {'core_processes': 1 if count is None else count,
                                  'core_timeout_seconds': 43200})
        for kind in ('core', 'canaries', 'placement'):
            for option in (['--core-processes', '6'], ['--core-processes', '8'],
                           ['--core-timeout-seconds', '345600']):
                with self.subTest(collection=kind, option=option), patch.object(sys, 'argv',
                        ['run-official-wpt.py', kind, *base, '--prepared', 'prepared', *option]), \
                        patch.object(cli, 'execute') as execute, patch('sys.stderr'):
                    with self.assertRaises(SystemExit):
                        cli.main()
                    execute.assert_not_called()

    def test_generation_gate_rejects_partial_or_changed_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            plan = root / 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'
            plan.parent.mkdir(parents=True)
            plan.write_text('full source plan')
            source = root / 'implementation.py'
            source.write_text('generated implementation and regression fixture')
            record = {'path': source.name, 'sha256': workflow.sha256(source)}
            requirement = {'implementation_generated': True, 'tests_generated': True,
                           'implementation_sources': [record], 'test_sources': [record]}
            gate = {'schema_version': 1, 'plan_sha256': workflow.sha256(plan),
                    'requirements': {key: copy.deepcopy(requirement) for key in workflow.REQUIREMENTS}}
            path = root / 'gate.json'
            path.write_text(json.dumps(gate))
            with patch.object(workflow, 'ROOT', root):
                self.assertEqual(len(workflow.generation_gate(path)['requirements']), 51)
                source.write_text('changed after declaration')
                with self.assertRaisesRegex(ValueError, 'changed'):
                    workflow.generation_gate(path)
                gate['requirements'].pop('W-05')
                path.write_text(json.dumps(gate))
                with self.assertRaisesRegex(ValueError, '51'):
                    workflow.generation_gate(path)


    def test_owned_execution_preserves_cleanup_failure_and_post_run_gate_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned executable fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {name: {'sha256': 'a' * 64} for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}}, 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **profile_metadata(), 'checkout': str(root), 'python': str(python), 'candidate_sha256': 'a' * 64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {}, 'inventory': 'inventory',
                    'manifest': 'manifest', 'policy': 'policy'}
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            def run(command, output, **kwargs):
                self.assertEqual(kwargs['cwd'], root)
                self.assertTrue((output / 'preflight.json').is_file())
                gate.write_text('{"changed": true}')
                return {'returncode': 0, 'timed_out': False, 'error': None,
                        'cleanup_complete': False, 'elapsed_ns': 1}
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), patch.object(workflow, 'read_inventory'), patch.object(workflow, 'runner_command', return_value=['pinned', 'wpt', '--processes', '1']), patch.object(workflow, 'run_owned', side_effect=run) as owned:
                result = workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
            owned.assert_called_once()
            self.assertEqual(result['returncode'], 0)
            self.assertFalse(result['custody_complete'])
            self.assertFalse(result['cleanup_complete'])
            self.assertTrue(any('generation declaration changed' in error for error in result['errors']))
            self.assertTrue((root / 'output/execution.json').is_file())

    def test_reserved_integrity_exit_rejects_complete_reports_after_restored_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root/'gate'; gate.write_text('{}')
            contract = root/'contract'; contract.write_text('{}')
            python = root/'python'; python.write_text('fixture')
            prepared = root/'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {
                name: {'sha256': 'a'*64} for name in
                ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}},
                'candidate_sha256': 'a'*64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **profile_metadata(), 'checkout': str(root),
                    'python': str(python), 'candidate_sha256': 'a'*64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {},
                    'inventory': 'inventory', 'manifest': 'manifest', 'policy': 'policy'}
            (prepared/'workflow-lock.json').write_text(json.dumps(lock))
            for code in (1, 65):
                def run(command, output, **kwargs):
                    # The child has completed tests and restored inputs; only
                    # its sticky admission verdict distinguishes this failure.
                    (output/'wptreport.json').write_text(json.dumps({'results': [{'test': '/complete.html', 'status': 'OK'}]}))
                    return {'returncode': code, 'timed_out': False, 'error': None,
                            'cleanup_complete': True, 'elapsed_ns': 1}
                with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), patch.object(workflow, 'read_inventory'), patch.object(workflow, 'runner_command', return_value=['pinned', 'wpt', '--processes', '1']), patch.object(workflow, 'run_owned', side_effect=run):
                    result = workflow.execute(gate, contract, root, prepared, python, 'core', root/str(code))
                self.assertEqual(result['custody_complete'], code == 1)
                self.assertEqual(result['errors'], [] if code == 1 else ['official run admission or final integrity was rejected'])
                self.assertTrue(result['cleanup_complete'])



class OfficialPreparationLeaseTests(unittest.TestCase):
    def test_candidate_inspection_cannot_replace_gate_before_preparation_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); gate = root / 'gate'; gate.write_text('original')
            def load(_):
                gate.write_text('valid replacement')
                return {'fixture': {}, 'fixture_path': str(root / 'fixture')}
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', side_effect=load), \
                 patch.object(workflow, 'validate_fixture'), patch.object(workflow, 'read_inventory', return_value={}), \
                 patch.object(workflow.subprocess, 'check_output', return_value=workflow.REVISION), \
                 patch.object(workflow.subprocess, 'run') as launch:
                with self.assertRaisesRegex(ValueError, 'declaration changed'):
                    workflow.prepare(gate, 'contract', 'inventory', root, 'manifest', 'policy', 'python', root / 'output')
                launch.assert_not_called()
                self.assertFalse((root / 'output').exists())


class UpstreamManifestCacheTests(unittest.TestCase):
    """Exercise pinned upstream cache writes, without a browser or subprocess."""

    @staticmethod
    def inventory(root):
        return {path.relative_to(root).as_posix():
                ('directory' if path.is_dir() else workflow.sha256(path))
                for path in sorted(root.rglob('*'))}

    @staticmethod
    def fixture(root, subdir='cases'):
        root.mkdir()
        (root / subdir).mkdir()
        (root / '.gitignore').write_text('ignored.html\n.wptcache/\n')
        page = ('<!doctype html><meta charset=utf-8>'
                '<script src="/resources/testharness.js"></script>'
                '<script src="/resources/testharnessreport.js"></script>'
                '<script>test(() => {}, "manifest cache fixture");</script>')
        (root / subdir / 'fixture.html').write_text(page)
        (root / subdir / 'ignored.html').write_text(page)
        (root / 'not-selected').mkdir()
        (root / 'not-selected/another.html').write_text(page)

    @contextmanager
    def entry_environment(self, tests, output, url_base, subdir='cases', required=None, gate=None, argv=None):
        # The existing qualification launcher includes pinned checkout/tools on
        # PYTHONPATH. Import the real manifest implementation and its vendored
        # dependencies; never replace the cache behavior with a mock.
        import localpaths
        with patch.object(sys, 'path', list(sys.path)):
            from manifest import manifest as upstream
            checkout = Path(localpaths.__file__).resolve().parents[1]
            self.assertEqual(Path(upstream.__file__).resolve(),
                             checkout / 'tools/manifest/manifest.py')
            source = Path(__file__).resolve().parents[1] / 'wpt-upstream-entry.py'
            spec = importlib.util.spec_from_file_location('manifest_cache_entry', source)
            entry = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(entry)
            original = self.inventory(tests)
            if gate is None:
                gate = output.parent / 'fixture-gate.json'
                gate.write_text('{}\n', encoding='utf-8')
            gate_sha256 = workflow.sha256(gate)
            def source_intact():
                self.assertEqual(self.inventory(tests), original,
                                 'manifest preparation mutated its frozen source root')
                self.assertEqual(workflow.sha256(gate), gate_sha256)
            lease = SimpleNamespace(check=source_intact)
            def fixture_admission(path, *, validator, expected):
                self.assertEqual(Path(path).resolve(), gate.resolve())
                self.assertIs(validator, workflow.generation_gate)
                self.assertEqual(expected, gate_sha256)
                source_intact()
                return lease
            def pinned_git(argv, **kwargs):
                if argv == ['git', '-C', str(checkout), 'rev-parse', 'HEAD']:
                    return workflow.REVISION + '\n'
                if argv == ['git', '-C', str(checkout), 'status', '--porcelain',
                            '--untracked-files=all', '--', 'tools']:
                    return ''
                self.fail('unexpected subprocess: ' + repr(argv))
            if argv is None:
                required = required if required is not None else [
                    ('testharness', url_base + subdir + '/fixture.html')]
                argv = [str(source), '--checkout', str(checkout), '--generation-gate',
                        str(gate), 'manifest', '--tests', str(tests), '--subdir', subdir,
                        '--output', str(output), '--url-base', url_base,
                        *(argument for kind, url in required
                          for argument in ('--require-test', kind, url))]
            # Temporary fixture roots have no VCS identity; disable only the Git
            # hashing optimization. File discovery, ignore rules, manifest
            # generation and both cache writers remain upstream-owned.
            # The child bootstrap validates inherited parent/child gate paths
            # before constructing its lease. Give this isolated fixture one
            # consistent actual gate identity, restoring the caller afterward.
            with patch.dict(os.environ, {'ORYN_GENERATION_GATE': str(gate),
                                         'ORYN_GENERATION_GATE_SHA256': gate_sha256}), \
                 patch('generation_gate.GenerationLease', side_effect=fixture_admission), \
                 patch.object(entry.subprocess, 'check_output', side_effect=pinned_git), \
                 patch.object(upstream.vcs, 'git', return_value=None), \
                 patch.object(sys, 'argv', argv):
                yield entry, upstream

    def test_real_upstream_canary_and_placement_caches_preserve_frozen_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            prepared = root / 'prepared'; prepared.mkdir()
            observed_caches = []
            for label, subdir, url_base in (('canary', 'canaries', '/__oryn_qualification__/'),
                                            ('placement', 'shared-memory', '/__oryn_shared_memory__/')):
                with self.subTest(collection=label):
                    tests = root / (label + '-sources'); self.fixture(tests, subdir)
                    original = self.inventory(tests)
                    output = prepared / (label + '-manifest.json')
                    with self.entry_environment(tests, output, url_base, subdir) as (entry, _):
                        self.assertEqual(entry.main(), 0)
                    self.assertEqual(self.inventory(tests), original)
                    self.assertFalse((tests / '.wptcache').exists())
                    manifest = json.loads(output.read_text())
                    self.assertEqual(manifest['url_base'], url_base)
                    self.assertEqual(set(manifest['items']['testharness']), {subdir})
                    self.assertEqual(set(manifest['items']['testharness'][subdir]), {'fixture.html'})
                    self.assertNotIn('ignored.html', manifest['items']['testharness'][subdir])
                    cache = prepared / (output.name + '.cache')
                    self.assertTrue((cache / 'mtime.json').is_file())
                    self.assertTrue((cache / 'gitignore2.json').is_file())
                    mtime = json.loads((cache / 'mtime.json').read_text())
                    self.assertEqual(mtime['/tests_root'], str(tests))
                    self.assertEqual(mtime['/manifest_path'][0], str(output))
                    observed_caches.append(cache)
            self.assertEqual(len(set(observed_caches)), 2)

    def test_real_upstream_write_failure_preserves_error_and_source_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tests = root / 'frozen-sources'; self.fixture(tests)
            original = self.inventory(tests)
            prepared = root / 'prepared'; prepared.mkdir()
            output = prepared / 'canary-manifest.json'
            failure = OSError('retained upstream manifest write failure')
            with self.entry_environment(tests, output, '/__oryn_qualification__/') as (entry, upstream), \
                 patch.object(upstream, 'write', side_effect=failure):
                with self.assertRaises(OSError) as captured:
                    entry.main()
            self.assertIs(captured.exception, failure)
            self.assertEqual(self.inventory(tests), original)
            self.assertFalse((tests / '.wptcache').exists())
            self.assertTrue((prepared / (output.name + '.cache')).is_dir())
            self.assertFalse(output.exists())


    def test_local_manifest_rejects_root_escape_and_missing_subdirectories(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tests = root / 'sources'; self.fixture(tests)
            outside = root / 'outside'; outside.mkdir()
            (tests / 'escape').symlink_to(outside, target_is_directory=True)
            prepared = root / 'prepared'; prepared.mkdir()
            for index, subdir in enumerate(('', '.', '..', '../outside', str(outside),
                                            'cases/../cases', 'cases//child', 'missing', 'escape')):
                with self.subTest(subdir=subdir):
                    output = prepared / ('manifest-' + str(index) + '.json')
                    with self.entry_environment(tests, output, '/__oryn_qualification__/', subdir) as (entry, _):
                        with self.assertRaisesRegex(ValueError, 'subdirectory'):
                            entry.main()
                    self.assertFalse(output.exists())
                    self.assertFalse((prepared / (output.name + '.cache')).exists())

    def test_real_upstream_missing_or_wrong_type_required_test_rejects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            tests = root / 'sources'; self.fixture(tests)
            prepared = root / 'prepared'; prepared.mkdir()
            prefix = '/__oryn_qualification__/cases/'
            for index, required in enumerate(([('testharness', prefix + 'absent.html')],
                                              [('reftest', prefix + 'fixture.html')])):
                with self.subTest(required=required):
                    output = prepared / ('manifest-' + str(index) + '.json')
                    original = self.inventory(tests)
                    with self.entry_environment(tests, output, '/__oryn_qualification__/',
                                                required=required) as (entry, _):
                        with self.assertRaisesRegex(ValueError, 'required local manifest tests missing or wrong type'):
                            entry.main()
                    self.assertTrue(output.is_file())
                    self.assertEqual(self.inventory(tests), original)

    def test_actual_local_collection_preparation_preserves_scope_routes_and_custody(self):
        # Copy the actual fixture bytes; manifests, saved-manifest loading,
        # include filtering, URL-to-file mapping and header parsing stay pinned
        # upstream code. Only admission and subprocess boundaries are isolated.
        import localpaths
        from wptrunner import config, testloader, wptcommandline
        from wptserve.handlers import filesystem_path, load_headers
        from oryn_wptrunner import placement as native_placement
        import wpt_canary_qualification as canary
        import wpt_shared_placement as placement

        source_root = workflow.ROOT
        checkout = Path(localpaths.__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            repository = root / 'repository'
            plugin = repository / 'tools/wptrunner-oryn'
            fixtures = repository / 'crates/oryn-native/tests/fixtures'
            canary_root = plugin / 'canaries'
            placement_root = fixtures / 'shared-memory'
            shutil.copytree(source_root / 'tools/wptrunner-oryn/canaries', canary_root)
            shutil.copytree(source_root / 'crates/oryn-native/tests/fixtures/shared-memory', placement_root)
            for selected in (canary_root, placement_root):
                nested = selected / 'support/nested/resource.js'
                nested.parent.mkdir(parents=True)
                nested.write_text('// source-custody regression fixture\n')
                sibling = selected.parent / 'not-selected'
                sibling.mkdir()
                shutil.copyfile(canary_root / 'assertion-failure.html', sibling / 'unexpected.html')
            originals = {path: self.inventory(path) for path in (plugin, fixtures)}
            gate = root / 'gate.json'; gate.write_text('{}\n')
            gate_hash = workflow.sha256(gate)
            contract = root / 'contract.json'; contract.write_text('{}\n')
            policy = root / 'policy.json'; policy.write_text('{}\n')
            inventory = root / 'inventory'; inventory.mkdir()
            (inventory / 'plan.json').write_text('{}\n')
            (inventory / 'coverage.jsonl').write_text('{}\n')
            # This official input is only hashed in the isolated preparation
            # fixture. The two local manifests are always generated upstream.
            official_manifest = root / 'official-input'; official_manifest.write_text('frozen official input\n')
            prepared = root / 'prepared'
            bound = {**profile_metadata(), 'candidate_sha256': 'a' * 64,
                     'sha256': workflow.sha256(contract), 'fixture': {},
                     'fixture_path': str(root / 'fixture.json')}
            core = {'automatic': {'url': '/unchanged/official.html', 'test_type': 'testharness'},
                    'assessment': {'url': '/unchanged/assessment.html', 'test_type': 'manual'}}

            def intact():
                self.assertEqual(workflow.sha256(gate), gate_hash)
                for path, original in originals.items():
                    self.assertEqual(self.inventory(path), original)

            lease = SimpleNamespace(check=intact, environment=lambda: {
                **os.environ, 'ORYN_GENERATION_GATE': str(gate),
                'ORYN_GENERATION_GATE_SHA256': gate_hash})
            calls = []

            def generate(command, *, check, env):
                self.assertTrue(check)
                self.assertEqual(env['ORYN_GENERATION_GATE_SHA256'], gate_hash)
                tests = Path(command[command.index('--tests') + 1])
                output = Path(command[command.index('--output') + 1])
                url_base = command[command.index('--url-base') + 1]
                subdir = command[command.index('--subdir') + 1]
                calls.append((tests, subdir, url_base))
                with self.entry_environment(tests, output, url_base, subdir,
                                            gate=gate, argv=command[1:]) as (entry, _):
                    self.assertEqual(entry.main(), 0)
                return SimpleNamespace(returncode=0)

            with patch.object(workflow, 'ROOT', repository), patch.object(workflow, 'PLUGIN', plugin), \
                 patch.object(workflow, 'GenerationLease', return_value=lease), \
                 patch.object(workflow, 'load_contract', return_value=bound), \
                 patch.object(workflow, 'validate_fixture'), \
                 patch.object(workflow, 'read_inventory', return_value=core), \
                 patch.object(workflow.subprocess, 'check_output', return_value=workflow.REVISION + '\n'), \
                 patch.object(workflow.subprocess, 'run', side_effect=generate):
                lock = workflow.prepare(gate, contract, inventory, checkout, official_manifest,
                                        policy, Path(sys.executable), prepared, core_processes=4, core_timeout_seconds=345600)

            self.assertEqual(calls, [(plugin, 'canaries', '/__oryn_qualification__/'),
                                     (fixtures, 'shared-memory', '/__oryn_shared_memory__/')])
            self.assertEqual((prepared / 'core.include').read_text(), '/unchanged/official.html\n')
            self.assertEqual(lock['core_total'], 2)
            self.assertEqual(lock['core_processes'], 4)
            self.assertEqual(lock['core_timeout_seconds'], 345600)
            for selected in (canary_root, placement_root):
                for path in selected.rglob('*'):
                    if path.is_file():
                        self.assertEqual(lock['inputs'][str(path)], workflow.sha256(path))
            self.assertFalse(any('/not-selected/' in path for path in lock['inputs']))

            paths = wptcommandline.get_test_paths(config.read(str(prepared / 'wptrunner.ini')))
            self.assertEqual(paths['/'].tests_path, str(checkout))
            local = {base: data for base, data in paths.items() if base != '/'}
            self.assertEqual(set(local), {'/__oryn_qualification__/', '/__oryn_shared_memory__/'})
            # Match runner --no-manifest-update/--no-manifest-download behavior.
            loaded = testloader.ManifestLoader(local, force_manifest_update=False,
                                               manifest_download=False).load()
            by_base = {manifest.url_base: (manifest, data) for manifest, data in loaded.items()}
            selected_rows = {}
            for collection, base, subdir in (('canaries', '/__oryn_qualification__/', 'canaries'),
                                             ('placement', '/__oryn_shared_memory__/', 'shared-memory')):
                manifest, data = by_base[base]
                self.assertEqual(data['tests_path'], str(plugin if collection == 'canaries' else fixtures))
                self.assertTrue(all(Path(path).parts[0] == subdir for _, path, _ in manifest))
                include = (prepared / (collection + '.include')).read_text().splitlines()
                select = testloader.TestFilter(loaded, include=include, explicit=True)
                items = [(kind, item) for kind, _, tests in select(manifest.itertypes('testharness', 'reftest'))
                         for item in tests]
                selected_rows[collection] = {item.id: kind for kind, item in items}
                for _, item in items:
                    request = SimpleNamespace(url_parts=urlsplit(item.id))
                    served = Path(filesystem_path(data['tests_path'], request, base))
                    self.assertEqual(served, Path(data['tests_path']) / item.path)
                    self.assertTrue(served.is_file())
                    if item.item_type == 'reftest':
                        self.assertEqual([tuple(reference) for reference in item.references],
                                         [(canary.MOUNT + 'reference-green.html', '==')])
                        reference = SimpleNamespace(url_parts=urlsplit(item.references[0][0]))
                        self.assertEqual(Path(filesystem_path(data['tests_path'], reference, base)),
                                         canary_root / 'reference-green.html')
                self.assertEqual(set(selected_rows[collection]), set(include))

            controls = json.loads((canary_root / 'controls.json').read_text())
            self.assertEqual(controls['mount'], canary.MOUNT)
            self.assertEqual({row['path'] for row in controls['controls']}, set(canary.EXPECTED))
            self.assertEqual(selected_rows['canaries'], {
                canary.MOUNT + row['path']: row['type'] for row in controls['controls']})
            self.assertEqual(len(selected_rows['canaries']), 5)
            self.assertEqual(selected_rows['placement'], {placement.TEST: 'testharness'})
            self.assertEqual(native_placement.TEST, placement.TEST)
            self.assertEqual(workflow.PLACEMENT_TEST, placement.TEST)
            worker_url = urljoin(placement.TEST, 'worker.js?placement=1')
            self.assertEqual(worker_url, '/__oryn_shared_memory__/shared-memory/worker.js?placement=1')
            request = SimpleNamespace(url_parts=urlsplit(worker_url))
            self.assertEqual(Path(filesystem_path(str(fixtures), request, '/__oryn_shared_memory__/')),
                             placement_root / 'worker.js')
            page = placement_root / 'engine-lifecycle.https.html'
            headers = load_headers(None, str(page))
            self.assertIn((b'Cross-Origin-Opener-Policy', b'same-origin'), headers)
            self.assertIn((b'Cross-Origin-Embedder-Policy', b'require-corp'), headers)
            self.assertIn('/__oryn_qualification_missing_helper__.js',
                          (canary_root / 'missing-helper.html').read_text())
            intact()

    def test_canary_resource_observation_is_collection_scoped_at_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned executable fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {
                name: {'sha256': 'a' * 64}
                for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}},
                'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, 'core_processes': 4, 'core_timeout_seconds': 345600, **profile_metadata(), 'checkout': str(root),
                    'python': str(python), 'candidate_sha256': 'a' * 64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {},
                    'inventory': 'inventory', 'manifest': 'manifest', 'policy': 'policy'}
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            for kind in ('core', 'canaries', 'placement'):
                with self.subTest(collection=kind):
                    def run(command, output, *, timeout, environment, cwd):
                        self.assertEqual(timeout, 345600 if kind == 'core' else 43200)
                        if kind == 'canaries':
                            self.assertEqual(environment['ORYN_WPT_CANARY_RESOURCE_AUDIT'], '1')
                        else:
                            self.assertNotIn('ORYN_WPT_CANARY_RESOURCE_AUDIT', environment)
                        if kind == 'placement':
                            context = json.loads((output / 'placement-context.json').read_text())
                            self.assertEqual(context['test'], workflow.PLACEMENT_TEST)
                            self.assertEqual(environment['ORYN_WPT_PLACEMENT_CONTEXT'],
                                             str(output / 'placement-context.json'))
                        return {'returncode': 1 if kind == 'canaries' else 0,
                                'timed_out': False, 'error': None,
                                'cleanup_complete': True, 'elapsed_ns': 1}
                    with patch.dict(os.environ, {'ORYN_WPT_CANARY_RESOURCE_AUDIT': 'stale'}), \
                         patch.object(workflow, 'generation_gate'), \
                         patch.object(workflow, 'load_contract', return_value=bound), \
                         patch.object(workflow, 'read_inventory'), \
                         patch.object(workflow, 'runner_command', side_effect=lambda *args, core_processes: ['pinned', 'wpt', '--processes', str(core_processes if kind == 'core' else 1)]), \
                         patch.object(workflow, 'run_owned', side_effect=run) as owned:
                        result = workflow.execute(gate, contract, root, prepared, python,
                                                  kind, root / (kind + '-output'))
                    owned.assert_called_once()
                    self.assertTrue(result['custody_complete'])
                    preflight = json.loads((root / (kind + '-output') / 'preflight.json').read_text())
                    for receipt in (result, preflight):
                        self.assertEqual(receipt['core_processes'], 4)
                        self.assertEqual(receipt['processes'], 4 if kind == 'core' else 1)
                        self.assertEqual(receipt['core_timeout_seconds'], 345600)
                        self.assertEqual(receipt['timeout_seconds'], 345600 if kind == 'core' else 43200)

if __name__ == '__main__':
    unittest.main()

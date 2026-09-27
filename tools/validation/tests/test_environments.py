"""Offline runtime custody tests; no real Python staging, pip or network runs."""
import copy
import json
import importlib.machinery
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import environments as e
from oryn_validation.inputs import _file, snapshot_input


def dylib_command(name, kind='LC_LOAD_DYLIB'):
    return '\n cmd ' + kind + '\n cmdsize 80\n name ' + name + ' (offset 24)\n'


class Environments(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.store, self.donor, self.stdlib = [self.root / x for x in ('store', 'donor', 'framework/lib/python3.14')]
        for p in (self.store, self.donor, self.stdlib):
            p.mkdir(parents=True)
        self.write(self.donor / 'example.py', b'value = 1\n')
        self.write(self.donor / 'example-1.dist-info/METADATA', b'Name: example\nVersion: 1\n')
        self.write(self.donor / 'example-1.dist-info/WHEEL', b'Wheel-Version: 1.0\nTag: py3-none-any\n')
        for name in ('os.py', 'json/__init__.py', 'sysconfig/__init__.py'):
            self.write(self.stdlib / name, b'# stdlib fixture\n')
        self.python = self.root / 'system/python3.14'
        self.write(self.python, b'fixture interpreter, never executed\n', executable=True)
        self.lock = {'kind': 'wpt-python-dependency-lock', 'schema_version': 1,
                     'python_version': '3.14.7', 'plugin_policy': 'runner-source-export',
                     'files': e._payload(self.donor),
                     'distributions': [{'name': 'example', 'version': '1',
                                       'metadata': 'example-1.dist-info/METADATA',
                                       'wheel_metadata': 'example-1.dist-info/WHEEL',
                                       'archive_sha256': None}]}
        self.lock_path = self.root / 'lock.json'
        self.write(self.lock_path, (json.dumps(self.lock, sort_keys=True, indent=2) + '\n').encode())
        self.enterContext(patch.object(e, 'DEFAULT_LOCK', self.lock_path))
        self.enterContext(patch.object(e, 'ResourceLease'))
        self.system = {'libraries': [], 'os_shared_cache_names': ['/usr/lib/libSystem.B.dylib'],
                       'macos_build': 'test-only'}
        self.system_observer = e._system_libraries
        self.libs = self.enterContext(patch.object(e, '_system_libraries', return_value=self.system))
        self.launch = self.enterContext(patch.object(e, 'run_owned', side_effect=self.probe))

    def cleanup(self):
        if hasattr(self, 'root'):
            for path in self.root.rglob('*'):
                if path.is_dir() and not path.is_symlink():
                    path.chmod(0o755)
        self.temp.cleanup()

    def write(self, path, data, executable=False):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755 if executable else 0o644)

    def probe(self, argv, *, cwd, output, env, timeout_seconds, grace_seconds):
        self.assertEqual(env, {'PATH': '/usr/bin:/bin', **e._runtime_env(cwd)})
        self.assertEqual(argv, [str(cwd / 'bin/python'), '-B', '-s', '-c', e.PROBE])
        self.assertEqual((timeout_seconds, grace_seconds), (120, 15))
        output.mkdir(parents=True)
        actual = {'version': '3.14.7', 'system': 'Darwin', 'architecture': 'arm64',
                  'executable': str(cwd / 'bin/python'), 'prefix': str(cwd), 'base_prefix': str(cwd),
                  'stdlib': str(cwd / 'lib/python3.14'), 'path': ['', str(cwd / 'lib/python3.14')],
                  'distributions': [['example', '1']],
                  'interpreter_sha256': _file(cwd / 'bin/python')['sha256'],
                  'lock_sha256': _file(cwd / 'dependency-lock.json')['sha256']}
        result = {'argv': argv, 'returncode': 0, 'error': None, 'cleanup_complete': True,
                  'cleanup_errors': [], 'timed_out': False, 'forced_cleanup': False,
                  'cancellation_signal': None}
        for name, value in [('stdout.log', actual), ('process-result.json', result),
                            ('preflight.json', {'argv': argv, 'cwd': str(cwd), 'timeout_seconds': 120,
                                                'owner_pid': 100, 'child_pid': 101,
                                                'child_identity': [[12345, 678], 100]})]:
            (output / name).write_text(json.dumps(value))
        (output / 'stderr.log').write_text('')
        return result

    def prepare(self, **kwargs):
        paths = dict(environment_path='environments/python', identity_path='identities/python.json',
                     diagnostics_path='evidence/python-probe')
        paths.update(kwargs)
        assets = e.prepare_environment(self.donor, self.python, self.stdlib, self.store, **paths)
        self.identity = json.loads((self.store / 'identities/python.json').read_text())
        return assets

    def refresh_membership(self):
        self.identity['environment'] = snapshot_input(self.store, 'environments/python', kind='directory')

    def test_stages_regular_runtime_and_locked_payload_without_installer(self):
        self.write(self.donor / 'oryn_wptrunner/provenance.py', b'old plugin excluded')
        self.write(self.donor / 'oryn_wptrunner-0.1.dist-info/METADATA', b'old metadata')
        self.write(self.donor / '__pycache__/example.pyc', b'cache excluded')
        (self.stdlib / 'site-packages').symlink_to(self.donor, target_is_directory=True)
        original = e._payload(self.donor)
        assets = self.prepare()
        self.assertFalse(assets['qualified'])
        self.assertEqual(self.launch.call_count, 1)
        self.assertEqual(e._payload(self.donor), original)
        self.assertFalse((self.store / 'environments/python/lib/python3.14/site-packages/oryn_wptrunner').exists())
        self.assertTrue((self.store / 'environments/python/bin/python').is_file())
        self.assertNotIn(str(self.donor), json.dumps(self.identity))
        self.assertEqual(e.verify_environment(self.identity, self.store)['distributions'], 1)
        self.assertEqual(e.runtime_environment(self.identity, self.store)['PYTHONHOME'],
                         str(self.store / 'environments/python'))

    def test_same_size_dependency_change_and_new_member_rejected(self):
        self.write(self.donor / 'example.py', b'value = 2\n')
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            self.prepare()
        self.write(self.donor / 'example.py', b'value = 1\n')
        self.write(self.donor / 'extra.py', b'extra')
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            self.prepare()
        self.launch.assert_not_called()

    def test_payload_symlink_and_path_escape_rejected(self):
        (self.donor / 'evil').symlink_to(self.python)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.prepare()
        (self.donor / 'evil').unlink()
        with self.assertRaises(ValueError):
            self.prepare(environment_path='../outside')
        self.launch.assert_not_called()

    def test_output_aliases_and_existing_attempt_rejected(self):
        with self.assertRaisesRegex(ValueError, 'fresh separate'):
            self.prepare(identity_path='environments/python')
        self.prepare()
        with self.assertRaisesRegex(ValueError, 'fresh separate'):
            self.prepare()

    def test_wheel_metadata_and_versions_are_not_inferred(self):
        lock = copy.deepcopy(self.lock)
        lock['distributions'][0]['version'] = '2'
        with self.assertRaisesRegex(ValueError, 'versions'):
            e._dependencies(self.donor, lock)
        lock = copy.deepcopy(self.lock)
        lock['distributions'][0]['wheel_metadata'] = 'absent/WHEEL'
        with self.assertRaisesRegex(ValueError, 'METADATA/WHEEL'):
            e._dependencies(self.donor, lock)

    def test_failed_or_unsettled_probe_preserved_without_identity(self):
        def failed(*args, **kwargs):
            result = self.probe(*args, **kwargs)
            result.update(returncode=1, cleanup_complete=False, cleanup_errors=['test failure'])
            (kwargs['output'] / 'process-result.json').write_text(json.dumps(result))
            return result
        self.launch.side_effect = failed
        with self.assertRaisesRegex(ValueError, 'failed or did not settle'):
            self.prepare()
        self.assertFalse((self.store / 'identities/python.json').exists())
        self.assertFalse(json.loads((self.store / 'evidence/python-probe/process-result.json').read_text())['cleanup_complete'])

    def test_donor_drift_during_probe_rejected(self):
        def drift(*args, **kwargs):
            result = self.probe(*args, **kwargs)
            self.write(self.donor / 'example.py', b'value = 2\n')
            return result
        self.launch.side_effect = drift
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            self.prepare()
        self.assertFalse((self.store / 'identities/python.json').exists())

    def test_environment_mutation_during_probe_rejected(self):
        def drift(*args, **kwargs):
            result = self.probe(*args, **kwargs)
            self.write(kwargs['cwd'] / 'injected.py', b'not in admitted runtime')
            return result
        self.launch.side_effect = drift
        with self.assertRaisesRegex(ValueError, 'membership'):
            self.prepare()
        self.assertFalse((self.store / 'identities/python.json').exists())

    def test_staged_plugin_cannot_enter_even_with_refreshed_membership(self):
        self.prepare()
        site = self.store / 'environments/python/lib/python3.14/site-packages'
        site.chmod(0o755)
        self.write(site / 'oryn_wptrunner/__init__.py', b'wrong plugin')
        self.refresh_membership()
        with self.assertRaisesRegex(ValueError, 'installed Oryn plugin'):
            e.verify_environment(self.identity, self.store)

    def test_staged_byte_drift_and_system_library_drift_rejected(self):
        self.prepare()
        self.libs.return_value = {**self.system, 'macos_build': 'different'}
        with self.assertRaisesRegex(ValueError, 'system Python dependency changed'):
            e.verify_environment(self.identity, self.store)
        self.libs.return_value = self.system
        path = self.store / 'environments/python/lib/python3.14/site-packages/example.py'
        path.chmod(0o644); path.write_text('value = 2\n')
        self.refresh_membership()
        with self.assertRaisesRegex(ValueError, 'payload differs'):
            e.verify_environment(self.identity, self.store)

    def test_probe_path_version_and_exact_command_are_bound(self):
        self.prepare()
        diagnostics = self.store / 'evidence/python-probe'
        actual = json.loads((diagnostics / 'stdout.log').read_text())
        actual['path'].append('/outside/site-packages')
        (diagnostics / 'stdout.log').write_text(json.dumps(actual))
        self.identity['probe'] = snapshot_input(self.store, 'evidence/python-probe', kind='directory')
        with self.assertRaisesRegex(ValueError, 'sys.path escapes'):
            e.verify_environment(self.identity, self.store)
        actual['path'].pop()
        actual['interpreter_sha256'] = '0' * 64
        (diagnostics / 'stdout.log').write_text(json.dumps(actual))
        self.identity['probe'] = snapshot_input(self.store, 'evidence/python-probe', kind='directory')
        with self.assertRaisesRegex(ValueError, 'ownership or exact command'):
            e.verify_environment(self.identity, self.store)

    def test_relocation_uses_new_store_and_preserves_original_probe(self):
        self.prepare()
        moved = self.root / 'relocated'
        self.store.rename(moved)
        self.assertTrue(e.verify_environment(self.identity, moved)['valid'])
        self.assertEqual(e.runtime_environment(self.identity, moved)['PYTHONHOME'], str(moved / 'environments/python'))

    def test_runtime_asset_join_and_lock_override_rejected(self):
        self.prepare()
        identity = copy.deepcopy(self.identity)
        identity['python'] = identity['dependency_lock']
        with self.assertRaisesRegex(ValueError, 'asset paths'):
            e.verify_environment(identity, self.store)
        alternative = self.root / 'other-lock.json'
        alternative.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'source-owned pinned lock'):
            e._lock(alternative)

    def test_missing_birth_or_false_parent_custody_rejected(self):
        self.prepare()
        path = self.store / 'evidence/python-probe/preflight.json'
        value = json.loads(path.read_text())
        value['child_identity'][1] = 999
        path.write_text(json.dumps(value))
        self.identity['probe'] = snapshot_input(self.store, 'evidence/python-probe', kind='directory')
        with self.assertRaisesRegex(ValueError, 'kernel birth custody'):
            e.verify_environment(self.identity, self.store)

    def test_macho_system_shared_cache_names_reject_lexical_escape(self):
        native = self.root / 'native'
        native.mkdir()
        self.write(native / 'python', b'\xfe\xed\xfa\xcf' + b'fake')
        with patch.object(e, '_otool', side_effect=lambda p, flag:
                          dylib_command('/usr/lib/../../outside')):
            with self.assertRaisesRegex(ValueError, 'noncanonical'):
                # Call the original function, not the fixture's system observer.
                self.system_observer(native)

    def test_macho_internal_loader_and_explicit_system_observation(self):
        native = self.root / 'native'
        native.mkdir()
        for name in ('bin/python', 'lib/runtime.dylib'):
            self.write(native / name, b'\xfe\xed\xfa\xcf' + b'fake')
        def otool(path, flag):
            load = '@loader_path/../lib/runtime.dylib' if path.name == 'python' else '/usr/lib/libSystem.B.dylib'
            return dylib_command(load)
        with patch.object(e, '_otool', side_effect=otool), patch.object(e.subprocess, 'check_output', return_value='os-build\n'):
            observed = self.system_observer(native)
        self.assertEqual(observed['libraries'], [])
        self.assertEqual(observed['os_shared_cache_names'], ['/usr/lib/libSystem.B.dylib'])
        def os_only(path, flag):
            return dylib_command('/usr/lib/libSystem.B.dylib')
        with patch.object(e, '_otool', side_effect=os_only), patch.object(e.subprocess, 'check_output', return_value='os-build\n'):
            observed = self.system_observer(native)
        self.assertEqual(observed, {'libraries': [], 'os_shared_cache_names': ['/usr/lib/libSystem.B.dylib'],
                                    'macos_build': 'os-build'})

    def add_framework_aliases(self):
        framework = self.stdlib.parents[1] / 'Python'
        self.write(framework, b'framework library fixture', executable=True)
        for name, target in e.STDLIB_LINKS.items():
            alias = self.stdlib / name
            alias.parent.mkdir(parents=True, exist_ok=True)
            alias.symlink_to(target)
        return framework

    def test_only_declared_framework_aliases_materialize_as_exact_regular_files(self):
        framework = self.add_framework_aliases()
        self.prepare()
        self.assertEqual(len(self.identity['stdlib_materializations']), 2)
        for row in self.identity['stdlib_materializations']:
            alias = self.stdlib / row['path']
            staged = self.store / 'environments/python/lib/python3.14' / row['path']
            self.assertTrue(alias.is_symlink())
            self.assertFalse(staged.is_symlink())
            self.assertEqual(_file(staged), _file(framework))
        self.assertTrue(e.verify_environment(self.identity, self.store)['valid'])
        self.identity['stdlib_materializations'][0]['materialized']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'materialization identity'):
            e.verify_environment(self.identity, self.store)

    def test_framework_alias_retarget_and_undeclared_link_fail_before_probe(self):
        self.add_framework_aliases()
        alias = self.stdlib / next(iter(e.STDLIB_LINKS))
        alias.unlink(); alias.symlink_to('../../../../system/python3.14')
        with self.assertRaisesRegex(ValueError, 'spelling changed'):
            self.prepare()
        alias.unlink(); alias.symlink_to('../../../Python')
        (self.stdlib / 'unexpected.py').symlink_to(self.python)
        with self.assertRaisesRegex(ValueError, 'undeclared symlink'):
            self.prepare()
        self.launch.assert_not_called()

    def test_framework_target_change_during_probe_fails_without_identity(self):
        framework = self.add_framework_aliases()
        def drift(*args, **kwargs):
            result = self.probe(*args, **kwargs)
            framework.write_bytes(b'changed framework bytes')
            return result
        self.launch.side_effect = drift
        with self.assertRaisesRegex(ValueError, 'source changed while staging'):
            self.prepare()
        self.assertFalse((self.store / 'identities/python.json').exists())

    def test_macho_self_id_is_not_an_unresolved_load(self):
        native = self.root / 'native-id'
        native.mkdir()
        library = native / 'extension.so'
        self.write(library, b'\xfe\xed\xfa\xcf' + b'fixture')
        own = '@rpath/extension.so'
        def otool(path, flag):
            return dylib_command(own, 'LC_ID_DYLIB')
        with patch.object(e, '_otool', side_effect=otool), patch.object(e.subprocess, 'check_output', return_value='os-build\n'):
            self.assertEqual(self.system_observer(native)['libraries'], [])
        def unresolved(path, flag):
            output = otool(path, flag)
            return output + dylib_command('@rpath/missing.dylib')
        with patch.object(e, '_otool', side_effect=unresolved):
            with self.assertRaisesRegex(ValueError, 'unresolved or ambiguous'):
                self.system_observer(native)
        # A real load with the same spelling as the ID must not be discarded.
        with patch.object(e, '_otool', return_value=dylib_command(own, 'LC_ID_DYLIB') + dylib_command(own)):
            with self.assertRaisesRegex(ValueError, 'unresolved or ambiguous'):
                self.system_observer(native)
        for kind in ('LC_LOAD_WEAK_DYLIB', 'LC_REEXPORT_DYLIB', 'LC_LAZY_LOAD_DYLIB', 'LC_LOAD_UPWARD_DYLIB'):
            self.assertEqual(e._dylib_loads(dylib_command(own, kind)), [own])


    def add_host_startup_hook(self):
        path = self.stdlib / 'sitecustomize.py'
        self.write(path, b"raise RuntimeError('external pth must never execute')\n")
        self.enterContext(patch.object(e, 'HOMEBREW_SITE', _file(path)))
        return path

    def test_exact_host_hook_omitted_before_discovery_and_runner_hook_remains_available(self):
        original = self.add_host_startup_hook()
        original_bytes = original.read_bytes()
        self.write(self.stdlib / 'site.py', b'# unchanged standard site processing\n')
        self.prepare()
        staged = self.store / 'environments/python/lib/python3.14'
        self.assertEqual(self.identity['stdlib_omissions'], [e._site_omission()])
        self.assertEqual(original.read_bytes(), original_bytes)
        self.assertEqual((staged / 'site.py').read_bytes(), (self.stdlib / 'site.py').read_bytes())
        self.assertIsNone(importlib.machinery.PathFinder.find_spec('sitecustomize', [str(staged)]))
        # Normal site startup in -c, -m and multiprocessing children performs
        # this same lookup; no post-execution sys.path filtering is involved.
        bootstrap = self.root / 'runner/bootstrap'
        self.write(bootstrap / 'sitecustomize.py', b'# separately pinned WPT hook\n')
        selected = importlib.machinery.PathFinder.find_spec('sitecustomize', [str(bootstrap), str(staged)])
        self.assertEqual(selected.origin, str(bootstrap / 'sitecustomize.py'))
        self.assertTrue(e.verify_environment(self.identity, self.store)['valid'])

    def test_unknown_host_hook_and_source_mutation_rejected(self):
        source = self.add_host_startup_hook()
        source.write_bytes(b'unreviewed startup code')
        with self.assertRaisesRegex(ValueError, 'unreviewed stdlib sitecustomize'):
            self.prepare()
        self.launch.assert_not_called()

    def test_hook_change_during_probe_and_reintroduction_rejected(self):
        source = self.add_host_startup_hook()
        def drift(*args, **kwargs):
            result = self.probe(*args, **kwargs)
            source.write_bytes(b'changed original hook')
            return result
        self.launch.side_effect = drift
        with self.assertRaisesRegex(ValueError, 'unreviewed stdlib sitecustomize'):
            self.prepare()
        self.assertFalse((self.store / 'identities/python.json').exists())

    def test_staged_hook_and_omission_claim_tampering_rejected(self):
        self.add_host_startup_hook()
        self.prepare()
        identity = copy.deepcopy(self.identity)
        identity['stdlib_omissions'][0]['source']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'normalization differs'):
            e.verify_environment(identity, self.store)
        staged = self.store / 'environments/python/lib/python3.14'
        staged.chmod(0o755)
        self.write(staged / 'sitecustomize.py', b'external code')
        self.refresh_membership()
        with self.assertRaisesRegex(ValueError, 'cannot enter staged runtime'):
            e.verify_environment(self.identity, self.store)


class SourceDependencyLock(unittest.TestCase):
    """Exercise real committed lock data, not only mocked fixture metadata."""

    def test_real_source_lock_is_accepted_by_production_loader(self):
        lock = e._lock()
        self.assertEqual(len(lock['files']), 1607)
        self.assertEqual(len(lock['distributions']), 40)
        paths = [row['path'] for row in lock['files']]
        self.assertEqual(paths, sorted(set(paths)))
        identities = {row['path'] for row in lock['files']}
        for distribution in lock['distributions']:
            self.assertIn(distribution['metadata'], identities)
            self.assertIn(distribution['wheel_metadata'], identities)

    def test_out_of_order_real_lock_records_are_rejected(self):
        value = json.loads(e.DEFAULT_LOCK.read_text())
        value['files'][0], value['files'][1] = value['files'][1], value['files'][0]
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'lock.json'
            path.write_text(json.dumps(value))
            with patch.object(e, 'DEFAULT_LOCK', path):
                with self.assertRaisesRegex(ValueError, 'sorted and unique'):
                    e._lock()

    @unittest.skipUnless(os.environ.get('ORYN_VALIDATION_DEPENDENCY_DONOR'),
                         'set explicit migration donor for read-only payload verification')
    def test_selected_actual_donor_matches_real_source_lock(self):
        donor = Path(os.environ['ORYN_VALIDATION_DEPENDENCY_DONOR']).absolute()
        # No subprocess, staging, installation or writes to the donor occur here.
        # The migration donor may contain the old plugin, which is never copied.
        e._dependencies(donor, e._lock(), allow_plugin=True)

    @unittest.skipUnless(all(os.environ.get(name) for name in (
        'ORYN_VALIDATION_DEPENDENCY_DONOR', 'ORYN_VALIDATION_SYSTEM_PYTHON', 'ORYN_VALIDATION_STDLIB')),
        'set explicit donor, Python and stdlib for read-only complete source preflight')
    def test_selected_actual_stdlib_and_native_dependency_graph_preflight(self):
        result = e.inspect_environment_sources(os.environ['ORYN_VALIDATION_DEPENDENCY_DONOR'],
            os.environ['ORYN_VALIDATION_SYSTEM_PYTHON'], os.environ['ORYN_VALIDATION_STDLIB'])
        self.assertEqual(result['dependency_files'], 1607)
        self.assertEqual({r['path'] for r in result['stdlib_materializations']}, set(e.STDLIB_LINKS))
        self.assertEqual(result['stdlib_omissions'], [e._site_omission()])
        self.assertFalse(result['runtime_executed'])
        self.assertFalse(result['qualified'])


if __name__ == '__main__':
    unittest.main()

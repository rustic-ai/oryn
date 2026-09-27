"""Offline runtime custody tests; no real Python staging, pip or network runs."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import environments as e
from oryn_validation.inputs import _file, snapshot_input


class Environments(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.store, self.donor, self.stdlib = [self.root / x for x in ('store', 'donor', 'stdlib')]
        for p in (self.store, self.donor, self.stdlib):
            p.mkdir()
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
                          str(p) + ':\n\t/usr/lib/../../outside (compatibility version 1.0.0)\n'
                          if flag == '-L' else ''):
            with self.assertRaisesRegex(ValueError, 'noncanonical'):
                # Call the original function, not the fixture's system observer.
                self.system_observer(native)

    def test_macho_internal_loader_and_explicit_system_observation(self):
        native = self.root / 'native'
        native.mkdir()
        for name in ('bin/python', 'lib/runtime.dylib'):
            self.write(native / name, b'\xfe\xed\xfa\xcf' + b'fake')
        def otool(path, flag):
            if flag == '-l':
                return ''
            load = '@loader_path/../lib/runtime.dylib' if path.name == 'python' else '/usr/lib/libSystem.B.dylib'
            return str(path) + ':\n\t' + load + ' (compatibility version 1.0.0)\n'
        with patch.object(e, '_otool', side_effect=otool), patch.object(e.subprocess, 'check_output', return_value='os-build\n'):
            observed = self.system_observer(native)
        self.assertEqual(observed['libraries'], [])
        self.assertEqual(observed['os_shared_cache_names'], ['/usr/lib/libSystem.B.dylib'])
        def os_only(path, flag):
            return str(path) + ':\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)\n' if flag == '-L' else ''
        with patch.object(e, '_otool', side_effect=os_only), patch.object(e.subprocess, 'check_output', return_value='os-build\n'):
            observed = self.system_observer(native)
        self.assertEqual(observed, {'libraries': [], 'os_shared_cache_names': ['/usr/lib/libSystem.B.dylib'],
                                    'macos_build': 'os-build'})


if __name__ == '__main__':
    unittest.main()

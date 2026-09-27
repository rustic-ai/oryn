"""Relay custody checks use fake Mach-O bytes; no relay is launched."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import fixture_relay as relay


class FixtureRelay(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.store = Path(self.temp.name).resolve() / 'store'
        self.path = 'environments/relay/payload/bin/socat'
        self.binary = self.store / self.path
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b'\xcf\xfa\xed\xfe' + b'never executed')
        self.binary.chmod(0o555)
        self.system = {'libraries': [{'selected_path': '/opt/homebrew/opt/example/lib/a.dylib',
            'resolved_path': '/opt/homebrew/Cellar/example/1/lib/a.dylib',
            'sha256': 'a' * 64, 'size': 44, 'executable': False}],
            'os_shared_cache_names': ['/usr/lib/libSystem.B.dylib'], 'macos_build': 'test-build'}
        self.observe = self.enterContext(patch.object(relay.environments, '_system_libraries',
                                                     return_value=copy.deepcopy(self.system)))

    def prepare(self):
        return relay.prepare_relay_identity(self.store, executable_path=self.path,
            identity_path='environments/relay/identity.json')

    def test_prepare_verify_and_relocate_exact_payload(self):
        result = self.prepare()
        self.assertEqual(set(result['assets']), {'fixture_relay', 'fixture_relay_environment',
                                                'fixture_relay_identity'})
        identity = result['identity']
        self.assertEqual(identity, json.loads((self.store / 'environments/relay/identity.json').read_text()))
        moved = self.store.parent / 'relocated'
        shutil.copytree(self.store, moved)
        self.assertTrue(relay.verify_relay(identity, moved)['valid'])
        self.observe.assert_called_with(moved / 'environments/relay/payload',
                                       native_paths=[moved / self.path])

    def test_homebrew_resolution_bytes_and_os_build_drift_rejected(self):
        identity = self.prepare()['identity']
        changes = [({'macos_build': 'other'}, None),
                   ({'os_shared_cache_names': []}, None),
                   ({'resolved_path': '/opt/homebrew/Cellar/example/2/lib/a.dylib'}, 0),
                   ({'sha256': 'b' * 64}, 0), ({'executable': True}, 0)]
        for change, library in changes:
            with self.subTest(change=change):
                observed = copy.deepcopy(self.system)
                (observed if library is None else observed['libraries'][library]).update(change)
                self.observe.return_value = observed
                with self.assertRaisesRegex(ValueError, 'dependency identity changed'):
                    relay.verify_relay(identity, self.store)

    def test_executable_mode_bytes_and_payload_additions_rejected(self):
        identity = self.prepare()['identity']
        self.binary.chmod(0o444)
        with self.assertRaises(ValueError):
            relay.verify_relay(identity, self.store)
        self.binary.chmod(0o555)
        added = self.binary.parent / 'undeclared.dylib'
        added.write_bytes(b'extra')
        with self.assertRaises(ValueError):
            relay.verify_relay(identity, self.store)
        added.unlink()
        self.binary.chmod(0o755)
        self.binary.write_bytes(b'changed')
        with self.assertRaises(ValueError):
            relay.verify_relay(identity, self.store)

    def test_symlink_and_non_macho_not_admitted(self):
        original = self.binary.parent.parent.parent / 'original'
        self.binary.rename(original)
        self.binary.symlink_to(original)
        with self.assertRaises(ValueError):
            self.prepare()
        self.binary.unlink()
        self.binary.write_bytes(b'#!/bin/sh\n')
        self.binary.chmod(0o555)
        with self.assertRaisesRegex(ValueError, 'Mach-O'):
            self.prepare()

    def test_identity_cannot_enter_payload_or_overwrite(self):
        with self.assertRaisesRegex(ValueError, 'outside its payload'):
            relay.prepare_relay_identity(self.store, executable_path=self.path,
                identity_path='environments/relay/payload/identity.json')
        self.prepare()
        with self.assertRaises(ValueError):
            self.prepare()

    def test_library_walk_detects_concurrent_payload_change(self):
        identity = self.prepare()['identity']
        def mutate(*args, **kwargs):
            (self.binary.parent / 'surprise').write_bytes(b'changed during inspection')
            return self.system
        self.observe.side_effect = mutate
        with self.assertRaises(ValueError):
            relay.verify_relay(identity, self.store)

    def test_strict_asset_and_identity_shape(self):
        identity = self.prepare()['identity']
        for mutate in (lambda x: x.update(extra=True),
                       lambda x: x.update(schema_version=True),
                       lambda x: x.update(tool='other'),
                       lambda x: x['executable'].update(scope='bundle')):
            bad = copy.deepcopy(identity)
            mutate(bad)
            with self.assertRaises(ValueError):
                relay.verify_relay(bad, self.store)


if __name__ == '__main__':
    unittest.main()

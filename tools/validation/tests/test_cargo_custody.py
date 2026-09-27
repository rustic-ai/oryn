"""Registry-cache formats and rejection checks without invoking Cargo."""
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import cargo_custody as subject


class RegistryCustodyTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.root = self.home / 'registry/src/example/demo-1.0.0'; self.root.mkdir(parents=True)
        self.archive = self.home / 'registry/cache/example/demo-1.0.0.crate'
        self.archive.parent.mkdir(parents=True)
        self.package = {'name': 'demo', 'version': '1.0.0', 'manifest_path': str(self.root / 'Cargo.toml')}
        self.make()

    def make(self, mode=0o644, members=None):
        members = members or [('Cargo.toml', b'[package]\n', mode), ('src/lib.rs', b'// source\n', 0o644)]
        self.files = {}
        with tarfile.open(self.archive, 'w:gz') as output:
            for name, data, permissions in members:
                member = tarfile.TarInfo('demo-1.0.0/' + name); member.size = len(data); member.mode = permissions
                output.addfile(member, io.BytesIO(data))
                path = self.root / name; path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data); path.chmod(permissions & 0o777 & ~0o022)
                self.files[name] = hashlib.sha256(data).hexdigest()
        self.locked = {'checksum': subject._hash(self.archive)}
        (self.root / '.cargo-ok').write_text('{"v":1}')

    def observe(self):
        return subject._registry(self.package, self.locked, self.home)

    def metadata(self):
        (self.root / '.cargo-checksum.json').write_text(json.dumps({'package': self.locked['checksum'], 'files': self.files}))

    def test_normal_cache_without_vendor_checksum_and_with_valid_checksum(self):
        missing = self.observe(); self.assertEqual(missing, self.observe())
        self.metadata(); present = self.observe()
        self.assertEqual(missing['files_sha256'], present['files_sha256'])
        self.assertNotEqual(missing['observed_state_sha256'], present['observed_state_sha256'])
        (self.root / '.cargo-ok').write_text('{"v":2}')
        self.assertNotEqual(present['observed_state_sha256'], self.observe()['observed_state_sha256'])

    def test_invalid_present_checksum_is_never_ignored(self):
        self.metadata(); path = self.root / '.cargo-checksum.json'
        value = json.loads(path.read_text()); value['files']['Cargo.toml'] = '0' * 64
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'checksum metadata'): self.observe()
        path.write_text('{"package":"x","package":"y","files":{}}')
        with self.assertRaisesRegex(ValueError, 'repeats'): self.observe()

    def test_masked_modes_are_explicit_but_executable_and_special_bits_rejected(self):
        self.make(mode=0o100664); self.observe()
        path = self.root / 'Cargo.toml'; path.chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'mode differs'): self.observe()
        self.make(mode=0o4755)
        with self.assertRaisesRegex(ValueError, 'unsupported source mode'): self.observe()

    def test_extra_empty_directory_and_nonregular_metadata_rejected(self):
        extra = self.root / 'extra'; extra.mkdir()
        with self.assertRaisesRegex(ValueError, 'population'): self.observe()
        extra.rmdir(); (self.root / '.cargo-checksum.json').symlink_to('Cargo.toml')
        with self.assertRaisesRegex(ValueError, 'nonregular'): self.observe()
        (self.root / '.cargo-checksum.json').unlink(); os.mkfifo(self.root / 'pipe')
        with self.assertRaisesRegex(ValueError, 'nonregular'): self.observe()

    def test_checksum_creation_during_audit_rejected(self):
        original = subject._hash
        def create(path):
            result = original(path)
            if Path(path).name == 'lib.rs': self.metadata()
            return result
        with patch.object(subject, '_hash', side_effect=create):
            with self.assertRaisesRegex(ValueError, 'changed during'): self.observe()

    def test_dirty_bytes_and_same_size_restored_mtime_rejected(self):
        target = self.root / 'src/lib.rs'; before = target.stat()
        target.write_bytes(b'X' * before.st_size)
        os.utime(target, ns=(before.st_atime_ns, before.st_mtime_ns))
        with self.assertRaisesRegex(ValueError, 'differs from locked archive'): self.observe()

    def test_noncanonical_and_duplicate_archive_members_rejected(self):
        for name in ('src/../lib.rs', 'src//lib.rs'):
            with tarfile.open(self.archive, 'w:gz') as output:
                member = tarfile.TarInfo('demo-1.0.0/' + name); member.size = 1
                output.addfile(member, io.BytesIO(b'x'))
            self.locked['checksum'] = subject._hash(self.archive)
            with self.assertRaisesRegex(ValueError, 'escapes'): self.observe()
        self.make(members=[('Cargo.toml', b'x', 0o644), ('Cargo.toml', b'x', 0o644)])
        with self.assertRaisesRegex(ValueError, 'duplicates'): self.observe()


if __name__ == '__main__':
    unittest.main()

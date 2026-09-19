"""Source archive regressions; no build, Git invocation or qualification."""
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from completion_source_snapshot import inventory, source_digest, write, validate, INDEX


class SourceSnapshotTests(unittest.TestCase):
    def test_reproducible_capture_and_wrong_source_rejection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'a.rs').write_text('generated native source')
            (root / 'b.py').write_text('generated regression')
            records = inventory(root, ['b.py', 'a.rs']); identity = source_digest(records)
            first = root / 'one.tar'; second = root / 'two.tar'
            write(first, root, records); write(second, root, records)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            self.assertEqual(validate(first, identity)['files'], records)
            with self.assertRaisesRegex(ValueError, 'exact source-tree'):
                validate(first, 'a' * 64)
            (root / 'a.rs').write_text('changed after source freeze')
            with self.assertRaisesRegex(ValueError, 'changed while capturing'):
                write(root / 'changed.tar', root, records)

    def test_unsafe_archive_links_cannot_be_a_source_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'unsafe.tar'
            with tarfile.open(path, 'w') as archive:
                member = tarfile.TarInfo(INDEX); member.type = tarfile.SYMTYPE; member.linkname = '/etc/passwd'
                archive.addfile(member)
            with self.assertRaisesRegex(ValueError, 'unsafe entry'):
                validate(path, 'a' * 64)

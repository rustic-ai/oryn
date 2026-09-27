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

    def test_valid_archive_reuses_tarinfo_without_name_lookups(self):
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for number in range(128):
                (root / ('source-%03d.rs' % number)).write_text('native source %d' % number)
            records = inventory(root, [path.name for path in root.iterdir()])
            archive = root / 'sources.tar'
            expected = write(archive, root, records)
            # This rejects the old quadratic lookup path deterministically,
            # without a wall-time assertion or suppressing actual file hashes.
            with patch.object(tarfile.TarFile, 'getmember', side_effect=AssertionError('name lookup')):
                self.assertEqual(validate(archive, source_digest(records)), expected)

    def fixture_archive(self, path, records, members, *, index_first=False, duplicate_index=False):
        index = dict(schema_version=1, source_tree_sha256=source_digest(records), files=records)
        encoded = json.dumps(index).encode()
        def add_index(archive):
            info = tarfile.TarInfo(INDEX)
            info.size = len(encoded)
            archive.addfile(info, io.BytesIO(encoded))
        with tarfile.open(path, 'w') as archive:
            if index_first:
                add_index(archive)
            for name, payload, mode, kind in members:
                info = tarfile.TarInfo(name)
                info.mode = mode
                info.type = kind
                info.size = len(payload) if kind == tarfile.REGTYPE else 0
                archive.addfile(info, io.BytesIO(payload) if info.size else None)
            if not index_first:
                add_index(archive)
            if duplicate_index:
                add_index(archive)

    def test_archive_content_size_and_mode_checks_survive_linear_lookup(self):
        records = [dict(path='a.rs', size=4, mode=0o644, sha256=hashlib.sha256(b'good').hexdigest())]
        for payload, mode, message in [(b'evil', 0o644, 'bytes changed'),
                                       (b'longer', 0o644, 'metadata changed'),
                                       (b'good', 0o755, 'metadata changed')]:
            with self.subTest(payload=payload, mode=mode), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'changed.tar'
                self.fixture_archive(path, records, [('a.rs', payload, mode, tarfile.REGTYPE)])
                with self.assertRaisesRegex(ValueError, message):
                    validate(path, source_digest(records))

    def test_archive_duplicate_and_index_position_checks_survive_linear_lookup(self):
        records = [dict(path='a.rs', size=4, mode=0o644, sha256=hashlib.sha256(b'good').hexdigest())]
        member = ('a.rs', b'good', 0o644, tarfile.REGTYPE)
        for members, options in [([member, member], {}), ([member], {'duplicate_index': True}),
                                 ([member], {'index_first': True})]:
            with self.subTest(members=members, options=options), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'changed.tar'
                self.fixture_archive(path, records, members, **options)
                with self.assertRaises(ValueError):
                    validate(path, source_digest(records))

    def test_archive_member_and_index_order_must_match_sorted_identity(self):
        records = [dict(path=name, size=4, mode=0o644, sha256=hashlib.sha256(b'good').hexdigest())
                   for name in ['a.rs', 'b.rs']]
        members = [(record['path'], b'good', 0o644, tarfile.REGTYPE) for record in records]
        for selected_records, selected_members in [(records, list(reversed(members))),
                                                    (list(reversed(records)), list(reversed(members)))]:
            with self.subTest(records=selected_records), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'changed.tar'
                self.fixture_archive(path, selected_records, selected_members)
                with self.assertRaisesRegex(ValueError, 'exact source-tree'):
                    validate(path, source_digest(selected_records))

    def test_archive_unsafe_path_and_nonfile_type_are_still_rejected(self):
        for name, kind in [('../escape.rs', tarfile.REGTYPE), ('/absolute.rs', tarfile.REGTYPE),
                           ('a.rs', tarfile.DIRTYPE), ('a.rs', tarfile.LNKTYPE),
                           ('a.rs', tarfile.SYMTYPE)]:
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as directory:
                records = [dict(path=name, size=0, mode=0o644, sha256=hashlib.sha256(b'').hexdigest())]
                path = Path(directory) / 'changed.tar'
                self.fixture_archive(path, records, [(name, b'', 0o644, kind)])
                with self.assertRaisesRegex(ValueError, 'unsafe entry'):
                    validate(path, source_digest(records))

"""Real private log creation/append, with no native processes or sockets."""
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation.admission import legacy_algorithms
legacy_algorithms()
from oryn_wptrunner import custody
from wpt_execution_evidence import protocol_custody


class PrivateProtocolCustody(unittest.TestCase):
    def test_canonical_creation_survives_umask_and_real_begin_teardown(self):
        for mask in (0o000, 0o022, 0o077, 0o777):
            with self.subTest(umask=oct(mask)), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'custody.jsonl'
                before = os.umask(mask)
                try:
                    custody.create(path)
                finally:
                    os.umask(before)
                metadata = path.stat()
                self.assertEqual(stat.S_IMODE(metadata.st_mode), 0o600)
                self.assertEqual(metadata.st_uid, os.getuid())
                self.assertEqual(metadata.st_nlink, 1)
                with patch.dict(os.environ, ORYN_WPT_PROTOCOL_CUSTODY=str(path),
                                ORYN_WPT_CANDIDATE_SHA256='a' * 64):
                    session = custody.begin()
                    custody.append(session, 'teardown')
                self.assertEqual(protocol_custody(path, 'a' * 64), 1)
                self.assertEqual([json.loads(line)['phase'] for line in path.read_text().splitlines()],
                                 ['connect', 'teardown'])

    def test_existing_destinations_are_never_repaired_or_overwritten(self):
        for kind in ('file', 'symlink', 'hardlink'):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary); target = root / 'target'; target.write_text('preserve')
                target.chmod(0o644)
                path = root / 'custody'
                if kind == 'file': path.write_text('preserve')
                elif kind == 'symlink': path.symlink_to(target)
                else: os.link(target, path)
                with self.assertRaises(FileExistsError): custody.create(path)
                self.assertEqual(path.read_text(), 'preserve')
                self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o644)

    def test_writer_rejects_mode_link_and_owner_changes_without_appending(self):
        for change in ('mode', 'hardlink', 'symlink', 'owner'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'custody'; custody.create(path)
                if change == 'mode': path.chmod(0o644)
                if change == 'hardlink': os.link(path, path.with_name('alias'))
                if change == 'symlink':
                    target = path.with_name('target'); path.rename(target); path.symlink_to(target)
                with patch.dict(os.environ, ORYN_WPT_PROTOCOL_CUSTODY=str(path),
                                ORYN_WPT_CANDIDATE_SHA256='a' * 64):
                    if change == 'owner':
                        with patch.object(custody.os, 'getuid', return_value=os.getuid() + 1):
                            with self.assertRaises(ValueError): custody.begin()
                    else:
                        with self.assertRaises((OSError, ValueError)): custody.begin()
                self.assertEqual(path.read_bytes(), b'')

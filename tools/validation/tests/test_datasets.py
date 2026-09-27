"""Tiny real Git repositories exercise dataset exports; never export upstream here."""
import base64
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from oryn_validation import datasets
from oryn_validation.inputs import snapshot_input
from oryn_validation.production import validate_source_export


class DatasetExports(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source, self.store = self.root / 'source', self.root / 'store'
        self.source.mkdir(); self.store.mkdir()
        self.git('init', '-q')
        self.link = 'tools/third_party/websockets/docs/_static/favicon.ico'
        self.target = 'tools/third_party/websockets/logo/favicon.ico'
        self.write(self.target, b'image')
        self.write('tests/example.html', b'<html></html>')
        self.write('tool.sh', b'#!/bin/sh\ntrue\n', executable=True)
        (self.source / self.link).parent.mkdir(parents=True)
        (self.source / self.link).symlink_to('../../logo/favicon.ico')
        self.commit()

    def git(self, *argv):
        return subprocess.check_output(['git', '-C', str(self.source), *argv], stderr=subprocess.STDOUT)

    def write(self, name, data, executable=False):
        path = self.source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        path.chmod(0o755 if executable else 0o644)

    def commit(self):
        self.git('add', '-A')
        self.git('-c', 'user.name=Dataset Test', '-c', 'user.email=dataset@example.invalid',
                 '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
        self.commit_id = self.git('rev-parse', 'HEAD').decode().strip()

    def export(self):
        result = datasets.export_dataset(self.source, self.store, export_path='upstream/wpt',
                                          identity_path='identities/wpt.json', expected_commit=self.commit_id)
        self.exported = self.store / 'upstream/wpt'
        self.identity = json.loads((self.store / 'identities/wpt.json').read_text())
        self.membership = result['wpt_checkout']
        return result

    def fresh_membership(self):
        return snapshot_input(self.store, 'upstream/wpt', kind='directory')

    def verify(self, membership=None, identity=None):
        return datasets.verify_dataset(identity or self.identity, self.exported,
                                       membership or self.membership, expected_commit=self.commit_id)

    def test_export_preserves_git_proof_and_normalizes_only_declared_link(self):
        result = self.export()
        proof = self.verify()
        self.assertEqual(result['normalizations'], 1)
        self.assertEqual(proof['commit'], self.commit_id)
        self.assertEqual(result['files'], 4)
        self.assertFalse((self.exported / '.git').exists())
        self.assertFalse((self.exported / self.link).is_symlink())
        self.assertTrue((self.source / self.link).is_symlink())
        self.assertEqual((self.exported / self.link).read_bytes(), b'image')
        self.assertTrue(proof['tracked']['tool.sh']['executable'])
        self.assertEqual(self.git('status', '--porcelain').decode(), '')
        self.assertEqual(proof['tracked'][self.link], proof['tracked'][self.target])

    def test_production_default_rejects_unreviewed_revision(self):
        self.export()
        with self.assertRaisesRegex(ValueError, 'pinned'):
            datasets.verify_dataset(self.identity, self.exported, self.membership)

    def test_same_size_mutation_rejected_even_with_updated_membership(self):
        self.export()
        path = self.exported / 'tests/example.html'
        path.chmod(0o644); path.write_bytes(b'<hxml></html>')
        with self.assertRaisesRegex(ValueError, 'bytes, mode or membership'):
            self.verify()
        with self.assertRaisesRegex(ValueError, 'blob differs'):
            self.verify(self.fresh_membership())

    def test_normalized_link_cannot_substitute_new_bytes(self):
        self.export()
        path = self.exported / self.link
        path.chmod(0o644); path.write_bytes(b'other')
        identity = copy.deepcopy(self.identity)
        identity['normalizations'][0].update(datasets.builds._identity(path))
        with self.assertRaisesRegex(ValueError, 'materialized'):
            self.verify(self.fresh_membership(), identity)

    def test_normalization_original_blob_is_proven_by_git(self):
        self.export()
        identity = copy.deepcopy(self.identity)
        identity['normalizations'][0]['blob'] = base64.b64encode(b'elsewhere').decode()
        with self.assertRaisesRegex(ValueError, 'link blob'):
            self.verify(identity=identity)
        identity = copy.deepcopy(self.identity)
        identity['normalizations'] = []
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.verify(identity=identity)

    def test_executable_drift_rejected_even_with_fresh_membership(self):
        self.export()
        (self.exported / 'tool.sh').chmod(0o444)
        with self.assertRaisesRegex(ValueError, 'executable mode'):
            self.verify(self.fresh_membership())

    def test_extra_file_rejected_even_with_fresh_membership(self):
        self.export()
        self.exported.chmod(0o755)
        (self.exported / 'extra').write_text('undeclared')
        with self.assertRaisesRegex(ValueError, 'membership'):
            self.verify(self.fresh_membership())

    def test_test_symlink_is_not_an_approved_normalization(self):
        (self.source / 'tests/link.html').symlink_to('example.html')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            self.export()
        self.assertFalse((self.store / 'upstream/wpt').exists())

    def test_escape_and_missing_target_rejected_before_export(self):
        (self.source / self.link).unlink()
        (self.source / self.link).symlink_to('../../../../../../outside')
        self.commit()
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            self.export()
        (self.source / self.link).unlink()
        (self.source / self.link).symlink_to('../../logo/favicon.ico')
        (self.source / self.target).unlink()
        self.commit()
        with self.assertRaisesRegex(ValueError, 'missing'):
            self.export()

    def test_dirty_source_rejected_without_copying(self):
        (self.source / 'tool.sh').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'clean'):
            self.export()
        self.assertFalse((self.store / 'upstream/wpt').exists())

    def test_native_source_still_rejects_normalized_git_symlink(self):
        self.export()
        identity = {key: self.identity[key] for key in
                    ('schema_version', 'commit', 'commit_object', 'trees')}
        identity['kind'] = 'git-source-export'
        with self.assertRaisesRegex(ValueError, 'symlink'):
            validate_source_export(identity, self.exported, self.membership)

    def test_changed_declared_materialized_mode_rejected(self):
        self.export()
        identity = copy.deepcopy(self.identity)
        identity['normalizations'][0]['executable'] = True
        with self.assertRaisesRegex(ValueError, 'executable mode'):
            self.verify(identity=identity)


if __name__ == '__main__':
    unittest.main()

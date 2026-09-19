import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from oryn_wptrunner import fixture_repairs


class FixtureRepairTests(unittest.TestCase):
    def test_exact_declared_paths_and_both_source_hashes_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'test.html').write_text('<script src="missing.js"></script>')
            (root / 'actual.js').write_text('original helper bytes')
            manifest = root / 'repairs.json'
            manifest.write_text(json.dumps({'schema_version': 1, 'upstream_revision': fixture_repairs.WPT_REVISION, 'repairs': [
                {'method': 'GET', 'request_path': '/missing.js', 'source_path': 'test.html', 'source_sha256': fixture_repairs.digest(root / 'test.html'),
                 'target_path': 'actual.js', 'target_sha256': fixture_repairs.digest(root / 'actual.js')}]}))
            with patch.object(fixture_repairs, 'MANIFEST', manifest):
                self.assertEqual(fixture_repairs.validate(manifest, fixture_repairs.digest(manifest), root), [('GET', '/missing.js', '/actual.js')])
                (root / 'actual.js').write_text('altered assertion')
                with self.assertRaisesRegex(ValueError, 'bytes changed'):
                    fixture_repairs.validate(manifest, fixture_repairs.digest(manifest), root)
                (root / 'actual.js').write_text('original helper bytes')
                (root / 'missing.js').write_text('existing upstream file')
                with self.assertRaisesRegex(ValueError, 'cannot override'):
                    fixture_repairs.validate(manifest, fixture_repairs.digest(manifest), root)

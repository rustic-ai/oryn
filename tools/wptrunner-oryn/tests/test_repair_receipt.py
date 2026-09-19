import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from oryn_wptrunner import repair_receipt as receipts


class ReceiptTests(unittest.TestCase):
    def test_actual_bytes_and_failures_are_retained_without_success_fabrication(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({'repairs': [{'request_path': '/a', 'target_sha256': hashlib.sha256(b'actual').hexdigest()}]}))
            candidate = root / 'candidate.json'
            candidate.write_text(json.dumps({'artifacts': {'fixture_repairs': {'path': str(manifest), 'sha256': receipts.digest(manifest)}}}))
            class Response:
                status = 200
                def __enter__(self): return self
                def __exit__(self, *_): pass
                def read(self, limit): return b'actual'
                def geturl(self): return 'https://web-platform.test/a'
            with patch.object(receipts, 'MANIFEST', manifest), patch.object(receipts, 'validate'):
                value = receipts.collect(candidate, root, 'https://web-platform.test', lambda *_args, **_kwargs: Response())
                self.assertEqual(value['candidate_sha256'], receipts.digest(candidate))
                self.assertEqual(value['requests'][0]['target_sha256'], hashlib.sha256(b'actual').hexdigest())
                self.assertNotIn('error', value['requests'][0])
                def fail(*_args, **_kwargs): raise OSError('fixture unavailable')
                value = receipts.collect(candidate, root, 'https://web-platform.test', fail)
                self.assertEqual(value['requests'][0]['error'], 'fixture unavailable')
                self.assertNotIn('status', value['requests'][0])
                with self.assertRaisesRegex(ValueError, 'explicit HTTP'):
                    receipts.collect(candidate, root, 'https://web-platform.test/path', fail)

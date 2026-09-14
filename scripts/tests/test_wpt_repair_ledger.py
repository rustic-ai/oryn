import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
spec = importlib.util.spec_from_file_location("repair", Path(__file__).resolve().parents[1] / "wpt-repair-evidence.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class LedgerTests(unittest.TestCase):
    def test_source_hashes_and_duplicate_ids_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'evidence.json'
            path.write_text(json.dumps({'assertions':[{'id':'a|window|?one','status':'failed'}],
                'run_fingerprint':'version-one','binary':{'sha256':'parent'},
                'runtime_metadata':{'worker_binary_sha256':'worker'}}))
            source = {'path':str(path),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
            rows, origins = module.load_sources({'evidence_sources':[source]})
            self.assertEqual(list(rows), ['a|window|?one'])
            self.assertEqual(origins['a|window|?one']['run_fingerprint'], 'version-one')
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                module.load_sources({'evidence_sources':[source,source]})
            path.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                module.load_sources({'evidence_sources':[source]})

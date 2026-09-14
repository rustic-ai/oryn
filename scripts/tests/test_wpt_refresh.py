import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('refresh', Path(__file__).resolve().parents[1] / 'refresh-wpt-composite.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RefreshTests(unittest.TestCase):
    def test_exact_ids_identity_completion_and_history(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            origin = {'path': 'original', 'sha256': 'old', 'binary': {'sha256': 'parent'},
                      'worker_sha256': 'worker', 'run_fingerprint': 'old-run'}
            ledger = root / 'ledger.jsonl'
            ledger.write_text(json.dumps({'id': 'a|window|', 'status': 'failed', 'original': origin}) + '\n' +
                              json.dumps({'id': 'b|window|', 'status': 'passed', 'original': origin}) + '\n')
            (root / 'summary.json').write_text(json.dumps({'complete': True, 'selected': 2,
                                                          'ledger_sha256': module.digest(ledger)}))
            inventory = root / 'inventory.json'
            inventory.write_text(json.dumps({'tests': [{'path': 'a', 'globals': ['window'], 'variants': ['']}]}))
            replacement = root / 'new.json'
            data = {'inventory_sha256': module.digest(inventory),
                    'assertions': [{'id': 'a|window|', 'status': 'failed'}],
                    'outcome': {'status': 'failed', 'required_finished': 1, 'required_total': 1},
                    'binary': origin['binary'], 'runtime_metadata': {'worker_binary_sha256': 'worker'},
                    'run_fingerprint': 'new-run'}
            replacement.write_text(json.dumps(data))
            entries, summary = module.refresh(root, replacement, inventory)
            self.assertEqual(summary['selected'], 2)
            self.assertEqual(entries[0]['superseded_sources'], [origin])
            self.assertEqual(entries[1]['original'], origin)
            self.assertNotIn('replacement', entries[1])
            for patch, message in [
                ({'inventory_sha256': 'stale'}, 'inventory/ID mismatch'),
                ({'assertions': [{'id': 'absent', 'status': 'failed'}]}, 'inventory/ID mismatch'),
                ({'assertions': data['assertions'] * 2}, 'inventory/ID mismatch'),
                ({'outcome': {}}, 'incomplete replacement'),
                ({'binary': {'sha256': 'different'}}, 'binary changed'),
            ]:
                replacement.write_text(json.dumps(dict(data, **patch)))
                with self.assertRaisesRegex(ValueError, message):
                    module.refresh(root, replacement, inventory)

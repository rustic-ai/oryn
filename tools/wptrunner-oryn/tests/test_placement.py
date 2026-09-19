import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from oryn_wptrunner.placement import capture, TEST


class PlacementCaptureTests(unittest.TestCase):
    def test_capture_uses_authenticated_session_diagnostic_and_never_supplies_ids(self):
        calls = []
        class Session:
            def send_session_command(self, method, path, body):
                calls.append((method, path, body))
                return {'schema': 1, 'pages': [], 'agents': []}
        with tempfile.TemporaryDirectory() as folder:
            config = {'schema': 1, 'test': TEST, 'nonce': 'a' * 64,
                      'candidate_sha256': 'b' * 64, 'run_contract_sha256': 'c' * 64,
                      'output': str(Path(folder) / 'placement.json')}
            executor = SimpleNamespace(protocol=SimpleNamespace(webdriver=Session()))
            capture(executor, SimpleNamespace(url='/other-test.html'), config)
            self.assertFalse(Path(config['output']).exists())
            capture(executor, SimpleNamespace(url=TEST), config)
            self.assertEqual(calls, [('POST', 'oryn/diagnostics/placement', {'nonce': 'a' * 64})])
            self.assertEqual(json.loads(Path(config['output']).read_text())['snapshot']['pages'], [])
            with self.assertRaises(FileExistsError):
                capture(executor, SimpleNamespace(url=TEST), config)

    def test_diagnostic_failure_is_preserved_as_unqualified_evidence(self):
        class Session:
            def send_session_command(self, *args):
                raise RuntimeError('native diagnostic unavailable')
        with tempfile.TemporaryDirectory() as folder:
            config = {'schema': 1, 'test': TEST, 'nonce': 'a' * 64, 'candidate_sha256': 'b' * 64,
                      'run_contract_sha256': 'c' * 64, 'output': str(Path(folder) / 'placement.json')}
            capture(SimpleNamespace(protocol=SimpleNamespace(webdriver=Session())), SimpleNamespace(url=TEST), config)
            record = json.loads(Path(config['output']).read_text())
            self.assertIn('unavailable', record['error'])
            self.assertNotIn('snapshot', record)

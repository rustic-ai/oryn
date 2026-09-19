"""New input locks preserve the original behavior contract and reject drift."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import workflow_input_lock as locks
from workflow_contract import DEFAULT_CONTRACT


class WorkflowInputLockTests(unittest.TestCase):
    def test_official_launch_and_ledger_entrypoints_are_frozen_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); (root / 'scripts').mkdir()
            names = ['run-official-wpt.py', 'reconcile-full-wpt-qualification.py', 'merge-wpt-qualification-ledger.py', 'workflow_raw_evidence.py']
            for name in names:
                (root / 'scripts' / name).write_text('original entrypoint')
            fixture_dir = root / 'benchmarks/app-compatibility'
            fixture_dir.mkdir(parents=True)
            fixture_names = ['fixtures.lock.json', 'fixtures-v1.lock.json', 'fixtures-v2.lock.json']
            for name in fixture_names:
                (fixture_dir / name).write_text('original fixture lock')
            expected = {'scripts/' + name for name in names} | {
                'benchmarks/app-compatibility/' + name for name in fixture_names}
            with patch.object(locks, 'baseline', return_value={'regression_inputs': []}):
                snapshot = locks.snapshot(root, 'a' * 64)
                self.assertEqual({record['path'] for record in snapshot['inputs']}, expected)
                path = root / 'lock.json'; path.write_text(json.dumps(snapshot))
                for name in sorted(expected):
                    source = root / name
                    original = source.read_bytes()
                    source.write_text('changed frozen input')
                    with self.subTest(path=name), self.assertRaisesRegex(ValueError, 'stale'):
                        locks.load(path, 'a' * 64, root)
                    source.write_bytes(original)

    def test_refresh_changes_only_input_identity_and_rejects_stale_bytes(self):
        original = json.loads(DEFAULT_CONTRACT.read_text())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / 'frontend.py'; source.write_text('new implementation')
            with patch.object(locks, 'baseline', return_value=original), patch.object(locks, 'input_paths', return_value=['frontend.py']):
                snapshot = locks.snapshot(root, 'a' * 64)
                path = root / 'workflow-v2.json'; path.write_text(json.dumps(snapshot))
                contract = locks.load(path, 'a' * 64, root)
                for field in original:
                    if field != 'regression_inputs': self.assertEqual(contract[field], original[field])
                self.assertEqual(len(contract['regression_inputs']), 1)
                source.write_text('substituted frontend')
                with self.assertRaisesRegex(ValueError, 'stale'):
                    locks.load(path, 'a' * 64, root)
                with self.assertRaisesRegex(ValueError, 'identity mismatch'):
                    locks.load(path, 'b' * 64, root)

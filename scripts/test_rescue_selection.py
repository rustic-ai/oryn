"""The rescue subset must preserve originals and never satisfy the full denominator."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import unittest

from workflow_contract import DEFAULT_CONTRACT, planned_attempts, select_attempts
from workflow_evidence import reconcile
from workflow_execution import execute_contract

spec = importlib.util.spec_from_file_location('rescue_runner', Path(__file__).with_name('run-oil-rescue.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class RescueSelectionTests(unittest.TestCase):
    def setUp(self):
        self.contract = json.loads(DEFAULT_CONTRACT.read_text())
        self.plan = planned_attempts(self.contract)
        self.candidate = dict(parent_sha256='a'*64, worker_sha256='b'*64, source_tree_sha256='c'*64)

    def test_exact_approved_matrix_preserves_every_original_specification(self):
        before = copy.deepcopy(self.plan)
        selected = runner.rescue_plan(self.contract)
        self.assertEqual(len(selected), 132)
        self.assertEqual(sum(item['mode'] == 'serial' for item in selected), 87)
        self.assertEqual(sum(item['mode'] == 'mixed_family' for item in selected), 45)
        self.assertEqual(self.plan, before)
        self.assertTrue(all(item in self.plan for item in selected))
        self.assertEqual({(item['cell'], item['sample']) for item in selected if item['mode'] == 'serial'},
                         {(item['cell'], sample) for item in self.plan for sample in (1, 2, 3)})

    def test_selection_rejects_unknown_duplicate_empty_and_reordered_specs(self):
        identity = self.plan[0]['attempt_id']
        for invalid in ([], [identity, identity], ['invented'], [self.plan[0]], identity):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                select_attempts(self.plan, invalid)
        ids = [self.plan[2]['attempt_id'], identity]
        self.assertEqual(select_attempts(self.plan, ids), [self.plan[0], self.plan[2]])
        self.assertEqual(len(select_attempts(self.plan)), 688)

    def test_cancelled_focused_execution_keeps_all_132_required_originals(self):
        ids = [item['attempt_id'] for item in runner.rescue_plan(self.contract)]
        stop = threading.Event()
        stop.set()
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / 'run'
            summary = execute_contract(self.contract, self.candidate, {}, out, {},
                                       stop=stop, attempt_ids=ids)
            records = [json.loads(line) for line in (out/'attempts.jsonl').read_text().splitlines()]
            self.assertEqual({item['attempt_id'] for item in records}, set(ids))
            self.assertTrue(all(item['disposition'] == 'not_executed' for item in records))
            self.assertEqual(summary['planned'], 132)
            self.assertEqual(summary['scope'], 'focused_contract_execution')
            self.assertFalse(summary['family_qualified'])
            self.assertFalse(summary['structurally_valid'])
            # Omitting selection cannot turn a focused run into full coverage.
            full = reconcile(self.contract, self.candidate, records, out, {})
            self.assertEqual(full['planned'], 688)
            self.assertEqual(len(full['missing_attempts']), 556)
            self.assertFalse(full['structurally_valid'])


if __name__ == '__main__':
    unittest.main()

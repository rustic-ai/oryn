import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import sys
from unittest import mock
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

spec = importlib.util.spec_from_file_location('cohort', Path(__file__).resolve().parents[1] / 'run-full-wpt-baseline.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class CohortTests(unittest.TestCase):
    def test_final_summary_uses_terminal_outcome_not_last_checkpoint(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan_path = root / 'plan.json'
            plan_path.write_text('{}')
            terminal = {'status': 'failed', 'required_finished': 1}
            (root / 'cohort.json').write_text(json.dumps({
                'outcome': terminal, 'parent_progress': None,
                'assertions': [{'id': 'a', 'status': 'failed'}]}))
            args = SimpleNamespace(output=root, tier=None, plan=plan_path,
                                   checkout=root, binary=root / 'oryn', jobs=1)
            plan = dict(total_manifest_tests=1, runnable=1, unsupported=0,
                        official_manifest_sha256='manifest', policy_sha256='policy')
            with mock.patch.object(module, 'cohort_inventory', return_value=({}, {'a'})), \
                    mock.patch.object(module.subprocess, 'Popen') as process:
                process.return_value.wait.return_value = 1
                self.assertEqual(module.run_cohort(args, plan, []), 0)
            summary = json.loads((root / 'summary.json').read_text())
            self.assertTrue(summary['complete'])
            self.assertEqual(summary['outcome'], terminal)
            self.assertIsNone(summary['parent_progress'])

    def test_parent_failure_cannot_publish_completed_cohort(self):
        row = {'execution': {'transport': 'persistent_session'}}
        self.assertTrue(module.parent_integrity_errors([row]))
        row['execution']['parent_shutdown'] = {
            'returncode': 0, 'error': None, 'shutdown_acknowledged': True}
        self.assertEqual(module.parent_integrity_errors([row]), [])
        row['execution']['error'] = 'worker pool capacity exhausted'
        self.assertEqual(module.parent_integrity_errors([row]), ['primary_worker_pool_exhausted'])

    def test_mixed_ledger_retains_unsupported_and_does_not_qualify_missing_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            coverage = root / 'coverage.jsonl'
            coverage.write_text('\n'.join(json.dumps(row) for row in [
                {'id': 'reftest:/a.html', 'status': 'unsupported', 'root': 'html', 'tier': 'core'},
                {'id': 'testharness:/b.html', 'runner_id': 'b.html|window|', 'status': 'not_run', 'root': 'dom', 'tier': 'core'}]))
            data = {'assertions': [{'id': 'b.html|window|', 'status': 'passed',
                                    'result': {'complete': True, 'harness': 0, 'tests': [{'name': 'x', 'status': 0}]}}]}
            result = module.publish_cohort_ledger(root, coverage, data, {'sha256': 'source'})
            self.assertEqual(result['qualification_counts'], {'unsupported': 1, 'unresolved': 1})
            rows = [json.loads(line) for line in (root / 'ledger.jsonl').read_text().splitlines()]
            self.assertEqual(rows[1]['status'], 'passed')
            self.assertEqual(rows[1]['evidence']['sha256'], 'source')

    def test_preserves_exact_ids_and_rejects_duplicate_or_wrong_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'inventory.json'
            path.write_text(json.dumps({'upstream_revision': 'revision', 'tests': [
                {'path': 'a.any.js', 'globals': ['window'], 'variants': ['?a']},
                {'path': 'a.any.js', 'globals': ['dedicatedworker'], 'variants': ['?b']}
            ]}))
            shard = {'inventory': str(path), 'executions': 2}
            plan = {'upstream_revision': 'revision'}
            inventory, ids = module.cohort_inventory(plan, [shard])
            self.assertEqual(ids, {'a.any.js|window|?a', 'a.any.js|dedicatedworker|?b'})
            self.assertEqual(inventory['required_execution_total'], 2)
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                module.cohort_inventory(plan, [shard, shard])
            with self.assertRaisesRegex(ValueError, 'count mismatch'):
                module.cohort_inventory(plan, [dict(shard, executions=3)])

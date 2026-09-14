import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('summary', Path(__file__).resolve().parents[1] / 'summarize-wpt-cohort.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SummaryTests(unittest.TestCase):
    def test_runtime_integrity_takes_precedence_over_assertions(self):
        row = {'status': 'passed', 'execution': {'completion_valid': True},
               'result': {'harness': 0, 'tests': [{'status': 0}]}}
        self.assertEqual(module.failure_type(row), 'raw_pass')
        row['execution']['cleanup_errors'] = ['crashed before teardown']
        self.assertEqual(module.failure_type(row), 'teardown_or_replacement_failure')
        row['execution']['error'] = 'worker exceeded generated platform operation wall time limit'
        self.assertEqual(module.failure_type(row), 'timeout_or_worker_limit')
        row['execution']['error'] = 'worker exceeded worker pool capacity limit'
        self.assertEqual(module.failure_type(row), 'harness_pool_capacity')
        row['execution']['error'] = 'sandboxed native worker exceeded RSS limit'
        self.assertEqual(module.failure_type(row), 'worker_resource_limit')

    def test_incomplete_subtests_are_not_assertion_failures(self):
        row = {'status': 'failed', 'execution': {'completion_valid': True},
               'result': {'harness': 0, 'tests': [{'status': 2}]}}
        self.assertEqual(module.failure_type(row), 'incomplete_or_invalid_completion')

    def test_unsupported_denominator_is_retained(self):
        report = module.summarize({'assertions': [{'id': 'a', 'status': 'environment_blocked'}]},
            [{'runner_id': 'a', 'root': 'dom', 'tier': 'core', 'status': 'not_run'},
             {'root': 'css', 'tier': 'extended', 'status': 'unsupported'}])
        self.assertEqual(report['manifest_entries'], 2)
        self.assertEqual(report['unsupported'], 1)
        self.assertEqual(report['by_tier']['core']['types']['environment_blocked'], 1)
        self.assertEqual(report['by_tier_root']['core']['dom']['types']['environment_blocked'], 1)

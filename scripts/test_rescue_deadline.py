"""Absolute rescue deadlines include admission and process startup time."""
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import completion_execution as execution


class RescueDeadlineTests(unittest.TestCase):
    def run_case(self, times, *, deadline=150):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        with mock.patch.object(execution, 'GenerationLease'), \
                mock.patch.object(execution.subprocess, 'Popen') as popen, \
                mock.patch.object(execution.time, 'time', side_effect=times), \
                mock.patch.object(execution, 'supervise', return_value={
                    'returncode': 0, 'error': None, 'timed_out': False,
                    'cleanup_complete': True, 'cleanup_errors': [],
                    'cancellation_signal': None, 'forced_cleanup': False, 'events': []}) as supervise:
            result = execution.run_owned(['fake-child'], Path(temporary.name), timeout=100,
                                         deadline=deadline, environment={
                                             'ORYN_GENERATION_GATE': 'fake-gate',
                                             'ORYN_GENERATION_GATE_SHA256': 'a'*64})
        return result, popen, supervise

    def test_admission_consuming_budget_prevents_any_launch(self):
        result, popen, supervise = self.run_case([151])
        popen.assert_not_called()
        supervise.assert_not_called()
        self.assertTrue(result['timed_out'])
        self.assertIn('before launch', result['error'])

    def test_child_budget_excludes_admission_and_startup_elapsed_time(self):
        result, popen, supervise = self.run_case([125, 130])
        popen.assert_called_once()
        self.assertEqual(supervise.call_args.kwargs['timeout'], 20)
        self.assertEqual(supervise.call_args.kwargs['grace'], 60)
        self.assertEqual(result['returncode'], 0)

    def test_startup_crossing_deadline_enters_immediate_owned_settlement(self):
        _, popen, supervise = self.run_case([149, 151])
        popen.assert_called_once()
        self.assertEqual(supervise.call_args.kwargs['timeout'], 0)

    def test_unspecified_deadline_keeps_existing_relative_timeout(self):
        _, _, supervise = self.run_case([], deadline=None)
        self.assertEqual(supervise.call_args.kwargs['timeout'], 100)


if __name__ == '__main__':
    unittest.main()

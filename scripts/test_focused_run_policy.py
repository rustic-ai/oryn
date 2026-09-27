"""Explicit acceptance-driven runs preserve cancellation and the historic rescue limit."""
import contextlib
import importlib.util
import io
from pathlib import Path
import threading
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('focused_runner', Path(__file__).with_name('run-oil-rescue.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class FocusedRunPolicyTests(unittest.TestCase):
    arguments = ['--generation-gate', 'gate', '--generation-gate-sha256', 'a' * 64,
                 '--binary', 'parent', '--worker', 'worker', '--signed-source-sha256', 'b' * 64,
                 '--output', 'output']

    def test_acceptance_mode_has_no_global_timer(self):
        args = runner.parse_args(self.arguments + ['--until-complete'])
        self.assertIsNone(args.deadline)
        self.assertIsNone(args.deadline_timestamp)
        stop = threading.Event()
        with mock.patch.object(runner.threading, 'Timer') as timer:
            self.assertIsNone(runner.start_deadline_timer(stop, args.deadline_timestamp))
        timer.assert_not_called()
        # The same event remains available to signal handlers and active workers.
        stop.set()
        self.assertTrue(stop.is_set())

    def test_old_rescue_default_still_rejects_after_deadline(self):
        with mock.patch.object(runner.time, 'time', return_value=2_000_000_000), \
                contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            runner.parse_args(self.arguments)

    def test_explicit_deadline_retains_timer_and_stop_callback(self):
        with mock.patch.object(runner.time, 'time', return_value=100):
            args = runner.parse_args(self.arguments + ['--deadline', '1970-01-01T00:03:20Z'])
            stop = threading.Event()
            with mock.patch.object(runner.threading, 'Timer') as timer:
                actual = runner.start_deadline_timer(stop, args.deadline_timestamp)
            timer.assert_called_once_with(100, stop.set)
            self.assertIs(actual, timer.return_value)
            actual.start.assert_called_once_with()
            self.assertTrue(actual.daemon)

    def test_conflicting_modes_and_invalid_configuration_rejected(self):
        for extra in (['--until-complete', '--deadline', '2099-01-01T00:00:00Z'],
                      ['--deadline', 'invalid'], ['--deadline', '2099-01-01T00:00:00'],
                      ['--until-complete', '--jobs', '0']):
            with self.subTest(extra=extra), contextlib.redirect_stderr(io.StringIO()), \
                    self.assertRaises(SystemExit):
                runner.parse_args(self.arguments + extra)


if __name__ == '__main__':
    unittest.main()

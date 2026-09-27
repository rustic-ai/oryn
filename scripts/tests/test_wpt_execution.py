import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('runner', Path(__file__).resolve().parents[1] / 'run-upstream-wpt.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ExecutionTests(unittest.TestCase):
    def test_session_teardown_and_protocol_failures_override_pass(self):
        completion = {'complete': True, 'completion_count': 1, 'harness': 0,
                      'tests': [{'name': 'one', 'status': 0}]}
        for changed in ({}, {'error': 'WorkerCrashed'}, {'cleanup_errors': ['timeout']},
                        {'replacement_error': 'spawn failed'},
                        {'completion': dict(completion, completion_count=2)}):
            message = dict(completion=completion, cleanup_errors=[], **{})
            message.update(changed)
            session = Mock()
            session.run.return_value = message
            with patch.object(runner.Handler, 'session_client', session), \
                 patch.object(runner, 'fs_path_assets', return_value=[]):
                row = runner.run_session_case(Path('binary'), Path('.'), Path('a.html'),
                    'a.html|window|', 'core', 'window', '', 'http://localhost/a.html', 30)
            self.assertEqual(row['status'], 'failed' if changed else 'passed')
            self.assertFalse(row['resource_evidence_complete'])
            session.close.assert_not_called()

    def test_cold_session_shutdown_failure_overrides_pass(self):
        session = Mock()
        session.run.return_value = {'completion': {'complete': True, 'completion_count': 1,
            'harness': 0, 'tests': [{'name': 'one', 'status': 0}]}}
        session.close.return_value = {'returncode': 1, 'error': None,
                                     'shutdown_acknowledged': True}
        with patch.object(runner.Handler, 'session_client', None), \
             patch.object(runner, 'WptSession', return_value=session), \
             patch.object(runner, 'fs_path_assets', return_value=[]):
            row = runner.run_session_case(Path('binary'), Path('.'), Path('a.html'),
                'a.html|window|', 'core', 'window', '', 'http://localhost/a.html', 30)
        self.assertEqual(row['status'], 'failed')
        self.assertEqual(row['failure_domain'], 'runtime')

    def run_payload(self, payload, returncode=0, stderr=''):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'test.html').write_text('<script src=/resources/testharness.js></script>')
            process = subprocess.CompletedProcess([], returncode,
                '__ORYN_WPT_DONE__' + json.dumps(payload), stderr)
            with patch.object(runner.subprocess, 'run', return_value=process):
                return runner.run_case(Path('unused'), root, 8000, Path('test.html'),
                                       'core', 'window', '', 30, False, None)

    def test_passing_payload_cannot_override_failed_exit(self):
        payload = {'complete': True, 'harness': 0, 'tests': [{'name': 'one', 'status': 0}]}
        for code in (1, -9):
            row = self.run_payload(payload, code, 'WorkerCrashed after completion')
            self.assertEqual(row['status'], 'failed')
            self.assertEqual(row['failure_domain'], 'runtime')
            self.assertEqual(row['result'], payload)
            self.assertEqual(row['execution']['returncode'], code)
            self.assertIn('WorkerCrashed', row['execution']['stderr'])

    def test_malformed_completion_is_a_harness_failure(self):
        for payload in [[], None, {'complete': 'true', 'harness': 0, 'tests': []},
                        {'complete': True, 'harness': False, 'tests': []},
                        {'complete': True, 'harness': 0, 'tests': ['bad']}]:
            row = self.run_payload(payload)
            self.assertEqual(row['status'], 'failed')
            self.assertFalse(row['execution']['completion_valid'])

    def test_passing_execution_retains_diagnostics(self):
        row = self.run_payload({'complete': True, 'harness': 0,
                                'tests': [{'name': 'one', 'status': 0}]}, stderr='diagnostic')
        self.assertEqual(row['status'], 'passed')
        self.assertEqual(row['execution']['stderr'], 'diagnostic')


class SessionDiagnosticRetentionTests(unittest.TestCase):
    def test_async_failure_evidence_is_retained_with_primary_and_cleanup_errors(self):
        events = [{'page_id': '42', 'sequence': 9, 'event': {'kind': 'console',
            'level': 'error', 'message': 'navigation: original child failure'}}]
        audit = {'complete': True, 'overflow': False}
        for primary, cleanup in [(None, []), ('outer_execution_timeout', ['close failed'])]:
            session = Mock()
            session.run.return_value = {'completion': {'complete': True, 'completion_count': 1,
                'harness': 0, 'tests': [{'name': 'original assertion', 'status': 0}]},
                'error': primary, 'cleanup_errors': cleanup,
                'diagnostic_events': events, 'diagnostic_audit': audit,
                'policy_events': [{'event': {'kind': 'request_started'}}]}
            with patch.object(runner.Handler, 'session_client', session), \
                 patch.object(runner, 'fs_path_assets', return_value=[]):
                row = runner.run_session_case(Path('binary'), Path('.'), Path('a.html'),
                    'a.html|window|', 'core', 'window', '', 'http://localhost/a.html', 30)
            self.assertEqual(row['diagnostic_events'], events)
            self.assertEqual(row['diagnostic_audit'], audit)
            self.assertEqual(row['execution']['cleanup_errors'], cleanup)
            self.assertEqual(row['status'], 'failed' if primary else 'passed')
            self.assertEqual(row['policy_events'], [{'event': {'kind': 'request_started'}}])

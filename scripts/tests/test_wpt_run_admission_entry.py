"""Run-only bootstrap scope ordering; no upstream execution or subprocess."""
from contextlib import contextmanager
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from oryn_wptrunner import provenance

SPEC = importlib.util.spec_from_file_location('run_admission_entry', Path(__file__).resolve().parents[1]/'wpt-upstream-entry.py')
entry = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(entry)


class RunAdmissionEntryTests(unittest.TestCase):
    def run_case(self, failure=None, final_failure=None):
        events = []
        @contextmanager
        def scope(*args):
            events.append(('admit', args))
            try: yield
            finally: events.append('full-final')
        def final():
            events.append('generation-final')
            if final_failure: raise final_failure
        gate = SimpleNamespace(check=lambda: events.append('generation-check'), final_check=final)
        def main():
            events.append('upstream-main')
            try:
                if failure: raise failure
                return 0
            finally: events.append('upstream-cleanup')
        with patch.object(provenance, 'run_admission', side_effect=scope):
            try:
                result = entry.run_upstream(main, ['--binary','runtime','--webdriver-binary=driver',
                    '--oryn-run-contract','contract','--processes','8'], gate, Path('gate'))
            except BaseException as error:
                return events,error
        return events,result

    def test_run_context_spans_upstream_cleanup_then_full_and_generation_final(self):
        events,result = self.run_case()
        self.assertEqual(result,0)
        self.assertEqual(events, ['generation-check', ('admit', ('contract',Path('gate'),'runtime','driver')),
                         'upstream-main','upstream-cleanup','full-final','generation-final'])

    def test_exception_and_systemexit_do_not_skip_either_finalization(self):
        for failure in (RuntimeError('upstream'), SystemExit(3)):
            events,result = self.run_case(failure)
            self.assertIs(result,failure)
            self.assertEqual(events[-3:],['upstream-cleanup','full-final','generation-final'])

    def test_successful_systemexit_does_not_hide_generation_integrity_failure(self):
        _,result = self.run_case(SystemExit(0),ValueError('gate changed'))
        self.assertIsInstance(result,ValueError)
        self.assertEqual(str(result),'gate changed')

    def test_nonzero_primary_preserved_with_secondary_generation_failure(self):
        failure=RuntimeError('original')
        _,result=self.run_case(failure,ValueError('gate changed'))
        self.assertIs(result,failure)
        self.assertTrue(any('gate changed' in note for note in failure.__notes__))

    def test_process_boundary_preserves_primary_traceback_and_rejects_integrity(self):
        for primary in (RuntimeError('upstream'), SystemExit(1), SystemExit(0)):
            def main():
                with provenance._checked_exit(lambda: (_ for _ in ()).throw(ValueError('restored but revoked'))):
                    raise primary
            with patch('traceback.print_exception') as report:
                with self.assertRaises(SystemExit) as caught:
                    entry.process_exit(main)
            self.assertEqual(caught.exception.code, 65)
            if not isinstance(primary, SystemExit) or primary.code != 0:
                self.assertIs(report.call_args.args[0], primary)
                self.assertTrue(any('restored but revoked' in note for note in primary.__notes__))

    def test_ordinary_failure_and_return_codes_are_not_integrity_rejections(self):
        self.assertEqual(entry.process_exit(lambda: 1), 1)
        primary = SystemExit(1)
        def main(): raise primary
        with self.assertRaises(SystemExit) as caught: entry.process_exit(main)
        self.assertIs(caught.exception, primary)

    def test_full_final_failure_still_runs_generation_final(self):
        events = []
        @contextmanager
        def scope(*args):
            try: yield
            finally:
                events.append('full-final')
                raise ValueError('full verification failed')
        gate = SimpleNamespace(check=lambda: None, final_check=lambda: events.append('generation-final'))
        with patch.object(provenance, 'run_admission', side_effect=scope):
            with self.assertRaisesRegex(ValueError, 'full verification failed'):
                entry.run_upstream(lambda: 0, ['--oryn-run-contract', 'contract'], gate, Path('gate'))
        self.assertEqual(events, ['full-final', 'generation-final'])

    def test_startup_rejection_uses_reserved_integrity_exit(self):
        import os
        def main():
            with provenance.run_admission(None, Path('gate')):
                self.fail('missing envelope admitted')
        with patch.dict(os.environ, {}, clear=True), patch('traceback.print_exception') as report:
            with self.assertRaises(SystemExit) as caught: entry.process_exit(main)
        self.assertEqual(caught.exception.code, 65)
        self.assertIsInstance(report.call_args.args[0], ValueError)

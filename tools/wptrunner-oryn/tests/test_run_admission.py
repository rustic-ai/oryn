"""Run admission custody/call-count tests; no browser or upstream execution."""
import copy
import json
import os
from pathlib import Path
import pickle
import tempfile
import threading
import unittest
from unittest.mock import patch

from oryn_wptrunner import provenance as p
from suite_input_lease import MetadataSnapshot


class RunAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name).resolve()
        self.path = self.root / 'contract.json'
        self.path.write_text('{}')
        self.gate = self.root / 'gate.json'
        self.gate.write_text('{}')
        self.package = self.root / 'package'
        self.package.mkdir()
        (self.package / 'plugin.py').write_text('plugin')
        self.tls = self.root / 'certificate.pem'
        self.tls.write_text('certificate')
        artifacts = {}
        for name in ('runtime', 'driver'):
            target = self.root / name
            target.write_text(name)
            artifacts[name] = {'path': str(target), 'sha256': p.digest(target)}
        self.value = {'path': str(self.path), 'sha256': p.digest(self.path),
                      'candidate': {'artifacts': artifacts}, 'fixture': {'nested': [1]}}
        self.full = patch.object(p, 'load_contract', side_effect=lambda *a, **k: copy.deepcopy(self.value)).start()
        self.addCleanup(patch.stopall)
        patch.object(p, '_RUN_ADMISSION', None).start()
        patch.dict(os.environ, {'ORYN_CORE_COLLECTION': 'core'}, clear=False).start()
        self.capture_patch = patch.object(p, '_run_snapshot', side_effect=self.snapshot)
        self.capture = self.capture_patch.start()

    def snapshot(self, *args):
        return (MetadataSnapshot([self.path, self.gate, self.tls,
                                 self.root/'runtime', self.root/'driver'],
                    membership=lambda: p._plugin_members(self.package)),)

    def scope(self):
        return p.run_admission(self.path, self.gate, self.root/'runtime', self.root/'driver')

    def get(self):
        return p.load_run_contract(self.path, self.root/'runtime', self.root/'driver')

    def test_restarts_validate_once_at_admission_and_once_at_final_only(self):
        with self.scope():
            for _ in range(5):
                self.assertEqual(self.get(), self.value)
            self.assertEqual(self.full.call_count, 1)
        self.assertEqual(self.full.call_count, 2)

    def test_concurrent_manager_threads_share_one_admission_and_copy_isolation(self):
        results, errors = [], []
        def get():
            try:
                value = self.get(); value['fixture']['nested'].append(2); results.append(value)
            except BaseException as error:
                errors.append(error)
        with self.scope():
            threads = [threading.Thread(target=get) for _ in range(8)]
            for thread in threads: thread.start()
            for thread in threads:
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())
            self.assertEqual(errors, [])
            self.assertEqual(len(results), 8)
            self.assertEqual(self.get()['fixture']['nested'], [1])
        self.assertEqual(self.full.call_count, 2)

    def test_standalone_and_explicit_strict_loads_never_consult_cache(self):
        self.get(); self.get()
        self.assertEqual(self.full.call_count, 2)
        with self.scope():
            p.load_contract(self.path)
            self.get()
        self.assertEqual(self.full.call_count, 5)

    def test_path_scope_violation_is_sticky_even_when_original_is_requested(self):
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                with self.assertRaisesRegex(ValueError, 'path changed'):
                    p.load_run_contract(self.root/'other')
                with self.assertRaisesRegex(ValueError, 'revoked'): self.get()
        self.assertEqual(self.full.call_count, 2)

    def test_binary_path_violation_cannot_admit_an_equal_content_copy(self):
        alternate = self.root/'alternate'; alternate.write_text('runtime')
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                with self.assertRaisesRegex(ValueError, 'binary differs'):
                    p.load_run_contract(self.path, alternate, self.root/'driver')
        self.assertEqual(self.full.call_count, 2)

    def test_binary_content_is_hashed_on_every_launch_even_if_metadata_helper_passes(self):
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope() as yielded:
                self.assertIsNone(yielded)
                scope = p._RUN_ADMISSION
                with patch.object(scope, '_metadata'):
                    (self.root/'runtime').write_text('changed')
                    with self.assertRaisesRegex(ValueError, 'artifact missing or changed'): self.get()
        self.assertEqual(self.full.call_count, 2)

    def test_collection_change_and_restore_revokes_scope(self):
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                os.environ['ORYN_CORE_COLLECTION'] = 'placement'
                with self.assertRaisesRegex(ValueError, 'collection changed'): self.get()
                os.environ['ORYN_CORE_COLLECTION'] = 'core'
                with self.assertRaisesRegex(ValueError, 'revoked'): self.get()

    def test_same_size_rewrite_restored_mtime_and_original_restore_stay_revoked(self):
        before = self.tls.stat()
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                self.tls.write_text('replacement')
                os.utime(self.tls, ns=(before.st_atime_ns, before.st_mtime_ns))
                with self.assertRaisesRegex(ValueError, 'metadata changed'): self.get()
                self.tls.write_text('certificate')
                with self.assertRaisesRegex(ValueError, 'revoked'): self.get()
        self.assertEqual(self.full.call_count, 2)

    def test_plugin_addition_and_removal_are_membership_changes_but_bytecode_is_not(self):
        with self.scope():
            cache = self.package/'__pycache__'; cache.mkdir(); (cache/'plugin.pyc').write_bytes(b'cache')
            self.get()
        for action in ('add', 'remove'):
            with self.subTest(action=action):
                with self.assertRaisesRegex(ValueError, 'final verification failed'):
                    with self.scope():
                        if action == 'add': (self.package/'new.py').write_text('new')
                        else: (self.package/'new.py').unlink()
                        with self.assertRaisesRegex(ValueError, 'membership'): self.get()

    def test_parent_symlink_retarget_is_observed_even_for_identical_bytes(self):
        one = self.root/'one'; two = self.root/'two'; one.mkdir(); two.mkdir()
        for parent in (one,two): (parent/'cert').write_text('same')
        link = self.root/'alias'; link.symlink_to(one, target_is_directory=True)
        self.tls = link/'cert'
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                link.unlink(); link.symlink_to(two, target_is_directory=True)
                with self.assertRaisesRegex(ValueError, 'metadata changed'): self.get()

    def test_missing_nested_input_created_after_admission_is_not_ignored(self):
        self.tls.unlink()
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                self.tls.write_text('created')
                with self.assertRaisesRegex(ValueError, 'metadata changed'): self.get()

    def test_change_during_full_admission_prevents_any_use_and_still_finalizes(self):
        def full(*args, **kwargs):
            self.tls.write_text('changed during admission')
            return copy.deepcopy(self.value)
        self.full.side_effect = full
        with self.assertRaisesRegex(ValueError, 'metadata changed'):
            with self.scope(): self.fail('admission must fail')
        self.assertEqual(self.full.call_count, 2)
        self.assertIsNone(p._RUN_ADMISSION)

    def test_final_full_runs_on_success_exception_and_nonzero_systemexit(self):
        for error in (None, RuntimeError('primary'), SystemExit(7)):
            before = self.full.call_count
            if error is None:
                with self.scope(): self.get()
            else:
                with self.assertRaises(type(error)) as caught:
                    with self.scope(): raise error
                self.assertIs(caught.exception, error)
            self.assertEqual(self.full.call_count-before, 2)

    def test_final_failure_preserves_primary_exception_and_adds_integrity_note(self):
        primary = RuntimeError('primary upstream failure')
        self.full.side_effect = [copy.deepcopy(self.value), ValueError('final bad')]
        with self.assertRaises(RuntimeError) as caught:
            with self.scope(): raise primary
        self.assertIs(caught.exception, primary)
        self.assertTrue(any('final bad' in note for note in primary.__notes__))

    def test_final_full_failure_makes_successful_systemexit_nonzero(self):
        self.full.side_effect = [copy.deepcopy(self.value), ValueError('final bad')]
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope(): raise SystemExit(0)
        self.assertIsNone(p._RUN_ADMISSION)

    def test_final_semantics_change_fails_even_if_metadata_checker_returns(self):
        changed = copy.deepcopy(self.value); changed['fixture']['nested'] = [2]
        self.full.side_effect = [copy.deepcopy(self.value), changed]
        with self.assertRaisesRegex(ValueError, 'final full contract differs'):
            with self.scope(): self.get()

    def test_pid_mismatch_does_not_trust_inherited_admission(self):
        with self.scope() as yielded:
            self.assertIsNone(yielded)
            scope = p._RUN_ADMISSION
            with patch.object(p.os, 'getpid', return_value=scope.pid+1):
                with self.assertRaisesRegex(ValueError, 'process boundary'): self.get()
            self.get()
        self.assertEqual(self.full.call_count, 2)

    def test_overlap_closed_handles_and_explicit_new_scope(self):
        with self.scope() as yielded:
            self.assertIsNone(yielded)
            scope = p._RUN_ADMISSION
            with self.assertRaisesRegex(ValueError, 'overlapping'):
                with self.scope(): self.fail('overlap admitted')
            with self.assertRaises(TypeError): pickle.dumps(scope)
        with self.assertRaisesRegex(ValueError, 'not active'): scope.get(self.path)
        with self.scope(): self.get()
        self.assertEqual(self.full.call_count, 4)

    def test_snapshot_captures_envelope_before_discovering_candidate(self):
        self.capture_patch.stop()
        def mutate(*args, **kwargs):
            self.path.write_text('{"changed":true}')
            return MetadataSnapshot([self.gate])
        self.path.write_text(json.dumps({'candidate': {'path': str(self.root/'candidate')}}))
        with patch('suite_input_lease.candidate_snapshot', side_effect=mutate):
            with self.assertRaisesRegex(ValueError, 'metadata changed'):
                p._run_snapshot(self.path, self.gate)

    def test_inherited_scope_rejected_before_global_or_scope_lock(self):
        with self.scope():
            scope = p._RUN_ADMISSION
            with patch.object(p.os, 'getpid', return_value=scope.pid+1), patch.object(p, '_RUN_LOCK') as lock:
                lock.__enter__.side_effect = AssertionError('inherited mutex acquired')
                with self.assertRaisesRegex(ValueError, 'process boundary'):
                    with self.scope(): self.fail('inherited admission accepted')
                lock.__enter__.assert_not_called()

    def test_installed_plugin_snapshot_uses_canonical_lease_source_root(self):
        import suite_input_lease as lease
        self.capture_patch.stop()
        self.path.write_text(json.dumps({'candidate': {'path': str(self.root/'candidate')}}))
        installed = self.root/'venv/lib/python3.14/site-packages/oryn_wptrunner'
        installed.mkdir(parents=True)
        (installed/'provenance.py').write_text('installed plugin')
        with patch.object(p, '__file__', str(installed/'provenance.py')), patch.object(lease, 'candidate_snapshot', return_value=MetadataSnapshot([self.gate])) as capture:
            p._run_snapshot(self.path, self.gate)
        self.assertEqual(capture.call_args.args, (str(self.root/'candidate'), self.gate, Path(lease.__file__).resolve().parents[1]))

    def test_fork_before_scope_publication_rejected_before_global_mutex(self):
        with patch.object(p.os, 'getpid', return_value=p._RUN_PID+1), patch.object(p, '_RUN_LOCK') as lock:
            lock.__enter__.side_effect = AssertionError('inherited mutex acquired')
            with self.assertRaisesRegex(ValueError, 'process boundary'):
                with self.scope(): self.fail('fork accepted')
            lock.__enter__.assert_not_called()

    def test_admission_timing_two_records_flush_to_upstream_logger_without_retiming(self):
        sink = p._AdmissionTiming()
        sink.info('begin'); sink.info('end')
        with patch('mozlog.get_default_logger') as logger:
            sink.flush()
            self.assertEqual([call.args[0] for call in logger.return_value.info.call_args_list], ['begin', 'end'])
            sink.flush()
            self.assertEqual(logger.return_value.info.call_count, 2)
        self.assertEqual(sink.pending, [])

    def test_early_admission_timing_failure_retains_two_raw_stderr_records(self):
        sink = p._AdmissionTiming()
        sink.info('begin'); sink.info('end')
        with patch('mozlog.get_default_logger', return_value=None), patch.object(p.sys.stderr, 'write') as stderr:
            sink.flush()
            stderr.assert_not_called()
            sink.flush(final=True)
            self.assertEqual([call.args[0] for call in stderr.call_args_list], ['begin\n', 'end\n'])
        self.assertEqual(sink.pending, [])

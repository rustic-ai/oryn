"""Failure injection for fixture ownership and family dispatch admission."""
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest import mock

import workflow_cleanup as cleanup
import workflow_native_adapter as adapter
from workflow_contract import DEFAULT_CONTRACT, planned_attempts
from workflow_execution import execute_contract
from workflow_family_recipes import RECIPES


class CleanupTests(unittest.TestCase):
    def setUp(self):
        custody = mock.Mock()
        custody.live.return_value = []
        custody.discovery_status.return_value = (True, [])
        patcher = mock.patch.object(adapter, "NativeCustody", return_value=custody)
        patcher.start()
        self.addCleanup(patcher.stop)

    def tearDown(self):
        # All retained owners in this suite are mocks, never real processes.
        with cleanup._LOCK:
            cleanup._RETAINED.clear()

    def test_every_owner_and_secret_attempted_and_primary_preserved(self):
        first = mock.Mock(); first.close.side_effect = OSError('first close')
        second = mock.Mock(); second.close.return_value = dict(returncode=0, forced=False)
        secret = mock.Mock(); secret.unlink.side_effect = OSError('unlink')
        cases = [SimpleNamespace(oracle=first), SimpleNamespace(oracle=second)]
        stop = threading.Event(); primary = ValueError('business failure')
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(cleanup.CleanupFailure) as raised:
                cleanup.cleanup_cases(cases, [dict(secret_path=secret)], stop, Path(directory), primary)
            receipt = json.loads((Path(directory)/'fixture-cleanup.json').read_text())
        second.close.assert_called_once_with()
        self.assertIsNone(cases[1].oracle)
        self.assertIs(cases[0].oracle, first)
        self.assertIs(raised.exception.primary, primary)
        self.assertTrue(stop.is_set())
        self.assertIn('unlink', str(raised.exception))
        self.assertIn('first close', str(raised.exception))
        self.assertEqual(receipt['primary_error'], 'business failure')

    def test_exited_parent_does_not_authorize_group_signal(self):
        process = mock.Mock(pid=123, returncode=0)
        process.poll.return_value = 0
        with mock.patch.object(cleanup.os, 'killpg', side_effect=[None, ProcessLookupError()]) as kill, mock.patch.object(cleanup.time, 'sleep'):
            result = cleanup.settle_group(process)
        self.assertTrue(result['group_settled'])
        self.assertEqual(result['errors'], [])
        self.assertEqual([row.args for row in kill.call_args_list], [(123, 0), (123, 0)])

    def test_survivor_deadline_records_unknown_settlement(self):
        process = mock.Mock(pid=123, returncode=None)
        process.poll.return_value = None
        process.wait.side_effect = TimeoutError('still live')
        with mock.patch.object(cleanup.os, 'killpg'), mock.patch.object(cleanup.time, 'monotonic', return_value=0):
            result = cleanup.settle_group(process, timeout=0)
        self.assertFalse(result['group_settled'])
        self.assertIsNone(result['returncode'])
        self.assertEqual(len(result['errors']), 2)

    def test_prepared_case_task_generation_failure_closes_owned_oracle(self):
        owner = mock.Mock(origin='http://fixture'); owner.close.return_value = dict(returncode=0, forced=False)
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(adapter.support, 'Oracle', return_value=owner), mock.patch.object(adapter, 'load', return_value=mock.Mock()), mock.patch.object(adapter, 'read_case_contract', return_value=dict(cases=[dict(id='case', upload=False)])), mock.patch.object(adapter, 'family_lines', side_effect=ValueError('task generation')):
            with self.assertRaisesRegex(ValueError, 'task generation'):
                adapter.PreparedCase(('forms', 1, 'case'), Path(directory)/'case')
        owner.close.assert_called_once_with()

    def test_prepared_create_and_cleanup_failure_preserve_both(self):
        owner = mock.Mock(origin='http://fixture'); owner.request.side_effect = ValueError('create failure'); owner.close.side_effect = OSError('close failure')
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(adapter.support, 'Oracle', return_value=owner), mock.patch.object(adapter, 'load', return_value=mock.Mock()), mock.patch.object(adapter, 'read_case_contract', return_value=dict(cases=[dict(id='case', upload=False)])):
            with self.assertRaises(cleanup.CleanupFailure) as raised:
                adapter.PreparedCase(('forms', 1, 'case'), Path(directory)/'case')
        self.assertTrue(raised.exception.custody_lost)
        self.assertIn('create failure', str(raised.exception))
        self.assertIn('close failure', str(raised.exception))
        self.assertEqual(raised.exception.owners, (owner,))

    def test_oracle_prelaunch_failures_close_stderr(self):
        for fail_generation in (False, True):
            with self.subTest(fail_generation=fail_generation), tempfile.TemporaryDirectory() as directory:
                stderr = mock.Mock()
                with mock.patch('builtins.open', return_value=stderr), mock.patch.object(adapter.support, 'check_execution_generation', side_effect=[None, ValueError('generation')] if fail_generation else None), mock.patch.object(adapter.support.subprocess, 'Popen', side_effect=OSError('spawn')):
                    with self.assertRaises(cleanup.CleanupFailure) as raised:
                        adapter.support.Oracle(Path(directory))
                stderr.close.assert_called_once_with()
                self.assertFalse(raised.exception.custody_lost)

    def test_oracle_wait_failure_still_attempts_every_pipe(self):
        oracle = adapter.support.Oracle.__new__(adapter.support.Oracle)
        oracle.process = mock.Mock(returncode=None)
        oracle.process.poll.return_value = None
        oracle.process.wait.side_effect = TimeoutError('wait failed')
        oracle.stderr = mock.Mock()
        oracle.process.stdin.close.side_effect = OSError('stdin close')
        with mock.patch.object(adapter.support, 'settle_group', return_value=dict(group_settled=False, returncode=None, errors=['kill failed'])):
            result = oracle.close()
        oracle.process.stdout.close.assert_called_once_with()
        oracle.stderr.close.assert_called_once_with()
        self.assertFalse(result['resources_settled'])
        self.assertIn('wait failed', repr(result))
        self.assertIn('kill failed', repr(result))
        self.assertIn('stdin close', repr(result))

    def test_dispatch_latches_only_lost_custody_preserves_original_population(self):
        contract = json.loads(DEFAULT_CONTRACT.read_text())
        candidate = dict(parent_sha256='a'*64, worker_sha256='b'*64, source_tree_sha256='c'*64)
        planned = planned_attempts(contract)
        self.assertEqual(len(planned), 688)
        self.assertEqual(len({a['cell'] for a in planned}), 29)
        for lost in (True, False):
            calls = []
            def fail(attempt, directory, stop):
                calls.append(attempt['attempt_id'])
                if lost:
                    raise cleanup.CleanupFailure(ValueError('business failure'), ['unsettled group'], True)
                raise ValueError('ordinary business failure')
            with self.subTest(lost=lost), tempfile.TemporaryDirectory() as directory:
                execute_contract(contract, candidate, {a['cell']:fail for a in planned}, Path(directory)/'run', {}, workers=1)
                records = [json.loads(line) for line in (Path(directory)/'run/attempts.jsonl').read_text().splitlines()]
            self.assertEqual({r['attempt_id'] for r in records}, {a['attempt_id'] for a in planned})
            self.assertEqual(len(calls), 1 if lost else len(planned))
            if lost:
                self.assertTrue(any('not executed' in e for r in records for e in r['errors']))

    def test_face_mapping_preserves_exact_independent_upload_projection(self):
        recipe = ('forms', 4, 'native_face_authorized_multipart')
        self.assertIn(recipe, RECIPES['forms.controls'])
        projected = adapter.expected_recipe(recipe)
        contract = adapter.read_case_contract(*recipe)
        case = next(c for c in contract['cases'] if c['id'] == recipe[2])
        expected = adapter.forms_expected(contract, case, adapter.upload_metadata())
        self.assertEqual(projected['outcome'], expected)
        fields = expected['receipts'][0]['fields']
        self.assertEqual([v for k,v in fields if k == 'face_duplicate'], ['first', 'second'])
        self.assertTrue(expected['receipts'][0]['files'])
        self.assertNotIn('excluded', [k for k,v in fields])

    def test_face_validator_rejects_changed_upload_or_duplicate_order(self):
        import copy
        recipe = ('forms', 4, 'native_face_authorized_multipart')
        contract = adapter.read_case_contract(*recipe)
        case = next(c for c in contract['cases'] if c['id'] == recipe[2])
        expected = adapter.expected_recipe(recipe)['outcome']
        data = dict(receipts=copy.deepcopy(expected['receipts']),
                    requests=[dict(r, body_sha256='a'*64, body_bytes=1) for r in expected['requests']],
                    case=dict(accepted=expected['accepted']), denials=expected['denials'])
        frames = [dict(kind='structured', value=v) for v in case['expected_text']]
        frames += [dict(kind='action', result=dict(execution_domain='native')) for step in case['steps'] if step.split()[0] in ('click','type','upload')]
        self.assertEqual(adapter.validate_forms(case, contract, 'a'*32, 'http://fixture', frames, data), [])
        for mutation in ('bytes', 'order'):
            changed = copy.deepcopy(data)
            if mutation == 'bytes':
                changed['receipts'][0]['files'][0]['sha256'] = '0'*64
            else:
                changed['receipts'][0]['fields'].reverse()
            self.assertIn('extended forms durable method/body/receipt outcome mismatch', adapter.validate_forms(case, contract, 'a'*32, 'http://fixture', frames, changed))

    def test_native_unsettled_group_latches_stop_and_keeps_receipt(self):
        process = mock.Mock(pid=123, returncode=0)
        sampler = mock.Mock(); sampler.finish.return_value = dict(samples=[], errors=[])
        watcher = mock.Mock(); watcher.is_alive.return_value = False
        stop = threading.Event()
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(adapter, 'check_execution_generation'), mock.patch.object(adapter.subprocess, 'Popen', return_value=process), mock.patch.object(adapter, 'ProcessSampler', return_value=sampler), mock.patch.object(adapter.threading, 'Thread', return_value=watcher), mock.patch.object(adapter, 'settle_group', return_value=dict(parent_pid=123, process_group=123, returncode=0, group_settled=False, errors=['survivor'])):
            with self.assertRaises(cleanup.CleanupFailure):
                adapter.execute_native(Path('/native'), Path('/worker'), {}, Path(directory), stop)
            receipt = json.loads((Path(directory)/'native-process-cleanup.json').read_text())
        self.assertTrue(stop.is_set())
        self.assertFalse(receipt['group_settled'])
        self.assertEqual(receipt['returncode'], 0)

    def test_malformed_cleanup_cannot_release_owner(self):
        for receipt in (None, {}, dict(returncode=False,forced=False),
                        dict(returncode=None,group_settled=True,resources_settled=True)):
            owner = mock.Mock(); owner.close.return_value = receipt
            _, errors, settled = cleanup.close_owner(owner)
            self.assertTrue(errors)
            self.assertFalse(settled)

    def test_secret_write_and_close_failures_keep_both_and_retain_stream(self):
        stream = mock.Mock()
        stream.write.side_effect = OSError('primary write failure')
        stream.close.side_effect = OSError('secondary close failure')
        with self.assertRaises(cleanup.CleanupFailure) as raised:
            cleanup.write_owned_stream(stream, b'secret')
        self.assertIn('primary write failure', str(raised.exception))
        self.assertIn('secondary close failure', str(raised.exception))
        self.assertTrue(raised.exception.custody_lost)
        self.assertIs(raised.exception.owners[0].stream, stream)
        stream.close.side_effect = None
        receipt = cleanup.retry_retained_cleanup()
        self.assertTrue(receipt[0]['settled'])
        self.assertEqual(stream.close.call_count, 2)

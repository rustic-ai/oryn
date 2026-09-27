"""Rescue regressions: provenance, timeout typing, cancellation and bounded custody."""
import json
from pathlib import Path
import signal
import subprocess
import tempfile
import threading
from unittest import TestCase, mock

import workflow_cleanup as cleanup
import workflow_native_adapter as adapter
from workflow_contract import DEFAULT_CONTRACT, planned_attempts
from workflow_execution import execute_contract


class CustodyTests(TestCase):
    def test_detached_child_retained_and_reused_pid_never_signalled(self):
        process = mock.Mock(pid=10)
        records = {10: ((1, 1), 1), 11: ((2, 2), 10)}
        with mock.patch.object(cleanup, 'process_identity', side_effect=lambda pid: records.get(pid)):
            owner = cleanup.NativeCustody(process)
            owner.observe([dict(pid=10,parent_pid=1),dict(pid=11,parent_pid=10)])
            records[10] = None
            records[11] = ((2, 2), 1)
            self.assertEqual(owner.live(), [11])
            with mock.patch.object(cleanup.os, 'kill') as kill:
                owner.signal(signal.SIGKILL, [])
                kill.assert_called_once_with(11, signal.SIGKILL)
            records[11] = ((3, 3), 1)
            with mock.patch.object(cleanup.os, 'kill') as kill:
                owner.signal(signal.SIGKILL, [])
                kill.assert_not_called()
            self.assertEqual(owner.live(), [])

    def test_fallback_pid_discovery_cannot_admit_wrong_kernel_parent(self):
        records = {10: ((1, 1), 1), 11: ((2, 2), 99)}
        with mock.patch.object(cleanup, 'process_identity', side_effect=lambda pid: records.get(pid)):
            owner = cleanup.NativeCustody(mock.Mock(pid=10))
            owner.observe([dict(pid=11,parent_pid=10)])
            self.assertEqual(owner.owned, {10: (1, 1)})

    def test_same_record_resource_birth_reused_before_signal_is_rejected(self):
        records = {10: ((1, 1), 1), 11: ((3, 3), 10)}
        with mock.patch.object(cleanup, 'process_identity', side_effect=lambda pid: records.get(pid)):
            owner = cleanup.NativeCustody(mock.Mock(pid=10))
            owner.observe([dict(pid=11,parent_pid=10,kernel_birth=[2,2])])
            with mock.patch.object(cleanup.os, 'kill') as kill:
                owner.signal(signal.SIGKILL, [])
                self.assertNotIn(mock.call(11, signal.SIGKILL), kill.call_args_list)

    def test_permission_signal_failure_is_not_success(self):
        with mock.patch.object(cleanup, 'process_identity', return_value=((1,1),1)):
            owner = cleanup.NativeCustody(mock.Mock(pid=10))
            with mock.patch.object(cleanup.os, 'kill', side_effect=PermissionError('denied')):
                with self.assertRaisesRegex(RuntimeError, "denied"):
                    owner.signal(signal.SIGKILL, [])

    def test_transient_group_permission_race_requires_positive_absence(self):
        process=mock.Mock(pid=10,returncode=-9)
        owner=mock.Mock();owner.live.return_value=[];owner.discovery_status.return_value=(True,[])
        with mock.patch.object(cleanup.os,'killpg',side_effect=[PermissionError('race'),ProcessLookupError()]) as observe, mock.patch.object(cleanup.time,'sleep'):
            result=cleanup.settle_group(process,custody=owner)
        self.assertTrue(result['group_settled'])
        self.assertEqual(result['errors'],[])
        self.assertIn('race',str(result['observations']))
        self.assertEqual(observe.call_count,2)

    def test_persistent_permission_error_remains_uncertain_despite_parent_exit(self):
        process=mock.Mock(pid=10,returncode=-9)
        owner=mock.Mock();owner.live.return_value=[];owner.discovery_status.return_value=(True,[])
        with mock.patch.object(cleanup.os,'killpg',side_effect=PermissionError('denied')):
            result=cleanup.settle_group(process,timeout=0,custody=owner)
        self.assertFalse(result['group_settled'])
        self.assertIn('denied',str(result['errors']))

    def test_absent_original_group_does_not_hide_detached_retained_child(self):
        process=mock.Mock(pid=10,returncode=0)
        owner=mock.Mock();owner.live.return_value=[11];owner.discovery_status.return_value=(True,[])
        with mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
            result=cleanup.settle_group(process,timeout=0,custody=owner)
        self.assertFalse(result['group_settled'])
        owner.signal.assert_called_with(signal.SIGKILL,mock.ANY)


class ExecutionDispositionTests(TestCase):
    def tearDown(self):
        with cleanup._LOCK:
            cleanup._RETAINED.clear()

    def test_nested_fixture_cleanup_retains_timeout(self):
        timeout=cleanup.NativeExecutionFailure('native timeout',timed_out=True)
        error=cleanup.CleanupFailure(cleanup.CleanupFailure(timeout,['worker denied'],True),['fixture close'])
        result=cleanup.failure_details(error)
        self.assertTrue(result['timed_out'])
        self.assertFalse(result['interrupted'])
        self.assertEqual(result['primary_error'],'native timeout')
        self.assertEqual(result['cleanup_errors'],['fixture close','worker denied'])

    def test_timeout_stops_custody_lost_run_without_erasing_688_originals(self):
        contract=json.loads(DEFAULT_CONTRACT.read_text())
        candidate=dict(parent_sha256='a'*64,worker_sha256='b'*64,source_tree_sha256='c'*64)
        plan=planned_attempts(contract)
        calls=[]
        def execute(attempt,directory,stop):
            calls.append(attempt['attempt_id'])
            raise cleanup.CleanupFailure(cleanup.CleanupFailure(cleanup.NativeExecutionFailure('180s',timed_out=True),['permission denied'],True),[],False)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)/'run'
            execute_contract(contract,candidate,{a['cell']:execute for a in plan},root,{},workers=1)
            rows=[json.loads(line) for line in (root/'attempts.jsonl').read_text().splitlines()]
        self.assertEqual(len(rows),688)
        self.assertEqual(len(calls),1)
        self.assertEqual(rows[0]['disposition'],'timed_out')
        self.assertTrue(rows[0]['timed_out'])
        self.assertEqual(rows[0]['cleanup_errors'],['permission denied'])
        self.assertTrue(all(row['disposition']=='not_executed' for row in rows[1:]))

    def test_timeout_at_navigation_action_and_close_never_replays_plan(self):
        for phase in ('navigation','action','close'):
            for unresolved in (False,True):
                with self.subTest(phase=phase,unresolved=unresolved),tempfile.TemporaryDirectory() as directory:
                    process=mock.Mock(pid=10,returncode=-9)
                    process.wait.side_effect=subprocess.TimeoutExpired('native',180)
                    owner=mock.Mock();owner.live.return_value=[];owner.discovery_status.return_value=(True,[])
                    sampler=mock.Mock();sampler.finish.return_value=dict(samples=[],errors=[])
                    watcher=mock.Mock();watcher.is_alive.return_value=False
                    receipt=dict(parent_pid=10,process_group=10,returncode=-9,group_settled=not unresolved,errors=['denied'] if unresolved else [])
                    with mock.patch.object(adapter,'check_execution_generation'),mock.patch.object(adapter.subprocess,'Popen',return_value=process) as spawn,mock.patch.object(adapter,'NativeCustody',return_value=owner),mock.patch.object(adapter,'ProcessSampler',return_value=sampler),mock.patch.object(adapter.threading,'Thread',return_value=watcher),mock.patch.object(adapter,'settle_group',return_value=receipt):
                        with self.assertRaises(RuntimeError) as raised:
                            adapter.execute_native(Path('/native'),Path('/worker'),dict(phase=phase,action_id='original'),Path(directory),threading.Event())
                    details=cleanup.failure_details(raised.exception)
                    self.assertTrue(details['timed_out'])
                    self.assertEqual(spawn.call_count,1)
                    self.assertEqual(json.loads((Path(directory)/'native-plan.json').read_text())['action_id'],'original')
                    self.assertTrue(json.loads((Path(directory)/'native-process-cleanup.json').read_text())['timed_out'])

    def test_cancelled_native_execution_is_interrupted_and_never_replayed(self):
        process=mock.Mock(pid=10,returncode=-9);process.poll.return_value=None;process.wait.return_value=-9
        owner=mock.Mock();owner.live.return_value=[];owner.discovery_status.return_value=(True,[])
        sampler=mock.Mock();sampler.finish.return_value=dict(samples=[],errors=[])
        stop=threading.Event()
        def thread(*,target,daemon):
            watcher=mock.Mock();watcher.is_alive.return_value=False
            def start():
                stop.set()
                target()
            watcher.start.side_effect=start
            return watcher
        receipt=dict(parent_pid=10,process_group=10,returncode=-9,group_settled=True,errors=[])
        with tempfile.TemporaryDirectory() as directory,mock.patch.object(adapter,'check_execution_generation'),mock.patch.object(adapter.subprocess,'Popen',return_value=process) as spawn,mock.patch.object(adapter,'NativeCustody',return_value=owner),mock.patch.object(adapter,'ProcessSampler',return_value=sampler),mock.patch.object(adapter.threading,'Thread',side_effect=thread),mock.patch.object(adapter,'settle_group',return_value=receipt),mock.patch.object(adapter.os,'killpg') as group_signal:
            with self.assertRaises(cleanup.NativeExecutionFailure) as raised:
                adapter.execute_native(Path('/native'),Path('/worker'),{},Path(directory),stop)
            self.assertTrue(raised.exception.interrupted)
            self.assertFalse(raised.exception.timed_out)
            spawn.assert_called_once()
            group_signal.assert_not_called()
            owner.signal.assert_called_once_with(signal.SIGKILL,[])


class ReviewClosureTests(TestCase):
    def test_discovery_failure_latched_even_after_known_processes_disappear(self):
        process=mock.Mock(pid=10,returncode=0)
        with mock.patch.object(cleanup,'process_identity',return_value=None):
            owner=cleanup.NativeCustody(process)
            owner.mark_uncertain('kernel child identity denied')
            owner.seal()
            with mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
                receipt=cleanup.settle_group(process,custody=owner)
        self.assertFalse(receipt['group_settled'])
        self.assertIn('kernel child identity denied',str(receipt['errors']))

    def test_sampler_failure_revokes_custody(self):
        import workflow_resources as resources
        owner=mock.Mock();owner.observe.side_effect=OSError('identity denied')
        sampler=resources.ProcessSampler(10,custody=owner)
        with mock.patch.object(resources.sys,'platform','linux'),mock.patch.object(resources.subprocess,'check_output',return_value='10 1 1\n11 10 2\n'):
            sampler.collect()
        owner.mark_uncertain.assert_called_once()
        self.assertIn('identity denied',sampler.errors[0])

    def test_discovery_must_be_sealed_before_clean_receipt(self):
        process=mock.Mock(pid=10,returncode=0)
        with mock.patch.object(cleanup,'process_identity',return_value=None):
            owner=cleanup.NativeCustody(process)
            with mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
                receipt=cleanup.settle_group(process,custody=owner)
        self.assertFalse(receipt['group_settled'])
        self.assertIn('not sealed',str(receipt['errors']))

    def test_final_settlement_observes_sampler_last_publication(self):
        order=[]
        process=mock.Mock(pid=10,returncode=-9)
        process.wait.side_effect=subprocess.TimeoutExpired('native',180)
        owner=mock.Mock()
        owner.seal.side_effect=lambda:order.append('sealed')
        sampler=mock.Mock()
        def finish():
            order.append('last ownership publication; sampling joined')
            return dict(samples=[],errors=[])
        sampler.finish.side_effect=finish
        watcher=mock.Mock();watcher.is_alive.return_value=False
        def settle(*args,**kwargs):
            self.assertEqual(order,['last ownership publication; sampling joined','sealed'])
            return dict(parent_pid=10,process_group=10,returncode=-9,group_settled=True,errors=[])
        with tempfile.TemporaryDirectory() as directory,mock.patch.object(adapter,'check_execution_generation'),mock.patch.object(adapter.subprocess,'Popen',return_value=process),mock.patch.object(adapter,'NativeCustody',return_value=owner),mock.patch.object(adapter,'ProcessSampler',return_value=sampler),mock.patch.object(adapter.threading,'Thread',return_value=watcher),mock.patch.object(adapter,'settle_group',side_effect=settle) as settlement:
            with self.assertRaises(cleanup.NativeExecutionFailure):
                adapter.execute_native(Path('/native'),Path('/worker'),{},Path(directory),threading.Event())
        settlement.assert_called_once()

    def test_nested_uncertainty_inherits_stop_without_shared_native_event(self):
        inner=cleanup.CleanupFailure(cleanup.NativeExecutionFailure('timeout',timed_out=True),['uncertain'],True)
        outer=cleanup.CleanupFailure(inner,['fixture failure'],False)
        self.assertTrue(outer.custody_lost)
        self.assertTrue(cleanup.failure_details(outer)['timed_out'])

    def test_adapter_cannot_supply_disposition_or_nonboolean_interruption(self):
        contract=json.loads(DEFAULT_CONTRACT.read_text())
        candidate=dict(parent_sha256='a'*64,worker_sha256='b'*64,source_tree_sha256='c'*64)
        selected=planned_attempts(contract)[:1]
        for value in (dict(passed=False,disposition='passed'),dict(passed=False,interrupted='false')):
            with self.subTest(value=value),tempfile.TemporaryDirectory() as directory,mock.patch('workflow_execution.planned_attempts',return_value=selected):
                root=Path(directory)/'run'
                execute_contract(contract,candidate,{selected[0]['cell']:lambda *args:dict(value)},root,{})
                row=json.loads((root/'attempts.jsonl').read_text().splitlines()[0])
                self.assertEqual(row['disposition'],'failed')
                self.assertIs(row['interrupted'],False)


class PendingOwnershipAccountingTests(TestCase):
    def test_parent_exit_after_snapshot_keeps_child_uncertain_without_signalling(self):
        records={10:((1,1),1),11:((2,2),10)}
        with mock.patch.object(cleanup,'process_identity',side_effect=lambda pid:records.get(pid)):
            process=mock.Mock(pid=10,returncode=0)
            owner=cleanup.NativeCustody(process)
            records.pop(10)
            owner.observe([dict(pid=11,parent_pid=10,kernel_birth=[2,2])]);owner.seal()
            with mock.patch.object(cleanup.os,'kill') as kill, mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
                receipt=cleanup.settle_group(process,timeout=0,custody=owner)
            self.assertFalse(receipt['group_settled']);kill.assert_not_called()
            self.assertEqual(receipt['unresolved_candidates'],[dict(pid=11,birth=[2,2])])
            records.pop(11)
            with mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
                receipt=cleanup.settle_group(process,timeout=0,custody=owner)
            self.assertTrue(receipt['group_settled'],receipt)
            self.assertEqual(receipt['unresolved_candidates'],[])
            self.assertIn(dict(pid=11,birth=[2,2],status='unadmitted_absent',observed_birth=None),receipt['terminal_identity_observations'])

    def test_unadmitted_reused_pid_is_resolved_but_never_signalled(self):
        records={10:((1,1),1),11:((3,3),99)}
        with mock.patch.object(cleanup,'process_identity',side_effect=lambda pid:records.get(pid)):
            owner=cleanup.NativeCustody(mock.Mock(pid=10));records.pop(10)
            owner.observe([dict(pid=11,parent_pid=10,kernel_birth=[2,2])]);owner.seal()
            with mock.patch.object(cleanup.os,'kill') as kill:
                self.assertEqual(owner.live(),[]);owner.signal(signal.SIGKILL,[])
            kill.assert_not_called()
            self.assertEqual(owner.discovery_status(),(True,[]))
            self.assertEqual(owner.evidence()['terminal_identity_observations'][-1]['status'],'unadmitted_reused')

    def test_unadmitted_permission_error_cannot_become_absence(self):
        records={10:((1,1),1)}
        def identity(pid):
            if pid==11:raise PermissionError('denied')
            return records.get(pid)
        with mock.patch.object(cleanup,'process_identity',side_effect=identity):
            process=mock.Mock(pid=10,returncode=0);owner=cleanup.NativeCustody(process);records.pop(10)
            owner.observe([dict(pid=11,parent_pid=10,kernel_birth=[2,2])]);owner.seal()
            with mock.patch.object(cleanup.os,'killpg',side_effect=ProcessLookupError()):
                receipt=cleanup.settle_group(process,timeout=0,custody=owner)
            self.assertFalse(receipt['group_settled'])
            self.assertIn('denied',str(receipt['errors']))
            self.assertEqual(len(receipt['unresolved_candidates']),1)

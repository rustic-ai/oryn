"""Fault controls for durable, exhaustive full-contract scheduling."""
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest import mock
from concurrent.futures import Future

from workflow_contract import DEFAULT_CONTRACT, planned_attempts
from workflow_execution import ExecutionJournal, execute_contract


class WorkflowExecutionTests(unittest.TestCase):
    def test_oracle_publication_failure_closes_journal_and_preserves_denominator(self):
        cell = self.contract['families'][0]['cells'][0]['id']
        adapter = mock.Mock()
        journals = []
        def new_journal(*args):
            journal = ExecutionJournal(*args)
            journals.append(journal)
            return journal
        original_open = Path.open
        def failing_open(path, *args, **kwargs):
            if path.name == 'expected-oracles.json':
                raise OSError('oracle publication failed')
            return original_open(path, *args, **kwargs)
        stop = threading.Event()
        with mock.patch('workflow_execution.ExecutionJournal', side_effect=new_journal), \
                mock.patch.object(Path, 'open', failing_open):
            summary = execute_contract(self.contract, self.candidate, {cell: adapter},
                                       self.root/'oracle-failure', {}, stop=stop)
        adapter.assert_not_called()
        self.assertTrue(stop.is_set())
        self.assertTrue(journals[0].stream.closed)
        self.assertEqual(summary['infrastructure_errors'], ['oracle publication failed'])
        self.assertFalse(summary['structurally_valid'])
        records = self.records('oracle-failure')
        self.assertEqual({row['attempt_id'] for row in records},
                         {row['attempt_id'] for row in planned_attempts(self.contract)})
        self.assertTrue(all(row['passed'] is False for row in records))

    def test_mixed_failure_signals_cancellation_before_executor_join(self):
        stop = threading.Event()
        attempts = [row for row in planned_attempts(self.contract)
                    if row['mode'] == 'mixed_family'][:2]
        failed, queued = Future(), Future()
        failed.set_exception(OSError('journal write failed'))
        pool = mock.Mock()
        pool.submit.side_effect = [failed, queued]
        manager = mock.MagicMock()
        manager.__enter__.return_value = pool
        def joining(*args):
            self.assertTrue(stop.is_set())
            self.assertTrue(queued.cancelled())
            return False
        manager.__exit__.side_effect = joining
        with mock.patch('workflow_execution.planned_attempts', return_value=attempts), \
                mock.patch('workflow_execution.ThreadPoolExecutor', return_value=manager), \
                mock.patch('workflow_execution.as_completed', return_value=iter([failed])):
            summary = execute_contract(self.contract, self.candidate, {},
                                       self.root/'mixed-failure', {}, stop=stop)
        self.assertEqual(summary['infrastructure_errors'], ['journal write failed'])
        self.assertEqual({row['attempt_id'] for row in self.records('mixed-failure')},
                         {row['attempt_id'] for row in attempts})

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.contract = json.loads(DEFAULT_CONTRACT.read_text())
        self.candidate = dict(parent_sha256='a'*64,worker_sha256='b'*64,source_tree_sha256='c'*64)

    def tearDown(self):
        self.directory.cleanup()

    def records(self, name):
        return [json.loads(line) for line in (self.root/name/'attempts.jsonl').read_text().splitlines()]

    def test_missing_adapters_preserve_every_obligation(self):
        summary = execute_contract(self.contract,self.candidate,{},self.root/'missing',{})
        self.assertFalse(summary['structurally_valid'])
        self.assertFalse(summary['family_qualified'])
        self.assertEqual(len(self.records('missing')),len(planned_attempts(self.contract)))
        self.assertEqual(summary['missing_attempts'],[])

    def test_preexecution_cancellation_still_persists_failed_attempts(self):
        stop = threading.Event()
        stop.set()
        execute_contract(self.contract,self.candidate,{},self.root/'cancelled',{},stop=stop)
        self.assertTrue(all('cancellation' in row['errors'][0] for row in self.records('cancelled')))

    def test_adapter_cannot_change_the_frozen_identity_or_claim_success_without_evidence(self):
        cell = self.contract['families'][0]['cells'][0]['id']
        adapters = {cell: lambda attempt, directory, stop: dict(attempt_id='replacement',passed=True)}
        summary = execute_contract(self.contract,self.candidate,adapters,self.root/'identity',{})
        affected = [row for row in self.records('identity') if row['cell']==cell]
        self.assertTrue(all(row['passed'] is False and 'identity' in row['errors'][0] for row in affected))
        self.assertFalse(summary['structurally_valid'])

    def test_changed_candidate_prevents_all_execution(self):
        summary = execute_contract(self.contract,self.candidate,{},self.root/'candidate',{},candidate_probe=lambda: {})
        self.assertTrue(summary['infrastructure_errors'])
        self.assertEqual(len(self.records('candidate')),len(planned_attempts(self.contract)))

    def test_candidate_drift_before_attempt_never_calls_adapter(self):
        calls = []
        probes = iter([self.candidate])
        def probe():
            return next(probes, {})
        cell = self.contract['families'][0]['cells'][0]['id']
        adapters = {cell: lambda *args: calls.append(args)}
        execute_contract(self.contract,self.candidate,adapters,self.root/'drift',{},candidate_probe=probe)
        self.assertEqual(calls, [])
        self.assertTrue(all(row['passed'] is False for row in self.records('drift')))

    def test_transient_preattempt_drift_stops_later_adapters_after_restoration(self):
        for exceptional in (False, True):
            with self.subTest(exceptional=exceptional):
                calls = []
                probes = 0
                stop = threading.Event()
                def probe():
                    nonlocal probes
                    probes += 1
                    if probes == 2:
                        if exceptional:
                            raise ValueError('generation declaration became incomplete')
                        return dict(self.candidate, source_tree_sha256='d' * 64)
                    return self.candidate
                adapters = {row['cell']: lambda *args: calls.append(args)
                            for row in planned_attempts(self.contract)}
                name = 'restored-preattempt-' + str(exceptional)
                execute_contract(self.contract, self.candidate, adapters,
                                 self.root/name, {}, stop=stop, candidate_probe=probe)
                self.assertTrue(stop.is_set())
                self.assertEqual(calls, [])
                records = self.records(name)
                self.assertEqual({row['attempt_id'] for row in records},
                                 {row['attempt_id'] for row in planned_attempts(self.contract)})
                self.assertTrue(all(row['passed'] is False for row in records))
                self.assertTrue(any('adapter failure' in error
                                    for row in records for error in row['errors']))

    def test_journal_rejects_overwrite_and_duplicate_attempts(self):
        journal = ExecutionJournal(self.root/'journal',[],self.candidate)
        try:
            journal.append(dict(attempt_id='one',passed=False))
            with self.assertRaises(ValueError):
                journal.append(dict(attempt_id='one',passed=True))
            with self.assertRaises(FileExistsError):
                ExecutionJournal(self.root/'journal',[],self.candidate)
        finally:
            journal.close()

    def test_journal_retains_persisted_snapshot_after_caller_mutation(self):
        journal = ExecutionJournal(self.root/'immutable', [], self.candidate)
        original = dict(attempt_id='one', passed=False, errors=['original failure'],
                        details=dict(observed=['before']))
        try:
            journal.append(original)
            original['attempt_id'] = 'replacement'
            original['passed'] = True
            original['errors'].clear()
            original['details']['observed'].append('after')
            durable = self.records('immutable')
            self.assertEqual(journal.records, durable)
            self.assertEqual(journal.records[0]['errors'], ['original failure'])
            self.assertEqual(journal.records[0]['details']['observed'], ['before'])
            with self.assertRaises(ValueError):
                journal.append(dict(attempt_id='one', passed=True))
            for identity in (None, '', 1, True):
                with self.subTest(identity=identity), self.assertRaises(ValueError):
                    journal.append(dict(attempt_id=identity, passed=False))
        finally:
            journal.close()

    def test_contract_candidate_and_oracles_are_frozen_before_adapters_run(self):
        cell = self.contract['families'][0]['cells'][0]['id']
        expected_plan = planned_attempts(self.contract)
        original_candidate = dict(self.candidate)
        oracles = {cell: {'value': ['original']}}
        mutated = [False]
        def adapter(*args):
            if not mutated[0]:
                mutated[0] = True
                self.contract['families'][0]['cells'][0]['independent_outcome'] = 'changed'
                self.candidate['source_tree_sha256'] = 'd' * 64
                oracles[cell]['value'].append('changed')
            return dict(passed=False, errors=['intentional fixture failure'])
        summary = execute_contract(self.contract, self.candidate, {cell: adapter},
                                   self.root/'frozen-inputs', oracles)
        self.assertTrue(mutated[0])
        records = self.records('frozen-inputs')
        self.assertEqual(len(records), len(expected_plan))
        planned = {row['attempt_id']: row for row in expected_plan}
        for row in records:
            self.assertEqual(row['source_tree_sha256'], original_candidate['source_tree_sha256'])
            self.assertEqual(row['independent_outcome'], planned[row['attempt_id']]['independent_outcome'])
        self.assertEqual(json.loads((self.root/'frozen-inputs/expected-oracles.json').read_text()),
                         {cell: {'value': ['original']}})
        self.assertFalse(any('attempt specification mismatch' in error for error in summary['errors']))

    def test_malformed_adapter_errors_do_not_break_drift_record_or_denominator(self):
        cell = self.contract['families'][0]['cells'][0]['id']
        changed = [False]
        def adapter(*args):
            changed[0] = True
            return dict(passed=True, errors=None)
        def probe():
            if changed[0]:
                changed[0] = False
                return dict(self.candidate, source_tree_sha256='d' * 64)
            return self.candidate
        summary = execute_contract(self.contract, self.candidate, {cell: adapter},
                                   self.root/'malformed', {}, candidate_probe=probe)
        records = self.records('malformed')
        self.assertEqual(len(records), len(planned_attempts(self.contract)))
        affected = [row for row in records if any('malformed adapter errors' in error for error in row['errors'])]
        self.assertEqual(len(affected), 1)
        self.assertFalse(affected[0]['passed'])
        self.assertTrue(any('post-attempt custody' in error for error in affected[0]['errors']))
        self.assertEqual(summary['infrastructure_errors'], ['execution cancellation requested'])
        self.assertFalse(summary['structurally_valid'])

    def test_transient_post_attempt_drift_is_retained_even_after_bytes_restore(self):
        cell = self.contract['families'][0]['cells'][0]['id']
        changed = [False]
        calls = []
        def adapter(*args):
            calls.append(args)
            changed[0] = True
            return {'passed': True, 'native_observation': 'retained'}
        def probe():
            if changed[0]:
                changed[0] = False
                return dict(self.candidate, source_tree_sha256='d' * 64)
            return self.candidate
        execute_contract(self.contract, self.candidate, {cell: adapter}, self.root/'transient', {}, candidate_probe=probe)
        records = self.records('transient')
        self.assertEqual(len(calls), 1)
        self.assertEqual(len(records), len(planned_attempts(self.contract)))
        affected = [row for row in records if row.get('native_observation') == 'retained']
        self.assertEqual(len(affected), 1)
        self.assertFalse(affected[0]['passed'])
        self.assertTrue(any('post-attempt custody' in error for error in affected[0]['errors']))


class CancellationTerminalRetentionTests(unittest.TestCase):
    def test_completed_wave_then_interrupt_retains_receipts_and_132_denominator(self):
        from workflow_cleanup import NativeExecutionFailure
        contract=json.loads(DEFAULT_CONTRACT.read_text())
        plan=planned_attempts(contract)[:132]
        candidate=dict(parent_sha256='a'*64,worker_sha256='b'*64,source_tree_sha256='c'*64)
        stop=threading.Event()
        def adapter(attempt,directory,stop):
            for index,code in enumerate((0,-9)):
                wave=directory/f'mixed-{index}';wave.mkdir()
                terminal=dict(parent_pid=10+index,returncode=code,primary_error=None if index==0 else 'native executor interrupted',timed_out=False,interrupted=index==1)
                (wave/'native-execution-terminal.json').write_text(json.dumps(terminal))
                (wave/'native-process-cleanup.json').write_text(json.dumps(dict(terminal,process_group=10+index,group_settled=True,errors=[],discovery_sealed=True,unresolved_candidates=[])))
                (wave/'fixture-cleanup.json').write_text(json.dumps(dict(primary_error=terminal['primary_error'],errors=[],custody_lost=False,receipts=[dict(returncode=0,forced=False)])))
            stop.set()
            raise NativeExecutionFailure('native executor interrupted',interrupted=True)
        with tempfile.TemporaryDirectory() as temporary:
            output=Path(temporary)/'run'
            summary=execute_contract(contract,candidate,{p['cell']:adapter for p in plan},output,{},stop=stop,attempt_ids=[p['attempt_id'] for p in plan])
            records=[json.loads(line) for line in (output/'attempts.jsonl').read_text().splitlines()]
            self.assertEqual(len(records),132)
            interrupted=next(r for r in records if r['disposition']=='interrupted')
            self.assertEqual(interrupted['primary_error'],'native executor interrupted')
            self.assertFalse(interrupted['timed_out'])
            self.assertEqual(len(interrupted['terminal_evidence']),6)
            self.assertEqual(sum(r['disposition']=='not_executed' for r in records),131)
            diagnostic=summary['terminal_diagnostics'][interrupted['attempt_id']]
            self.assertEqual(diagnostic['cleanup_status'],'settled',diagnostic)
            self.assertTrue(all(d['cleanup_status']=='not_applicable' for key,d in summary['terminal_diagnostics'].items() if key!=interrupted['attempt_id']))
            self.assertFalse(summary['structurally_valid'])
            self.assertFalse(any('fixture cleanup failed' in e or 'missing artifact reference' in e for e in summary['errors']))

    def test_symlink_terminal_cannot_be_pinned_as_owned_receipt(self):
        from workflow_execution import capture_exception_terminals
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);directory=root/'attempt';directory.mkdir()
            outside=root/'unowned';outside.write_text('{}')
            (directory/'fixture-cleanup.json').symlink_to(outside)
            refs,errors=capture_exception_terminals(directory,root)
            self.assertEqual(refs,[]);self.assertTrue(errors)

"""Negative controls for genuine operation overlap and independent recipe values."""
import json
from pathlib import Path
import tempfile
import threading
import unittest
from unittest import mock
import workflow_native_adapter as native_adapter
from workflow_native_adapter import native_activity, DatabaseCellAdapter, mixed_recipes, ROOT
from workflow_family_recipes import expected, observed


class ExpectedProjectionCacheTests(unittest.TestCase):
    def test_suite_cache_reuses_recipe_and_keeps_context_values_independent(self):
        cache = {}
        adapter = native_adapter.MixedCellAdapter('research.compare', '/native', '/worker', expected_cache=cache)
        with mock.patch.object(native_adapter, 'expected_recipe', return_value={'value': [1]}) as expected:
            first = adapter.expected(20)
            second = adapter.expected(20)
            expected.assert_called_once()
            first[0]['contexts'][0]['value'].append(2)
            self.assertEqual(first[0]['contexts'][1]['value'], [1])
            self.assertEqual(second[0]['contexts'][0]['value'], [1])
            fresh = native_adapter.MixedCellAdapter('research.compare', '/native', '/worker', expected_cache={})
            fresh.expected(1)
            self.assertEqual(expected.call_count, 2)


class WorkerAttestationTests(unittest.TestCase):
    def test_admitted_worker_is_not_rehashed_but_mismatch_still_fails(self):
        runtime = {'sandboxed': True, 'worker_binary_sha256': 'a'*64}
        with mock.patch.object(native_adapter.support, 'digest', side_effect=AssertionError('rehashed')):
            native_adapter.verify_worker_attestation(runtime, Path('/worker'), 'a'*64)
            with self.assertRaisesRegex(ValueError, 'attestation'):
                native_adapter.verify_worker_attestation(runtime, Path('/worker'), 'b'*64)
            with self.assertRaisesRegex(ValueError, 'attestation'):
                native_adapter.verify_worker_attestation(dict(runtime, sandboxed=False), Path('/worker'), 'a'*64)

    def test_standalone_execution_hashes_worker(self):
        with mock.patch.object(native_adapter.support, 'digest', return_value='a'*64) as digest:
            native_adapter.verify_worker_attestation({'sandboxed': True, 'worker_binary_sha256': 'a'*64}, Path('/worker'))
            digest.assert_called_once_with(Path('/worker'))


class NativeExecutionCleanupTests(unittest.TestCase):
    def setUp(self):
        custody = mock.Mock()
        custody.live.return_value = []
        custody.discovery_status.return_value = (True, [])
        patcher = mock.patch.object(native_adapter, "NativeCustody", return_value=custody)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_sampler_start_failure_reaps_child_and_preserves_cleanup_receipt(self):
        process=mock.Mock(pid=123,returncode=-9)
        process.poll.return_value=None
        process.wait.return_value=-9
        sampler=mock.Mock()
        sampler.start.side_effect=RuntimeError('sampler startup failed')
        sampler.finish.return_value={'samples':[],'errors':['not started']}
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with mock.patch.object(native_adapter,'check_execution_generation'), \
                    mock.patch.object(native_adapter.subprocess,'Popen',return_value=process), \
                    mock.patch.object(native_adapter,'ProcessSampler',return_value=sampler), \
                    mock.patch.object(native_adapter.os,'killpg',side_effect=[ProcessLookupError(),ProcessLookupError()]), \
                    self.assertRaisesRegex(RuntimeError,'sampler startup failed'):
                native_adapter.execute_native(Path('/native'),Path('/worker'),{},root,threading.Event())
            process.wait.assert_called_once()
            self.assertGreater(process.wait.call_args.kwargs['timeout'],0)
            self.assertLessEqual(process.wait.call_args.kwargs['timeout'],15)
            sampler.finish.assert_called_once_with()
            receipt=json.loads((root/'native-process-cleanup.json').read_text())
            self.assertEqual(receipt['returncode'],-9)
            self.assertEqual(receipt['errors'],[])
            self.assertIs(receipt['group_settled'],True)
            self.assertEqual(receipt['process_group'],process.pid)
            self.assertTrue((root/'native-resource-samples.json').exists())

    def test_timeout_exit_race_preserves_timeout_and_cleanup_artifacts(self):
        process=mock.Mock(pid=123,returncode=-9)
        process.poll.return_value=None
        process.wait.side_effect=[native_adapter.subprocess.TimeoutExpired('native',180),-9]
        sampler=mock.Mock();sampler.finish.return_value={'samples':[],'errors':[]}
        watcher=mock.Mock();watcher.is_alive.return_value=False
        stop=threading.Event()
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            with mock.patch.object(native_adapter,'check_execution_generation'), \
                    mock.patch.object(native_adapter.subprocess,'Popen',return_value=process), \
                    mock.patch.object(native_adapter,'ProcessSampler',return_value=sampler), \
                    mock.patch.object(native_adapter.threading,'Thread',return_value=watcher), \
                    mock.patch.object(native_adapter.os,'killpg',side_effect=[ProcessLookupError(),ProcessLookupError()]), \
                    self.assertRaisesRegex(RuntimeError,'timed out'):
                native_adapter.execute_native(Path('/native'),Path('/worker'),{},root,stop)
            self.assertEqual(process.wait.call_args_list[0],mock.call(timeout=180))
            self.assertEqual(len(process.wait.call_args_list),2)
            self.assertGreater(process.wait.call_args_list[1].kwargs['timeout'],0)
            self.assertLessEqual(process.wait.call_args_list[1].kwargs['timeout'],15)
            watcher.join.assert_called_once_with(timeout=1)
            self.assertTrue((root/'native-process-cleanup.json').exists())


class NativeActivityTests(unittest.TestCase):
    def evidence(self,spans):
        return dict(native_activity=dict(schema_version=1,overflow=False,operations=spans),
                    sessions=[dict(context_id=1),dict(context_id=2)])
    def span(self,context,start,end,terminal='completed'):
        return dict(context_id=context,start_ns=start,end_ns=end,terminal=terminal,operation_id=start+context)
    def test_idle_envelopes_cannot_manufacture_native_overlap(self):
        evidence=self.evidence([self.span(1,1,3),self.span(1,9,11),self.span(2,4,8)])
        with self.assertRaisesRegex(ValueError,'simultaneous'):native_activity(evidence,2)
    def test_boundary_touching_is_not_simultaneous(self):
        with self.assertRaises(ValueError):native_activity(self.evidence([self.span(1,1,3),self.span(2,3,5)]),2)
    def test_dropped_or_failed_operations_cannot_prove_completed_activity(self):
        for terminal in ('dropped','failed',None):
            with self.assertRaises(ValueError):native_activity(self.evidence([self.span(1,1,5),self.span(2,2,4,terminal)]),2)
    def test_distinct_completed_native_spans_preserve_exact_interval(self):
        activity=native_activity(self.evidence([self.span(1,1,5),self.span(2,2,4)]),2)
        self.assertEqual([(x['start_ns'],x['end_ns']) for x in activity],[(1,5),(2,4)])
    def test_overflow_rejected(self):
        evidence=self.evidence([self.span(1,1,5),self.span(2,2,4)])
        evidence['native_activity']['overflow']=True
        with self.assertRaises(ValueError):native_activity(evidence,2)
    def test_missing_behavior_never_aliases_a_narrower_case(self):
        for cell in ('unknown.family','forms.not_a_cell'):
            with self.assertRaises(ValueError):DatabaseCellAdapter(cell,Path('/native'),Path('/worker'))
    def test_expected_research_facts_come_from_corpus_before_execution(self):
        adapter=DatabaseCellAdapter('research.frames',Path('/native'),Path('/worker'))
        result=adapter.expected(5)
        self.assertEqual(len(result[0]['contexts']),5)
        self.assertIn('Parent access denied',result[0]['contexts'][0]['facts'])
    def test_gap_expectation_never_requires_nonexistent_slot(self):
        result=expected('booking',dict(gap=True),None)
        self.assertEqual(result['reservations'],[])
        self.assertEqual(result['statuses'],[])


class MixedRecipeTests(unittest.TestCase):
    def test_five_and_twenty_context_waves_include_all_five_real_families(self):
        for count in (5,20):
            rows=mixed_recipes(('shopping',2,'framed_persisted_checkout'),count)
            self.assertEqual(len(rows),count)
            self.assertEqual({row[0] for row in rows},{'research','forms','shopping','saas','booking'})
            self.assertEqual(rows[0],('shopping',2,'framed_persisted_checkout'))
    def test_single_context_control_keeps_requested_behavior(self):
        recipe=('forms',3,'multistep_custom_role')
        self.assertEqual(mixed_recipes(recipe,1),[recipe])


class ProbeTopologyTests(unittest.TestCase):
    def test_probes_are_attributed_but_not_counted_as_cohort(self):
        evidence=dict(native_activity=dict(schema_version=1,overflow=False,operations=[dict(context_id=1,start_ns=1,end_ns=5,terminal='completed',operation_id=1),dict(context_id=2,start_ns=2,end_ns=4,terminal='completed',operation_id=2)]),sessions=[dict(id='owner',context_id=1,adversarial_for=None),dict(id='probe',context_id=2,adversarial_for='owner')])
        self.assertEqual(len(native_activity(evidence,1)),1)
        evidence['sessions'][1]['adversarial_for']='absent'
        with self.assertRaises(ValueError):native_activity(evidence,1)


class CommandOutcomeEvidenceTests(unittest.TestCase):
    def test_interleaved_selection_and_structured_extractions_keep_command_order(self):
        from workflow_outcome_evidence import command_frames, extraction_values
        lines=['frame css(#child)','extract text --selector h1','frame parent','extract table --selector table','extract links']
        values=[{'selection':'child'},'facts',{'selection':'parent'},{'columns':['a'],'rows':[[1]]},[{'href':'/source'}]]
        steps=[dict(index=i,outputs=[dict(kind='structured',value=value)]) for i,value in enumerate(values)]
        frames=command_frames(steps,lines)
        self.assertEqual(extraction_values(frames,lines),[values[1],values[3],values[4]])
        self.assertEqual([row['command_provenance']['step'] for row in frames],list(range(5)))
        self.assertEqual([row['command_provenance']['output'] for row in frames],[0]*5)
        for bad in [steps[:-1],steps+steps[-1:], [steps[1],steps[0],*steps[2:]], [dict(steps[0],index=True),*steps[1:]]]:
            with self.assertRaises(ValueError):command_frames(bad,lines)
        for bad in [frames+[frames[-1]], list(reversed(frames)),[dict(frames[1],command_provenance={})], [dict(kind='structured',value='facts')]]:
            with self.assertRaises(ValueError):extraction_values(bad,lines)

    def test_recovery_extraction_keeps_phase_and_value_type(self):
        from workflow_outcome_evidence import command_frames, extraction_values
        lines=['extract text'];recovery=['frame parent','extract table']
        frames=command_frames([dict(index=0,outputs=[dict(kind='structured',value='initial')])],lines)
        frames+=command_frames([dict(index=0,outputs=[dict(kind='structured',value={'frame':0})]),dict(index=1,outputs=[dict(kind='structured',value={'rows':[[2]]})])],recovery,'recovery')
        self.assertEqual(extraction_values(frames,lines),['initial',{'rows':[[2]]}])
        self.assertEqual(frames[-1]['command_provenance'],dict(phase='recovery',step=1,oil='extract table',output=0))

    def test_functional_pass_requires_every_wave_accounting_and_settled_cleanup(self):
        from workflow_outcome_evidence import terminal_verdict
        good=dict(complete=True,errors=[]);closed=dict(group_settled=True,errors=[])
        self.assertTrue(terminal_verdict([], [good,good],[closed,closed])['passed'])
        for accounting in [[good,dict(complete=False,errors=['short worker missing'])],[],[good,None]]:
            result=terminal_verdict([],accounting,[closed])
            self.assertTrue(result['functional_passed']);self.assertFalse(result['resource_complete']);self.assertFalse(result['passed'])
        result=terminal_verdict([], [good],[closed,dict(group_settled=False,errors=['permission denied'])])
        self.assertTrue(result['functional_passed']);self.assertTrue(result['resource_complete']);self.assertFalse(result['cleanup_complete']);self.assertFalse(result['passed'])
        result=terminal_verdict(['original execute deadline'],[good],[closed])
        self.assertEqual(result['operation_errors'],['original execute deadline']);self.assertFalse(result['functional_passed']);self.assertTrue(result['cleanup_complete'])

    def test_session_cleanup_includes_companions_even_when_leader_settled(self):
        evidence=dict(process_cleanup=dict(group_settled=True,errors=[]),sessions=[dict(workers_retired=True,cleanup_errors=[]),dict(workers_retired=False,cleanup_errors=['child pending'])])
        row=native_adapter.session_cleanup(evidence)
        self.assertEqual(row['errors'],['session cleanup: child pending','session workers not retired'])
        self.assertEqual(evidence['process_cleanup']['errors'],[])


class ExtractionCardinalityTests(unittest.TestCase):
    def test_compensated_missing_and_extra_results_cannot_move_between_commands(self):
        from workflow_outcome_evidence import command_frames, extraction_values
        lines=['extract text --selector h1','extract text --selector h2']
        bad=[dict(index=0,outputs=[]),dict(index=1,outputs=[dict(kind='structured',value='one'),dict(kind='structured',value='two')])]
        with self.assertRaisesRegex(ValueError,'exactly one'):command_frames(bad,lines)
        frames=[dict(kind='structured',value=value,command_provenance=dict(phase='steps',step=1,oil=lines[1],output=i)) for i,value in enumerate(['one','two'])]
        with self.assertRaisesRegex(ValueError,'cardinality'):extraction_values(frames,lines)
        for ordinary in [[],[dict(kind='structured',value={}),dict(kind='structured',value=[])]]:
            # Ordinary command output shapes are not extraction cardinality contracts.
            self.assertEqual(len(command_frames([dict(index=0,outputs=ordinary)],['frame parent'])),len(ordinary))


class FailedResearchProjectionTests(unittest.TestCase):
    def test_incomplete_research_retains_original_ipc_failure_without_projection_exception(self):
        from types import SimpleNamespace
        original='sandboxed native worker exceeded wall time (phase=ipc_response, command=execute, message_id=10) limit'
        lines=['extract text --selector "#first"','click "Load foreign source"','extract text --selector "#second"']
        session=dict(context_id=1,errors=['step1: '+original],cleanup_errors=[],workers_retired=True,
            steps=[dict(index=0,error=None,outputs=[dict(kind='structured',value='first')]),dict(index=1,error=original,outputs=[])],recovery=[])
        case=SimpleNamespace(family='research',lines=lines,recovery_steps=[],runner=SimpleNamespace(validate=lambda *args:['missing extraction']),
            case={},identity='testcase',base='http://fixture/research/testcase',name='frames')
        outcome,actions,errors=native_adapter.replay_case_outcome(case,session,dict(visits=[]))
        self.assertTrue(any(original in error for error in errors))
        self.assertIn('native task truncated',errors)
        self.assertTrue(any('research projection:' in error for error in errors))
        self.assertEqual(outcome['outcome'],dict(facts=[],visits=[]));self.assertEqual(actions,[])
        self.assertEqual(session['steps'][0]['outputs'][0]['value'],'first')

    def test_complete_malformed_extraction_remains_failure_instead_of_throwing(self):
        from types import SimpleNamespace
        session=dict(context_id=1,errors=[],cleanup_errors=[],workers_retired=True,steps=[dict(index=0,error=None,outputs=[])],recovery=[])
        case=SimpleNamespace(family='research',lines=['extract table'],recovery_steps=[],runner=SimpleNamespace(validate=lambda *args:['missing extraction']),
            case={},identity='testcase',base='http://fixture/research/testcase',name='frames')
        outcome,actions,errors=native_adapter.replay_case_outcome(case,session,dict(visits=[]))
        self.assertTrue(any('exactly one' in error for error in errors))
        self.assertTrue(errors);self.assertEqual(outcome['outcome']['facts'],[])

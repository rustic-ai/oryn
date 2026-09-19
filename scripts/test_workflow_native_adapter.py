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


class NativeExecutionCleanupTests(unittest.TestCase):
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
                    mock.patch.object(native_adapter.os,'killpg',side_effect=[None,ProcessLookupError()]), \
                    self.assertRaisesRegex(RuntimeError,'sampler startup failed'):
                native_adapter.execute_native(Path('/native'),Path('/worker'),{},root,threading.Event())
            process.wait.assert_called_once_with(timeout=15)
            sampler.finish.assert_called_once_with()
            receipt=json.loads((root/'native-process-cleanup.json').read_text())
            self.assertEqual(receipt['returncode'],-9)
            self.assertEqual(receipt['errors'],[])
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
            self.assertEqual(process.wait.call_args_list,[mock.call(timeout=180),mock.call(timeout=15)])
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

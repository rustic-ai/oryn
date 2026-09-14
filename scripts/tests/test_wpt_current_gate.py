import copy
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_evidence import current_harness_errors


class CurrentGateTests(unittest.TestCase):
    def setUp(self):
        self.row = {'fixture': {'id':'fixture', 'server':'direct_wptserve', 'shutdown_complete':True,
            'generations':[{'backend':'oryn','id':'one'},{'backend':'chromium','id':'two'}]},
            'result':{'testdriver':[]}, 'chromium_reference':{'valid':True, 'same_subtests':True, 'initial_focus_verified':True, 'entry_focus_verified':True, 'entry_focus_phase':'report_ready', 'focus_isolation':'host_serialized',
                'same_statuses':False, 'completion':{'complete':True,'harness':0,'testdriver':[]}}}

    def test_different_engine_status_is_not_a_harness_failure(self):
        self.assertEqual(current_harness_errors(self.row), [])

    def test_reference_requires_verified_initial_focus(self):
        for value in (None, False, 1, 'true'):
            self.row['chromium_reference']['initial_focus_verified'] = value
            self.assertIn('reference_initial_focus_unverified', current_harness_errors(self.row))

    def test_reference_requires_host_focus_isolation(self):
        self.row['chromium_reference'].pop('focus_isolation')
        self.assertIn('reference_focus_isolation_unverified', current_harness_errors(self.row))

    def test_pre_navigation_focus_does_not_prove_document_entry_focus(self):
        self.row['chromium_reference'].pop('entry_focus_verified')
        self.assertIn('reference_entry_focus_unverified', current_harness_errors(self.row))

    def test_document_creation_is_not_the_test_entry_phase(self):
        self.row['chromium_reference']['entry_focus_phase'] = 'document_created'
        self.assertIn('reference_focus_phase_unverified', current_harness_errors(self.row))

    def test_reused_fixture_identity_rejected(self):
        self.row['fixture']['generations'][1]['id'] = 'one'
        self.assertIn('fixture_isolation_unverified', current_harness_errors(self.row))

    def test_missing_or_failed_teardown_rejected(self):
        for complete in (None, False):
            self.row['fixture']['shutdown_complete'] = complete
            self.assertIn('fixture_integrity_unverified', current_harness_errors(self.row))

    def test_reference_failure_is_independent_of_engine_outcome(self):
        self.row['chromium_reference']['error'] = 'timeout'
        self.assertIn('reference_completion_unverified', current_harness_errors(self.row))

    def test_operations_require_exactly_one_matching_terminal(self):
        log = [{'id':1,'operation':'click','backend':'oryn','outcome':'started'},
               {'id':1,'operation':'click','backend':'oryn','outcome':'completed'}]
        self.row['result']['testdriver'] = log
        self.assertEqual(current_harness_errors(self.row), [])
        for terminal in ('unsupported','failed','started'):
            row = copy.deepcopy(self.row)
            row['result']['testdriver'][1]['outcome'] = terminal
            self.assertTrue(current_harness_errors(row))
        log.append(log[-1])
        self.assertIn('automation_trace_unverified', current_harness_errors(self.row))

    def test_old_evidence_does_not_establish_current_gate(self):
        self.assertIn('fixture_integrity_unverified', current_harness_errors({}))

    def test_native_cross_realm_operations_are_disambiguated_and_complete(self):
        events = [{'id':1,'operation':'action_sequence','backend':'oryn',
                   'context_id':realm,'document_generation':1,'outcome':outcome}
                  for realm in (0,1) for outcome in ('started','completed')]
        for sequence,event in enumerate(events): event['sequence']=sequence
        self.row['result'].update(testdriver=events,automation_trace={
            'schema_version':1,'scope':'page_isolate','complete':True,'overflow':False,'events':events})
        self.assertEqual(current_harness_errors(self.row),[])
        self.row['result']['automation_trace']['complete']=False
        self.assertIn('native_automation_trace_incomplete',current_harness_errors(self.row))
        self.row['result']['automation_trace']['complete']=True
        events[2]['context_id']=0
        self.assertIn('automation_trace_unverified',current_harness_errors(self.row))

    def test_malformed_trace_rejects_instead_of_crashing_validation(self):
        for events in ('not an array', [None], [1], [{'id':0,'operation':'click','backend':'oryn','outcome':'started'}]):
            self.row['result']['testdriver']=events
            self.assertIn('automation_trace_unverified',current_harness_errors(self.row))

    def test_reference_bridge_failure_and_overflow_reject(self):
        self.row['chromium_reference']['automation_bridge_errors'] = ['child_bridge_initialization_failed']
        self.assertTrue(current_harness_errors(self.row))
        self.row['chromium_reference'].pop('automation_bridge_errors')
        self.row['chromium_reference']['automation_overflow'] = True
        self.assertTrue(current_harness_errors(self.row))

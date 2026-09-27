import unittest
from workflow_failures import attribute_native_failure

class FailureAttributionTests(unittest.TestCase):
    def test_expected_negative_is_not_failure_and_cleanup_cannot_be_hidden(self):
        result=attribute_native_failure(dict(sessions=[dict(id='booking',context_id=2,page_id=9,steps=[dict(index=4,error='stale reference',expected_error_matched=True)],errors=[],cleanup_errors=['worker still alive'],workers_retired=False)]))
        self.assertEqual([row['stage'] for row in result['failures']],['native_teardown','native_teardown'])
        self.assertFalse(result['root_causes_confirmed'])
    def test_timeout_does_not_claim_a_wpt_or_shared_root_cause(self):
        result=attribute_native_failure(dict(sessions=[dict(id='booking',context_id=2,page_id=9,steps=[dict(index=4,error='deadline',timed_out=True,expected_error_matched=False)],workers_retired=True)]))
        self.assertEqual(result['failures'][0]['stage'],'native_operation');self.assertIsNone(result['failures'][0]['confirmed_cause'])

class FollowupObservationAttributionTests(unittest.TestCase):
    diagnostic = ('Action completed; follow-up observation unavailable: '
                  'sandboxed native worker exceeded wall time '
                  '(phase=ipc_response, command=observe, message_id=5) limit. Cached aliases were cleared.')

    def native(self):
        return dict(sessions=[dict(id='context-8', context_id=13, page_id=13,
            steps=[dict(index=1, error=None, expected_error_matched=True, timed_out=False,
                        outputs=[dict(kind='action', result=dict(action_id=1, page_id=13,
                            diagnostics=[self.diagnostic], effects=[dict(kind='navigation',url='/sso')]))]),
                   dict(index=2, error='invalid page command: browser context has no active page',
                        expected_error_matched=False, timed_out=False, outputs=[])],
            errors=[],cleanup_errors=[],workers_retired=True)])

    def test_earliest_followup_deadline_exposed_without_reclassifying_action(self):
        import copy
        native=self.native();original=copy.deepcopy(native)
        result=attribute_native_failure(native)
        self.assertEqual(native,original)
        self.assertEqual(len(result['failures']),1)
        self.assertEqual(result['failures'][0]['step'],2)
        row=result['first_followup_observation_per_session'][0]
        self.assertEqual((row['action_id'],row['step'],row['command'],row['phase'],row['message_id']),
                         (1,1,'observe','ipc_response',5))
        self.assertTrue(row['action_completed'])
        self.assertTrue(row['observation_timed_out'])
        self.assertIsNone(row['timeout_limit_ms'])
        self.assertIsNone(row['confirmed_cause'])
        link=result['failures'][0]['preceding_followup_observation']
        self.assertEqual(link['source_pointer'],'/sessions/0/steps/0/outputs/0/result/diagnostics/0')
        self.assertFalse(result['root_causes_confirmed'])

    def test_later_unrelated_failure_does_not_inherit_old_diagnostic(self):
        native=self.native()
        native['sessions'][0]['steps'].insert(1,dict(index=99,error=None,outputs=[]))
        result=attribute_native_failure(native)
        self.assertNotIn('preceding_followup_observation',result['failures'][0])

    def test_completed_close_observation_error_is_not_guessed_timeout(self):
        native=self.native()
        native['sessions'][0]['steps'][0]['outputs'][0]['result']['diagnostics']=[
            'Action completed; follow-up observation unavailable: page closed. Cached aliases were cleared.']
        result=attribute_native_failure(native)
        row=result['followup_observations'][0]
        self.assertTrue(row['action_completed'])
        self.assertFalse(row['observation_timed_out'])
        self.assertIsNone(row['message_id'])
        self.assertNotIn('preceding_followup_observation',result['failures'][0])

    def test_non_action_and_failed_action_cannot_claim_completed_action(self):
        for mutation in ('output_kind','step_error','action_id'):
            native=self.native();step=native['sessions'][0]['steps'][0]
            if mutation=='output_kind':step['outputs'][0]['kind']='text'
            elif mutation=='step_error':step['error']='failed action'
            else:step['outputs'][0]['result']['action_id']=True
            with self.subTest(mutation=mutation):
                self.assertEqual(attribute_native_failure(native)['followup_observations'],[])

    def test_sessions_do_not_share_followup_predecessors(self):
        native=self.native();native['sessions'][0]['steps'].pop()
        native['sessions'].append(dict(id='different',context_id=14,page_id=14,
            steps=[dict(index=2,error='invalid page command: browser context has no active page',
                        expected_error_matched=False)],workers_retired=True))
        result=attribute_native_failure(native)
        self.assertNotIn('preceding_followup_observation',result['failures'][0])

    def test_only_first_observation_per_session_is_promoted_but_all_are_retained(self):
        import copy
        native=self.native();step=copy.deepcopy(native['sessions'][0]['steps'][0])
        step['index']=3;step['outputs'][0]['result']['action_id']=2
        native['sessions'][0]['steps'].append(step)
        result=attribute_native_failure(native)
        self.assertEqual(len(result['followup_observations']),2)
        self.assertEqual(len(result['first_followup_observation_per_session']),1)
        self.assertEqual(result['first_followup_observation_per_session'][0]['action_id'],1)

    def test_repeated_or_missing_step_indexes_cannot_bridge_intervening_step(self):
        for indexes in ((1, 1, 2), (None, None, None)):
            with self.subTest(indexes=indexes):
                native=self.native()
                steps=native['sessions'][0]['steps']
                steps.insert(1,dict(error=None,outputs=[]))
                for step,index in zip(steps,indexes):
                    if index is None:
                        step.pop('index',None)
                    else:
                        step['index']=index
                result=attribute_native_failure(native)
                self.assertNotIn('preceding_followup_observation',result['failures'][0])

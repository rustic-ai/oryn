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

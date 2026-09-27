import unittest

from oryn_validation.results import reconcile_wpt


class WptResultsTests(unittest.TestCase):
    def events(self, status='OK'):
        return [{'action': 'suite_start'}, {'action': 'test_start', 'test': '/a'},
                {'action': 'test_status', 'test': '/a', 'status': 'FAIL'},
                {'action': 'test_end', 'test': '/a', 'status': status}, {'action': 'suite_end'}]

    def reconcile(self, events, **changes):
        execution = dict(returncode=1, error=None, timed_out=False, cleanup_complete=True)
        execution.update(changes)
        return reconcile_wpt(['/a'], events, execution=execution)

    def test_observed_failure_does_not_invalidate_complete_original(self):
        result = self.reconcile(self.events())
        self.assertTrue(result['compatibility_complete'])
        self.assertEqual(result['outcomes'], {'OK': 1})
        self.assertEqual(result['subtest_outcomes'], {'FAIL': 1})
        self.assertEqual(result['assessment_total'], 752)
        self.assertEqual(result['qualification'], 'not-evaluated')

    def test_no_stitching_or_retry_duplicates(self):
        self.assertFalse(self.reconcile(self.events() + self.events())['compatibility_complete'])

    def test_missing_end_and_cleanup_failure_remain_incomplete(self):
        result = self.reconcile(self.events()[:2])
        self.assertEqual(result['missing_ends'], ['/a'])
        self.assertFalse(result['compatibility_complete'])
        self.assertFalse(self.reconcile(self.events(), cleanup_complete=False)['compatibility_complete'])
        self.assertFalse(self.reconcile(self.events(), timed_out=True)['compatibility_complete'])

    def test_end_before_start_and_unselected_url_rejected(self):
        events = self.events()
        events[1], events[3] = events[3], events[1]
        self.assertFalse(self.reconcile(events)['compatibility_complete'])
        events = self.events()
        events[3] = dict(action='test_end', test='/other', status='OK')
        self.assertFalse(self.reconcile(events)['compatibility_complete'])

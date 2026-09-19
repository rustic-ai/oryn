import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_canary_qualification as canary


class CanaryQualificationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.candidate = 'a' * 64
        self.report = {'run_info': {'product': 'oryn', 'oryn_candidate_sha256': self.candidate,
                       'oryn_executor': 'upstream-wptrunner', 'oryn_wpt_revision': canary.REVISION}, 'results': []}
        for path, (status, name, substatus) in canary.EXPECTED.items():
            self.report['results'].append({'test': canary.MOUNT + path, 'status': status,
                                          'subtests': [{'name': name, 'status': substatus}] if name else []})

    def records(self):
        report = self.root / 'wptreport.json'
        report.write_text(json.dumps(self.report))
        execution = self.root / 'execution.json'
        execution.write_text(json.dumps({'candidate_sha256': self.candidate, 'collection': 'canaries',
                                         'returncode': 1, 'custody_complete': True, 'cleanup_complete': True, 'timed_out': False, 'errors': [], 'artifacts': {'wptreport.json': canary.sha256(report)}}))
        audit = self.root / 'audit.json'
        audit.write_text(json.dumps({'candidate_sha256': self.candidate, 'report_sha256': canary.sha256(report),
                                     'cleanup_complete': True, 'all_workers_reaped': True, 'native_reference_rendering': True,
                                     'resources': {canary.MOUNT + 'missing-helper.html': [
                                         {'path': '/__oryn_qualification_missing_helper__.js', 'status': 404}]}}))
        return report, execution, audit

    def test_expected_failures_are_rejection_evidence_not_core_passes(self):
        result = canary.qualify(*self.records(), self.candidate)
        self.assertTrue(result['controls_rejected'])
        self.assertEqual(len(result['controls']), 5)
        self.assertIn('separately required', result['scope'])

    def test_each_incorrectly_passing_control_rejected(self):
        for index in range(5):
            original = copy.deepcopy(self.report)
            self.report['results'][index]['status'] = 'PASS'
            result = canary.qualify(*self.records(), self.candidate)
            self.assertFalse(result['controls_rejected'])
            self.report = original

    def test_missing_control_never_qualifies(self):
        self.report['results'].pop()
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            canary.qualify(*self.records(), self.candidate)

    def test_cleanup_and_missing_helper_must_be_observed(self):
        report, execution, audit = self.records()
        record = json.loads(audit.read_text())
        record['cleanup_complete'] = False
        record['resources'] = {}
        audit.write_text(json.dumps(record))
        result = canary.qualify(report, execution, audit, self.candidate)
        self.assertFalse(result['controls_rejected'])
        self.assertIn('cleanup_not_proven', result['errors'])
        self.assertIn('missing_helper_failure_not_observed', result['controls'][1]['errors'])


if __name__ == '__main__':
    unittest.main()

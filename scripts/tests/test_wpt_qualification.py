import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
spec = importlib.util.spec_from_file_location('qualify', Path(__file__).resolve().parents[1] / 'qualify-wpt-evidence.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.row = {'status': 'passed', 'resource_evidence_complete': True,
            'execution': {'id': 'test.html|window|:nonce', 'completion_valid': True, 'parent_shutdown': {
                'returncode': 0, 'shutdown_acknowledged': True}}, 'result': {
            'complete': True, 'completion_count': 1, 'harness': 0,
            'tests': [{'name': 'one', 'status': 0}]}}
        self.resource = {'sequence': 0, 'page_id': 1, 'worker_pid': 42,
            'url': 'http://localhost/test.html', 'query_sha256': None, 'status': 200,
            'mime': 'text/html', 'response_sha256': 'a' * 64, 'terminal': 'complete',
            'elapsed_ms': 1, 'destination': 'Document'}
        self.row['resources'] = [self.resource]
        self.review = {'expected_subtests': 1, 'verified_resources': {'0': {
            key: self.resource[key] for key in ('url', 'query_sha256', 'status', 'mime', 'response_sha256', 'terminal')}}}

    def test_historical_and_incomplete_resource_evidence_stays_unresolved(self):
        for field in ('execution', 'resource_evidence_complete'):
            row = copy.deepcopy(self.row)
            del row[field]
            self.assertEqual(module.classify(row, {'expected_subtests': 1})[0], 'unresolved')

    def test_pass_is_only_candidate_without_audit(self):
        self.assertEqual(module.classify(self.row)[0], 'review_candidate')

    def test_reviewed_native_calls_cannot_be_satisfied_by_an_empty_parent_log(self):
        self.review['expected_native_automation_calls']=1
        self.assertIn('reviewed_native_automation_missing',module.classify(self.row,self.review)[1])
        self.row['result']['automation_trace']={'complete':True,'scope':'page_isolate'}
        self.assertIn('reviewed_native_automation_missing',module.classify(self.row,self.review)[1])

    def test_fail_and_pass_use_identical_qualification_gate(self):
        for status, subtest in [('passed', 0), ('failed', 1)]:
            row = copy.deepcopy(self.row)
            row['status'] = status
            row['result']['tests'][0]['status'] = subtest
            self.assertEqual(module.classify(row, self.review), ('qualified', []))

    def test_uncertain_completion_cannot_be_overridden_by_review(self):
        for patch in [{'complete': False}, {'harness': 1}, {'tests': []},
                      {'tests': [{'status': 2}]}, {'tests': [{'status': 3}]}]:
            row = copy.deepcopy(self.row)
            row['result'].update(patch)
            self.assertEqual(module.classify(row, {'expected_subtests': 1})[0], 'unresolved')

    def test_diagnostic_blockers_cannot_be_overridden(self):
        for tag in ['harness_timeout', 'worker_wall_limit', 'outer_timeout',
                    'reported_worker_crash', 'generated_operation_wall_limit',
                    'testdriver_operation_not_implemented']:
            row = dict(self.row, diagnostic_tags=[tag])
            self.assertEqual(module.classify(row, {'expected_subtests': 1})[0], 'unresolved')

    def test_count_and_outcome_mismatches(self):
        self.assertIn('audited_subtest_count_mismatch', module.classify(self.row, {'expected_subtests': 2})[1])
        self.assertIn('outcome_inconsistent', module.classify(dict(self.row, status='failed'))[1])

    def test_environment_is_separate(self):
        self.assertEqual(module.classify({'status': 'environment_blocked', 'reason': 'ip_spaces'}),
                         ('environment_blocked', ['ip_spaces']))

    def test_hash_bound_overlay_and_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'test.html'
            source.write_text('<script>test(() => {});</script>')
            evidence = root / 'evidence.json'
            row = dict(self.row, id='test.html|window|', path='test.html')
            evidence.write_text(json.dumps({'assertions': [row], 'run_fingerprint': 'run',
                'binary': {'sha256': 'parent'}, 'runtime_metadata': {'worker_binary_sha256': 'worker'}}))
            origin = {'path': str(evidence), 'sha256': module.digest(evidence),
                      'run_fingerprint': 'run', 'binary': {'sha256': 'parent'}, 'worker_sha256': 'worker'}
            ledger = root / 'ledger.jsonl'
            ledger.write_text(json.dumps({'id': row['id'], 'status': 'passed', 'original': origin}))
            summary = root / 'summary.json'
            summary.write_text(json.dumps({'complete': True, 'selected': 1, 'ledger_sha256': module.digest(ledger)}))
            review = {'evidence_sha256': module.digest(evidence),
                      'source_files': {'test.html': module.digest(source)},
                      'fixture_audit': 'Self-contained', 'automation_audit': 'None',
                      'verified_dependencies': [], 'expected_subtests': 1}
            review.update(self.review)
            rows, report = module.build(summary, ledger, root, {row['id']: review})
            self.assertEqual(report['counts'], {'qualified': 1})
            self.assertEqual(rows[0]['status'], 'passed')
            for changed, expected in [
                    ({'evidence_sha256': 'stale'}, 'stale review'),
                    ({'verified_dependencies': ['/missing.js']}, 'dependency not verified'),
                    ({'source_files': {'test.html': 'stale'}}, 'source hash mismatch'),
                    ({'automation_audit': ''}, 'explicit fixture')]:
                with self.assertRaisesRegex(ValueError, expected):
                    module.build(summary, ledger, root, {row['id']: dict(review, **changed)})
            evidence.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'source hash mismatch'):
                module.build(summary, ledger, root, {})

    def test_incomplete_resource_and_automation_audits_fail_closed(self):
        row = copy.deepcopy(self.row)
        self.assertIn('observed_prerequisite_audit_incomplete',
            module.classify(row, {'expected_subtests': 1})[1])
        row['dependencies'] = ['/required.js']
        self.assertIn('static_prerequisite_audit_incomplete', module.classify(row, self.review)[1])
        row = copy.deepcopy(self.row)
        row['result']['testdriver'] = [{'operation': 'click'}]
        self.assertIn('automation_operation_audit_incomplete', module.classify(row, self.review)[1])

    def test_expected_error_allowance_is_bound_to_exact_response(self):
        row = copy.deepcopy(self.row)
        row['resources'][0]['status'] = 404
        review = copy.deepcopy(self.review)
        review['verified_resources']['0']['status'] = 404
        self.assertIn('unexpected_resource_error', module.classify(row, review)[1])
        review['expected_resource_errors'] = {'0': {'observation': dict(review['verified_resources']['0']),
                                                    'reason': 'Intentional missing resource fixture'}}
        self.assertEqual(module.classify(row, review), ('qualified', []))
        review['expected_resource_errors']['0']['observation']['status'] = 500
        self.assertIn('unexpected_resource_error', module.classify(row, review)[1])

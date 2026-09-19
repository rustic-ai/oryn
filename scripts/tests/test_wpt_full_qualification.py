"""Negative controls for official full-manifest qualification (source only)."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_full_qualification as gate


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'dom').mkdir()
        (self.root / 'dom/a.any.js').write_text('test(() => assert_true(true), "positive");')
        self.source_hash = gate.sha256(self.root / 'dom/a.any.js')
        self.row = {'id': 'testharness:/dom/a.any.worker.html?x=1',
                    'url': '/dom/a.any.worker.html?x=1', 'path': 'dom/a.any.js',
                    'test_type': 'testharness', 'tier': 'core', 'root': 'dom',
                    'sha256': self.source_hash, 'environment_requirements': []}
        self.core = {self.row['id']: self.row}
        self.candidate = 'a' * 64
        navigation = {'schema_version': 1, 'test': self.row['url'], 'interval_start': 'session-start',
                      'before': {'schema_version': 1, 'pages': []}, 'after': {'schema_version': 1, 'pages': []}}
        self.report = {'run_info': {'oryn_rendering_contracts': gate.render_contracts(), 'product': 'oryn', 'oryn_candidate_sha256': self.candidate,
                       'oryn_wpt_revision': gate.REVISION, 'oryn_executor': 'upstream-wptrunner',
                       'oryn_fixture_repairs_sha256': gate.sha256(Path(gate.__file__).resolve().parents[1] / 'tools/wptrunner-oryn/oryn_wptrunner/fixture-repairs.json')},
                       'results': [{'test': self.row['url'], 'status': 'OK',
                                    'subtests': [{'name': 'positive', 'status': 'PASS'}], 'extra': {'oryn_navigation': navigation}}]}

    def write_report(self, report, name='report.json'):
        path = self.root / name
        path.write_text(json.dumps(report))
        return path, report

    def review(self, report_path):
        digest = gate.sha256(report_path)
        value = {'id': self.row['id'], 'report_sha256': digest,
                 'candidate_sha256': self.candidate, 'source_sha256': self.source_hash,
                 'source_files': {self.row['path']: self.source_hash},
                 'expected_subtests': ['positive']}
        for key in ('reviewer', 'fixture_audit', 'automation_audit', 'cleanup_evidence',
                    'negative_control_evidence', 'resource_audit'):
            value[key] = 'independent audit record'
        for key in ('tls_validation_verified', 'focus_verified', 'resources_complete',
                    'cleanup_complete', 'negative_controls_rejected'):
            value[key] = True
        value['evidence_artifacts'] = {label: {'path': str(report_path), 'sha256': digest}
                                       for label in ('cleanup', 'negative_controls', 'resources', 'tls', 'focus')}
        return {self.row['id']: {digest: value}}

    def evaluate(self, report=None, review=True):
        pair = self.write_report(report or self.report)
        reviews = self.review(pair[0]) if review else {}
        return gate.reconcile(self.core, [pair], reviews, self.candidate, self.root)

    def test_raw_pass_requires_explicit_review(self):
        rows, summary = self.evaluate(review=False)
        self.assertTrue(rows[0]['attempts'][0]['raw_pass'])
        self.assertEqual(rows[0]['state'], 'open')
        self.assertFalse(summary['complete'])

    def test_valid_review_preserves_raw_and_qualified_separately(self):
        rows, summary = self.evaluate()
        self.assertEqual(rows[0]['state'], 'qualified_pass')
        self.assertTrue(summary['complete'])
        self.assertEqual(rows[0]['attempts'][0]['raw_subtests'], self.report['results'][0]['subtests'])

    def test_missing_duplicate_renamed_or_failed_subtest_stays_open(self):
        cases = [[], [{'name': 'different', 'status': 'PASS'}],
                 [{'name': 'positive', 'status': 'PASS'}] * 2,
                 [{'name': 'positive', 'status': 'FAIL'}],
                 [{'name': 'positive', 'status': 'MAYBE'}], None, ['malformed']]
        for subtests in cases:
            with self.subTest(subtests=subtests):
                report = copy.deepcopy(self.report)
                report['results'][0]['subtests'] = subtests
                rows, summary = self.evaluate(report)
                self.assertEqual(rows[0]['state'], 'open')
                self.assertFalse(summary['complete'])

    def test_wrong_candidate_product_or_executor_rejected(self):
        for key in ('product', 'oryn_candidate_sha256', 'oryn_wpt_revision', 'oryn_executor'):
            report = copy.deepcopy(self.report)
            report['run_info'][key] = 'wrong'
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.evaluate(report)

    def test_report_missing_fragment_ownership_or_old_display_list_is_rejected(self):
        for contracts in ({}, {'render_contract_version': 2, 'local_effects_contract_version': 1},
                          {'render_contract_version': 3},
                          {'render_contract_version': 3, 'local_effects_contract_version': True}):
            report = copy.deepcopy(self.report)
            report['run_info']['oryn_rendering_contracts'] = contracts
            with self.subTest(contracts=contracts), self.assertRaisesRegex(ValueError, 'rendering contract'):
                self.evaluate(report)

    def test_unknown_duplicate_or_variant_dropped_id_rejected(self):
        for rows in [[dict(self.report['results'][0], test='/dom/a.any.worker.html')], self.report['results'] * 2]:
            report = dict(self.report, results=rows)
            with self.assertRaises(ValueError):
                self.evaluate(report)

    def test_source_change_invalidates_existing_review(self):
        pair = self.write_report(self.report)
        reviews = self.review(pair[0])
        (self.root / self.row['path']).write_text('changed test')
        rows, _ = gate.reconcile(self.core, [pair], reviews, self.candidate, self.root)
        self.assertEqual(rows[0]['state'], 'open')

    def test_each_prerequisite_and_cleanup_is_mandatory(self):
        pair = self.write_report(self.report)
        for field in ('tls_validation_verified', 'focus_verified', 'resources_complete',
                      'cleanup_complete', 'negative_controls_rejected'):
            reviews = self.review(pair[0])
            reviews[self.row['id']][gate.sha256(pair[0])][field] = False
            rows, _ = gate.reconcile(self.core, [pair], reviews, self.candidate, self.root)
            self.assertEqual(rows[0]['state'], 'open', field)

    def test_retry_does_not_erase_failure(self):
        failed = copy.deepcopy(self.report)
        failed['results'][0]['subtests'][0]['status'] = 'FAIL'
        first = self.write_report(failed, 'failed.json')
        second = self.write_report(self.report, 'passed.json')
        reviews = self.review(first[0])
        reviews[self.row['id']].update(self.review(second[0])[self.row['id']])
        rows, _ = gate.reconcile(self.core, [first, second], reviews, self.candidate, self.root)
        self.assertEqual(len(rows[0]['attempts']), 2)
        self.assertEqual(rows[0]['state'], 'open')

    def test_missing_and_manual_entries_stay_in_denominator(self):
        manual = dict(self.row, id='manual:/dom/manual.html', url='/dom/manual.html', test_type='manual')
        self.core[manual['id']] = manual
        rows, summary = gate.reconcile(self.core, [], {}, self.candidate, self.root)
        self.assertEqual(summary['core_total'], 2)
        self.assertEqual({row['state'] for row in rows}, {'assessment_required', 'not_run'})
        self.assertFalse(summary['complete'])

    def test_reftest_needs_native_comparison_and_readiness(self):
        self.row.update(id='reftest:/dom/a.html', url='/dom/a.html', test_type='reftest')
        self.core = {self.row['id']: self.row}
        self.report['results'] = [{'test': self.row['url'], 'status': 'PASS', 'subtests': []}]
        rows, _ = self.evaluate()
        self.assertIn('native_reference_readiness_unverified', rows[0]['attempts'][0]['qualification_errors'])

    def test_assessment_requires_independent_reviewer_and_bound_artifacts(self):
        self.row.update(id='manual:/dom/a.any.js', url='/dom/a.any.js', test_type='manual')
        self.core = {self.row['id']: self.row}
        artifact = self.root / 'observations.txt'
        artifact.write_text('independent observed behavior and procedure')
        procedure = self.root / 'procedures.jsonl'
        procedure.write_text(json.dumps({'id': self.row['id'], 'url': self.row['url'], 'path': self.row['path'],
            'source_sha256': self.source_hash, 'inventory_source_sha256': self.source_hash,
            'upstream_revision': gate.REVISION, 'procedure': ['source instructions'], 'expected_observations': ['native output'],
            'required_platforms': [{'platform': 'macOS'}], 'cleanup': ['retire document']}))
        import wpt_manual_procedures
        procedure_patch = patch.object(wpt_manual_procedures, 'DEFAULT_CORPUS', procedure)
        procedure_patch.start()
        self.addCleanup(procedure_patch.stop)
        assessment = {'id': self.row['id'], 'candidate_sha256': self.candidate,
                      'source_sha256': self.source_hash, 'upstream_revision': gate.REVISION,
                      'independent': True, 'operator': 'operator', 'reviewer': 'reviewer',
                      'platform': 'macOS', 'platform_selection': {'platform': 'macOS'}, 'procedure': 'follow every source instruction',
                      'observations': 'recorded outcomes', 'assistive_technology': 'not applicable with rationale',
                      'cleanup_complete': True, 'status': 'PASS',
                      'procedure_source': {'path': str(procedure), 'sha256': gate.sha256(procedure)},
                      'platform_observations': [{'requirement': {'platform': 'macOS'}, 'observations': 'native output', 'artifacts': [{'path': str(artifact), 'sha256': gate.sha256(artifact)}]}],
                      'artifacts': [{'path': str(artifact), 'sha256': gate.sha256(artifact)}]}
        import wpt_assessment_packet
        packet_patch = patch.object(wpt_assessment_packet, 'DEFAULT_CORPUS', procedure)
        packet_patch.start(); self.addCleanup(packet_patch.stop)
        packet_path = self.root / 'packet.json'
        packet_path.write_text(json.dumps(wpt_assessment_packet.create(self.row['id'], self.candidate, self.root, 'macOS')))
        packet_ref = {'path': str(packet_path), 'sha256': gate.sha256(packet_path)}
        assessment['assessment_packet'] = packet_ref
        assessment['artifacts'].append(packet_ref)
        rows, summary = gate.reconcile(self.core, [], {}, self.candidate, self.root,
                                       assessments={self.row['id']: [assessment]})
        self.assertEqual(rows[0]['state'], 'qualified_pass')
        self.assertTrue(summary['complete'])
        assessment['reviewer'] = assessment['operator']
        rows, summary = gate.reconcile(self.core, [], {}, self.candidate, self.root,
                                       assessments={self.row['id']: [assessment]})
        self.assertEqual(rows[0]['state'], 'open')
        self.assertFalse(summary['complete'])

    def test_manual_record_cannot_waive_automatable_failure(self):
        with self.assertRaisesRegex(ValueError, 'cannot waive'):
            gate.reconcile(self.core, [], {}, self.candidate, self.root,
                           assessments={self.row['id']: [{'status': 'PASS'}]})

    def test_actual_manifest_blob_rejects_source_mutation(self):
        content = (self.root / self.row['path']).read_bytes()
        blob = hashlib.sha1(b'blob ' + str(len(content)).encode() + b'\0' + content).hexdigest()
        manifest = self.root / 'MANIFEST.json'
        manifest.write_text(json.dumps({'version': 9, 'items': {'testharness': {'dom': {
            'a.any.js': [blob, [self.row['url'], {}]]}}}}))
        policy = Path(__file__).resolve().parents[2] / 'benchmarks/conformance/wpt-capability-tiers-v2.json'
        with patch.object(gate, 'MANIFEST_SHA256', gate.sha256(manifest)):
            gate.bind_inventory(self.core, manifest, policy, self.root)
            (self.root / self.row['path']).write_text('mutated after manifest pin')
            self.row['sha256'] = gate.sha256(self.root / self.row['path'])
            with self.assertRaisesRegex(ValueError, 'pinned manifest blob'):
                gate.bind_inventory(self.core, manifest, policy, self.root)

    def test_pinned_denominator_cannot_be_replaced_by_old_backlog(self):
        plan = {'upstream_revision': gate.REVISION, 'official_manifest_sha256': gate.MANIFEST_SHA256,
                'policy_sha256': gate.POLICY_SHA256, 'total_manifest_tests': 1}
        with self.assertRaisesRegex(ValueError, 'denominator'):
            gate.validate_inventory([self.row], plan)
        with patch.object(gate, 'CORE_TOTAL', 1):
            self.assertEqual(gate.validate_inventory([self.row], plan), self.core)
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                gate.validate_inventory([self.row, self.row], plan)




class NavigationEvidenceTests(unittest.TestCase):
    def evidence(self):
        before = {'page_id': 1, 'snapshot': {'retired': False, 'active': 0, 'pruned_outcomes': 0, 'outcomes': []}}
        after = copy.deepcopy(before)
        after['snapshot']['outcomes'] = [{'stage': 'Execution', 'id': 1, 'target': 'native-frame', 'result': {'Err': 'navigation rejected'}, 'error_truncated': False, 'execution': {'Failed': {'message': 'navigation rejected'}}}]
        return {'extra': {'oryn_navigation': {'schema_version': 1, 'test': '/a', 'interval_start': 'session-start', 'before': {'pages': [before]}, 'after': {'pages': [after]}}}}

    def test_exact_failed_outcome_requires_independent_source_bound_review(self):
        result = self.evidence()
        self.assertIn('native_navigation_failure_review_mismatch', gate.navigation_errors({'url': '/a'}, result, {}))
        expected = {'page_id': 1, **result['extra']['oryn_navigation']['after']['pages'][0]['snapshot']['outcomes'][0]}
        review = {'expected_navigation_errors': [expected], 'navigation_error_source_evidence': 'pinned navigation rejection assertion'}
        self.assertEqual(gate.navigation_errors({'url': '/a'}, result, review), [])
        for field, value in [('active', 1), ('pruned_outcomes', 1)]:
            altered = copy.deepcopy(result)
            altered['extra']['oryn_navigation']['after']['pages'][0]['snapshot'][field] = value
            self.assertTrue(gate.navigation_errors({'url': '/a'}, altered, review))
        result['extra']['oryn_navigation']['after']['pages'][0]['snapshot']['outcomes'][0]['error_truncated'] = True
        self.assertIn('native_navigation_error_truncated', gate.navigation_errors({'url': '/a'}, result, review))

    def test_admission_acknowledgement_never_qualifies_as_execution(self):
        result = self.evidence()
        item = result['extra']['oryn_navigation']['after']['pages'][0]['snapshot']['outcomes'][0]
        item.update(stage='Admission', result={'Ok': 7}, execution=None)
        self.assertIn('native_navigation_admission_is_not_completion', gate.navigation_errors({'url': '/a'}, result, {}))

    def test_terminal_outcome_payload_and_generation_are_exact(self):
        result = self.evidence()
        item = result['extra']['oryn_navigation']['after']['pages'][0]['snapshot']['outcomes'][0]
        item.update(result={'Ok': 7}, execution={'Committed': {'document_generation': 7}})
        self.assertEqual(gate.navigation_errors({'url': '/a'}, result, {}), [])
        item['result'] = {'Ok': 8}
        self.assertIn('native_navigation_execution_mismatch', gate.navigation_errors({'url': '/a'}, result, {}))
        for malformed in [True, -1, '7', {}, 2**64]:
            item.update(result={'Ok': malformed}, execution=None)
            self.assertIn('native_navigation_outcome_malformed', gate.navigation_errors({'url': '/a'}, result, {}))

    def test_baseline_outcomes_also_require_terminal_schema(self):
        result = self.evidence()
        item = copy.deepcopy(result['extra']['oryn_navigation']['after']['pages'][0]['snapshot']['outcomes'][0])
        item.pop('execution')
        result['extra']['oryn_navigation']['before']['pages'][0]['snapshot']['outcomes'].append(item)
        self.assertIn('native_navigation_outcome_malformed', gate.navigation_errors({'url': '/a'}, result, {}))

    def test_missing_collection_and_retired_page_history_cannot_qualify(self):
        result = self.evidence()
        result['extra']['oryn_navigation']['after']['pages'] = []
        self.assertIn('native_navigation_retired_history_missing', gate.navigation_errors({'url': '/a'}, result, {}))
        self.assertTrue(gate.navigation_errors({'url': '/a'}, {}, {}))

if __name__ == '__main__':
    unittest.main()

"""Supervisor custody regression sources; no subprocess or native execution."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from completion_execution import reference
from completion_provenance import ROOT, digest
from wpt_execution_evidence import verify, feature_profile, cookie_profile


class ExecutionEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        def record(name, value):
            path = self.root / name
            path.write_text(json.dumps(value))
            return reference(path)
        fixture = {'web_features': {'usermedia_element': 'standard'}, 'third_party_cookies': 'block'}
        features, cookies = feature_profile.from_fixture(fixture), cookie_profile.from_fixture(fixture)
        self.profiles = {'feature_environment_sha256': feature_profile.environment_digest(fixture), 'fixture_environment_sha256': cookie_profile.environment_digest(fixture), 'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features), 'cookie_profile': cookies, 'cookie_profile_sha256': cookie_profile.digest(cookies)}
        candidate_record = record('candidate.json', {'schema_version': 1, 'artifacts': {'fixture_contract': record('fixture.json', fixture)}})
        self.candidate = candidate_record['sha256']
        contract = record('run.json', {'candidate': candidate_record})
        gate = record('gate.json', {})
        lock = record('lock.json', {**self.profiles, 'candidate_sha256': self.candidate,
                                   'run_contract_sha256': contract['sha256'], 'inputs': {}})
        self.report = self.root / 'wptreport.json'
        self.report.write_text(json.dumps({'run_info': {'oryn_' + key: value for key, value in self.profiles.items()}, 'results': [{'status': 'FAIL'}]}))
        preflight = record('preflight.json', {'schema_version': 1, 'candidate_sha256': self.candidate,
            'collection': 'core', 'argv': ['pinned', 'wpt'], 'generation_gate': gate,
            'run_contract': contract, 'workflow_lock': lock,
            'producer_source': reference(ROOT / 'scripts/wpt_official_workflow.py'),
            'supervisor_source': reference(ROOT / 'scripts/completion_execution.py')})
        custody = self.root / 'protocol-custody.jsonl'
        custody.write_text(''.join(json.dumps({'schema_version': 1, 'candidate_sha256': self.candidate,
            'session': 'b' * 32, 'phase': phase, 'success': True, 'error_type': None}) + '\n'
            for phase in ('connect', 'teardown')))
        self.execution = self.root / 'execution.json' 
        self.value = {**self.profiles, 'schema_version': 1, 'candidate_sha256': self.candidate, 'collection': 'core',
            'argv': ['pinned', 'wpt'], 'returncode': 1, 'timed_out': False, 'launch_error': None,
            'cleanup_complete': True, 'custody_complete': True, 'errors': [], 'preflight': preflight,
            'workflow_lock_sha256': lock['sha256'], 'artifacts': {'wptreport.json': digest(self.report), 'protocol-custody.jsonl': digest(custody)}}

    def check(self):
        self.execution.write_text(json.dumps(self.value))
        return verify(self.report, self.execution, self.candidate, 'core')

    def test_genuine_failing_suite_is_preserved_as_failing_raw_outcome(self):
        self.assertEqual(self.check()['returncode'], 1)
        self.assertEqual(json.loads(self.report.read_text())['results'][0]['status'], 'FAIL')

    def test_report_feature_profile_cannot_be_relabelled_even_with_rebound_raw_hash(self):
        report = json.loads(self.report.read_text())
        altered = feature_profile.from_fixture({'web_features': {'usermedia_element': 'legacy'}})
        report['run_info']['oryn_feature_profile'] = altered
        report['run_info']['oryn_feature_profile_sha256'] = feature_profile.digest(altered)
        self.report.write_text(json.dumps(report))
        self.value['artifacts']['wptreport.json'] = digest(self.report)
        with self.assertRaisesRegex(ValueError, 'feature profile changed'):
            self.check()

    def test_cleanup_timeout_drift_and_abnormal_exit_never_certify_custody(self):
        for field, value in [('cleanup_complete', False), ('custody_complete', False),
                             ('timed_out', True), ('errors', ['input drift']), ('returncode', -9)]:
            original = copy.deepcopy(self.value)
            self.value[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.check()
            self.value = original

    def test_changed_report_or_preflight_cannot_inherit_execution(self):
        self.report.write_text('{"results": [{"status": "PASS"}]}')
        with self.assertRaisesRegex(ValueError, 'report not bound'):
            self.check()
        self.value['artifacts']['wptreport.json'] = digest(self.report)
        (self.root / 'preflight.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'changed'):
            self.check()

    def test_another_run_preflight_is_rejected_even_with_equal_contents(self):
        other = self.root / 'foreign-preflight.json'
        other.write_bytes((self.root / 'preflight.json').read_bytes())
        self.value['preflight'] = reference(other)
        with self.assertRaisesRegex(ValueError, 'another run'):
            self.check()


    def test_missing_failed_and_duplicate_teardown_cannot_qualify_raw_results(self):
        path = self.root / 'protocol-custody.jsonl'
        original = path.read_text().splitlines()
        for lines in (original[:1], original + original[1:],
                      [original[0], json.dumps(dict(json.loads(original[1]), success=False, error_type='UnknownErrorException'))]):
            path.write_text('\n'.join(lines) + '\n')
            self.value['artifacts'][path.name] = digest(path)
            with self.assertRaisesRegex(ValueError, 'protocol'):
                self.check()

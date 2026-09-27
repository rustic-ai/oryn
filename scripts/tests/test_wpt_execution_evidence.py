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
            'collection': 'core', 'argv': ['pinned', 'wpt', '--processes', '1'], 'generation_gate': gate,
            'run_contract': contract, 'workflow_lock': lock,
            'producer_source': reference(ROOT / 'scripts/wpt_official_workflow.py'),
            'supervisor_source': reference(ROOT / 'scripts/completion_execution.py')})
        custody = self.root / 'protocol-custody.jsonl'
        custody.write_text(''.join(json.dumps({'schema_version': 1, 'candidate_sha256': self.candidate,
            'session': 'b' * 32, 'phase': phase, 'success': True, 'error_type': None}) + '\n'
            for phase in ('connect', 'teardown')))
        self.execution = self.root / 'execution.json' 
        self.value = {**self.profiles, 'schema_version': 1, 'candidate_sha256': self.candidate, 'collection': 'core',
            'argv': ['pinned', 'wpt', '--processes', '1'], 'returncode': 1, 'timed_out': False, 'launch_error': None,
            'cleanup_complete': True, 'custody_complete': True, 'errors': [], 'preflight': preflight,
            'workflow_lock_sha256': lock['sha256'], 'artifacts': {'wptreport.json': digest(self.report), 'protocol-custody.jsonl': digest(custody)}}

    def check(self):
        self.execution.write_text(json.dumps(self.value))
        return verify(self.report, self.execution, self.candidate, 'core')

    def bind_process_policy(self, count, *, effective=None, argv_count=None):
        preflight_path = self.root / 'preflight.json'
        preflight = json.loads(preflight_path.read_text())
        lock_path = Path(preflight['workflow_lock']['path'])
        lock = json.loads(lock_path.read_text())
        lock['core_processes'] = count
        lock_path.write_text(json.dumps(lock))
        effective = count if effective is None else effective
        argv_count = effective if argv_count is None else argv_count
        for record in (preflight, self.value):
            record.update(core_processes=count, processes=effective,
                          argv=['pinned', 'wpt', '--processes', str(argv_count)])
        preflight['workflow_lock'] = reference(lock_path)
        preflight_path.write_text(json.dumps(preflight))
        self.value['preflight'] = reference(preflight_path)
        self.value['workflow_lock_sha256'] = digest(lock_path)

    def test_parallel_process_policy_and_interleaved_sessions_validate_real_evidence(self):
        for count in (1, 2, 4, 6, 8):
            with self.subTest(count=count):
                self.bind_process_policy(count)
                path = self.root / 'protocol-custody.jsonl'
                sessions = [format(index + 1, '032x') for index in range(count)]
                path.write_text(''.join(json.dumps({'schema_version': 1,
                    'candidate_sha256': self.candidate, 'session': session,
                    'phase': phase, 'success': True, 'error_type': None}) + '\n'
                    for phase in ('connect', 'teardown') for session in sessions))
                self.value['artifacts'][path.name] = digest(path)
                self.assertEqual(self.check()['processes'], count)

    def test_rebound_receipts_cannot_disagree_with_frozen_manager_count(self):
        for count, effective, argv_count in ((2, 1, 1), (4, 4, 2), (6, 4, 6), (8, 8, 6),
                                           (6, 6, 8), (True, True, 1), (3, 3, 3)):
            with self.subTest(count=count, effective=effective, argv_count=argv_count):
                self.bind_process_policy(count, effective=effective, argv_count=argv_count)
                with self.assertRaisesRegex(ValueError, 'process'):
                    self.check()
        self.bind_process_policy(2)
        del self.value['core_processes']
        with self.assertRaisesRegex(ValueError, 'process policy'):
            self.check()

    def test_frozen_timeout_and_outer_floor_are_checked_in_actual_receipts(self):
        self.bind_process_policy(2)
        preflight_path = self.root / 'preflight.json'
        preflight = json.loads(preflight_path.read_text())
        lock_path = Path(preflight['workflow_lock']['path'])
        lock = json.loads(lock_path.read_text())
        lock['core_timeout_seconds'] = 345600
        lock_path.write_text(json.dumps(lock))
        for record in (preflight, self.value):
            record.update(core_timeout_seconds=345600, timeout_seconds=345600,
                          outer_timeout_floor_seconds=345690)
        preflight['workflow_lock'] = reference(lock_path)
        preflight_path.write_text(json.dumps(preflight))
        self.value['preflight'] = reference(preflight_path)
        self.value['workflow_lock_sha256'] = digest(lock_path)
        self.assertEqual(self.check()['timeout_seconds'], 345600)
        for field in ('core_timeout_seconds', 'timeout_seconds', 'outer_timeout_floor_seconds'):
            original = self.value[field]
            self.value[field] = 43200
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'policy mismatch'):
                self.check()
            self.value[field] = original
        del self.value['timeout_seconds']
        with self.assertRaisesRegex(ValueError, 'policy mismatch'):
            self.check()

    def test_historical_lock_is_serial_and_duplicate_process_flags_are_rejected(self):
        from wpt_execution_evidence import verify_execution_policy
        self.assertEqual(self.check()['returncode'], 1)
        for argv in (['wpt', '--processes', '2'],
                     ['wpt', '--processes', '1', '--processes=2'],
                     ['wpt', '--processes=1'], ['wpt', '--processes'], ['wpt'],
                     *[['wpt', '--processes', '1', alias, '4'] for alias in
                       ('--p', '--pr', '--pro', '--proc', '--proce', '--proces', '--process', '--processe')],
                     *[['wpt', '--processes', '1', alias + '=4'] for alias in
                       ('--p', '--pr', '--pro', '--proc', '--proce', '--proces', '--process', '--processe')]):
            with self.subTest(argv=argv), self.assertRaisesRegex(ValueError, 'process argv'):
                verify_execution_policy({}, 'core', {'argv': argv})
        for value in (True, False, None, '345600', 1.5, 0, -1, 604801):
            with self.subTest(timeout=value), self.assertRaisesRegex(ValueError, 'Core timeout'):
                verify_execution_policy({'core_timeout_seconds': value}, 'core')
        for kind in ('canaries', 'placement'):
            with self.subTest(collection=kind):
                good = {'core_processes': 4, 'processes': 1, 'argv': ['wpt', '--processes', '1']}
                self.assertEqual(verify_execution_policy({'core_processes': 4}, kind, good)['processes'], 1)
                with self.assertRaisesRegex(ValueError, 'process argv'):
                    verify_execution_policy({'core_processes': 4}, kind, dict(good, argv=['wpt', '--processes', '4']))

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

    def test_six_eight_counts_preserve_serial_budgets_and_reject_missing_teardown(self):
        from wpt_execution_evidence import execution_metadata, verify_execution_policy
        for count in (6, 8):
            with self.subTest(count=count):
                self.bind_process_policy(count)
                path = self.root / 'protocol-custody.jsonl'
                sessions = [format(index + 1, '032x') for index in range(count)]
                rows = [{'schema_version': 1, 'candidate_sha256': self.candidate,
                         'session': session, 'phase': phase, 'success': True, 'error_type': None}
                        for phase in ('connect', 'teardown') for session in sessions]
                path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
                self.value['artifacts'][path.name] = digest(path)
                self.assertEqual(self.check()['processes'], count)
                path.write_text(''.join(json.dumps(row) + '\n' for row in rows[:-1]))
                self.value['artifacts'][path.name] = digest(path)
                with self.assertRaisesRegex(ValueError, 'protocol'):
                    self.check()
                for kind in ('core', 'canaries', 'placement'):
                    lock = {'core_processes': count, 'core_timeout_seconds': 345600}
                    policy = execution_metadata(kind, count, 345600)
                    effective = count if kind == 'core' else 1
                    self.assertEqual(policy['processes'], effective)
                    self.assertEqual(policy['timeout_seconds'], 345600 if kind == 'core' else 43200)
                    self.assertEqual(policy['outer_timeout_floor_seconds'],
                                     (345600 if kind == 'core' else 43200) + 90)
                    record = dict(policy, argv=['wpt', '--processes', str(effective)])
                    self.assertEqual(verify_execution_policy(lock, kind, record), policy)
                    with self.assertRaisesRegex(ValueError, 'policy mismatch'):
                        verify_execution_policy(lock, kind, dict(record, timeout_seconds=1))
                    if kind != 'core':
                        with self.assertRaisesRegex(ValueError, 'process argv'):
                            verify_execution_policy(lock, kind,
                                                    dict(record, argv=['wpt', '--processes', str(count)]))

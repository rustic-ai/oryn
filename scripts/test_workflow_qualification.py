import copy
import unittest
from pathlib import Path
import tempfile
from completion_provenance import digest, workflow_identity
from workflow_qualification import validate_profile_candidate
from workflow_qualification import planned_profiles,validate_profiles,SAMPLES,WARM_ACTIONS


def records():
    result=[]
    for row in planned_profiles():
        if row['kind']=='page':
            data=dict(worker_lifecycle={'isolated':'isolated_on_demand','isolated-prewarmed':'isolated_prewarmed','recycled':'recycled'}[row['mode']],pages=5,warm_action_count=WARM_ACTIONS,completed_warm_action_count=WARM_ACTIONS,first_cold_verification_launch_handshake_ms=250,warm_actions_per_second=300,cached_isolated_launch_ms=[50]*4,reset_or_close_ms=[10]*5,recycled_javascript_state_erased=True)
        elif row['kind']=='group':data=dict(pages=row['pages'],passed=True,page_measurements=[{}]*row['pages'],worker_rss_bytes=[64*1024*1024]*row['pages'],parent_rss_bytes=100,total_worker_rss_bytes=64*1024*1024*row['pages'])
        else:data=dict(runtime=dict(sandboxed=True),status='passed',false_preservation=dict(numerator=0),worker_replacement=dict(status='passed'))
        data['runtime']=dict(sandboxed=True,worker_binary_sha256='a'*64)
        if row['kind']!='churn':data['process_lifecycle']=dict(complete=True,processes=[dict(binary_sha256='a'*64,reaped_unix_ns=2,exit_code=0)])
        result.append(dict(id=row['id'],returncode=0,timed_out=False,cleanup_complete=True,errors=[],result=data))
    return result

class QualificationTests(unittest.TestCase):
    def test_profile_replay_binds_every_executable_and_raw_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            names = ('workflow_executor', 'worker', 'page_benchmark', 'group_benchmark', 'churn')
            paths = {name: root / name for name in names}
            for name, path in paths.items(): path.write_text(name)
            bound = dict(sha256='c'*64, artifacts=paths, record=dict(source_tree_sha256='b'*64,
                         artifacts={name: dict(sha256=digest(path)) for name, path in paths.items()}))
            inputs = dict(candidate=workflow_identity(bound, paths['workflow_executor'], paths['worker'], 'b'*64),
                          candidate_file_sha256='c'*64,
                          executables={name: digest(paths[artifact]) for name, artifact in
                                       dict(page='page_benchmark',group='group_benchmark',churn='churn',worker='worker').items()})
            rows = [dict(result=dict(runtime=dict(worker_binary_sha256=digest(paths['worker']))))]
            validate_profile_candidate(inputs, rows, bound)
            for field in inputs['executables']:
                changed = copy.deepcopy(inputs); changed['executables'][field] = 'd'*64
                with self.assertRaisesRegex(ValueError, 'executables'):
                    validate_profile_candidate(changed, rows, bound)
            changed = copy.deepcopy(rows); changed[0]['result']['runtime']['worker_binary_sha256']='d'*64
            with self.assertRaisesRegex(ValueError, 'raw profile worker'):
                validate_profile_candidate(inputs, changed, bound)
            changed = copy.deepcopy(inputs); changed['candidate']['candidate_manifest_sha256']='d'*64
            with self.assertRaisesRegex(ValueError, 'candidate inputs'):
                validate_profile_candidate(changed, rows, bound)

    def test_missing_profile_error_custody_is_not_success(self):
        rows=records();rows[0].pop('errors')
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])

    def test_fixed_boundary_profile_does_not_certify_core_or_independent_contexts(self):
        result=validate_profiles(records())
        self.assertTrue(result['profile_checks_passed']);self.assertFalse(result['qualified']);self.assertFalse(result['independent_context_qualification'])
        self.assertEqual(len(planned_profiles()),SAMPLES*7)
    def test_retry_cannot_replace_missing_failed_or_duplicate_attempt(self):
        cases=[records()[:-1],records()+[records()[0]]]
        failed=records();failed[0]['returncode']=1;cases.append(failed)
        for rows in cases:self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_lowered_sample_count_or_missing_cleanup_is_rejected(self):
        for mutate in (lambda row:row['result'].update(warm_action_count=1),lambda row:row.update(cleanup_complete=False)):
            rows=records();mutate(rows[0]);self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_cold_warm_binding_and_rss_fixed_targets(self):
        for key,value,kind in [('first_cold_verification_launch_handshake_ms',251,'isolated:'),('warm_actions_per_second',299,'isolated:'),('cached_isolated_launch_ms',[51]*4,'isolated-prewarmed:'),('worker_rss_bytes',[161*1024*1024],'group:')]:
            rows=records()
            for row in rows:
                if row['id'].startswith(kind) or (key=='cached_isolated_launch_ms' and row['id'].startswith('recycled:')):row['result'][key]=value
            self.assertFalse(validate_profiles(rows)['profile_checks_passed'],key)
    def test_nonfinite_measurements_do_not_pass_comparison(self):
        rows=records();rows[0]['result']['first_cold_verification_launch_handshake_ms']=float('nan')
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_churn_false_preservation_and_worker_replacement_fail(self):
        rows=records();rows[-1]['result']['false_preservation']['numerator']=1
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])

class IntegratedQualificationTests(unittest.TestCase):
    def test_profile_pass_cannot_replace_missing_context_resources(self):
        from workflow_qualification import integrated_q_checks
        result=integrated_q_checks(dict(profile_checks_passed=True),dict(structurally_valid=True,planned=1,validated=['one']),[])
        self.assertFalse(result['q_component_checks_passed']);self.assertFalse(result['qualified'])
    def test_incomplete_workflow_denominator_stays_failed(self):
        from workflow_qualification import integrated_q_checks
        result=integrated_q_checks(dict(profile_checks_passed=True),dict(structurally_valid=True,planned=2,validated=['one']),[])
        self.assertIn('full frozen workflow denominator not passing',result['errors'])

class ProfileAttestationTests(unittest.TestCase):
    def test_runner_failures_cannot_be_hidden_by_valid_native_measurements(self):
        for errors in (['resource accounting incomplete'], None, False, ''):
            rows=records();rows[0]['errors']=errors
            self.assertFalse(validate_profiles(rows)['profile_checks_passed'])

    def test_malformed_profiles_report_failure_instead_of_aborting_reconciliation(self):
        for population in (None, {}, [None], [dict(id=[])], [dict(id='unknown')]):
            with self.subTest(population=population):
                self.assertFalse(validate_profiles(population)['profile_checks_passed'])
        for key,value in [('result',None),('returncode',False),('timed_out',None)]:
            rows=records();rows[0][key]=value
            self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
        for key,value in [('runtime',None),('process_lifecycle',None),
                          ('cached_isolated_launch_ms',None),('reset_or_close_ms',1)]:
            rows=records();rows[0]['result'][key]=value
            self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_absent_or_false_like_worker_reap_evidence_never_passes(self):
        for field,value in [('reaped_unix_ns',True),('reaped_unix_ns',-1),
                            ('exit_code',False),('exit_code','0'),('signal',0)]:
            rows=records();rows[0]['result']['process_lifecycle']['processes'][0][field]=value
            self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
        rows=records();rows[0]['result']['runtime'].pop('worker_binary_sha256')
        rows[0]['result']['process_lifecycle']['processes'][0].pop('binary_sha256')
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_each_raw_profile_needs_its_own_production_runtime(self):
        rows=records();rows[0]['result'].pop('runtime')
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_complete_flag_cannot_replace_checked_reap(self):
        rows=records();rows[0]['result']['process_lifecycle']['processes'][0]['reaped_unix_ns']=None
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])
    def test_process_binary_must_match_raw_runtime(self):
        rows=records();rows[0]['result']['process_lifecycle']['processes'][0]['binary_sha256']='b'*64
        self.assertFalse(validate_profiles(rows)['profile_checks_passed'])

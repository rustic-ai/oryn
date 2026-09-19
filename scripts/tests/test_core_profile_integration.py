"""Real custody/final consumer regression sources; no native execution or services."""
import copy
from contextlib import ExitStack
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import completion_provenance
import completion_qualification as final
import core_profile_contract as profiles
import wpt_execution_evidence as custody
from core_profile_fixtures import ScopeFixture, write


class CoreProfileIntegrationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.fixture = ScopeFixture(temporary.name)
        self.stack = ExitStack(); self.addCleanup(self.stack.close)
        self.stack.enter_context(self.fixture.admitted())
        self.stack.enter_context(patch.object(completion_provenance, 'load_candidate', return_value=self.fixture.candidate))
        self.stack.enter_context(patch.object(profiles, 'validate_inventory_binding'))

    def execution(self, profile='primary', run='original', original=None, collection='core'):
        envelope_path, envelope = self.fixture.envelope(profile, run, original)
        root = Path(envelope['envelope']['outputs'][collection]); root.mkdir(parents=True)
        prepared = Path(envelope['envelope']['prepared']); prepared.mkdir(parents=True)
        context = profiles.identity(envelope, collection)
        fields = profiles.profile_fields(envelope['fixture'])
        report = root/'wptreport.json'
        write(report, {'run_info': {**{'oryn_'+key:value for key,value in context.items()},
            **{'oryn_'+key:value for key,value in fields.items()}, 'os':'mac','processor':'aarch64',
            'oryn_fixture_sha256':envelope['profile_sha256'],'oryn_run_contract_sha256':envelope['envelope_sha256']},
            'results':[{'test':'/synthetic.html','status':'FAIL'}]})
        lock = write(prepared/'workflow-lock.json', {**{key:envelope[key] for key in profiles.BASE_FIELDS}, **fields,
            'schema_version':2,'run_contract_sha256':envelope['envelope_sha256'],'inputs':{},
            'manifest':str(self.fixture.root/'manifest'),'policy':str(self.fixture.root/'policy'),
            'inventory':str(self.fixture.root/'inventory')})
        preflight = write(root/'preflight.json', dict(context,schema_version=2,collection=collection,argv=['unchanged-official-runner'],
            generation_gate=write(self.fixture.root/'gate.json',{}),run_contract=profiles.reference(envelope_path),workflow_lock=lock,
            producer_source=profiles.reference(custody.ROOT/'scripts/wpt_official_workflow.py'),
            supervisor_source=profiles.reference(custody.ROOT/'scripts/completion_execution.py')))
        protocol = root/'protocol-custody.jsonl'
        protocol.write_text(''.join(json.dumps({'schema_version':1,'candidate_sha256':context['candidate_sha256'],
            'session':'a'*32,'phase':phase,'success':True,'error_type':None})+'\n' for phase in ('connect','teardown')))
        execution = dict(context,**fields,schema_version=2,collection=collection,argv=['unchanged-official-runner'],
            returncode=1 if collection!='placement' else 0,timed_out=False,launch_error=None,
            cleanup_complete=True,custody_complete=True,errors=[],preflight=preflight,workflow_lock_sha256=lock['sha256'],
            artifacts={p.name:profiles.digest(p) for p in root.iterdir() if p.is_file()})
        write(root/'execution.json',execution)
        return report, execution, envelope

    def verify(self, report, profile='primary'):
        return custody.verify(report,report.parent/'execution.json',self.fixture.candidate['sha256'],'core',profile_id=profile)

    def test_actual_execution_verifier_retains_raw_failure_and_profile_identity(self):
        report,_,envelope=self.execution()
        result=self.verify(report)
        self.assertEqual(result['returncode'],1)
        self.assertEqual(result['profile_context'],profiles.identity(envelope,'core'))
        self.assertEqual(json.loads(report.read_text())['results'][0]['status'],'FAIL')

    def test_report_relabel_is_rejected_even_with_updated_report_digest(self):
        report,execution,_=self.execution()
        value=json.loads(report.read_text());value['run_info']['oryn_profile_id']='legacy';write(report,value)
        execution['artifacts']['wptreport.json']=profiles.digest(report);write(report.parent/'execution.json',execution)
        with self.assertRaisesRegex(ValueError,'profile/envelope identity'):
            self.verify(report)

    def test_report_platform_and_added_raw_artifact_are_rejected(self):
        report,execution,_=self.execution()
        value=json.loads(report.read_text());value['run_info']['os']='linux';write(report,value)
        execution['artifacts']['wptreport.json']=profiles.digest(report);write(report.parent/'execution.json',execution)
        with self.assertRaisesRegex(ValueError,'platform differs'):self.verify(report)
        value['run_info']['os']='mac';write(report,value)
        execution['artifacts']['wptreport.json']=profiles.digest(report);write(report.parent/'execution.json',execution)
        (report.parent/'unrecorded.json').write_text('{}')
        with self.assertRaisesRegex(ValueError,'artifact inventory changed'):self.verify(report)

    def test_original_and_retry_are_retained_but_orphan_and_duplicate_original_reject(self):
        report,_,_=self.execution();retry,_,_=self.execution(run='retry',original='original')
        context=profiles.load_profile_context(self.fixture.candidate['path'],'primary',[report,retry])
        self.assertEqual(context['original_report_sha256'],profiles.digest(report))
        self.assertEqual(len(context['reports']),2)
        with self.assertRaisesRegex(ValueError,'exactly one designated original'):
            profiles.load_profile_context(self.fixture.candidate['path'],'primary',[retry])
        other,_,_=self.execution(run='another-original')
        with self.assertRaisesRegex(ValueError,'exactly one designated original'):
            profiles.load_profile_context(self.fixture.candidate['path'],'primary',[report,other])

    def test_execution_mutation_during_context_verification_cannot_be_recaptured(self):
        report,_,envelope=self.execution()
        original_verify=custody.verify
        def mutate(*args,**kwargs):
            result=original_verify(*args,**kwargs)
            (report.parent/'execution.json').write_text('{}')
            return result
        with patch.object(custody,'verify',side_effect=mutate):
            with self.assertRaisesRegex(ValueError,'missing or changed'):
                profiles.load_profile_context(self.fixture.candidate['path'],'primary',[report])

    def test_another_profile_cannot_satisfy_requested_profile(self):
        report,_,_=self.execution('allow')
        with self.assertRaisesRegex(ValueError,'another required profile'):
            self.verify(report,'primary')

    def test_schema1_custody_cannot_downgrade_required_profile(self):
        report,execution,_=self.execution();execution['schema_version']=1
        write(report.parent/'execution.json',execution)
        with self.assertRaises(ValueError):self.verify(report)

    def test_special_controls_cannot_be_satisfied_by_another_envelope(self):
        _,_,envelope=self.execution()
        context=profiles.identity(envelope,'core')
        profile={'run_contract':envelope['envelope_path'],'prepared':envelope['envelope']['prepared'],
            'canaries':{'directory':envelope['envelope']['outputs']['canaries'],'audit':str(self.fixture.root/'audit')},
            'placement':{'directory':envelope['envelope']['outputs']['placement']}}
        wrong=dict(profiles.identity(envelope,'canaries'),profile_id='allow')
        with patch.object(custody,'verify',return_value={'profile_context':wrong}):
            with self.assertRaisesRegex(ValueError,'profile/envelope identity'):
                final.profile_controls(profile,context,self.fixture.candidate)

    def test_profile_plan_exact_members_controls_and_no_cross_profile_file_reuse(self):
        registry=profiles.validate_registry(self.fixture.candidate)
        members={}
        for name in registry['value']['required_profile_ids']:
            path,envelope=self.fixture.envelope(name)
            base=self.fixture.root/'review'/name
            members[name]={'run_contract':str(path),'prepared':envelope['envelope']['prepared'],
                'reports':[envelope['envelope']['outputs']['core']+'/wptreport.json'],
                **{key:str(base/(key+'.json')) for key in ('reviews','assessments','attribution','disposition_applications')},
                'canaries':{'directory':envelope['envelope']['outputs']['canaries'],'audit':str(base/'audit.json')},
                'placement':{'directory':envelope['envelope']['outputs']['placement']}}
        plan={'schema_version':2,'non_core_scope':registry['value']['non_core_claims'],
            'core':{**{key:str(self.fixture.root/key) for key in ('inventory','manifest','policy','checkout')},'profiles':members}}
        profiles.validate_plan(plan,self.fixture.candidate)
        for mutation in (lambda p:p['core']['profiles'].pop('legacy'),
                         lambda p:p['core']['profiles']['allow'].update(reviews=p['core']['profiles']['primary']['reviews']),
                         lambda p:p['core']['profiles']['allow'].pop('canaries'),
                         lambda p:p.update(schema_version=1),
                         lambda p:p['non_core_scope'].update(workflows='allow')):
            changed=copy.deepcopy(plan);mutation(changed)
            with self.assertRaises(ValueError):profiles.validate_plan(changed,self.fixture.candidate)


    def test_upstream_run_info_emits_whole_envelope_identity_with_distinct_fixture_hash(self):
        from oryn_wptrunner import provenance
        _, envelope = self.fixture.envelope('allow')
        bound = {**profiles.profile_fields(envelope['fixture']), 'profile_envelope': envelope,
            'candidate_sha256':envelope['candidate_sha256'], 'sha256':envelope['envelope_sha256'],
            'fixture_sha256':envelope['profile_sha256'], 'fixture_repairs_sha256':'f'*64,
            'rendering_contracts':{}}
        with patch.object(provenance,'load_contract',return_value=bound), \
             patch.dict(provenance.os.environ,{'ORYN_CORE_COLLECTION':'canaries'},clear=True):
            info=provenance.run_info_extras(None)
        profiles.check_identity(info,profiles.identity(envelope,'canaries'),prefix='oryn_')
        self.assertEqual(info['oryn_profile_sha256'],bound['fixture_sha256'])
        self.assertNotEqual(info['oryn_profile_sha256'],info['oryn_feature_profile_sha256'])
        with patch.object(provenance,'load_contract',return_value=bound), patch.dict(provenance.os.environ,{},clear=True):
            with self.assertRaisesRegex(ValueError,'admitted collection'):
                provenance.run_info_extras(None)

    def test_retry_launch_reuses_only_its_own_designated_original_controls(self):
        _, original = self.fixture.envelope()
        _, retry = self.fixture.envelope(run='retry', original='original')
        profile = {'run_contract': original['envelope_path']}
        with patch.object(final, 'profile_controls', return_value={'verified':True}) as controls:
            self.assertEqual(final.profile_original_controls(profile,retry,self.fixture.candidate),{'verified':True})
            self.assertEqual(controls.call_args.args[1],profiles.identity(original,'core'))
            foreign=dict(retry,original_run_id='foreign-original')
            with self.assertRaisesRegex(ValueError,'planned original'):
                final.profile_original_controls(profile,foreign,self.fixture.candidate)
            foreign=dict(retry,profile_id='allow')
            with self.assertRaisesRegex(ValueError,'substitutes'):
                final.profile_original_controls(profile,foreign,self.fixture.candidate)

    def test_official_prepare_rejects_foreign_output_before_manifest_launch(self):
        import wpt_official_workflow as workflow
        path, envelope = self.fixture.envelope()
        gate = self.fixture.root/'gate';gate.write_text('{}')
        bound = {'fixture':envelope['fixture'],'fixture_path':envelope['fixture_path'],
                 'profile_envelope':envelope,'sha256':envelope['envelope_sha256'],
                 'candidate_sha256':self.fixture.candidate['sha256']}
        with patch.object(workflow,'generation_gate'), patch.object(workflow,'load_contract',return_value=bound), \
             patch.object(workflow,'read_inventory',return_value={}), \
             patch.object(workflow.subprocess,'check_output',return_value=profiles.REVISION), \
             patch.object(workflow.subprocess,'run') as launch:
            with self.assertRaisesRegex(ValueError,'prepared output differs'):
                workflow.prepare(gate,path,'inventory',self.fixture.root,'manifest','policy','python',self.fixture.root/'foreign')
            launch.assert_not_called()
        self.assertFalse((self.fixture.root/'foreign').exists())

    def test_official_execute_requires_original_controls_and_records_collection_environment(self):
        import wpt_official_workflow as workflow
        report, _, envelope = self.execution()
        output=report.parent
        for path in output.iterdir():path.unlink()
        output.rmdir()
        prepared=Path(envelope['envelope']['prepared']);gate=self.fixture.root/'gate.json'
        python=self.fixture.root/'python';python.write_text('synthetic executable bytes')
        lock=json.loads((prepared/'workflow-lock.json').read_text())
        lock.update(checkout=str(self.fixture.root),python=str(python),core_total=16737,
            manifest_sha256='a'*64,policy_sha256='b'*64,coverage_sha256='c'*64,core_include_sha256='d'*64)
        write(prepared/'workflow-lock.json',lock)
        bound={**profiles.profile_fields(envelope['fixture']),'profile_envelope':envelope,
            'sha256':envelope['envelope_sha256'],'candidate_sha256':self.fixture.candidate['sha256'],
            'candidate':{'artifacts':{name:{'sha256':'a'*64} for name in ('runtime','driver','source_snapshot','worker_manifest')}}}
        plan={'core':{'profiles':{'primary':{'run_contract':envelope['envelope_path']}}}}
        self.fixture.candidate['artifacts']['qualification_plan']=self.fixture.root/'plan.json'
        with ExitStack() as stack:
            stack.enter_context(patch.object(workflow,'generation_gate'))
            stack.enter_context(patch.object(workflow,'load_contract',return_value=bound))
            stack.enter_context(patch.object(workflow,'read_inventory'))
            stack.enter_context(patch.object(workflow,'runner_command',return_value=['synthetic','runner']))
            stack.enter_context(patch.object(final,'profile_prerequisites'))
            stack.enter_context(patch.object(final,'read',return_value=plan))
            with patch.object(final,'profile_original_controls',side_effect=ValueError('original controls missing')), \
                 patch.object(workflow,'run_owned') as launch:
                with self.assertRaisesRegex(ValueError,'original controls missing'):
                    workflow.execute(gate,envelope['envelope_path'],self.fixture.root,prepared,python,'core',output)
                launch.assert_not_called();self.assertFalse(output.exists())
            def run(command, directory, **kwargs):
                self.assertEqual(kwargs['environment']['ORYN_CORE_COLLECTION'],'core')
                self.assertEqual(kwargs['environment']['ORYN_WPT_CANDIDATE_SHA256'],self.fixture.candidate['sha256'])
                return {'returncode':1,'timed_out':False,'error':None,'cleanup_complete':True,'elapsed_ns':1}
            with patch.object(final,'profile_original_controls',return_value={}) as controls, \
                 patch.object(workflow,'run_owned',side_effect=run):
                result=workflow.execute(gate,envelope['envelope_path'],self.fixture.root,prepared,python,'core',output)
                self.assertGreaterEqual(controls.call_count,3)
                self.assertEqual(result['schema_version'],2)
                self.assertEqual(result['profile_id'],'primary')
                self.assertTrue(result['custody_complete'])
                self.assertEqual(result['returncode'],1)


if __name__ == '__main__':unittest.main()

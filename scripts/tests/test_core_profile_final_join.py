"""Final profile membership/output and recursive original-input custody sources."""
import copy
from contextlib import ExitStack
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import completion_qualification as final
import core_profile_contract as profiles
import wpt_full_qualification
from core_profile_fixtures import ScopeFixture, write


class CoreAggregateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.fixture = ScopeFixture(temporary.name)
        self.stack = ExitStack(); self.addCleanup(self.stack.close)
        self.stack.enter_context(self.fixture.admitted())
        root = self.fixture.root
        inventory = root/'inventory'; inventory.mkdir()
        (inventory/'coverage.jsonl').write_text(''); write(inventory/'plan.json', {})
        plan = {'core': {'inventory': str(inventory), 'profiles': {name: {'reports': [name]} for name in self.fixture.fixtures}}}
        self.fixture.candidate['record']['artifacts']['qualification_plan'] = write(root/'plan.json', plan)
        self.fixture.save()
        self.registry = profiles.validate_registry(self.fixture.candidate)
        self.stack.enter_context(patch.object(profiles, 'validate_plan', return_value=self.registry))
        self.ids = {str(index) for index in range(16737)}
        self.stack.enter_context(patch.object(wpt_full_qualification, 'validate_inventory', return_value=self.ids))
        self.contexts, members = {}, {}
        for name in self.fixture.fixtures:
            _, envelope = self.fixture.envelope(name)
            context = profiles.identity(envelope, 'core')
            context.update(original_report_sha256='e'*64, reports={'e'*64:dict(context)})
            self.contexts[name] = context
            rows = [{'id':str(index),'state':'qualified_pass','obligation_state':'qualified_pass',
                     'qualified_pass':True,'raw_pass':index < 15985} for index in range(16737)]
            ledger = root/(name+'.jsonl'); ledger.write_text(''.join(json.dumps(row)+'\n' for row in rows))
            summary = {**{key:context[key] for key in profiles.IDENTITY_FIELDS},'schema_version':2,
                'core_total':16737,'automatic_total':15985,'assessment_total':752,'counts':{'qualified_pass':16737},
                'complete':True,'raw_pass_count':15985,'qualified_pass_count':16737,'reviewed_disposition_count':0,
                'open_count':0,'inputs':{},'ledger_sha256':profiles.digest(ledger),
                'original_report_sha256':context['original_report_sha256'],'report_identities':context['reports']}
            members[name] = {'summary':write(root/(name+'-summary.json'),summary),'ledger':profiles.reference(ledger),
                'counts':summary['counts'],'raw_pass_count':15985,'qualified_pass_count':16737,'reviewed_disposition_count':0,
                'open_count':0,'profile_sha256':context['profile_sha256'],'original_report_sha256':context['original_report_sha256']}
        self.stack.enter_context(patch.object(profiles,'load_profile_context',side_effect=lambda candidate,name,reports:self.contexts[name]))
        self.aggregate = {'schema_version':2,'aggregation':'independent_required_profiles',
            'candidate_sha256':self.fixture.candidate['sha256'],'registry_sha256':self.registry['sha256'],
            'core_total_per_profile':16737,'required_profile_ids':list(self.fixture.fixtures),'profile_count':3,
            'primary_profile_id':'primary','profiles':members,'complete':True,'errors':[],
            'single_profile_conformance':True,'non_core_scope':self.registry['value']['non_core_claims']}

    def test_all_three_full_ledgers_are_required_without_a_union(self):
        self.assertIs(profiles.validate_core_aggregate(self.aggregate,self.fixture.candidate),self.aggregate)
        for change in (lambda a:a['profiles'].pop('legacy'), lambda a:a.update(profile_count=2),
                       lambda a:a.update(core_total_per_profile=16736), lambda a:a.update(candidate_sha256='f'*64),
                       lambda a:a.update(registry_sha256='f'*64), lambda a:a.update(single_profile_conformance=False),
                       lambda a:a['non_core_scope'].update(workflows='allow'),
                       lambda a:a['profiles']['legacy'].update(profile_sha256=a['profiles']['primary']['profile_sha256'])):
            mutated=copy.deepcopy(self.aggregate);change(mutated)
            with self.assertRaises(ValueError):profiles.validate_core_aggregate(mutated,self.fixture.candidate)

    def test_final_consumer_rejects_mutated_ledger_and_omitted_retry_identity(self):
        member=self.aggregate['profiles']['legacy']; path=Path(member['summary']['path'])
        value=json.loads(path.read_text());value['report_identities']={}
        member['summary']=write(path,value)
        with self.assertRaisesRegex(ValueError,'original/retry'):
            profiles.validate_core_aggregate(self.aggregate,self.fixture.candidate)
        value['report_identities']=self.contexts['legacy']['reports'];member['summary']=write(path,value)
        Path(member['ledger']['path']).write_text('mutated after accepted Core result')
        with self.assertRaisesRegex(ValueError,'missing or changed'):
            profiles.validate_core_aggregate(self.aggregate,self.fixture.candidate)


class ProfileInputInventoryTests(unittest.TestCase):
    def test_nested_policy_sources_and_raw_additions_remain_final_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); checkout=root/'checkout';checkout.mkdir()
            source=checkout/'assertion.js';source.write_text('pinned original assertion')
            evidence=root/'independent-evidence';evidence.write_text('actual retained evidence bytes')
            review=write(root/'review.json',{'source_files':{'assertion.js':profiles.digest(source)},
                'evidence_artifacts':{str(evidence):profiles.digest(evidence)}})
            policy=write(root/'policy.json',{'review':review})
            leaf=write(root/'leaf.json',{})
            before=write(root/'before.json',{'artifacts':{}})
            preflight=write(root/'before-preflight.json',{key:leaf for key in ('candidate','generation_gate','producer_source')})
            receipt=write(root/'before-receipt.json',{'artifacts':[],'preflight':preflight})
            diagnosis={key:leaf for key in ('original_failure','regression_before','regression_after','regression_receipt')}
            diagnosis.update(regression_before_candidate=before,regression_before_receipt=receipt)
            write(root/'diagnosis.json',diagnosis)
            for name in ('workflows','profiles','receipt','prepared','canaries','placement','original','inventory'):
                (root/name).mkdir();write(root/name/'original.json',{})
            write(root/'inventory/plan.json',{});(root/'inventory/coverage.jsonl').write_text('')
            member={key:str(root/(key+'.json')) for key in ('run_contract','reviews','assessments','attribution','disposition_applications')}
            for value in member.values():write(value,{})
            member.update(reports=[str(root/'original/wptreport.json')],prepared=str(root/'prepared'),
                canaries={'directory':str(root/'canaries'),'audit':str(root/'audit.json')},placement={'directory':str(root/'placement')})
            write(root/'audit.json',{});write(root/'original/wptreport.json',{})
            write(root/'manifest.json',{})
            plan={'schema_version':2,'core':{'inventory':str(root/'inventory'),'manifest':str(root/'manifest.json'),
                'policy':str(root/'policy.json'),'checkout':str(checkout),'profiles':{'primary':member}},
                'workflows':str(root/'workflows'),'profiles':str(root/'profiles'),
                'receipts':{'test':str(root/'receipt/receipt.json')},'booking_resolution':str(root/'diagnosis.json')}
            candidate={'record':{'artifacts':{'core_disposition_policy':policy}}}
            with patch.object(profiles,'validate_plan',return_value={'inputs':[]}):
                original=final.input_inventory(plan,candidate)
                self.assertTrue({str(source),str(evidence),review['path']} <= {record['path'] for record in original})
                (root/'original/late.json').write_text('{}')
                self.assertNotEqual(final.input_inventory(plan,candidate),original)
                source.write_text('changed assertion after review')
                with self.assertRaisesRegex(ValueError,'missing or changed'):
                    final.input_inventory(plan,candidate)


if __name__ == '__main__': unittest.main()

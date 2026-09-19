"""Source regression coverage for every recovery-cell fault plan and fixed oracle."""
import json
import unittest
from workflow_contract import DEFAULT_CONTRACT
from workflow_native_adapter import MixedCellAdapter,expected_recipe,read_case_contract,family_lines,load,ROOT,research
from workflow_fault_recipes import PREFERRED,decorate

class FaultRecipeTests(unittest.TestCase):
    def test_every_frozen_recovery_cell_has_an_explicit_fault_boundary(self):
        contract=json.loads(DEFAULT_CONTRACT.read_text())
        required={cell['id'] for family in contract['families'] for cell in family['cells'] if cell['recovery_sensitive']}
        self.assertEqual(set(PREFERRED),required)
    def test_all_faults_have_real_steps_and_independent_expected_outcomes(self):
        for cell,name in PREFERRED.items():
            adapter=MixedCellAdapter(cell,ROOT/'native-unbuilt',ROOT/'worker-unbuilt')
            recipe=next(recipe for recipe in adapter.recipes() if recipe[2]==name)
            family,version,_=recipe;contract=read_case_contract(*recipe)
            case=next(case for case in contract['cases'] if case['id']==name)
            runner=research if family=='research' else load('fault_test_'+family,ROOT/f'scripts/run-oil-{family}-outcomes.py')
            lines=family_lines(family,runner,contract,case,'http://example.test/base')
            for fault in ('delayed_response','worker_failure','duplicate_delivery','cancellation'):
                with self.subTest(cell=cell,fault=fault):
                    plan=decorate(family,case,lines,fault,'http://example.test/base')
                    self.assertLess(plan['fault']['before_step'],len(plan['lines']))
                    self.assertEqual(plan['fault']['kind'],fault)
                    self.assertTrue(expected_recipe(recipe,fault)['outcome'])
                    if fault=='cancellation':
                        self.assertTrue(plan['recovery_steps']);self.assertEqual(plan['arm']['skip_matches']>=1,True)
                        self.assertIn('expected_error',plan['case']['step_options'][str(plan['fault']['before_step'])])
                    if fault=='delayed_response':self.assertIsNotNone(plan['arm'])
                    if fault=='duplicate_delivery':self.assertTrue(plan['lines'][plan['fault']['before_step']].startswith('click '))
    def test_fault_decoration_does_not_mutate_frozen_case_or_task(self):
        recipe=('research',3,'generation_bound_stale_reference')
        case=read_case_contract(*recipe)['cases'][0];saved=json.dumps(case,sort_keys=True);lines=list(case['steps'])
        decorate('research',case,lines,'duplicate_delivery','http://example.test/base')
        self.assertEqual(json.dumps(case,sort_keys=True),saved);self.assertEqual(lines,case['steps'])
    def test_duplicate_research_retains_both_actual_extra_requests(self):
        recipe=('research',3,'generation_bound_stale_reference')
        before=expected_recipe(recipe)['outcome'];after=expected_recipe(recipe,'duplicate_delivery')['outcome']
        self.assertEqual(len(after['visits']),len(before['visits'])+2)
    def test_mixed_expected_outcomes_include_companion_backend_oracles(self):
        outcomes=MixedCellAdapter('forms.confinement',ROOT/'native',ROOT/'worker').expected(5,'mixed_family')
        self.assertEqual({item['family'] for item in outcomes[0]['contexts']},{'forms','research','shopping','saas','booking'})
        owner=next(item for item in outcomes[0]['contexts'] if item['family']=='forms')
        self.assertEqual(owner['outcome']['denials'],[dict(path='receipt',status=403)])

class CommitRecipeTests(unittest.TestCase):
    def test_every_mutation_recovery_cell_targets_final_real_mutation(self):
        from types import SimpleNamespace
        from workflow_native_adapter import commit_recipe
        for cell,name in PREFERRED.items():
            if cell.startswith('research.'):continue
            adapter=MixedCellAdapter(cell,ROOT/'native-unbuilt',ROOT/'worker-unbuilt')
            recipe=next(recipe for recipe in adapter.recipes() if recipe[2]==name)
            family,version,_=recipe;contract=read_case_contract(*recipe)
            case=next(case for case in contract['cases'] if case['id']==name)
            runner=load('commit_recipe_'+family,ROOT/f'scripts/run-oil-{family}-outcomes.py')
            lines=family_lines(family,runner,contract,case,'http://127.0.0.1:1234/base')
            boundary=commit_recipe(SimpleNamespace(family=family,case=case,lines=lines))
            with self.subTest(cell=cell):
                self.assertTrue(lines[boundary['before_step']].startswith('click '))
                self.assertEqual(boundary['resume_after'],boundary['before_step'])
                self.assertEqual(boundary['skip_matches'],sum(line==lines[boundary['before_step']] for line in lines)-1)
                self.assertLess(boundary['before_step'],len(lines)-1)

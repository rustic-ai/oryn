"""Synthetic policy/application controls. No real disposition is created."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_reviewed_dispositions as disposition
import wpt_full_qualification as gate
import test_wpt_full_qualification as qualification_fixture


class DispositionTests(unittest.TestCase):
    def setUp(self):
        self.f = qualification_fixture.QualificationTests('runTest')
        self.f.setUp()
        self.addCleanup(self.f.doCleanups)
        self.root = self.f.root
        self.coverage = [self.f.row]
        # Imported history is synthetic only. Production ledger bytes are tested
        # independently below, never edited or copied into a candidate policy.
        defects = []
        for n in range(2):
            relative = f'editing/synthetic-{n}.html'
            source = self.root / relative
            source.parent.mkdir(exist_ok=True)
            source.write_text('synthetic historical predicate')
            variants = [f'?variant={index}' for index in range(4)]
            defects.append(dict(id=f'history-{n}', path=relative, source_sha256=disposition.digest(source),
                                variants=variants, assertion_name_contains='historical assertion',
                                source_predicates=['synthetic historical predicate']))
            self.coverage += [dict(id='testharness:/' + relative + variant, path=relative,
                                   url='/' + relative + variant, tier='necessary', test_type='testharness',
                                   sha256=disposition.digest(source)) for variant in variants]
        self.legacy = self.root / 'synthetic-legacy.json'
        self.legacy.write_text(json.dumps(dict(schema_version=1, upstream=dict(revision=gate.REVISION), defects=defects)))
        for name, value in [('APPROVED_LEDGER', self.legacy), ('APPROVED_LEDGER_SHA256', disposition.digest(self.legacy))]:
            mock = patch.object(disposition, name, value); mock.start(); self.addCleanup(mock.stop)
        self.context = dict(candidate_sha256=self.f.candidate, registry_sha256='b'*64, profile_id='baseline',
                            profile_sha256='c'*64, envelope_sha256='d'*64, run_id='original',
                            collection_id='original-core', attempt_id='attempt-1', original_run_id='original', platform='macOS')
        self.evidence = self.root / 'independent-evidence.txt'
        self.evidence.write_text('Synthetic independent source/assertion/observed mechanism evidence.')
        self.decision = dict(id='synthetic-defect', version=1, previous_decision=None,
            kind='pinned_upstream_assertion_defect', author='Author', upstream_revision=gate.REVISION,
            manifest_sha256=gate.MANIFEST_SHA256, inventory_policy_sha256=gate.POLICY_SHA256,
            targets=[dict(id=self.f.row['id'], path=self.f.row['path'], url=self.f.row['url'],
                          **{'global':'dedicatedworker'}, variant='?x=1', source_sha256=self.f.source_hash,
                          profiles={'baseline':'c'*64}, platforms=['macOS'], expected_subtests=['positive'])],
            source_files={self.f.row['path']: self.f.source_hash},
            source_predicates=[dict(path=self.f.row['path'], text=(self.root/self.f.row['path']).read_text(), line=1, occurrences=1)],
            assertion=dict(name='positive', raw_status='FAIL', message_sha256=disposition.message_digest('synthetic exact failure')),
            expected_affected_assertions=1, upstream_resolution=None, inventory_decision=None)
        self.policy_path = self.root / 'policy.json'
        self.applications_path = self.root / 'applications.json'
        self.freeze()
        self.report = copy.deepcopy(self.f.report)
        self.report['results'][0]['subtests'][0].update(status='FAIL', message='synthetic exact failure')
        self.bind_report()
        self.application = self.application_for(self.report_hash, self.context)
        self.write_applications([self.application])

    def reference(self, path):
        return {'path':str(path.resolve()), 'sha256':disposition.digest(path)}

    def freeze(self, decisions=None):
        decisions = [self.decision] if decisions is None else decisions
        for index, decision in enumerate(decisions):
            payload_hash = disposition.object_digest({k:v for k,v in decision.items() if k not in {'upstream_resolution','inventory_decision'}})
            for field, kind in [('upstream_resolution','independently_reviewed_upstream_resolution'),
                                ('inventory_decision','versioned_inventory_decision')]:
                path = self.root / f'{index}-{field}.json'
                path.write_text(json.dumps(dict(schema_version=1, kind=kind, decision_id=decision['id'],
                    decision_version=decision['version'], decision_sha256=payload_hash, author=decision['author'],
                    reviewer='Independent reviewer', approved=True, rationale='Synthetic reviewed source defect',
                    evidence_artifacts=[self.reference(self.evidence)])))
                decision[field] = self.reference(path)
        self.policy_record = dict(schema_version=1, policy_id='synthetic-policy', revision=1, previous_policy=None,
            upstream_revision=gate.REVISION, manifest_sha256=gate.MANIFEST_SHA256, inventory_policy_sha256=gate.POLICY_SHA256,
            coverage_sha256='e'*64, approved_ledger=self.reference(self.legacy), decisions=decisions)
        self.policy_path.write_text(json.dumps(self.policy_record))
        self.candidate = dict(sha256=self.f.candidate, core_profiles={'baseline':'c'*64},
            record={'artifacts':{'core_disposition_policy':self.reference(self.policy_path)}})
        self.context.update(coverage_sha256='e'*64, profiles={'baseline':'c'*64}, inputs=[self.reference(self.evidence)])

    def bind_report(self):
        self.report['run_info'].update({'oryn_'+key:self.context[key] for key in disposition.IDENTITY_FIELDS})
        self.pair = self.f.write_report(self.report)
        self.report_hash = gate.sha256(self.pair[0])
        self.reviews = self.f.review(self.pair[0])
        self.review = self.reviews[self.f.row['id']][self.report_hash]
        self.review.update({key:self.context[key] for key in disposition.IDENTITY_FIELDS})
        self.review['expected_subtests'] = [row['name'] for row in self.report['results'][0]['subtests']]
        self.context.update(original_report_sha256=self.report_hash,
                            reports={self.report_hash:disposition.profile_identity(self.context)})

    def application_for(self, report_hash, context):
        return dict(**disposition.profile_identity(context), id=self.f.row['id'], report_sha256=report_hash,
            decision_id=self.decision['id'], decision_sha256=disposition.object_digest({k:v for k,v in self.decision.items() if k not in {'upstream_resolution','inventory_decision'}}),
            policy_sha256=disposition.digest(self.policy_path), assertion_name='positive', raw_status='FAIL',
            message_sha256=disposition.message_digest('synthetic exact failure'), source_files=self.decision['source_files'],
            prerequisite_review_sha256=disposition.object_digest(self.reviews[self.f.row['id']][report_hash]),
            operator='Operator', reviewer='Application reviewer', review=None, mechanism_evidence=[self.reference(self.evidence)])

    def write_applications(self, items):
        for index, item in enumerate(items):
            path = self.root / f'application-review-{index}.json'
            path.write_text(json.dumps(dict(schema_version=1, kind='reviewed_disposition_application',
                application_sha256=disposition.object_digest({k:v for k,v in item.items() if k!='review'}),
                operator=item['operator'], reviewer=item['reviewer'], approved=True, observations='Exact synthetic failure mechanism observed.')))
            item['review'] = self.reference(path)
        self.applications_path.write_text(json.dumps(dict(schema_version=1, policy_sha256=disposition.digest(self.policy_path), applications=items)))

    def policy(self):
        return disposition.load_policy(self.policy_path, candidate=self.candidate, coverage=self.coverage,
                                       checkout=self.root, profile_context=self.context)

    def apply(self, errors=None):
        policy = self.policy().load_applications(self.applications_path)
        receipts = policy.apply(self.f.row, self.report['results'][0], self.report_hash, self.review,
                                errors or [], self.context)
        policy.finish()
        return receipts

    def test_exact_review_retains_raw_fail_and_distinct_satisfied_state(self):
        # Reusing a TestCase as a fixture must not export its cases into this
        # module's discovery population. Both original source populations stay.
        loader = unittest.TestLoader()
        self.assertEqual(loader.loadTestsFromModule(sys.modules[__name__]).countTestCases(), 15)
        self.assertEqual(loader.loadTestsFromModule(qualification_fixture).countTestCases(), 20)
        with patch.object(gate, 'CORE_TOTAL', 1):
            rows, summary = gate.reconcile(self.f.core, [self.pair], self.reviews, self.f.candidate, self.root,
                profile_context=self.context, disposition_policy=self.policy(), disposition_applications=self.applications_path)
        self.assertTrue(summary['complete'])
        self.assertEqual(rows[0]['obligation_state'], 'satisfied_by_reviewed_disposition')
        self.assertFalse(rows[0]['raw_pass']); self.assertFalse(rows[0]['qualified_pass'])
        self.assertFalse(rows[0]['attempts'][0]['raw_pass'])
        self.assertEqual(rows[0]['attempts'][0]['raw_subtests'], self.report['results'][0]['subtests'])
        self.assertEqual(summary['raw_attempt_counts'], {'raw_nonpass':1})
        self.assertEqual(summary['imported_approved_core_dispositions'], 0)

    def test_new_scope_kind_source_global_variant_profile_and_population_reject(self):
        original = copy.deepcopy(self.decision)
        changes = [('kind','waive'), ('upstream_revision','wrong'), ('expected_affected_assertions',2)]
        for key,value in changes:
            self.decision = copy.deepcopy(original); self.decision[key]=value; self.freeze()
            with self.subTest(key=key), self.assertRaises(ValueError): self.policy()
        for key,value in [('id','testharness:/dom/*'), ('global','window'), ('variant','?x=2'),
                          ('source_sha256','f'*64), ('profiles',{'undeclared':'c'*64}), ('platforms',['any']),
                          ('expected_subtests',['positive','positive'])]:
            self.decision=copy.deepcopy(original); self.decision['targets'][0][key]=value; self.freeze()
            with self.subTest(key=key), self.assertRaises(ValueError): self.policy()
        self.decision=copy.deepcopy(original); self.decision['source_predicates'][0]['occurrences']=2; self.freeze()
        with self.assertRaises(ValueError): self.policy()

    def test_unknown_fields_duplicate_decisions_and_unversioned_changes_reject(self):
        self.decision['root_prefix']='dom'; self.freeze()
        with self.assertRaises(ValueError): self.policy()
        self.decision.pop('root_prefix'); self.decision['version']=2; self.freeze()
        with self.assertRaises(ValueError): self.policy()
        self.decision['version']=1; self.freeze([self.decision,copy.deepcopy(self.decision)])
        with self.assertRaises(ValueError): self.policy()

    def test_bad_harness_extra_failures_missing_and_duplicate_assertions_reject(self):
        for status in ('TIMEOUT','ERROR','CRASH','SKIP','PRECONDITION_FAILED'):
            self.report['results'][0]['status']=status
            with self.subTest(status=status), self.assertRaises(ValueError): self.apply()
        self.report['results'][0]['status']='OK'
        original=copy.deepcopy(self.report['results'][0]['subtests'])
        for subtests in ([], original*2, original+[dict(name='unexpected',status='FAIL')],
                         [dict(name='positive',status='PASS')], [dict(name='positive',status='NOTRUN')]):
            self.report['results'][0]['subtests']=subtests
            with self.subTest(subtests=subtests), self.assertRaises(ValueError): self.apply()

    def test_prerequisites_cleanup_and_review_identity_cannot_be_disposed(self):
        for errors in (['independent_prerequisite_review_missing'], ['cleanup_complete'], ['source_hash_mismatch']):
            with self.subTest(errors=errors), self.assertRaises(ValueError): self.apply(errors)
        self.review['cleanup_complete']=False
        with self.assertRaises(ValueError): self.apply()

    def test_application_absence_duplicates_unused_raw_identity_and_message_reject(self):
        original=copy.deepcopy(self.application)
        for items in ([], [copy.deepcopy(original),copy.deepcopy(original)]):
            self.write_applications(items)
            with self.subTest(items=len(items)), self.assertRaises(ValueError): self.apply()
        for key,value in [('id','testharness:/unknown'), ('candidate_sha256','f'*64), ('profile_id','foreign'),
                          ('report_sha256','f'*64), ('envelope_sha256','f'*64), ('run_id','later'),
                          ('message_sha256','f'*64), ('prerequisite_review_sha256','f'*64),
                          ('source_files',{}), ('decision_sha256','f'*64)]:
            item=copy.deepcopy(original);item[key]=value;self.write_applications([item])
            with self.subTest(key=key), self.assertRaises(ValueError): self.apply()

    def test_self_review_whitespace_and_case_equivalents_reject(self):
        for reviewer in ('Operator',' OPERATOR '):
            item=copy.deepcopy(self.application);item['reviewer']=reviewer;self.write_applications([item])
            with self.subTest(reviewer=reviewer), self.assertRaises(ValueError): self.apply()
        self.freeze()
        path=Path(self.decision['upstream_resolution']['path']);value=json.loads(path.read_text())
        value['reviewer']=' AUTHOR ';path.write_text(json.dumps(value));self.decision['upstream_resolution']=self.reference(path)
        self.policy_record['decisions']=[self.decision];self.policy_path.write_text(json.dumps(self.policy_record))
        self.candidate['record']['artifacts']['core_disposition_policy']=self.reference(self.policy_path)
        with self.assertRaises(ValueError): self.policy()

    def test_policy_candidate_and_evidence_mutation_reject(self):
        policy=self.policy().load_applications(self.applications_path)
        self.evidence.write_text('changed after intake')
        with self.assertRaises(ValueError): policy.recheck()
        with self.assertRaises(ValueError): self.policy()
        self.candidate['record']['artifacts']['core_disposition_policy']['sha256']='f'*64
        with self.assertRaises(ValueError): self.policy()

    def test_policy_merge_cannot_replace_previously_verified_context_hash(self):
        policy=self.policy()
        finish=policy.finish
        def mutate_after_validation():
            finish()
            canonical = str(self.evidence.resolve())
            self.assertIn(canonical, policy.inputs)
            self.evidence.write_text('changed after earlier context verification')
            policy.inputs[canonical]=disposition.digest(self.evidence)
        with patch.object(gate,'CORE_TOTAL',1),patch.object(policy,'finish',side_effect=mutate_after_validation),\
             self.assertRaisesRegex(ValueError,'input identity changed'):
            gate.reconcile(self.f.core,[self.pair],self.reviews,self.f.candidate,self.root,
                profile_context=self.context,disposition_policy=policy,disposition_applications=self.applications_path)

    def test_historical_scope_cannot_be_promoted_to_core(self):
        self.coverage[1]['tier']='core'
        with self.assertRaisesRegex(ValueError,'zero-Core'): self.policy()

    def test_cross_profile_missing_original_and_denominator_reject(self):
        with self.assertRaisesRegex(ValueError,'denominator'):
            gate.reconcile(self.f.core,[self.pair],self.reviews,self.f.candidate,self.root,
                profile_context=self.context,disposition_policy=self.policy(),disposition_applications=self.applications_path)
        with patch.object(gate,'CORE_TOTAL',1):
            context=copy.deepcopy(self.context);context['original_report_sha256']='f'*64
            with self.assertRaisesRegex(ValueError,'original'):
                gate.reconcile(self.f.core,[self.pair],self.reviews,self.f.candidate,self.root,
                    profile_context=context,disposition_policy=self.policy(),disposition_applications=self.applications_path)
        with self.assertRaisesRegex(ValueError,'downgrade'):
            gate.reconcile(self.f.core,[self.pair],self.reviews,self.f.candidate,self.root)

    def test_unaffected_failure_cannot_hide_behind_one_valid_disposition(self):
        self.decision['targets'][0]['expected_subtests']=['positive','unaffected']
        self.freeze()
        self.report['results'][0]['subtests'].append(dict(name='unaffected',status='FAIL',message='other defect'))
        self.bind_report()
        self.application=self.application_for(self.report_hash,self.context)
        self.write_applications([self.application])
        with self.assertRaisesRegex(ValueError,'unaffected assertion'):
            self.apply()

    def test_later_pass_cannot_replace_failed_original_in_same_profile(self):
        self.freeze(decisions=[])
        self.write_applications([])
        retry_context=dict(disposition.profile_identity(self.context),run_id='retry',attempt_id='attempt-2',
                           collection_id='retry-core',envelope_sha256='f'*64)
        retry=copy.deepcopy(self.report)
        retry['run_info'].update({'oryn_'+key:retry_context[key] for key in disposition.IDENTITY_FIELDS})
        retry['results'][0]['subtests'][0]['status']='PASS'
        retry_pair=self.f.write_report(retry,'retry.json');retry_hash=gate.sha256(retry_pair[0])
        retry_reviews=self.f.review(retry_pair[0]);retry_review=retry_reviews[self.f.row['id']][retry_hash]
        retry_review.update(retry_context)
        self.reviews[self.f.row['id']].update(retry_reviews[self.f.row['id']])
        self.context['reports'][retry_hash]=retry_context
        with patch.object(gate,'CORE_TOTAL',1):
            rows,summary=gate.reconcile(self.f.core,[self.pair,retry_pair],self.reviews,self.f.candidate,self.root,
                profile_context=self.context,disposition_policy=self.policy(),disposition_applications=self.applications_path)
        self.assertEqual(rows[0]['state'],'open');self.assertFalse(summary['complete'])
        self.assertEqual(len(rows[0]['attempts']),2)
        self.assertEqual(summary['raw_attempt_counts'],{'raw_nonpass':1,'raw_pass':1})

    def test_missing_automatic_population_and_foreign_report_profile_reject(self):
        second=dict(self.f.row,id='testharness:/dom/second.html',url='/dom/second.html')
        with patch.object(gate,'CORE_TOTAL',2),self.assertRaisesRegex(ValueError,'complete automatic'):
            gate.reconcile({**self.f.core,second['id']:second},[self.pair],self.reviews,self.f.candidate,self.root,
                profile_context=self.context,disposition_policy=self.policy(),disposition_applications=self.applications_path)
        self.report['run_info']['oryn_profile_id']='other'
        pair=self.f.write_report(self.report);digest=gate.sha256(pair[0])
        self.context['original_report_sha256']=digest
        self.context['reports']={digest:disposition.profile_identity(self.context)}
        with patch.object(gate,'CORE_TOTAL',1),self.assertRaisesRegex(ValueError,'raw report profile'):
            gate.reconcile(self.f.core,[pair],self.reviews,self.f.candidate,self.root,
                profile_context=self.context,disposition_policy=self.policy(),disposition_applications=self.applications_path)


class ApprovedLedgerScopeTests(unittest.TestCase):
    def test_unchanged_real_ledger_has_eight_necessary_and_zero_core_variants(self):
        self.assertEqual(disposition.digest(disposition.APPROVED_LEDGER), disposition.APPROVED_LEDGER_SHA256)
        spec=importlib.util.spec_from_file_location('disposition_tiers',disposition.ROOT/'scripts/catalog-wpt-capabilities.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        policy=json.loads((disposition.ROOT/'benchmarks/conformance/wpt-capability-tiers-v2.json').read_text())
        ledger=json.loads(disposition.APPROVED_LEDGER.read_text())
        tiers=[module.classify(item['path'],policy)[0] for item in ledger['defects'] for _ in item['variants']]
        self.assertEqual(tiers,['necessary']*8)
        self.assertNotIn('core',tiers)

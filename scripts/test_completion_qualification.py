"""Final integration negatives use controlled validator fixtures, never native execution."""
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import completion_qualification as qualification


class CompletionQualificationTests(unittest.TestCase):
    def test_cycle_free_booking_path_pins_completed_diagnosis_before_use(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();diagnosis=root/'diagnosis.json';plan=root/'plan.json'
            # Plan/candidate are frozen before their output diagnosis exists.
            plan.write_text(json.dumps(dict(booking_resolution=str(diagnosis))))
            candidate=root/'candidate.json';candidate.write_text(json.dumps(dict(qualification_plan=qualification.reference(plan))))
            original_candidate=qualification.reference(candidate)
            diagnosis.write_text(json.dumps(dict(candidate_manifest_sha256=original_candidate['sha256'])))
            frozen=[qualification.booking_evidence_record(json.loads(plan.read_text()))]
            self.assertEqual(qualification.booking_evidence_record(json.loads(plan.read_text()),frozen),frozen[0])
            self.assertEqual(qualification.reference(candidate),original_candidate)
            diagnosis.write_text('{"substituted":true}')
            with self.assertRaises(ValueError):qualification.booking_evidence_record(json.loads(plan.read_text()),frozen)
            other=root/'other.json';other.write_text('{}')
            with self.assertRaisesRegex(ValueError,'absent'):
                qualification.booking_evidence_record(dict(booking_resolution=str(other)),frozen)
            with self.assertRaisesRegex(ValueError,'output path'):
                qualification.booking_evidence_record(dict(booking_resolution=qualification.reference(other)))

    def test_booking_before_after_requires_both_original_receipts(self):
        import completion_execution as execution
        from completion_provenance import REQUIRED_ARTIFACTS
        from completion_source_snapshot import inventory,write,source_digest
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();source=root/'old-source';source.mkdir();(source/'implementation.rs').write_text('old implementation')
            records=inventory(source,['implementation.rs']);snapshot=root/'old.tar';write(snapshot,source,records)
            placeholder=root/'artifact';placeholder.write_text('retained bytes')
            plan=root/'plan.json';plan.write_text(json.dumps(dict(booking_regression_name='booking_mechanism')))
            gate=root/'gate.json';gate.write_text('{}')
            before_records={name:qualification.reference(placeholder) for name in REQUIRED_ARTIFACTS}
            before_records.update(source_snapshot=qualification.reference(snapshot),qualification_plan=qualification.reference(plan),booking_regression=qualification.reference(placeholder))
            old_record=dict(schema_version=1,source_tree_sha256=source_digest(records),artifacts=before_records)
            before_path=root/'before-candidate.json';before_path.write_text(json.dumps(old_record))
            before=dict(path=before_path,sha256=qualification.digest(before_path),record=old_record,
                        artifacts={name:Path(record['path']) for name,record in before_records.items()})
            final_records=inventory(qualification.ROOT,['scripts/completion_qualification.py']);final_snapshot=root/'final.tar';write(final_snapshot,qualification.ROOT,final_records)
            after_path=root/'after-candidate.json';after_path.write_text('{"final":true}')
            after=dict(path=after_path,sha256=qualification.digest(after_path),record=dict(source_tree_sha256=source_digest(final_records)),artifacts=dict(before['artifacts'],source_snapshot=final_snapshot))
            for name,bound,code in [('before',before,101),('after',after,0)]:
                def run(argv,target,**kwargs):
                    status='FAILED' if code else 'ok';counts='0 passed; 1 failed;' if code else '1 passed; 0 failed;'
                    (target/'stdout.log').write_text('test booking_mechanism ... '+status+'\ntest result: '+status+'. '+counts)
                    (target/'stderr.log').write_text('')
                    return dict(argv=argv,returncode=code,timed_out=False,error=None,cleanup_complete=True)
                with patch.object(execution,'generation_gate'),patch.object(execution,'load_candidate',return_value=bound),patch.object(execution,'source_hash',return_value=bound['record']['source_tree_sha256']),patch.object(execution,'run_owned',side_effect=run):
                    execution.produce('booking-regression',bound['path'],gate,root/name)
            original=qualification.ROOT/'benchmarks/evidence/booking-signed-2026-09-14/required_blocks-01/result.json'
            value=dict(schema_version=1,candidate_manifest_sha256=after['sha256'],after_candidate_manifest_sha256=after['sha256'],
                       confirmed_cause='test fixture diagnosis',diagnosis='test fixture evidence',author='a',reviewer='b',regression_name='booking_mechanism',
                       original_failure=qualification.reference(original),changed_sources=[qualification.reference(qualification.ROOT/'scripts/completion_qualification.py')],
                       original_trace={name:qualification.reference(original.parent/name) for name in qualification.BOOKING_ORIGINAL_TRACE},
                       regression_before=qualification.reference(root/'before/stdout.log'),regression_before_receipt=qualification.reference(root/'before/receipt.json'),
                       regression_before_candidate=qualification.reference(before_path),regression_after=qualification.reference(root/'after/stdout.log'),regression_receipt=qualification.reference(root/'after/receipt.json'))
            diagnosis=root/'diagnosis.json';diagnosis.write_text(json.dumps(value))
            qualification.booking_resolution(qualification.reference(diagnosis),after)
            saved_sources=value['changed_sources'];value['changed_sources']=[qualification.reference(qualification.ROOT/'scripts/workflow_qualification.py')];diagnosis.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError,'candidate difference'):
                qualification.booking_resolution(qualification.reference(diagnosis),after)
            value['changed_sources']=saved_sources
            saved=value['original_trace']['stderr.txt'];value['original_trace']['stderr.txt']=qualification.reference(placeholder);diagnosis.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError,'trace substituted'):
                qualification.booking_resolution(qualification.reference(diagnosis),after)
            value['original_trace']['stderr.txt']=saved
            value['regression_before_receipt']=qualification.reference(root/'after/receipt.json');diagnosis.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError,'identity mismatch'):
                qualification.booking_resolution(qualification.reference(diagnosis),after)

    def test_framework_join_forwards_build_identity_to_component_validator(self):
        candidate={'record':{'source_tree_sha256':'a'*64,'artifacts':{
            'runtime':{'sha256':'b'*64},'worker':{'sha256':'c'*64},
            'framework_build':{'sha256':'d'*64}}}}
        seen=[]
        def validate(summary,identity,*args):
            seen.append(identity)
            return []
        validator=SimpleNamespace(validate_summary=validate,FRAMEWORKS=[])
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve()
            summary={'records':[],'build_manifest_sha256':'d'*64}
            (root/'summary.json').write_text(json.dumps(summary))
            with patch.object(qualification,'module',side_effect=[validator,SimpleNamespace()]):
                self.assertEqual(qualification.framework(root,candidate),summary)
        self.assertEqual(seen,[dict(parent_sha256='b'*64,worker_sha256='c'*64,
                                    source_tree_sha256='a'*64,framework_build_sha256='d'*64)])

    def exercise(self, fail=None, core_complete=True, q_candidate=True, input_drift=False, gate_drift=False, restore_gate=False, component_gate_drift=False, identity_drift=None, drift_stage='component', receipt_failure=None, component_failure=None, current_profiles=False, generated_drift=None, historical_opt_in=True, registry_candidate=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate_path = root / 'candidate.json'; candidate_path.write_text('{}')
            plan_path = root / 'plan.json'
            gate_path = root / 'gate.json'; gate_path.write_text('{}')
            diagnosis_path = root / 'diagnosis.json'; diagnosis_path.write_text('{}')
            security_raw = root / 'security/raw'; security_raw.mkdir(parents=True)
            (security_raw / 'security-result-v3.json').write_text(json.dumps(dict(fingerprint=dict(worker=dict(sha256='b'*64), parent=dict(sha256='e'*64), source_tree_sha256='a'*64))))
            plan = dict(schema_version=1, receipts={name: str(root/name/'receipt.json') for name in ('frameworks', 'security', 'architecture', 'reliability', 'native-prerequisites', 'fixture-topology', 'native-media')},
                        core=dict(inventory='inventory', manifest='manifest', policy='policy', checkout='checkout', reviews='reviews', reports=['report']),
                        workflows='workflows', profiles='profiles', booking_resolution=str(diagnosis_path))
            if current_profiles: plan['schema_version'] = 2
            plan_path.write_text(json.dumps(plan))
            bound = dict(path=candidate_path, sha256='c'*64, artifacts={'core_profile_registry': 'synthetic'} if registry_candidate else {}, record=dict(source_tree_sha256='a'*64,
                         artifacts=dict(qualification_plan=qualification.reference(plan_path), worker=dict(sha256='b'*64), runtime=dict(sha256='e'*64))))
            output = root / 'final'
            commands = []
            identity_changed = False
            identity_observed = False
            def validator(argv, target, admission):
                nonlocal identity_changed
                commands.append(argv)
                if fail and fail in argv[1]:
                    raise ValueError('intentional validator failure')
                if 'reconcile-full' in argv[1]:
                    folder = output / 'core-ledger'; folder.mkdir()
                    (folder / 'summary.json').write_text(json.dumps(dict(complete=core_complete, core_total=16737, counts={'qualified_pass':16737})))
                    if gate_drift:
                        gate_path.write_text('{"different_generation_declaration":true}')
                    if identity_drift and drift_stage == 'command':
                        identity_changed = True
                elif 'qualify-workflow-q' in argv[1]:
                    if generated_drift == 'ledger': (output/'core-ledgers/primary/ledger.jsonl').write_text('changed after Core accepted')
                    elif generated_drift == 'summary': (output/'core-ledgers/primary/summary.json').write_text('{}')
                    elif generated_drift == 'aggregate': (output/'core-profiles-summary.json').write_text('{}')
                    elif generated_drift == 'added': (output/'core-ledgers/primary/late.json').write_text('{}')
                    elif generated_drift == 'removed': (output/'core-ledgers/primary/ledger.jsonl').unlink()
                    (output / 'q.json').write_text(json.dumps(dict(candidate_manifest_sha256=('c' if q_candidate else 'd')*64, q_component_checks_passed=True)))
                return dict(returncode=0)
            with ExitStack() as stack:
                gate_checks = []
                def accept_gate(path):
                    nonlocal identity_changed
                    gate_checks.append(path)
                    if restore_gate and len(gate_checks) > 1 and (not identity_drift or identity_observed):
                        gate_path.write_text('{}')
                        identity_changed = False
                stack.enter_context(patch.object(qualification, 'generation_gate', side_effect=accept_gate))
                for name, value in [('load_candidate', bound), ('source_hash', 'a'*64),
                                    ('verify_receipt', {}), ('workflow', {}), ('framework', {}), ('architecture', {}), ('booking_resolution', {}), ('native_prerequisites', {}), ('fixture_topology', {}), ('native_media', {})]:
                    stack.enter_context(patch.object(qualification, name, return_value=value))
                if receipt_failure:
                    def reject_receipt(path, kind, candidate):
                        if kind == receipt_failure:
                            raise ValueError('original native receipt rejected')
                        return {}
                    stack.enter_context(patch.object(qualification, 'verify_receipt', side_effect=reject_receipt))
                if component_failure:
                    stack.enter_context(patch.object(qualification, component_failure,
                                                     side_effect=ValueError('native evidence rejected')))
                if component_gate_drift:
                    def failed_component(*args):
                        gate_path.unlink()
                        raise ValueError('component failed after removing declaration')
                    stack.enter_context(patch.object(qualification, 'fixture_topology', side_effect=failed_component))
                if identity_drift:
                    def candidate_identity(path):
                        nonlocal identity_observed
                        if identity_changed:
                            identity_observed = True
                        return dict(bound, sha256='d'*64) if identity_changed and identity_drift == 'manifest' else bound
                    stack.enter_context(patch.object(qualification, 'load_candidate', side_effect=candidate_identity))
                    stack.enter_context(patch.object(qualification, 'source_hash', side_effect=lambda: ('d' if identity_changed and identity_drift == 'source' else 'a')*64))
                    if drift_stage == 'component':
                        def changed_component(*args):
                            nonlocal identity_changed
                            identity_changed = True
                            raise ValueError('component changed candidate before failing')
                        stack.enter_context(patch.object(qualification, 'fixture_topology', side_effect=changed_component))
                inventory_calls = 0
                def inventory(plan, candidate=None):
                    nonlocal inventory_calls, identity_changed
                    inventory_calls += 1
                    if identity_drift and drift_stage == 'preflight' and inventory_calls == 1:
                        identity_changed = True
                    return [dict(path='changed',sha256='d'*64)] if input_drift and inventory_calls > 1 else [qualification.reference(diagnosis_path)]
                if current_profiles:
                    import core_profile_contract, architecture_prebuilt
                    stack.enter_context(patch.object(core_profile_contract, 'validate_plan'))
                    def generated_core(*args):
                        members = {}
                        for name in ('primary', 'allow', 'legacy'):
                            folder = output/'core-ledgers'/name; folder.mkdir(parents=True)
                            (folder/'summary.json').write_text('{"accepted_synthetic_fixture":true}')
                            (folder/'ledger.jsonl').write_text('{"accepted_synthetic_fixture":true}\n')
                            members[name] = {key: qualification.reference(folder/filename)
                                for key, filename in [('summary','summary.json'),('ledger','ledger.jsonl')]}
                        aggregate = dict(profiles=members, complete=True)
                        (output/'core-profiles-summary.json').write_text(json.dumps(aggregate))
                        aggregate['generated_artifacts'] = [qualification.reference(output/'core-profiles-summary.json')]
                        aggregate['generated_artifacts'] += [r for member in members.values() for r in member.values()]
                        return aggregate
                    stack.enter_context(patch.object(qualification, 'qualify_core_profiles', side_effect=generated_core))
                    def architecture_after_security(receipt, candidate, security, security_receipt):
                        self.assertTrue(any('validate_security_result' in argv[1] for argv in commands))
                        self.assertEqual(security['fingerprint']['worker']['sha256'], 'b'*64)
                        return {'synthetic_original_architecture_join': True}
                    stack.enter_context(patch.object(architecture_prebuilt, 'qualify_architecture', side_effect=architecture_after_security))
                stack.enter_context(patch.object(qualification, 'input_inventory', side_effect=inventory))
                stack.enter_context(patch.object(qualification, 'validate_command', side_effect=validator))
                result = qualification.qualify(candidate_path, gate_path, plan_path, output, allow_historical=historical_opt_in)
            return result, commands

    def test_all_actual_validator_domains_are_required_for_positive_join(self):
        result, commands = self.exercise()
        self.assertTrue(result['qualified'])
        self.assertEqual(set(result['components']), {'core','workflows','frameworks','q','security','architecture','reliability','booking_resolution','native_prerequisites','fixture_topology','native_media'})
        security = next(argv for argv in commands if 'validate_security_result' in argv[1])
        self.assertIn('--require-raw', security)
        self.assertNotIn('--allow-historical', security)

    def test_historical_route_requires_explicit_opt_in_and_only_passes_it_to_core(self):
        with self.assertRaisesRegex(ValueError, 'current completion requires'):
            self.exercise(historical_opt_in=False)
        with self.assertRaisesRegex(ValueError, 'cannot downgrade'):
            self.exercise(registry_candidate=True)
        result, commands = self.exercise()
        self.assertEqual(result['qualification_scope'], 'historical_single_profile')
        self.assertFalse(result['current_plan_complete'])
        core = next(argv for argv in commands if 'reconcile-full' in argv[1])
        self.assertIn('--allow-historical', core)

    def test_required_profile_outputs_are_rechecked_after_later_validators(self):
        positive, commands = self.exercise(current_profiles=True, historical_opt_in=False)
        self.assertTrue(positive['qualified'])
        self.assertTrue(positive['current_plan_complete'])
        for mutation in ('ledger', 'summary', 'aggregate', 'added', 'removed'):
            with self.subTest(mutation=mutation):
                result, _ = self.exercise(current_profiles=True, generated_drift=mutation, historical_opt_in=False)
                self.assertFalse(result['qualified'])
                self.assertTrue(any('generated Core integrity' in error for error in result['errors']))

    def test_architecture_join_cannot_run_after_failed_security(self):
        result, _ = self.exercise(current_profiles=True, fail='validate_security_result', historical_opt_in=False)
        self.assertFalse(result['qualified'])
        self.assertNotIn('architecture', result['components'])
        self.assertTrue(any('architecture requires successfully validated' in error for error in result['errors']))

    def test_native_domains_require_both_original_receipt_and_evidence(self):
        for receipt, component in [('native-media', 'native_media'),
                                   ('native-prerequisites', 'native_prerequisites'),
                                   ('fixture-topology', 'fixture_topology')]:
            with self.subTest(receipt=receipt):
                result, _ = self.exercise(receipt_failure=receipt)
                self.assertFalse(result['qualified'])
                self.assertNotIn(component, result['components'])
                self.assertTrue(any('original native receipt rejected' in error
                                    for error in result['errors']))
            with self.subTest(component=component):
                result, _ = self.exercise(component_failure=component)
                self.assertFalse(result['qualified'])
                self.assertNotIn(component, result['components'])
                self.assertTrue(any('native evidence rejected' in error
                                    for error in result['errors']))

    def test_component_exit_failure_never_becomes_final_acceptance(self):
        for name in ('reconcile-full', 'qualify-workflow-q', 'validate_security_result'):
            result, _ = self.exercise(fail=name)
            self.assertFalse(result['qualified'], name)
            self.assertTrue(result['errors'])

    def test_partial_core_or_foreign_q_candidate_cannot_pass(self):
        self.assertFalse(self.exercise(core_complete=False)[0]['qualified'])
        self.assertFalse(self.exercise(q_candidate=False)[0]['qualified'])

    def test_raw_input_drift_after_successful_validators_stays_failed(self):
        result, _ = self.exercise(input_drift=True)
        self.assertFalse(result['qualified'])
        self.assertTrue(any('input integrity' in error for error in result['errors']))

    def test_changed_generation_declaration_fails_even_when_restored_before_final_check(self):
        for restore in (False, True):
            result, commands = self.exercise(gate_drift=True, restore_gate=restore)
            self.assertFalse(result['qualified'])
            self.assertTrue(any('generation gate changed' in error for error in result['errors']))
            self.assertFalse(any('qualify-workflow-q' in argv[1] for argv in commands))
            self.assertFalse(any('validate_security_result' in argv[1] for argv in commands))
            self.assertEqual(result['generation_gate']['sha256'], hashlib.sha256(b'{}').hexdigest())

    def test_global_gate_precedes_all_final_validation(self):
        with patch.object(qualification, 'generation_gate', side_effect=ValueError('incomplete')), patch.object(qualification, 'load_candidate') as load:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                qualification.qualify('candidate', 'gate', 'plan', 'output')
            load.assert_not_called()

    def test_candidate_drift_stops_later_actions_even_if_restored_at_final_check(self):
        for identity in ('manifest', 'source'):
            for stage in ('preflight', 'component', 'command'):
                with self.subTest(identity=identity, stage=stage):
                    result, commands = self.exercise(identity_drift=identity, drift_stage=stage, restore_gate=True)
                    self.assertFalse(result['qualified'])
                    self.assertEqual(len(commands), 1 if stage == 'command' else 0)
                    self.assertTrue(any('candidate changed during qualification' in error for error in result['errors']))
                    self.assertTrue(any('final integrity: candidate changed during qualification' in error for error in result['errors']))

    def test_failed_component_cannot_hide_removed_gate_or_resume_after_restoration(self):
        result, commands = self.exercise(component_gate_drift=True, restore_gate=True)
        self.assertFalse(result['qualified'])
        self.assertEqual(commands, [])
        self.assertTrue(any('fixture_topology: generation gate changed' in error for error in result['errors']))
        self.assertTrue(any('final integrity: generation gate changed' in error for error in result['errors']))
        self.assertEqual(result['generation_gate']['sha256'], hashlib.sha256(b'{}').hexdigest())

    def test_architecture_pending_release_attestation_rejects_even_cutover_ready(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve()
            (path/'stdout.log').write_text(json.dumps(dict(cutover_ready=True, pending=[], legacy_production_markers=[], pending_release_attestations=['macos-app-sandbox'])))
            with self.assertRaisesRegex(ValueError, 'attestations'):
                qualification.architecture(path/'receipt.json')

    def test_booking_green_repeat_is_not_confirmed_mechanism_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve()/'diagnosis.json'
            path.write_text(json.dumps(dict(schema_version=1, candidate_manifest_sha256='a'*64, passed=True)))
            with self.assertRaisesRegex(ValueError, 'confirmed diagnosis'):
                qualification.booking_resolution(qualification.reference(path), dict(sha256='a'*64))

    def test_booking_diagnosis_cannot_substitute_original_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            unrelated = root / 'failure.json'; unrelated.write_text('{"passed":false,"case":"different"}')
            path = root / 'diagnosis.json'
            value = dict(schema_version=1, candidate_manifest_sha256='a'*64,
                         confirmed_cause='claimed cause', diagnosis='claimed diagnosis', author='author', reviewer='reviewer',
                         regression_name='booking_mechanism', original_failure=qualification.reference(unrelated))
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'preserved original failed attempt'):
                qualification.booking_resolution(qualification.reference(path), dict(sha256='a'*64))
            original = qualification.ROOT / 'benchmarks/evidence/booking-signed-2026-09-14/required_blocks-01/result.json'
            copied = root / 'original-copy.json'; copied.write_bytes(original.read_bytes())
            value['original_failure'] = qualification.reference(copied)
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'source-bound implementation'):
                qualification.booking_resolution(qualification.reference(path), dict(sha256='a'*64))

    def test_topology_component_rejects_failed_and_foreign_observations_before_join(self):
        for record in ({'candidate_manifest_sha256': 'foreign', 'topology_passed': True, 'errors': []},
                       {'candidate_manifest_sha256': 'c' * 64, 'topology_passed': False, 'errors': []},
                       {'candidate_manifest_sha256': 'c' * 64, 'topology_passed': True, 'errors': ['peer failed']}):
            with patch.object(qualification, 'read', return_value=record), self.assertRaisesRegex(ValueError, 'topology failed'):
                qualification.fixture_topology('/tmp/topology/receipt.json', {'sha256': 'c' * 64})

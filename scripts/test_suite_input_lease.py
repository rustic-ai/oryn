"""Metadata invalidation and suite admission/finalization, without native jobs."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import generation_gate as gates
from suite_input_lease import MetadataSnapshot, candidate_snapshot, source_members, workflow_members

ROOT = Path(__file__).resolve().parents[1]


class SuiteInputLeaseTests(unittest.TestCase):
    def test_historical_absent_reference_is_tracked_until_created(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            missing = root / 'old-target' / 'binary'
            snapshot = MetadataSnapshot([missing])
            snapshot.check()
            missing.parent.mkdir()
            missing.write_text('replacement')
            with self.assertRaisesRegex(ValueError, 'metadata changed'):
                snapshot.check()

    def test_restored_mtime_does_not_hide_same_size_content_change_or_restore(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input'; path.write_bytes(b'original')
            old = path.stat(); lease = MetadataSnapshot([path])
            path.write_bytes(b'modified'); os.utime(path, ns=(old.st_atime_ns, old.st_mtime_ns))
            with self.assertRaises(ValueError): lease.check()
            path.write_bytes(b'original'); os.utime(path, ns=(old.st_atime_ns, old.st_mtime_ns))
            with self.assertRaisesRegex(ValueError, 'revoked'): lease.check()

    def test_replacement_and_symlink_retarget_are_detected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); a = root/'a'; b = root/'b'
            a.write_bytes(b'same'); b.write_bytes(b'same'); link=root/'link'; link.symlink_to(a)
            snapshot = MetadataSnapshot([link])
            link.unlink(); link.symlink_to(b)
            with self.assertRaises(ValueError): snapshot.check()
            snapshot = MetadataSnapshot([a]); os.replace(b,a)
            with self.assertRaises(ValueError): snapshot.check()

    def test_real_git_membership_detects_add_delete_but_excludes_active_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            subprocess.run(['git','init','-q',str(root)],check=True)
            (root/'.gitignore').write_text('artifacts/\n')
            (root/'code.py').write_text('original')
            snapshot=MetadataSnapshot([],membership=lambda:source_members(root))
            (root/'artifacts').mkdir();(root/'artifacts'/'running.log').write_text('progress')
            snapshot.check()
            (root/'new.py').write_text('new source')
            with self.assertRaisesRegex(ValueError,'membership'):snapshot.check()
            (root/'new.py').unlink()
            snapshot=MetadataSnapshot([],membership=lambda:source_members(root))
            (root/'code.py').unlink()
            with self.assertRaisesRegex(ValueError,'membership'):snapshot.check()

    def test_ignored_workflow_population_and_suite_helper_are_bound(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            subprocess.run(['git','init','-q',str(root)],check=True)
            (root/'.gitignore').write_text('test-harness/workflows/ignored.json\n')
            for folder in ('scripts','test-harness/workflows','tools/wptrunner-oryn','benchmarks/app-compatibility'):
                (root/folder).mkdir(parents=True,exist_ok=True)
            for name in ('fixtures.lock.json','fixtures-v1.lock.json','fixtures-v2.lock.json'):
                (root/'benchmarks/app-compatibility'/name).write_text('{}')
            helper=root/'scripts/suite_input_lease.py';helper.write_text('helper')
            with patch('workflow_input_lock.baseline',return_value={'regression_inputs':[]}):
                self.assertIn(str(helper),workflow_members(root))
                snapshot=MetadataSnapshot([],membership=lambda:tuple(sorted(set(source_members(root))|set(workflow_members(root)))))
                ignored=root/'test-harness/workflows/ignored.json';ignored.write_text('{}')
                self.assertNotIn(str(ignored),source_members(root))
                with self.assertRaisesRegex(ValueError,'membership'):snapshot.check()

    def test_bundle_membership_addition_and_nested_parent_symlink_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); bundle=root/'app';bundle.mkdir();(bundle/'bin').write_text('bin')
            snapshot=MetadataSnapshot([],trees=[bundle]);(bundle/'injected').write_text('extra')
            with self.assertRaisesRegex(ValueError,'membership'):snapshot.check()
            first=root/'first';second=root/'second';first.mkdir();second.mkdir()
            for p in (first,second):(p/'input').write_text('same')
            link=root/'parent';link.symlink_to(first,target_is_directory=True)
            snapshot=MetadataSnapshot([link/'input']);link.unlink();link.symlink_to(second,target_is_directory=True)
            with self.assertRaises(ValueError):snapshot.check()

    def test_extensionless_reference_closure_and_relative_tls_workflow_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); source=root/'source.py';source.write_text('source')
            material=root/'external';material.mkdir();key=material/'key';key.write_text('key')
            helper=material/'helper';helper.write_text('helper')
            extensionless=material/'receipt';extensionless.write_text(json.dumps({'evidence_artifacts':{str(helper):'a'*64}}))
            fixture=material/'fixture';fixture.write_text(json.dumps({'tls':{'key':{'path':'key','sha256':'b'*64}},'receipt':{'path':str(extensionless),'sha256':'c'*64}}))
            lock=material/'workflow.json';lock.write_text(json.dumps({'base_contract_sha256':'d'*64,'inputs':[{'path':'source.py','sha256':'e'*64}]}))
            gate=root/'gate';gate.write_text('{}')
            candidate=root/'candidate';candidate.write_text(json.dumps({'artifacts':{'fixture':{'path':str(fixture),'sha256':'f'*64},'workflow_contract':{'path':str(lock),'sha256':'1'*64}}}))
            with patch('suite_input_lease.source_members',return_value=()), patch('suite_input_lease.workflow_members',return_value=()):
                snapshot=candidate_snapshot(candidate,gate,root)
                self.assertTrue({str(source),str(key),str(helper)} <= set(snapshot.files))
                helper.write_text('changed')
                with self.assertRaises(ValueError):snapshot.check()

    def test_discovery_to_capture_race_cannot_rebaseline_changed_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);gate=root/'gate';gate.write_text('{}')
            candidate=root/'candidate';candidate.write_text('{"artifacts":{}}')
            original=MetadataSnapshot
            def race(*args,**kwargs):
                candidate.write_text('{"artifacts":{},"replacement":true}')
                return original(*args,**kwargs)
            with patch('suite_input_lease.source_members',return_value=()),patch('suite_input_lease.workflow_members',return_value=()),patch('suite_input_lease.MetadataSnapshot',side_effect=race):
                with self.assertRaisesRegex(ValueError,'before snapshot admission'):
                    candidate_snapshot(candidate,gate,root)

    def test_architecture_legacy_tree_addition_and_removal_revoke_candidate(self):
        for change in ('add', 'remove'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                legacy = root/'legacy'; legacy.mkdir()
                source = legacy/'source.rs'; source.write_text('legacy')
                archive = root/'architecture.json'
                archive.write_text(json.dumps({'legacy_source_root': str(legacy)}))
                candidate = root/'candidate.json'
                candidate.write_text(json.dumps({'artifacts': {'architecture_prebuilt':
                    {'path': str(archive), 'sha256': 'a'*64}}}))
                gate = root/'gate'; gate.write_text('{}')
                with patch('suite_input_lease.source_members', return_value=()), patch('suite_input_lease.workflow_members', return_value=()):
                    snapshot = candidate_snapshot(candidate, gate, root)
                    self.assertIn(str(source), snapshot.files)
                    if change == 'add': (legacy/'new.rs').write_text('new')
                    else: source.unlink()
                    with self.assertRaisesRegex(ValueError, 'tree membership changed'):
                        snapshot.check()

    def test_candidate_signed_bundle_extra_resource_is_in_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            worker = root/'Worker.app/Contents/MacOS/worker'
            worker.parent.mkdir(parents=True); worker.write_text('worker')
            candidate = root/'candidate'
            candidate.write_text(json.dumps({'artifacts': {'worker':
                {'path': str(worker), 'sha256': 'a'*64}}}))
            gate = root/'gate'; gate.write_text('{}')
            with patch('suite_input_lease.source_members', return_value=()), patch('suite_input_lease.workflow_members', return_value=()):
                snapshot = candidate_snapshot(candidate, gate, root)
                (worker.parents[1]/'injected-resource').write_text('extra')
                with self.assertRaisesRegex(ValueError, 'tree membership changed'):
                    snapshot.check()

    def test_gate_change_before_fast_scope_does_not_admit_dispatch(self):
        with tempfile.TemporaryDirectory() as directory:
            gate=Path(directory)/'gate';gate.write_text('old')
            lease=gates.GenerationLease(gate,validator=lambda _:None)
            gate.write_text('replacement');snapshot=MetadataSnapshot([gate])
            with patch.object(gates,'_suite_execution_lease',None),patch.object(gates,'_execution_failure',None):
                with self.assertRaisesRegex(ValueError,'declaration changed'):
                    with lease.suite_checks(snapshot):self.fail('must not dispatch')

    def test_nested_checks_do_not_repeat_deep_validation_and_final_still_runs(self):
        with tempfile.TemporaryDirectory() as directory:
            gate=Path(directory)/'gate';gate.write_text('gate');calls=[]
            lease=gates.GenerationLease(gate,validator=lambda _:calls.append('deep'))
            snapshot=MetadataSnapshot([gate])
            with patch.object(gates,'_suite_execution_lease',None),patch.object(gates,'_execution_failure',None):
                with lease.suite_checks(snapshot):
                    for _ in range(20):gates.check_execution_generation()
                    self.assertEqual(calls,['deep','deep'])
                lease.final_check()
            self.assertEqual(calls,['deep','deep','deep'])

    def test_boundary_execution_does_no_io_and_final_detects_input_change(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = Path(directory)/'gate'; gate.write_text('gate')
            source = Path(directory)/'source'; source.write_text('before')
            lease = gates.GenerationLease(gate, validator=lambda _: None)
            snapshot = MetadataSnapshot([gate, source])
            with patch.object(gates, '_suite_execution_lease', None), patch.object(gates, '_execution_failure', None):
                with lease.suite_checks(snapshot, boundary_only=True):
                    source.write_text('after')
                    with patch.object(snapshot, 'check', side_effect=AssertionError('scan in hot path')), patch.object(gates, 'sha256', side_effect=AssertionError('hash in hot path')):
                        for _ in range(100):
                            gates.check_execution_generation()
                            lease.check()
                        lease.environment({})
                with self.assertRaisesRegex(ValueError, 'metadata changed'):
                    snapshot.check()
                self.assertFalse(lease.boundary_only)
                lease.final_check()

    def test_boundary_scope_does_not_hide_known_revocation(self):
        with tempfile.TemporaryDirectory() as directory:
            gate = Path(directory)/'gate'; gate.write_text('gate')
            lease = gates.GenerationLease(gate, validator=lambda _: None)
            with patch.object(gates, '_suite_execution_lease', None), patch.object(gates, '_execution_failure', None):
                with lease.suite_checks(MetadataSnapshot([gate]), boundary_only=True):
                    lease.failure = 'cancelled admission'
                    with self.assertRaisesRegex(ValueError, 'cancelled admission'):
                        gates.check_execution_generation()

    def test_final_deep_check_occurs_even_after_metadata_revocation(self):
        with tempfile.TemporaryDirectory() as directory:
            gate=Path(directory)/'gate';gate.write_text('gate');calls=[]
            lease=gates.GenerationLease(gate,validator=lambda _:calls.append('deep'))
            snapshot=MetadataSnapshot([gate])
            with patch.object(gates,'_suite_execution_lease',None),patch.object(gates,'_execution_failure',None):
                with lease.suite_checks(snapshot):
                    gate.write_text('changed')
                    with self.assertRaises(ValueError):lease.check()
                    gate.write_text('gate')
                with self.assertRaisesRegex(ValueError,'revoked'):lease.final_check()
            self.assertEqual(calls,['deep','deep','deep'])


class FullRunnerSuiteTests(unittest.TestCase):
    def run_fixture(self, execute):
        spec=importlib.util.spec_from_file_location('full_suite_runner',ROOT/'scripts/run-oil-full-contract.py')
        runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);gate=root/'gate';gate.write_text('gate')
            snapshot=MetadataSnapshot([gate]);bound={'sha256':'candidate'}
            argv=['run','--generation-gate',str(gate),'--candidate',str(root/'candidate'),'--binary','native','--worker','worker','--output',str(root/'out')]
            with patch.dict(os.environ, {}, clear=False),patch('sys.argv',argv),patch.object(runner,'generation_gate'),patch.object(runner,'candidate_snapshot',return_value=snapshot),patch.object(runner,'load_candidate',return_value=bound) as load,patch.object(runner,'for_candidate',return_value={}),patch.object(runner,'workflow_identity',return_value={'identity':'original'}),patch.object(runner.subprocess,'check_output',return_value='source') as source,patch.object(runner,'RESEARCH_RECIPES',{}),patch.object(runner,'RECIPES',{}),patch.object(runner,'planned_attempts',return_value=[]),patch.object(runner,'execute_contract',side_effect=execute),patch.object(gates,'_suite_execution_lease',None),patch.object(gates,'_execution_failure',None):
                try:return runner.main()
                finally:
                    self.assertEqual(load.call_count,2)
                    self.assertEqual(source.call_count,2)

    def test_many_attempt_probes_reuse_one_admission_then_one_final_validation(self):
        def execute(*args,**kwargs):
            with patch.object(MetadataSnapshot, 'check', side_effect=AssertionError('scan during execution')), patch.object(gates, 'sha256', side_effect=AssertionError('hash during execution')):
                for _ in range(25):
                    self.assertEqual(kwargs['candidate_probe'](),{'identity':'original'})
                    gates.check_execution_generation()
            return {'structurally_valid':True,'planned':688,'validated':[None]*688}
        self.assertEqual(self.run_fixture(execute),0)

    def test_changed_input_rejects_suite_at_finalization(self):
        def execute(*args, **kwargs):
            gates._suite_execution_lease.path.write_text('changed during run')
            kwargs['candidate_probe']()  # no scan in execution
            return {'structurally_valid': True, 'planned': 688, 'validated': [None]*688}
        with self.assertRaisesRegex(ValueError, 'metadata changed'):
            self.run_fixture(execute)

    def test_full_final_validation_runs_after_execution_exception(self):
        primary=RuntimeError('execution failed')
        def execute(*args,**kwargs):raise primary
        with self.assertRaises(RuntimeError) as caught:self.run_fixture(execute)
        self.assertIs(caught.exception,primary)

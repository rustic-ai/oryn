"""Barrier regressions patch execution; no native process is needed."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


class GenerationBarrierTests(unittest.TestCase):
    def setUp(self):
        # These orchestration tests model immutable bytes; byte-level lease
        # replacement is covered separately with real temporary files.
        self.addCleanup(patch.stopall)
        self.digest_patch = patch("generation_gate.sha256", return_value="fixture-gate-digest")
        self.digest_patch.start()
        patch.dict("os.environ", {}, clear=False).start()

    def test_shared_gate_rejects_replacement_while_sources_are_being_validated(self):
        self.digest_patch.stop()
        shared = module('generation_gate')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            plan = root / 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'
            plan.parent.mkdir(parents=True)
            plan.write_text('complete original plan')
            source = root / 'source.rs'
            source.write_text('implementation and regression source')
            record = dict(path='source.rs', sha256=shared.sha256(source))
            gate = dict(schema_version=1, plan_sha256=shared.sha256(plan), requirements={
                identifier: dict(implementation_generated=True, tests_generated=True,
                                 implementation_sources=[record], test_sources=[record])
                for identifier in shared.REQUIREMENTS})
            path = root / 'gate.json'
            path.write_text(json.dumps(gate))
            original_hash = shared.sha256
            replaced = False
            def digest(value):
                nonlocal replaced
                if Path(value) == source and not replaced:
                    replaced = True
                    # A different but structurally valid declaration must not
                    # acquire the validation of the previously parsed bytes.
                    path.write_text(json.dumps(dict(gate, review='replacement')))
                return original_hash(value)
            with patch.object(shared, 'sha256', side_effect=digest):
                with self.assertRaisesRegex(ValueError, 'declaration changed during source validation'):
                    shared.generation_gate(path, root=root)
            self.assertTrue(replaced)
            self.assertEqual(shared.generation_gate(path, root=root)['review'], 'replacement')

    def test_full_oil_gate_precedes_source_hash_fixture_and_evidence_creation(self):
        runner = module('run-oil-full-contract')
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / 'evidence'
            argv = ['run', '--generation-gate', 'gate', '--candidate', 'candidate',
                    '--binary', 'native', '--worker', 'worker', '--output', str(output)]
            with patch('sys.argv', argv), patch.object(runner, 'generation_gate', side_effect=ValueError('incomplete')), patch.object(runner.subprocess, 'check_output') as native, patch.object(runner, 'execute_contract') as execute, patch.object(runner, 'load_candidate') as candidate:
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    runner.main()
                native.assert_not_called()
                execute.assert_not_called()
                candidate.assert_not_called()
                self.assertFalse(output.exists())

    def test_full_oil_candidate_cannot_change_between_contract_and_execution_binding(self):
        runner = module('run-oil-full-contract')
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / 'evidence'
            argv = ['run', '--generation-gate', 'gate', '--candidate', 'candidate',
                    '--binary', 'native', '--worker', 'worker', '--output', str(output)]
            original = {'sha256': 'original-candidate'}
            replacement = {'sha256': 'replacement-candidate'}
            with patch('sys.argv', argv), \
                    patch.object(runner, 'generation_gate', return_value={}), \
                    patch.object(runner, 'digest', return_value='gate-digest'), \
                    patch.object(runner, 'load_candidate', side_effect=[original, replacement]), \
                    patch.object(runner, 'for_candidate', return_value={}) as contract, \
                    patch.object(runner, 'workflow_identity') as identity, \
                    patch.object(runner.subprocess, 'check_output') as native, \
                    patch.object(runner, 'execute_contract') as execute:
                with self.assertRaisesRegex(ValueError, 'changed after contract binding'):
                    runner.main()
                contract.assert_called_once_with(original)
                identity.assert_not_called()
                native.assert_not_called()
                execute.assert_not_called()
                self.assertFalse(output.exists())

    def test_profile_gate_precedes_output_or_process_creation(self):
        runner = module('run-workflow-qualification-profiles')
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / 'evidence'
            argv = ['run', '--generation-gate', 'gate', '--candidate', 'candidate',
                    '--worker', 'worker', '--page-benchmark', 'page', '--group-benchmark', 'group',
                    '--churn', 'churn', '--output', str(output)]
            with patch('sys.argv', argv), patch.object(runner, 'generation_gate', side_effect=ValueError('incomplete')), patch.object(runner.subprocess, 'Popen') as native, patch.object(runner, 'load_candidate') as candidate:
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    runner.main()
                native.assert_not_called()
                candidate.assert_not_called()
                self.assertFalse(output.exists())


    def test_profile_declaration_drift_during_setup_precedes_evidence_creation(self):
        runner = module('run-workflow-qualification-profiles')
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / 'evidence'
            argv = ['run', '--generation-gate', 'gate', '--candidate', 'candidate',
                    '--worker', 'worker', '--page-benchmark', 'page', '--group-benchmark', 'group',
                    '--churn', 'churn', '--output', str(output)]
            artifacts = {name: Path(path).resolve() for name, path in
                         [('page_benchmark', 'page'), ('group_benchmark', 'group'),
                          ('churn', 'churn'), ('worker', 'worker'),
                          ('workflow_executor', 'executor')]}
            bound = dict(sha256='candidate-digest', artifacts=artifacts,
                         record=dict(artifacts={name: dict(sha256='binary-digest')
                                                for name in artifacts}))
            with patch('sys.argv', argv), \
                    patch.object(runner, 'generation_gate', side_effect=[{}, ValueError('declaration drift')]), \
                    patch.object(runner, 'load_candidate', return_value=bound), \
                    patch.object(runner, 'workflow_identity', return_value={}), \
                    patch.object(runner, 'digest', return_value='binary-digest'), \
                    patch.object(runner, 'reference', return_value={'sha256': 'binary-digest'}), \
                    patch.object(runner.subprocess, 'check_output', return_value='source-digest'), \
                    patch.object(runner.subprocess, 'Popen') as native, \
                    patch.object(runner, 'persist') as persist:
                with self.assertRaisesRegex(ValueError, 'declaration drift'):
                    runner.main()
                native.assert_not_called()
                persist.assert_not_called()
                self.assertFalse(output.exists())

    def test_profile_valid_replacement_during_candidate_loading_cannot_rebind_admission(self):
        runner = module('run-workflow-qualification-profiles')
        with tempfile.TemporaryDirectory() as root:
            output = Path(root) / 'evidence'
            argv = ['run', '--generation-gate', 'gate', '--candidate', 'candidate',
                    '--worker', 'worker', '--page-benchmark', 'page', '--group-benchmark', 'group',
                    '--churn', 'churn', '--output', str(output)]
            artifacts = {name: Path(path).resolve() for name, path in
                         [('page_benchmark', 'page'), ('group_benchmark', 'group'),
                          ('churn', 'churn'), ('worker', 'worker'),
                          ('workflow_executor', 'executor')]}
            bound = dict(sha256='candidate-digest', artifacts=artifacts,
                         record=dict(artifacts={name: dict(sha256='binary-digest')
                                                for name in artifacts}))
            declaration = ['original-gate']
            def load_candidate(_):
                declaration[0] = 'different-valid-gate'
                return bound
            def digest(path):
                return declaration[0] if path == Path('gate') else 'binary-digest'
            with patch('sys.argv', argv), \
                    patch.object(runner, 'generation_gate', return_value={}), \
                    patch.object(runner, 'load_candidate', side_effect=load_candidate), \
                    patch.object(runner, 'workflow_identity', return_value={}), \
                    patch.object(runner, 'digest', side_effect=digest), \
                    patch.object(runner, 'reference', side_effect=lambda _: {'sha256': declaration[0]}), \
                    patch.object(runner.subprocess, 'check_output', return_value='source-digest'), \
                    patch.object(runner, 'run_profile_process') as native, \
                    patch.object(runner, 'persist') as persist:
                with self.assertRaisesRegex(ValueError, 'profile generation declaration changed'):
                    runner.main()
                native.assert_not_called()
                persist.assert_not_called()
                self.assertFalse(output.exists())

    def test_freeze_revalidates_original_declaration_before_output_creation(self):
        self.digest_patch.stop()
        runner = module('freeze-completion-inputs')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('original')
            artifacts = root / 'artifacts.json'; artifacts.write_text('{}')
            output = root / 'output'
            def changed_inventory(*_args):
                gate.write_text('replacement that would independently validate')
                return []
            with patch('sys.argv', ['freeze', '--generation-gate', str(gate), '--artifacts', str(artifacts), '--output', str(output)]), \
                    patch.object(runner, 'generation_gate'), \
                    patch.object(runner, 'REQUIRED_ARTIFACTS', set()), \
                    patch.object(runner.subprocess, 'check_output', return_value=b''), \
                    patch.object(runner, 'inventory', side_effect=changed_inventory), \
                    patch.object(runner, 'source_digest', return_value='a' * 64), \
                    patch.object(runner, 'source_hash', return_value='a' * 64), \
                    patch.object(runner, 'write') as write, patch.object(runner, 'persist') as persist:
                with self.assertRaisesRegex(ValueError, 'generation declaration changed'):
                    runner.main()
                write.assert_not_called(); persist.assert_not_called()
                self.assertFalse(output.exists())

    def test_control_material_snapshot_and_prerequisite_entrypoints_are_gated(self):
        cases = [
            ('create-wpt-assessment-packet', ['--candidate', 'candidate', '--checkout', 'checkout', '--id', 'manual:/x', '--platform', 'macOS']),
            ('run-wpt-topology-probe', ['--candidate', 'candidate']),
            ('create-wpt-native-controls', ['--fixture-contract', 'fixture', '--ca-key', 'key',
                '--upstream-python', 'python', '--upstream-checkout', 'checkout']),
            ('freeze-completion-inputs', ['--artifacts', 'artifacts']),
            ('run-wpt-native-controls', ['--candidate', 'candidate']),
        ]
        for name, arguments in cases:
            runner = module(name)
            with tempfile.TemporaryDirectory() as directory:
                output = Path(directory).resolve() / 'not-created'
                with patch('sys.argv', ['run', '--generation-gate', 'gate', '--output', str(output), *arguments]), patch.object(runner, 'generation_gate', side_effect=ValueError('incomplete')):
                    with self.assertRaisesRegex(ValueError, 'incomplete'): runner.main()
                self.assertFalse(output.exists())

"""Source-only preparation/command regressions; subprocesses are never launched."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_official_workflow as workflow
from oryn_wptrunner import cookie_profile, feature_profile


def profile_metadata():
    profile = cookie_profile.from_fixture({'third_party_cookies': 'block'})
    features = feature_profile.from_fixture({'web_features': {'usermedia_element': 'standard'}})
    return {'feature_environment_sha256': '9' * 64, 'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features), 'cookie_profile': profile, 'cookie_profile_sha256': cookie_profile.digest(profile),
            'fixture_environment_sha256': 'f' * 64, 'manifest_sha256': 'b' * 64,
            'coverage_sha256': 'c' * 64, 'policy_sha256': 'd' * 64,
            'core_include_sha256': 'e' * 64, 'core_total': 42}



class OfficialWorkflowTests(unittest.TestCase):
    def test_feature_profile_drift_rejects_before_output_or_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **copy.deepcopy(bound), 'checkout': str(root), 'python': str(python),
                    'run_contract_sha256': bound['sha256'], 'inputs': {}}
            bound['feature_profile'] = feature_profile.from_fixture({'web_features': {'usermedia_element': 'legacy'}})
            bound['feature_profile_sha256'] = feature_profile.digest(bound['feature_profile'])
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), patch.object(workflow, 'run_owned') as launch:
                with self.assertRaisesRegex(ValueError, 'feature profile changed'):
                    workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
                launch.assert_not_called()
            self.assertFalse((root / 'output').exists())

    def test_preflight_gate_drift_prevents_upstream_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned executable fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {name: {'sha256': 'a' * 64} for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}}, 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **profile_metadata(), 'checkout': str(root), 'python': str(python), 'candidate_sha256': 'a' * 64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {}, 'inventory': 'inventory',
                    'manifest': 'manifest', 'policy': 'policy'}
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            original_persist = workflow.persist
            def persist(path, value):
                original_persist(path, value)
                if path.name == 'preflight.json':
                    gate.write_text('{"replacement":true}')
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), \
                 patch.object(workflow, 'read_inventory'), patch.object(workflow, 'runner_command', return_value=['pinned', 'wpt']), \
                 patch.object(workflow, 'persist', side_effect=persist), patch.object(workflow, 'run_owned') as launch:
                with self.assertRaisesRegex(ValueError, 'generation declaration changed'):
                    workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
                launch.assert_not_called()
            self.assertFalse((root / 'output/execution.json').exists())

    def test_no_preparation_side_effect_before_complete_generation(self):
        with patch.object(workflow, 'generation_gate', side_effect=ValueError('generation incomplete')), patch.object(workflow.subprocess, 'run') as spawn:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                workflow.prepare('gate', 'contract', 'inventory', 'checkout', 'manifest', 'policy', 'python', 'output')
            spawn.assert_not_called()

    def test_no_execution_before_complete_generation(self):
        with patch.object(workflow, 'generation_gate', side_effect=ValueError('generation incomplete')), patch.object(workflow, 'run_owned') as spawn:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                workflow.execute('gate', 'contract', 'checkout', 'prepared', 'python', 'core', 'output')
            spawn.assert_not_called()

    def test_command_uses_actual_upstream_and_exact_frozen_tls(self):
        bound = {'candidate': {'artifacts': {'runtime': {'path': '/candidate/runtime'}, 'driver': {'path': '/candidate/driver'}}},
                 'fixture_path': '/candidate/config/fixture.json',
                 'fixture': {'tls': {name: {'path': name + '.pem'} for name in ('ca', 'certificate', 'key')}}}
        with patch.object(workflow, 'load_contract', return_value=bound):
            command = workflow.runner_command('/pinned/python', '/pinned/wpt', '/candidate/run.json', '/prepared', 'canaries', '/new/output', '/gate.json')
        self.assertEqual(command[0], '/pinned/python')
        self.assertIn('wpt-upstream-entry.py', command[1])
        self.assertIn('--no-manifest-update', command)
        self.assertIn('--no-manifest-download', command)
        self.assertEqual(command[command.index('--retry-unexpected') + 1], '0')
        self.assertEqual(command[command.index('--processes') + 1], '1')
        self.assertEqual(command[command.index('--include-file') + 1], '/prepared/canaries.include')
        self.assertEqual(command[command.index('--ca-cert-path') + 1], '/candidate/config/ca.pem')
        self.assertNotIn('--reuse-window', command)
        self.assertNotIn('--binary-arg', command)

    def test_generation_gate_rejects_partial_or_changed_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            plan = root / 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'
            plan.parent.mkdir(parents=True)
            plan.write_text('full source plan')
            source = root / 'implementation.py'
            source.write_text('generated implementation and regression fixture')
            record = {'path': source.name, 'sha256': workflow.sha256(source)}
            requirement = {'implementation_generated': True, 'tests_generated': True,
                           'implementation_sources': [record], 'test_sources': [record]}
            gate = {'schema_version': 1, 'plan_sha256': workflow.sha256(plan),
                    'requirements': {key: copy.deepcopy(requirement) for key in workflow.REQUIREMENTS}}
            path = root / 'gate.json'
            path.write_text(json.dumps(gate))
            with patch.object(workflow, 'ROOT', root):
                self.assertEqual(len(workflow.generation_gate(path)['requirements']), 51)
                source.write_text('changed after declaration')
                with self.assertRaisesRegex(ValueError, 'changed'):
                    workflow.generation_gate(path)
                gate['requirements'].pop('W-05')
                path.write_text(json.dumps(gate))
                with self.assertRaisesRegex(ValueError, '51'):
                    workflow.generation_gate(path)


    def test_owned_execution_preserves_cleanup_failure_and_post_run_gate_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            gate = root / 'gate.json'; gate.write_text('{}')
            contract = root / 'contract.json'; contract.write_text('{}')
            python = root / 'python'; python.write_text('pinned executable fixture')
            prepared = root / 'prepared'; prepared.mkdir()
            bound = {**profile_metadata(), 'candidate': {'artifacts': {name: {'sha256': 'a' * 64} for name in ('runtime', 'driver', 'source_snapshot', 'worker_manifest')}}, 'candidate_sha256': 'a' * 64, 'sha256': workflow.sha256(contract)}
            lock = {'schema_version': 1, **profile_metadata(), 'checkout': str(root), 'python': str(python), 'candidate_sha256': 'a' * 64,
                    'run_contract_sha256': bound['sha256'], 'inputs': {}, 'inventory': 'inventory',
                    'manifest': 'manifest', 'policy': 'policy'}
            (prepared / 'workflow-lock.json').write_text(json.dumps(lock))
            def run(command, output, **kwargs):
                self.assertEqual(kwargs['cwd'], root)
                self.assertTrue((output / 'preflight.json').is_file())
                gate.write_text('{"changed": true}')
                return {'returncode': 0, 'timed_out': False, 'error': None,
                        'cleanup_complete': False, 'elapsed_ns': 1}
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', return_value=bound), patch.object(workflow, 'read_inventory'), patch.object(workflow, 'runner_command', return_value=['pinned', 'wpt']), patch.object(workflow, 'run_owned', side_effect=run) as owned:
                result = workflow.execute(gate, contract, root, prepared, python, 'core', root / 'output')
            owned.assert_called_once()
            self.assertEqual(result['returncode'], 0)
            self.assertFalse(result['custody_complete'])
            self.assertFalse(result['cleanup_complete'])
            self.assertTrue(any('generation declaration changed' in error for error in result['errors']))
            self.assertTrue((root / 'output/execution.json').is_file())


if __name__ == '__main__':
    unittest.main()


class OfficialPreparationLeaseTests(unittest.TestCase):
    def test_candidate_inspection_cannot_replace_gate_before_preparation_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); gate = root / 'gate'; gate.write_text('original')
            def load(_):
                gate.write_text('valid replacement')
                return {'fixture': {}, 'fixture_path': str(root / 'fixture')}
            with patch.object(workflow, 'generation_gate'), patch.object(workflow, 'load_contract', side_effect=load), \
                 patch.object(workflow, 'validate_fixture'), patch.object(workflow, 'read_inventory', return_value={}), \
                 patch.object(workflow.subprocess, 'check_output', return_value=workflow.REVISION), \
                 patch.object(workflow.subprocess, 'run') as launch:
                with self.assertRaisesRegex(ValueError, 'declaration changed'):
                    workflow.prepare(gate, 'contract', 'inventory', root, 'manifest', 'policy', 'python', root / 'output')
                launch.assert_not_called()
                self.assertFalse((root / 'output').exists())

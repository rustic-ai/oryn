"""No benchmark execution: exact source and original attempt receipt controls."""
import json
import importlib.util
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from completion_execution import reference
from completion_provenance import ROOT, digest
from workflow_qualification import planned_profiles, validate_profile_custody


class ProfileCustodyTests(unittest.TestCase):
    def runner(self):
        spec = importlib.util.spec_from_file_location(
            'profile_runner', ROOT / 'scripts/run-workflow-qualification-profiles.py')
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        # Process cleanup tests supply an explicit mocked admission; main-path
        # declaration tests retain the real GenerationLease.
        patcher = mock.patch.object(runner, "inherited_generation_lease")
        patcher.start(); self.addCleanup(patcher.stop)
        return runner

    def test_malformed_native_output_is_retained_as_failed_observation(self):
        runner = self.runner()
        cases = [None, [], 'text', {'runtime': None},
                 {'runtime': {}, 'process_lifecycle': []},
                 {'runtime': {}, 'process_lifecycle': {'processes': [None]}},
                 {'runtime': {}, 'process_lifecycle': {'processes': [{'leases': [None]}]}}]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for value in cases:
                with self.subTest(value=value):
                    encoded = json.dumps(value)
                    (root/'stdout.json').write_text(encoded)
                    result, errors = runner.read_profile_result(root, {}, 123, 'a'*64)
                    self.assertTrue(errors)
                    self.assertIsInstance(result, dict)
                    self.assertEqual((root/'stdout.json').read_text(), encoded)

    def test_incomplete_accounting_fails_without_rewriting_native_result(self):
        runner = self.runner()
        value = {'runtime': {'worker_binary_sha256': 'a'*64},
                 'process_lifecycle': {'processes': []}}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'stdout.json').write_text(json.dumps(value))
            with mock.patch.object(runner, 'reconcile_resources',
                                   return_value={'complete': False, 'errors': ['missing worker']}):
                result, errors = runner.read_profile_result(root, {}, 123, 'a'*64)
            self.assertEqual(result, value)
            self.assertIn('native resource accounting incomplete', errors)
            self.assertEqual(json.loads((root/'resource-accounting.json').read_text())['errors'],
                             ['missing worker'])

    def test_launch_failure_returns_failed_record_without_starting_sampler(self):
        runner = self.runner()
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(runner.subprocess, 'Popen', side_effect=OSError('launch failed')), \
                mock.patch.object(runner, 'ProcessSampler') as sampler:
            result = runner.run_profile_process(['fixture'], {}, Path(directory))
        sampler.assert_not_called()
        self.assertIsNone(result['returncode'])
        self.assertFalse(result['cleanup_complete'])
        self.assertTrue(any('launch failed' in error for error in result['errors']))

    def test_generation_recheck_failure_during_setup_prevents_all_profiles(self):
        runner = self.runner()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            args = SimpleNamespace(**{name: root/name for name in (
                'generation_gate', 'page_benchmark', 'group_benchmark', 'churn',
                'worker', 'candidate', 'output')})
            args.generation_gate.write_text('{}')
            artifacts = {name: getattr(args, name) for name in (
                'page_benchmark', 'group_benchmark', 'churn', 'worker')}
            artifacts['workflow_executor'] = root/'executor'
            bound = dict(artifacts=artifacts, sha256='a'*64,
                         record={'artifacts': {name: {'sha256': 'a'*64} for name in artifacts}})
            plan = planned_profiles()[:3]
            with mock.patch.object(runner.argparse.ArgumentParser, 'parse_args', return_value=args), \
                    mock.patch.object(runner, 'generation_gate', side_effect=[None, ValueError('gate changed'), ValueError('gate changed')]), \
                    mock.patch.object(runner, 'load_candidate', return_value=bound), \
                    mock.patch.object(runner, 'workflow_identity', return_value={}), \
                    mock.patch.object(runner, 'digest', return_value='a'*64), \
                    mock.patch.object(runner, 'reference', return_value={'sha256': 'a'*64}), \
                    mock.patch.object(runner, 'planned_profiles', return_value=plan), \
                    mock.patch.object(runner.subprocess, 'check_output', return_value='source'), \
                    mock.patch.object(runner, 'run_profile_process') as launch, \
                    mock.patch('builtins.print'):
                with self.assertRaisesRegex(ValueError, "gate changed"):
                    runner.main()
            launch.assert_not_called()
            self.assertFalse(args.output.exists())

    def test_sampler_start_failure_still_kills_and_reaps_owned_process(self):
        runner = self.runner()
        process = mock.Mock(pid=123)
        process.poll.return_value = None
        process.wait.return_value = -9
        sampler = mock.Mock()
        sampler.start.side_effect = RuntimeError('sampler start failed')
        sampler.finish.return_value = {'samples': [], 'errors': ['not started']}
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(runner.subprocess, 'Popen', return_value=process), \
                mock.patch.object(runner, 'ProcessSampler', return_value=sampler), \
                mock.patch.object(runner.os, 'killpg', side_effect=[None, ProcessLookupError()]) as kill:
            result = runner.run_profile_process(['fixture'], {}, Path(directory))
        kill.assert_any_call(123, runner.signal.SIGKILL)
        process.wait.assert_called_once_with(timeout=15)
        sampler.finish.assert_called_once_with()
        self.assertEqual(result['returncode'], -9)
        self.assertTrue(any('sampler start failed' in error for error in result['errors']))

    def test_observed_survivor_remains_failure_when_forced_cleanup_races_exit(self):
        runner = self.runner()
        for observations, expected in [
            ([ProcessLookupError()], True),
            ([PermissionError()], False),
            ([None, None], False),
            ([None, ProcessLookupError()], False),
            ([None, PermissionError()], False),
        ]:
            with self.subTest(observations=observations), \
                    mock.patch.object(runner.os, 'killpg', side_effect=observations) as kill:
                self.assertIs(runner.cleanup_process_group(123), expected)
                self.assertEqual(kill.call_count, len(observations))
                kill.assert_any_call(123, 0)
                if len(observations) == 2:
                    kill.assert_called_with(123, runner.signal.SIGKILL)

    def test_complete_population_binds_last_attempt_and_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate = root / 'gate.json'; gate.write_text('{}')
            attempts = {}
            for item in planned_profiles():
                path = root / item['id'].replace(':', '-') / 'attempt.json'
                path.parent.mkdir(); path.write_text(json.dumps({'id': item['id']}))
                attempts[path.relative_to(root).as_posix()] = digest(path)
            receipt = {'schema_version': 1, 'candidate_manifest_sha256': 'a' * 64,
                       'custody_complete': True, 'errors': [], 'attempts': attempts,
                       'generation_gate': reference(gate),
                       'producer_source': reference(ROOT / 'scripts/run-workflow-qualification-profiles.py')}
            path = root / 'execution-custody.json'
            path.write_text(json.dumps(receipt))
            self.assertEqual(len(validate_profile_custody(root, 'a' * 64)['attempts']), len(planned_profiles()))
            last = root / next(reversed(attempts))
            original = last.read_bytes(); last.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'attempt changed'):
                validate_profile_custody(root, 'a' * 64)
            last.write_bytes(original)
            receipt['custody_complete'] = False; receipt['errors'] = ['last-attempt source drift']
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, 'custody incomplete'):
                validate_profile_custody(root, 'a' * 64)
            receipt['custody_complete'] = True; receipt['errors'] = []
            receipt['attempts'].pop(next(reversed(attempts))); path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, 'denominator'):
                validate_profile_custody(root, 'a' * 64)

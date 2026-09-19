"""Regression sources for generation binding and broad source custody; never launch tools."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

PATH = Path(__file__).resolve().parent.parent / 'repair44-root-execution-custody.py'
SPEC = importlib.util.spec_from_file_location('reviewed_repair44_dispatch', PATH)
DISPATCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DISPATCH)


class DispatchCustodyTests(unittest.TestCase):
    def test_historical_environment_cannot_override_final_generation(self):
        gate_sha = 'a' * 64
        config = dict(environment={'ORYN_WPT_PYTHON': '/fixture/venv/bin/python',
                      'ORYN_GENERATION_GATE': '/stale/repair42.json',
                      'ORYN_GENERATION_GATE_SHA256': 'b' * 64},
                      path_prepend='/fixture/node', unset=['ORYN_PAGE_WORKER'])
        calls = []
        class Lease:
            def environment(self, base):
                calls.append(dict(base))
                return dict(base, ORYN_GENERATION_GATE=str(DISPATCH.GATE.resolve()),
                            ORYN_GENERATION_GATE_SHA256=gate_sha)
        with mock.patch.dict(os.environ, {'PATH': '/system', 'ORYN_PAGE_WORKER': '/stale/worker'}, clear=True), \
             mock.patch.object(DISPATCH, 'sha', return_value=DISPATCH.ENVIRONMENT_SHA), \
             mock.patch.object(Path, 'read_bytes', return_value=json.dumps(config).encode()):
            env, command = DISPATCH.build_environment(Lease(), True, Path('/fixture/launch.py'), gate_sha)
        self.assertEqual(calls[0]['ORYN_GENERATION_GATE'], '/stale/repair42.json')
        self.assertNotIn('ORYN_PAGE_WORKER', calls[0])
        self.assertEqual(env['ORYN_GENERATION_GATE'], str(DISPATCH.GATE.resolve()))
        self.assertEqual(env['ORYN_GENERATION_GATE_SHA256'], gate_sha)
        self.assertEqual(env['PATH'], '/fixture/node' + os.pathsep + '/system')
        self.assertEqual(command, ['/fixture/venv/bin/python', '/fixture/launch.py'])

    def test_inconsistent_final_generation_environment_is_rejected(self):
        class StaleLease:
            def environment(self, base):
                return dict(base, ORYN_GENERATION_GATE='/stale/gate', ORYN_GENERATION_GATE_SHA256='b' * 64)
        with self.assertRaisesRegex(ValueError, 'final child environment'):
            DISPATCH.build_environment(StaleLease(), False, Path('/fixture/native.py'), 'a' * 64)

    def fixture(self, root):
        (root / 'scripts').mkdir()
        source = root / 'scripts/source.py'
        source.write_text('original source\n')
        row = dict(path='scripts/source.py', sha256=hashlib.sha256(source.read_bytes()).hexdigest())
        previous = root / 'previous.json'
        previous.write_text(json.dumps(dict(sources=[row], authored_discovery_roots=['scripts'])))
        gate = root / 'gate.json'
        gate.write_text(json.dumps(dict(review_sources=[])))
        lease = root / 'lease.json'
        data = dict(sources=[row], source_count=1, authored_discovery_roots=['scripts'],
                    expected_declaration_path='gate.json')
        return source, previous, gate, lease, data

    def check_fixture(self, root, previous, gate, lease, data):
        lease.write_text(json.dumps(data))
        with mock.patch.multiple(DISPATCH, ROOT=root, GATE=gate, PREVIOUS_LEASE=previous,
                                 PREVIOUS_LEASE_SHA=DISPATCH.sha(previous)):
            return DISPATCH.check(lease, DISPATCH.sha(lease), DISPATCH.sha(gate))

    def test_historical_source_cannot_be_removed_from_current_lease(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, previous, gate, lease, data = self.fixture(root)
            data.update(sources=[], source_count=0)
            result = self.check_fixture(root, previous, gate, lease, data)
            self.assertTrue(any('dropped historical source' in error for error in result['errors']))

    def test_new_authored_source_cannot_escape_whole_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, previous, gate, lease, data = self.fixture(root)
            (root / 'scripts/new.py').write_text('new authored source\n')
            result = self.check_fixture(root, previous, gate, lease, data)
            self.assertIn('added:scripts/new.py', result['errors'])

    def test_changed_source_bytes_revoke_custody(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, previous, gate, lease, data = self.fixture(root)
            source.write_text('changed source\n')
            result = self.check_fixture(root, previous, gate, lease, data)
            self.assertIn('scripts/source.py', result['errors'])

    def test_dispatch_interrupt_kills_and_reaps_owned_group(self):
        process = mock.Mock(pid=246810)
        process.poll.return_value = None
        process.wait.side_effect = [KeyboardInterrupt(), -9]
        with mock.patch.object(DISPATCH.subprocess, 'Popen', return_value=process) as popen, \
             mock.patch.object(DISPATCH.os, 'killpg') as kill, \
             mock.patch.object(DISPATCH, 'group_exists', return_value=False), \
             mock.patch.object(DISPATCH.signal, 'signal'):
            record = DISPATCH.run_owned_dispatch(['fixture-command'], {})
        self.assertTrue(popen.call_args.kwargs['start_new_session'])
        kill.assert_called_once_with(process.pid, DISPATCH.signal.SIGKILL)
        process.wait.assert_called_with(timeout=15)
        self.assertIn('KeyboardInterrupt', record['execution_error'])
        self.assertTrue(record['group_settled'])
        self.assertFalse(record['cleanup_errors'])

    def test_leftover_descendant_is_failed_custody_even_when_reaped(self):
        process = mock.Mock(pid=246810)
        process.poll.return_value = 0
        process.wait.return_value = 0
        with mock.patch.object(DISPATCH.subprocess, 'Popen', return_value=process), \
             mock.patch.object(DISPATCH.os, 'killpg') as kill, \
             mock.patch.object(DISPATCH, 'group_exists', side_effect=[True, False, False]), \
             mock.patch.object(DISPATCH.signal, 'signal'):
            record = DISPATCH.run_owned_dispatch(['fixture-command'], {})
        self.assertEqual(record['returncode'], 0)
        self.assertTrue(record['group_settled'])
        self.assertEqual(record['cleanup_errors'], ['descendant group survived direct-child exit'])
        kill.assert_called_once_with(process.pid, DISPATCH.signal.SIGKILL)

    def python_launcher(self):
        path = PATH.parent / 'fq-python-regressions-12/launch.py'
        spec = importlib.util.spec_from_file_location('reviewed_python12_launcher', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_supervised_python_inherits_group_and_delegates_interrupted_cleanup(self):
        launch = self.python_launcher()
        process = mock.Mock(pid=246811)
        process.poll.return_value = None
        process.wait.side_effect = KeyboardInterrupt()
        env = {launch.SUPERVISOR_KEY: launch.SUPERVISOR_VALUE}
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(launch.os, 'getpid', return_value=246810), \
             mock.patch.object(launch.os, 'getpgrp', return_value=246810), \
             mock.patch.object(launch.subprocess, 'Popen', return_value=process) as popen, \
             mock.patch.object(launch.os, 'killpg') as kill:
            record = launch.run_child(['fixture-command'], env, Path(directory),
                                      {'run_id': 'fixture'}, mock.Mock(sha256='a' * 64))
            receipt = json.loads((Path(directory) / 'process.json').read_bytes())
        self.assertFalse(popen.call_args.kwargs['start_new_session'])
        self.assertNotIn(launch.SUPERVISOR_KEY, popen.call_args.kwargs['env'])
        kill.assert_not_called()
        self.assertIn('KeyboardInterrupt', record['execution_error'])
        self.assertIsNone(record['returncode'])
        self.assertEqual(receipt['supervision'], {'mode': 'outer-owned', 'group_id': 246810})
        self.assertEqual(record['supervision'], receipt['supervision'])

    def test_supervision_rejects_unknown_flag_or_nonleader_before_spawn(self):
        launch = self.python_launcher()
        for value, group in [('unknown', 246810), (launch.SUPERVISOR_VALUE, 246809)]:
            with self.subTest(value=value, group=group), \
                 mock.patch.object(launch.os, 'getpid', return_value=246810), \
                 mock.patch.object(launch.os, 'getpgrp', return_value=group), \
                 mock.patch.object(launch.subprocess, 'Popen') as popen:
                with self.assertRaisesRegex(ValueError, 'must lead'):
                    launch.run_child(['fixture-command'], {launch.SUPERVISOR_KEY: value},
                                     Path('/unused'), {}, mock.Mock())
                popen.assert_not_called()


if __name__ == '__main__':
    unittest.main()

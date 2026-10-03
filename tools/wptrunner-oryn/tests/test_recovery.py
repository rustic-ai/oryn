"""Exercise real upstream child teardown and manager restart, without a browser."""
from collections import deque
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import tempfile
import subprocess
import sys
import signal
from dataclasses import dataclass
from queue import Queue
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from oryn_wptrunner import custody, recovery
from oryn_wptrunner.protocol import OrynProtocol
from wptrunner import testrunner as upstream


class Recording:
    def set(self, value): pass
    def pause(self): pass


class FaultExecutor:
    extra_timeout = 1
    def __init__(self, logger, browser, **kwargs):
        self.logger = logger

    def setup(self, runner):
        self.runner = runner
        self.session = custody.begin()

    def run_test(self, test):
        result = SimpleNamespace(status='ERROR', expected='OK', known_intermittent=[],
                                 message='injected test error', extra={}, stack=None)
        self.runner.send_message('test_ended', test, (result, []))

    def teardown(self):
        def fail():
            raise RuntimeError('injected native teardown failure')
        OrynProtocol._teardown(SimpleNamespace(
            webdriver=SimpleNamespace(end=fail), _custody_session=self.session))


class SentinelExecutor(FaultExecutor):
    """Alternate failed retirement with a healthy fresh-generation control."""

    def run_test(self, test):
        self.failed = test.id.startswith('/fault/')
        result = SimpleNamespace(status='ERROR' if self.failed else 'OK',
                                 expected='OK', known_intermittent=[],
                                 message='injected test error' if self.failed else None,
                                 extra={}, stack=None)
        self.runner.send_message('test_ended', test, (result, []))

    def teardown(self):
        if getattr(self, 'failed', False):
            return super().teardown()
        OrynProtocol._teardown(SimpleNamespace(
            webdriver=SimpleNamespace(end=lambda: None), _custody_session=self.session))


@dataclass
class FixtureTest:
    id: str
    timeout: int = 1
    test_type: str = 'testharness'
    restart_after: bool = False

    def expected(self): return 'OK'


class FixtureBrowser:
    init_timeout = 10
    pid = None
    _process_settlement = None

    def __init__(self, *args, **kwargs): self._recovery_settled = False
    def setup(self): pass
    def cleanup(self): pass
    def settings(self, test): return {}
    def start(self, **kwargs): self._recovery_settled = False
    def stop(self, **kwargs): self._recovery_settled = True
    def check_crash(self, **kwargs): return False
    def is_alive(self): return not self._recovery_settled
    def executor_browser(self): return SimpleNamespace, {}


class ReadQueue(Queue):
    def get(self): return super().get(block=False)


def manager_fixture(index=0):
    mp = upstream.mpcontext.get_context()
    manager = recovery.OrynTestRunnerManager('recovery-test', index, None, {},
        upstream.StopFlag(1), recording=Recording())
    manager.logger = Mock()
    manager.command_queue, manager.remote_queue = mp.Queue(), mp.Queue()
    browser = SimpleNamespace(_recovery_settled=True, stop=lambda **kwargs: None)
    manager.browser = upstream.BrowserManager(manager.logger, browser, manager.command_queue)
    group = SimpleNamespace(subsuite='', test_type='testharness',
                            test_queue=deque([SimpleNamespace(id='/queued')]))
    pending = SimpleNamespace(id='/prefetched')
    manager.state = upstream.InitializingState(group, pending, 0)
    implementation = upstream.ExecutorImplementation(
        FaultExecutor, {}, SimpleNamespace, {})
    manager.test_implementations = {('', 'testharness'): implementation}
    manager.get_executor_implementation = lambda impl: impl
    return manager, group, pending


def start_and_stop(manager, group, pending):
    manager.state = upstream.InitializingState(group, pending, 0)
    manager.start_test_runner()
    while True:
        command, data = manager.command_queue.get(timeout=15)
        if command == 'init_succeeded':
            break
        if command == 'init_failed':
            raise AssertionError(data)
    manager.state = upstream.RestartingState(group, pending, False)
    return manager.restart_runner()


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'custody.jsonl'
        custody.create(self.path)
        env = patch.dict(os.environ, ORYN_WPT_PROTOCOL_CUSTODY=str(self.path),
                         ORYN_WPT_CANDIDATE_SHA256='a' * 64)
        env.start(); self.addCleanup(env.stop)

    def clean_manager(self, manager):
        if manager.test_runner_proc is not None:
            manager.test_runner_proc.terminate()
            manager.test_runner_proc.join(5)
        for queue in (manager.command_queue, manager.remote_queue):
            queue.cancel_join_thread(); queue.close()

    def test_real_teardown_failure_keeps_prefetch_and_queue_with_fresh_generation(self):
        manager, group, pending = manager_fixture()
        self.addCleanup(self.clean_manager, manager)
        old_flag, old_queue = manager.child_stop_flag, manager.command_queue
        next_state = start_and_stop(manager, group, pending)
        self.assertIs(next_state.test, pending)
        self.assertEqual([test.id for test in group.test_queue], ['/queued'])
        self.assertTrue(old_flag.is_set())
        self.assertFalse(manager.should_stop())
        self.assertIsNot(manager.child_stop_flag, old_flag)
        self.assertIsNot(manager.command_queue, old_queue)
        rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        self.assertEqual([row['phase'] for row in rows], ['connect', 'teardown'])
        self.assertFalse(rows[-1]['success'])

    def test_three_generations_without_progress_stop_but_keep_selection(self):
        manager, group, pending = manager_fixture()
        self.addCleanup(self.clean_manager, manager)
        start_and_stop(manager, group, pending)
        start_and_stop(manager, group, pending)
        with self.assertRaisesRegex(RuntimeError, 'three failed generations'):
            start_and_stop(manager, group, pending)
        self.assertIs(manager.state.test, pending)
        self.assertEqual(len(group.test_queue), 1)
        self.assertTrue(manager.child_stop_flag.is_set())

    def test_progress_resets_failure_budget(self):
        manager, group, pending = manager_fixture()
        self.addCleanup(self.clean_manager, manager)
        for completed in range(4):
            manager.test_count = completed
            start_and_stop(manager, group, pending)
            self.assertEqual(manager._consecutive_failures, 1)

    def test_cancellation_is_not_cleared(self):
        manager, group, pending = manager_fixture()
        self.addCleanup(self.clean_manager, manager)
        manager.parent_stop_flag.stop()
        start_and_stop(manager, group, pending)
        self.assertTrue(manager.should_stop())
        self.assertTrue(manager.child_stop_flag.is_set())

    def test_unsettled_driver_blocks_replacement(self):
        manager, group, pending = manager_fixture()
        self.addCleanup(self.clean_manager, manager)
        manager.browser.browser._recovery_settled = False
        with self.assertRaisesRegex(RuntimeError, 'process tree settlement'):
            start_and_stop(manager, group, pending)
        self.assertTrue(manager.child_stop_flag.is_set())

    def test_six_managers_keep_independent_queues(self):
        def exercise(index):
            manager, group, pending = manager_fixture(index)
            try:
                result = start_and_stop(manager, group, pending)
                return result.test.id, [test.id for test in group.test_queue], manager.should_stop()
            finally:
                self.clean_manager(manager)
        with ThreadPoolExecutor(max_workers=6) as pool:
            results = list(pool.map(exercise, range(6)))
        self.assertEqual(results, [('/prefetched', ['/queued'], False)] * 6)
        rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        self.assertEqual(sum(row['phase'] == 'teardown' for row in rows), 6)
        self.assertEqual(len({row['session'] for row in rows}), 6)

    def test_complete_upstream_manager_loops_preserve_all_urls_and_errors(self):
        # Every test result forces a restart and every real child teardown
        # raises. Before the repair, each lane loses four of its five URLs.
        total, workers = 30, 6
        suite = 'oryn-recovery-complete-selection'
        logger = upstream.structuredlog.StructuredLogger(suite)
        events = []
        logger.add_handler(events.append)
        selected = ['/fault/%d' % n for n in range(total)]
        logger.suite_start(selected)
        stop = upstream.StopFlag(workers)
        managers = []
        impl = upstream.TestImplementation(FaultExecutor, {'timeout_multiplier': 1}, FixtureBrowser, {})
        for index in range(workers):
            queue = ReadQueue()
            queue.put(SimpleNamespace(name=str(index), subsuite='', test_type='testharness',
                metadata={}, test_queue=deque(FixtureTest(url) for url in selected[index::workers])))
            manager = recovery.OrynTestRunnerManager(suite, index, queue,
                {('', 'testharness'): impl}, stop, recording=Recording())
            managers.append(manager)
            manager.start()
        try:
            stop.wait_for_all_managers_done(timeout=45)
        finally:
            stop.stop()
            for manager in managers:
                manager.join(5)
            logger.suite_end()
        self.assertTrue(all(not manager.is_alive() for manager in managers))
        starts = [row['test'] for row in events if row['action'] == 'test_start']
        ends = [row for row in events if row['action'] == 'test_end']
        self.assertCountEqual(starts, selected)
        self.assertCountEqual([row['test'] for row in ends], selected)
        self.assertEqual({row['status'] for row in ends}, {'ERROR'})
        teardown_errors = [row for row in events if row['action'] == 'log'
                           and 'injected native teardown failure' in row.get('message', '')]
        self.assertEqual(len(teardown_errors), total)
        rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        self.assertEqual(sum(row['phase'] == 'teardown' and not row['success'] for row in rows), total)

    def test_failed_teardown_then_healthy_sentinel_serial_and_six_managers(self):
        for workers in (1, 6):
            with self.subTest(workers=workers):
                suite = 'oryn-recovery-sentinels-%d' % workers
                logger = upstream.structuredlog.StructuredLogger(suite)
                events = []
                logger.add_handler(events.append)
                lanes = [[FixtureTest('/%s/%d/%d' % (kind, lane, case), restart_after=True)
                          for case in range(4) for kind in ('fault', 'sentinel')]
                         for lane in range(workers)]
                selected = [test.id for lane in lanes for test in lane]
                logger.suite_start(selected)
                stop = upstream.StopFlag(workers)
                managers = []
                impl = upstream.TestImplementation(
                    SentinelExecutor, {'timeout_multiplier': 1}, FixtureBrowser, {})
                before = len(self.path.read_text().splitlines())
                for index, tests in enumerate(lanes):
                    queue = ReadQueue()
                    queue.put(SimpleNamespace(name=str(index), subsuite='',
                        test_type='testharness', metadata={}, test_queue=deque(tests)))
                    manager = recovery.OrynTestRunnerManager(suite, index, queue,
                        {('', 'testharness'): impl}, stop, recording=Recording())
                    managers.append(manager)
                    manager.start()
                try:
                    stop.wait_for_all_managers_done(timeout=45)
                finally:
                    stop.stop()
                    for manager in managers:
                        manager.join(5)
                    logger.suite_end()
                self.assertTrue(all(not manager.is_alive() for manager in managers))
                starts = [row['test'] for row in events if row['action'] == 'test_start']
                ends = [row for row in events if row['action'] == 'test_end']
                self.assertCountEqual(starts, selected)
                self.assertCountEqual([row['test'] for row in ends], selected)
                for row in ends:
                    self.assertEqual(row['status'],
                                     'ERROR' if row['test'].startswith('/fault/') else 'OK')
                rows = [json.loads(line) for line in
                        self.path.read_text().splitlines()[before:]]
                connections = [row for row in rows if row['phase'] == 'connect']
                retirements = [row for row in rows if row['phase'] == 'teardown']
                self.assertEqual(len(connections), len(selected))
                self.assertCountEqual([row['session'] for row in retirements],
                                      [row['session'] for row in connections])
                self.assertEqual(sum(not row['success'] for row in retirements), 4 * workers)
                self.assertEqual(sum(row['success'] for row in retirements), 4 * workers)

    def test_abandoned_session_is_failed_once_and_never_fabricated(self):
        identity = custody.begin()
        custody.finish_abandoned(identity)
        custody.finish_abandoned(identity)
        custody.finish_abandoned('b' * 32)
        rows = [json.loads(line) for line in self.path.read_text().splitlines()]
        self.assertEqual([row['phase'] for row in rows], ['connect', 'teardown'])
        self.assertFalse(rows[-1]['success'])

    def test_scoped_manager_hook_restores_upstream_on_error(self):
        original = upstream.TestRunnerManager
        with self.assertRaisesRegex(ValueError, 'injected'):
            with recovery.manager_recovery():
                self.assertIs(upstream.TestRunnerManager, recovery.OrynTestRunnerManager)
                raise ValueError('injected')
        self.assertIs(upstream.TestRunnerManager, original)

    def test_process_settlement_rejects_retained_orphan_then_confirms_exit(self):
        root = subprocess.Popen([sys.executable, '-c',
            'import subprocess,sys; p=subprocess.Popen([sys.executable,"-c",'
            '"import time; time.sleep(60)"]); print(p.pid,flush=True); sys.stdin.read()'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        observer = None
        try:
            child = int(root.stdout.readline())
            observer = recovery.ProcessSettlement(root.pid)
            self.assertIn(child, observer.observe())
            root.terminate(); root.wait(timeout=5)
            with self.assertRaisesRegex(RuntimeError, 'descendants remain alive'):
                observer.finish()
            observer.tree.send(signal.SIGTERM, observer.observe(), [])
            # The observer retains exact births even after driver reparenting.
            import time
            deadline = time.monotonic() + 5
            while observer.observe() and time.monotonic() < deadline:
                time.sleep(0.01)
            observer.finish()
        finally:
            if root.poll() is None:
                root.terminate(); root.wait(timeout=5)
            if observer is not None:
                observer.done.set(); observer.thread.join(1)
                observer.tree.send(signal.SIGKILL, observer.observe(), [])
            root.stdin.close(); root.stdout.close()

    def test_unknown_birth_identity_cannot_authorize_recovery(self):
        with patch('completion_process_supervisor.OwnedTree', return_value=SimpleNamespace(owned={})):
            with self.assertRaisesRegex(RuntimeError, 'identity observation'):
                recovery.ProcessSettlement(123)

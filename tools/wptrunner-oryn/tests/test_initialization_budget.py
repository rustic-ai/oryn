"""Actual pinned initialization interfaces with synthetic time and I/O only."""
import errno
from contextlib import ExitStack
from queue import SimpleQueue
import unittest
from unittest.mock import Mock, patch

from oryn_wptrunner import product
from wptrunner import environment, testrunner
from wptrunner.browsers import base


class Clock:
    def __init__(self):
        self.value = 1000.0
        self.timers = []

    def now(self):
        return self.value

    def advance(self, seconds):
        target = self.value + seconds
        for timer in sorted(self.timers, key=lambda item: item.deadline):
            if not timer.cancelled and not timer.fired and timer.deadline <= target:
                self.value = max(self.value, timer.deadline)
                timer.fired = True
                timer.callback()
        self.value = target

    def timer(self, interval, callback):
        clock = self

        class Timer:
            def __init__(self):
                self.interval = interval
                self.deadline = clock.now() + interval
                self.callback = callback
                self.cancelled = self.fired = False

            def start(self):
                clock.timers.append(self)

            def cancel(self):
                self.cancelled = True

        return Timer()


class InitializationBudgetTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.events = []
        self.collectors = []
        self.processes = []
        self.validation_delays = [62]
        self.validation_error = None
        self.slot_delay = 0
        self.ready_delay = 0
        self.unavailable = False
        self.ready_at = None
        self.logger = Mock()
        self.browser = object.__new__(product.OrynBrowser)
        base.WebDriverBrowser.__init__(self.browser, self.logger, manager_number=0,
            binary='/synthetic/browser', webdriver_binary='/synthetic/driver', port=8123)
        self.browser._contract_path = '/synthetic/contract.json'
        self.browser._token_path = '/synthetic/token'
        self.browser._driver_token = 'a' * 64
        self.queue = SimpleQueue()
        self.manager = testrunner.BrowserManager(self.logger, self.browser, self.queue)
        self.manager.browser_settings = {}
        outer = self

        class Collector:
            def __init__(self, logger, identity):
                self.closed = False
                self.fd = 12
                self.service = {'service_id': str(len(outer.collectors)), 'pid': None}
                self.private_calls = 0
                self.finishes = []
                outer.collectors.append(self)

            def inherit_for_spawn(self):
                outer.events.append(('admission', outer.clock.now()))
                outer.clock.advance(outer.slot_delay)

            def private(self):
                self.private_calls += 1

            def published(self, pid):
                self.service['pid'] = pid

            def finish(self, *, process_stopped):
                self.finishes.append(process_stopped)
                self.closed = process_stopped

        class Process:
            def __init__(self, command, **kwargs):
                self.command = command
                self.environment = kwargs.get("env")
                self.pid = 100 + len(outer.processes)
                self.proc = object()
                self.returncode = None
                outer.processes.append(self)

            def run(self):
                outer.events.append(('spawn', outer.clock.now()))
                outer.ready_at = outer.clock.now() + outer.ready_delay

            def poll(self):
                return self.returncode

            def kill(self, *args, **kwargs):
                outer.events.append(('kill', outer.clock.now()))
                self.returncode = 0
                return 0

        class Socket:
            def connect(self, address):
                outer.events.append(('connect', outer.clock.now()))
                if outer.unavailable or outer.clock.now() < outer.ready_at:
                    raise ConnectionRefusedError(errno.ECONNREFUSED, 'synthetic unavailable')

            def close(self):
                pass

        def validate(*args):
            self.events.append(('validation_begin', self.clock.now()))
            self.clock.advance(self.validation_delays.pop(0))
            self.events.append(('validation_end', self.clock.now()))
            if self.validation_error is not None:
                raise self.validation_error
            return {'fixture_path': '/synthetic/fixture.json', 'fixture_sha256': 'b' * 64}

        stack = self.enterContext(ExitStack())
        # product.time and pinned upstream time refer to this same clock API.
        stack.enter_context(patch.object(base.time, 'time', self.clock.now))
        stack.enter_context(patch.object(environment.time, 'sleep', self.clock.advance))
        stack.enter_context(patch.object(testrunner.threading, 'Timer', self.clock.timer))
        stack.enter_context(patch.object(base.mozprocess, 'ProcessHandler', Process))
        # Process above has synthetic PIDs; kernel custody has separate real
        # process tests and must not inspect unrelated host PIDs here.
        stack.enter_context(patch('oryn_wptrunner.recovery.ProcessSettlement'))
        stack.enter_context(patch.object(environment.socket, 'socket', Socket))
        stack.enter_context(patch.object(product, 'Collector', Collector))
        self.validate = stack.enter_context(patch.object(product, 'load_contract', side_effect=validate))
        self.addCleanup(self.browser.stop)

    def test_typed_worker_uses_actual_driver_environment_snapshot(self):
        worker = '/sealed/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
        contract = {'fixture_path': '/synthetic/fixture.json', 'fixture_sha256': 'b' * 64,
                    'typed_run': {}, 'worker_binding': {'variable': 'ORYN_PAGE_WORKER', 'path': worker}}
        self.validate.side_effect = None
        self.validate.return_value = contract
        self.browser.env['ORYN_PAGE_WORKER'] = worker
        with patch.dict(product.os.environ, {'ORYN_PAGE_WORKER': '/unrelated/ambient'}):
            self.browser.start({})
        self.assertEqual(self.processes[0].environment['ORYN_PAGE_WORKER'], worker)
        self.browser.stop()
        self.browser.env['ORYN_PAGE_WORKER'] = '/injected/copied-driver-env'
        with patch.dict(product.os.environ, {'ORYN_PAGE_WORKER': worker}):
            with self.assertRaisesRegex(ValueError, 'driver environment differs'):
                self.browser.start({})
        self.assertEqual(len(self.processes), 1)

    def test_typed_missing_worker_binding_never_spawns(self):
        self.validate.side_effect = None
        self.validate.return_value = {'fixture_path': '/synthetic/fixture.json',
                                     'fixture_sha256': 'b' * 64, 'typed_run': {}}
        with self.assertRaisesRegex(ValueError, 'driver environment differs'):
            self.browser.start({})
        self.assertEqual(self.processes, [])

    def test_delayed_validation_precedes_spawn_under_both_real_upstream_timers(self):
        self.ready_delay = 2
        self.assertTrue(self.manager.init({}))
        self.assertEqual(self.browser.init_timeout, 120)
        self.assertEqual(self.clock.timers[0].interval, 120)
        self.assertEqual(self.clock.timers[0].deadline, self.browser.init_deadline)
        self.assertEqual(self.browser.init_deadline, 1120)
        self.assertEqual(self.clock.now(), 1064)
        self.assertEqual([name for name, _ in self.events[:4]],
                         ['validation_begin', 'validation_end', 'admission', 'spawn'])
        self.assertEqual(self.events[3][1], 1062)
        self.assertFalse(self.clock.timers[0].cancelled)
        self.assertFalse(self.collectors[0].closed)
        # The manager's timer remains active through subsequent executor/session setup.
        self.clock.advance(56)
        self.assertEqual(self.queue.get_nowait(), ('init_failed', ()))
        self.manager.after_init()
        self.assertTrue(self.clock.timers[0].cancelled)

    def test_expired_validation_cannot_spawn_and_releases_descriptor_custody(self):
        self.validation_delays = [121]
        with self.assertRaisesRegex(TimeoutError, 'deadline expired before driver spawn'):
            self.browser.start({})
        self.assertEqual(self.browser.init_deadline, 1120)
        self.assertEqual(self.clock.now(), 1121)
        self.assertEqual(self.processes, [])
        self.assertEqual([name for name, _ in self.events], ['validation_begin', 'validation_end', 'admission'])
        self.assertEqual(self.collectors[0].private_calls, 1)
        self.assertEqual(self.collectors[0].finishes, [True])
        self.assertTrue(self.collectors[0].closed)

    def test_descriptor_admission_consumes_original_budget_without_reset(self):
        self.validation_delays = [110]
        self.slot_delay = 11
        with self.assertRaises(TimeoutError):
            self.browser.start({})
        self.assertEqual(self.browser.init_deadline, 1120)
        self.assertEqual(self.clock.now(), 1121)
        self.assertEqual(self.processes, [])
        self.assertEqual(self.collectors[0].private_calls, 1)
        self.assertTrue(self.collectors[0].closed)

    def test_unavailable_driver_exhausts_only_remaining_initialization_time(self):
        self.unavailable = True
        self.assertFalse(self.manager.init({}))
        self.assertEqual(self.clock.now(), 1120)
        self.assertEqual(next(stamp for name, stamp in self.events if name == 'connect'), 1062)
        self.assertTrue(self.clock.timers[0].fired)
        self.assertTrue(self.clock.timers[0].cancelled)
        self.assertEqual(len(self.processes), 1)
        self.assertFalse(self.collectors[0].closed)
        # Readiness failure does not fabricate process cleanup; manager.stop owns it.
        self.manager.stop(force=True)
        self.assertTrue(self.collectors[0].closed)
        self.assertEqual(self.processes[0].poll(), 0)

    def test_validation_failure_preserves_exception_without_any_spawn(self):
        failure = ValueError('rejected actual contract')
        self.validation_error = failure
        with self.assertRaises(ValueError) as caught:
            self.browser.start({})
        self.assertIs(caught.exception, failure)
        self.assertEqual(self.processes, [])
        self.assertEqual(self.collectors, [])
        self.validate.assert_called_once_with('/synthetic/contract.json', '/synthetic/browser', '/synthetic/driver')

    def test_each_retry_revalidates_with_a_new_finite_shared_budget(self):
        self.validation_delays = [121, 62]
        self.assertFalse(self.manager.init({}))
        self.assertTrue(self.clock.timers[0].cancelled)
        self.assertTrue(self.collectors[0].closed)
        self.assertEqual(self.queue.get_nowait(), ('init_failed', ()))
        self.assertTrue(self.manager.init({}))
        self.assertEqual([timer.interval for timer in self.clock.timers], [120, 120])
        self.assertEqual([timer.deadline for timer in self.clock.timers], [1120, 1241])
        self.assertEqual(self.browser.init_deadline, 1241)
        self.assertEqual(self.validate.call_count, 2)
        self.assertEqual(len(self.processes), 1)
        self.manager.after_init()
        self.assertTrue(self.clock.timers[1].cancelled)

    def test_executor_timeouts_and_upstream_multiplier_are_unchanged(self):
        baseline = {'timeout_multiplier': 1.5, 'timeout': 42, 'script_timeout': 20}
        with patch.object(product, 'upstream_executor_kwargs', return_value=dict(baseline)), \
                patch('oryn_wptrunner.placement.configuration', return_value=None):
            actual = product.executor_kwargs(self.logger, 'testharness', None, {})
        for key, value in baseline.items():
            self.assertEqual(actual[key], value)
        self.assertIs(product.get_timeout_multiplier, base.get_timeout_multiplier)
        self.assertEqual(base.Browser.init_timeout, 30)

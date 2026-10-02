"""Oryn lifecycle recovery around the pinned upstream scheduler.

Test selection, results and retries remain upstream-owned. Only a failed
protocol teardown can clear a child stop latch, after physical settlement.
"""
from contextlib import contextmanager
import threading
from queue import Empty

from wptrunner import testrunner as upstream

_teardown_failed = None


def teardown_failed():
    if _teardown_failed is not None:
        _teardown_failed.set()


def start_runner(failed, session, *args):
    global _teardown_failed
    from . import custody
    _teardown_failed = failed
    custody.runner_session = session
    upstream.start_runner(*args)


class ProcessSettlement:
    """Retain ancestry/birth identities; never signal from a diagnostic PID.

    Upstream and native retirement own termination. This observer prevents a
    replacement generation while any retained process remains alive.
    """
    def __init__(self, pid):
        from completion_process_supervisor import OwnedTree
        self.tree = OwnedTree(pid)
        if pid not in self.tree.owned:
            raise RuntimeError('driver exited before recovery identity observation')
        self.error = None
        self.done = threading.Event()
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self._observe, name='oryn-process-settlement', daemon=True)
        self.thread.start()

    def observe(self):
        with self.lock:
            if self.error is not None:
                raise self.error
            try:
                return self.tree.observe()
            except Exception as error:
                self.error = error
                raise

    def _observe(self):
        while not self.done.wait(0.5):
            try:
                self.observe()
            except Exception:
                return

    def finish(self):
        self.done.set()
        self.thread.join(1)
        if self.thread.is_alive():
            raise RuntimeError('process observer did not settle')
        live = self.observe()
        if live:
            raise RuntimeError('owned driver descendants remain alive: ' + str(sorted(live)))


class OrynTestRunnerManager(upstream.TestRunnerManager):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self._teardown_failure = None
        self._session = None
        self._consecutive_failures = 0
        self._completed_at_recovery = 0
        self._selected_test = None

    def get_next_test(self):
        result = super().get_next_test()
        self._selected_test = result[1] if result is not None else None
        return result

    def test_ended(self, test, results):
        if isinstance(self.state, upstream.RunningState) and test == self.state.test:
            self._selected_test = None
        return super().test_ended(test, results)

    def start_test_runner(self):
        # Keep the upstream process/executor lifecycle; pass two explicit
        # generation-local records instead of recovering from arbitrary errors.
        mp = upstream.mpcontext.get_context()
        self._teardown_failure = mp.Event()
        self._session = mp.Array('c', 33, lock=False)
        impl = self.test_implementations[(self.state.test_group.subsuite, self.state.test_group.test_type)]
        self.executor_implementation = self.get_executor_implementation(impl)
        self.test_runner_proc = mp.Process(
            target=start_runner,
            args=(self._teardown_failure, self._session, self.remote_queue,
                  self.command_queue, self.executor_implementation, self.capture_stdio,
                  self.child_stop_flag, self.recording),
            name='TestRunner-%i' % self.manager_number)
        self.test_runner_proc.start()

    def stop_runner(self, force=False):
        runner = self.test_runner_proc
        result_queue = self.command_queue
        super().stop_runner(force=force)
        # Upstream can terminate without joining. Do not equate that attempt
        # (or the cleared test_runner_proc attribute) with actual process exit.
        if runner is not None:
            runner.join(5)
            if runner.is_alive():
                raise RuntimeError('old WPT runner did not exit')
            if runner.exitcode == 0 and result_queue is self.command_queue:
                # The child has joined and flushed its feeder. Preserve the
                # teardown traceback/logs before replacing the generation's
                # queue. Never read a forcibly terminated child's broken pipe.
                while True:
                    try:
                        command, data = result_queue.get_nowait()
                    except Empty:
                        break
                    if command == 'log':
                        self.log(*data)
                    elif command == 'error':
                        self.logger.error(*data)
                    else:
                        raise RuntimeError('unconsumed runner message at settlement: ' + command)
        browser = self.browser.browser if self.browser else None
        if (browser is not None and getattr(browser, '_process_settlement', None) is not None
                and not getattr(browser, '_recovery_settled', False)):
            raise RuntimeError('driver process tree settlement failed')
        if self._session is not None and self._session.value:
            from .custody import finish_abandoned
            finish_abandoned(self._session.value.decode('ascii'))
            self._session.value = b''
        if not (self.child_stop_flag.is_set() and self._teardown_failure is not None
                and self._teardown_failure.is_set()):
            return
        if self.parent_stop_flag.should_stop():
            return
        if browser is None or not getattr(browser, '_recovery_settled', False):
            raise RuntimeError('cannot recover before driver process tree settlement')
        if self.test_count > self._completed_at_recovery:
            self._consecutive_failures = 0
        self._completed_at_recovery = self.test_count
        self._consecutive_failures += 1
        self.logger.error('Oryn protocol teardown failed; physical generation settled; '
                          'pending tests retained for replacement')
        if self._consecutive_failures >= 3:
            raise RuntimeError('three failed generations without test progress')
        mp = upstream.mpcontext.get_context()
        # The stopped generation must never share flags or buffered messages
        # with its successor. state.test already owns the prefetched URL.
        for queue in (self.command_queue, self.remote_queue):
            queue.cancel_join_thread()
            queue.close()
        self.command_queue = mp.Queue()
        self.remote_queue = mp.Queue()
        self.browser.command_queue = self.command_queue
        self.child_stop_flag = mp.Event()

    def _cleanup_run_loop(self):
        # Upstream reports the deque, but omits its already-prefetched test.
        # Emit the full remaining selection on unrecoverable termination.
        if not isinstance(self.state, upstream.StopState):
            pending = self._selected_test
            if pending is not None:
                self.logger.error('Oryn manager stopped with selected test: ' + pending.id)
        super()._cleanup_run_loop()


@contextmanager
def manager_recovery():
    """One scoped substitution for this Oryn runner process, never disk patches."""
    original = upstream.TestRunnerManager
    if original is not OrynTestRunnerManager.__bases__[0]:
        raise RuntimeError('unexpected upstream manager implementation')
    upstream.TestRunnerManager = OrynTestRunnerManager
    try:
        yield
    finally:
        upstream.TestRunnerManager = original

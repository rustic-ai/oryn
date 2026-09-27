"""Real file/FD controls; fake processes never launch a driver or service."""
import json
import os
import shutil
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from oryn_wptrunner import native_timing as native
from oryn_wptrunner import product, protocol, timing


class NativeCollectorTests(unittest.TestCase):
    def collector(self, logger=None):
        rows = []
        if logger is None:
            logger = SimpleNamespace(info=lambda text: rows.append(json.loads(
                text.removeprefix('oryn-native-timing '))))
        collector = native.Collector(logger, None)
        self.addCleanup(shutil.rmtree, collector.directory, True)
        self.addCleanup(lambda: collector.finish(process_stopped=True))
        return collector, rows

    @staticmethod
    def write(collector, **changes):
        value = {'kind': 'oryn_native_timing', 'version': 1, 'pid': 17,
                 'phase': 'worker.ipc_ready', 'event': 'end', 'elapsed_ns': 41}
        value.update(changes)
        os.write(collector.fd, json.dumps(value).encode() + b'\n')

    def test_private_append_descriptor_and_pread_do_not_shift_shared_offset(self):
        import fcntl
        c, rows = self.collector()
        self.assertFalse(os.get_inheritable(c.fd))
        self.assertEqual(os.stat(c.directory).st_mode & 0o777, 0o700)
        self.assertEqual(os.fstat(c.fd).st_mode & 0o777, 0o600)
        self.assertTrue(fcntl.fcntl(c.fd, fcntl.F_GETFL) & os.O_APPEND)
        self.write(c)
        duplicate = os.dup(c.fd)
        self.addCleanup(os.close, duplicate)
        before = os.lseek(duplicate, 0, os.SEEK_CUR)
        c.finish(process_stopped=True)
        self.assertEqual(os.lseek(duplicate, 0, os.SEEK_CUR), before)
        self.assertEqual(rows[-1]['records'], 1)
        self.assertTrue(c.path.exists())

    def test_parallel_collectors_keep_identity_and_raw_files_separate(self):
        a, ar = self.collector()
        b, br = self.collector()
        threads = [threading.Thread(target=self.write, args=(c,), kwargs={'pid': pid})
                   for c, pid in ((a, 11), (b, 12))]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(1)
            self.assertFalse(thread.is_alive())
        a.finish(process_stopped=True)
        b.finish(process_stopped=True)
        self.assertNotEqual(a.service['service_id'], b.service['service_id'])
        self.assertEqual(ar[0]['record']['pid'], 11)
        self.assertEqual(br[0]['record']['pid'], 12)
        self.assertNotEqual(a.path, b.path)

    def test_pinned_mozprocess_default_passes_only_explicitly_inheritable_collector(self):
        import sys
        from mozprocess import ProcessHandler
        c, rows = self.collector()
        script = ("import json,os,sys; fd=int(sys.argv[1]); "
                  "os.set_inheritable(fd,False); "
                  "os.write(fd,(json.dumps({'kind':'oryn_native_timing','version':1,"
                  "'pid':os.getpid(),'phase':'test.child','event':'point'})+'\\n').encode())")
        process = ProcessHandler([sys.executable, '-c', script, str(c.fd)],
                                 processOutputLine=lambda line: None, storeOutput=False)
        try:
            c.inherit_for_spawn()
            try:
                process.run()
            finally:
                c.private()
            result = process.wait(timeout=5)
            self.assertEqual(result, 0)
        finally:
            if hasattr(process, 'proc') and process.poll() is None:
                process.kill(9, timeout=5)
            self.assertTrue(not hasattr(process, 'proc') or process.poll() is not None)
        c.finish(process_stopped=True)
        self.assertEqual(rows[0]['record']['pid'], process.pid)
        self.assertFalse(rows[-1]['missing_native_records'])

    def test_parallel_spawn_windows_never_make_both_collectors_inheritable(self):
        a, _ = self.collector()
        b, _ = self.collector()
        entered = threading.Event()
        a.inherit_for_spawn()
        def competing():
            b.inherit_for_spawn(timeout=2)
            try:
                self.assertFalse(os.get_inheritable(a.fd))
                self.assertTrue(os.get_inheritable(b.fd))
                entered.set()
            finally:
                b.private()
        thread = threading.Thread(target=competing)
        thread.start()
        try:
            self.assertFalse(entered.wait(0.02))
        finally:
            a.private()
            thread.join(3)
        self.assertFalse(thread.is_alive())
        self.assertTrue(entered.is_set())

    def test_spawn_lock_timeout_does_not_steal_other_collectors_window(self):
        a, _ = self.collector()
        b, _ = self.collector()
        a.inherit_for_spawn()
        try:
            with self.assertRaises(TimeoutError):
                b.inherit_for_spawn(timeout=0)
            b.private()
            self.assertTrue(os.get_inheritable(a.fd))
        finally:
            a.private()
        b.inherit_for_spawn(timeout=0)
        b.private()

    def test_failed_reset_closes_parent_fd_before_unlock_and_preserves_raw_inode(self):
        c, rows = self.collector()
        self.write(c)
        c.inherit_for_spawn()
        original_fd = c.fd
        with patch.object(native.os, 'set_inheritable', side_effect=OSError('reset failed')):
            c.private()
        with self.assertRaises(OSError):
            os.fstat(original_fd)
        self.assertTrue(c._fd_closed)
        other, _ = self.collector()
        other.inherit_for_spawn(timeout=0)
        other.private()
        c.finish(process_stopped=True)
        self.assertEqual(rows[-1]['records'], 1)
        self.assertTrue(any(row['event']=='descriptor_reset_error' for row in rows))

    def test_uncertain_close_quarantines_slot_across_later_cleanup_attempts(self):
        import errno
        c, rows = self.collector()
        c.inherit_for_spawn()
        try:
            with patch.object(native.os, 'set_inheritable', side_effect=OSError('reset')), \
                 patch.object(native.os, 'close', side_effect=OSError(errno.EINTR, 'uncertain')):
                with self.assertRaises(OSError):
                    c.private()
            with self.assertRaises(OSError):
                c.private()
            c.finish(process_stopped=True)
            self.assertTrue(c._spawn_window)
            self.assertFalse(c.closed)
            self.assertTrue(any(row.get('spawn_window_retained') is True for row in rows))
        finally:
            # Test owns the deliberately fault-injected descriptor; production
            # cannot establish this external proof and remains quarantined.
            os.close(c.fd)
            c._fd_closed = True
            c.closed = True
            c._spawn_window = False
            native._SPAWN_WINDOW.release()

    def test_published_hook_closes_inheritance_and_retains_actual_identity(self):
        c, _ = self.collector()
        c.inherit_for_spawn()
        self.assertTrue(os.get_inheritable(c.fd))
        with patch.object(native, 'process_record', return_value={
                'pid': 123, 'kernel_birth': [7, 8], 'parent_pid': 9,
                'identity_status': 'observed'}):
            c.published(123)
        self.assertFalse(os.get_inheritable(c.fd))
        self.assertEqual(c.service['kernel_birth'], [7, 8])

    def test_incomplete_invalid_and_oversized_lines_are_visible(self):
        c, rows = self.collector()
        os.write(c.fd, b'not-json\n' + b'x' * (native.MAX_LINE + 1) + b'\n')
        self.write(c)
        os.write(c.fd, b'{"partial":')
        c.finish(process_stopped=True)
        summary = rows[-1]
        self.assertEqual(summary['records'], 1)
        self.assertEqual(summary['invalid_records'], 1)
        self.assertEqual(summary['oversized_records'], 1)
        self.assertTrue(summary['partial_tail'])

    def test_version_bool_foreign_namespace_and_bad_pid_are_not_accepted(self):
        c, rows = self.collector()
        for changes in ({'version': True}, {'kind': 'foreign'}, {'pid': True}):
            self.write(c, **changes)
        c.finish(process_stopped=True)
        self.assertEqual(rows[-1]['invalid_records'], 3)
        self.assertTrue(rows[-1]['missing_native_records'])

    def test_native_cumulative_and_delta_records_remain_raw_not_summed(self):
        c, rows = self.collector()
        for calls in (2, 4):
            self.write(c, kind='oryn_context_rpc_progress', totals={'calls': calls})
        self.write(c, kind='oryn_context_parent_summary', version=2, stages_ns={'handler': 9})
        c.finish(process_stopped=True)
        records = [row['record'] for row in rows if row['event'] == 'record']
        self.assertEqual([r['totals']['calls'] for r in records[:2]], [2, 4])
        self.assertEqual(records[-1]['stages_ns'], {'handler': 9})

    def test_live_service_does_not_close_or_claim_descendant_settlement(self):
        c, rows = self.collector()
        c.finish(process_stopped=False)
        self.assertFalse(c.closed)
        self.assertEqual(rows[-1]['event'], 'pending')
        self.write(c)
        c.finish(process_stopped=True)
        self.assertFalse(rows[-1]['descendants_proven_stopped'])

    def test_repeated_finish_never_duplicates_records(self):
        c, rows = self.collector()
        self.write(c)
        c.finish(process_stopped=True)
        count = len(rows)
        c.finish(process_stopped=True)
        self.assertEqual(len(rows), count)

    def test_logger_and_read_failures_preserve_raw_file_and_primary_error(self):
        c, _ = self.collector(SimpleNamespace(info=lambda _: (_ for _ in ()).throw(OSError())))
        self.write(c)
        with patch.object(native.os, 'pread', side_effect=OSError('unavailable')):
            c.finish(process_stopped=True)
        summary = json.loads((c.directory / 'collector-summary.json').read_text())
        self.assertEqual(summary['read_error_type'], 'OSError')
        self.assertEqual(summary['log_failures'], 1)
        self.assertTrue(c.path.exists())
        self.assertTrue(c.closed)

    def test_cap_reports_partial_snapshot_without_reading_unbounded_file(self):
        c, rows = self.collector()
        self.write(c)
        with patch.object(native, 'MAX_BYTES', 16):
            c.finish(process_stopped=True)
        self.assertEqual(rows[-1]['read_bytes'], 16)
        self.assertTrue(rows[-1]['file_cap_reached'])
        self.assertFalse(rows[-1]['snapshot_complete'])


class NativeLaunchTimingTests(unittest.TestCase):
    def browser(self):
        browser = object.__new__(product.OrynBrowser)
        browser.logger = SimpleNamespace(info=lambda _: None, debug=lambda _: None)
        browser._proc = None
        browser._timing_identity = None
        browser._native_timing = native.Collector(browser.logger, None)
        browser._spawn_span = browser._ready_span = None
        self.addCleanup(shutil.rmtree, browser._native_timing.directory, True)
        self.addCleanup(lambda: browser._native_timing.finish(process_stopped=True))
        return browser

    def test_exact_upstream_hook_publishes_pid_before_tcp_ready_span(self):
        browser = self.browser()
        handler = browser.create_output_handler(['driver'])
        with patch.object(native, 'process_record', return_value={'pid': 71,
                'kernel_birth': [1, 2], 'parent_pid': 7, 'identity_status': 'observed'}):
            handler.after_process_start(71)
        self.assertEqual(handler.pid, 71)
        self.assertIsNone(browser._spawn_span)
        self.assertEqual(browser._ready_span.phase, 'driver.tcp_ready_wait')
        browser._ready_span.__exit__(None, None, None)
        browser._ready_span = None
        self.assertFalse(os.get_inheritable(browser._native_timing.fd))

    def test_spawn_failure_resets_inheritance_and_preserves_primary_exception(self):
        browser = self.browser()
        primary = OSError('actual spawn failure')
        def fail(*args, **kwargs):
            browser.create_output_handler(['driver'])
            raise primary
        with patch.object(product.WebDriverBrowser, 'start', side_effect=fail):
            with self.assertRaises(OSError) as caught:
                browser.start({})
        self.assertIs(caught.exception, primary)
        self.assertTrue(browser._native_timing.closed)
        self.assertIsNone(browser._spawn_span)

    def test_publication_reset_failure_keeps_real_pid_and_releases_spawn_lock(self):
        browser = self.browser()
        handler = browser.create_output_handler(['driver'])
        with patch.object(native.os, 'set_inheritable', side_effect=OSError('reset failed')):
            handler.after_process_start(71)
        self.assertEqual(handler.pid, 71)
        self.assertTrue(browser._native_timing._fd_closed)
        self.assertFalse(browser._native_timing._spawn_window)
        browser._ready_span.__exit__(None, None, None)
        browser._ready_span = None

    def test_failed_spawn_and_failed_reset_keep_original_error_and_unlock(self):
        browser = self.browser()
        primary = OSError('actual spawn failure')
        original = native.os.set_inheritable
        def set_flag(fd, value):
            if value is False:
                raise OSError('reset failure')
            return original(fd, value)
        def fail(*args, **kwargs):
            browser.create_output_handler(['driver'])
            raise primary
        with patch.object(native.os, 'set_inheritable', side_effect=set_flag), \
             patch.object(product.WebDriverBrowser, 'start', side_effect=fail):
            with self.assertRaises(OSError) as caught:
                browser.start({})
        self.assertIs(caught.exception, primary)
        self.assertFalse(browser._native_timing._spawn_window)
        self.assertTrue(browser._native_timing.closed)

    def test_live_failed_start_retains_file_and_descriptor_until_upstream_stop(self):
        browser = self.browser()
        with patch.object(product.WebDriverBrowser, 'is_alive', return_value=True), \
             patch.object(product.WebDriverBrowser, 'start', side_effect=TimeoutError):
            with self.assertRaises(TimeoutError):
                browser.start({})
        self.assertFalse(browser._native_timing.closed)
        self.assertFalse(os.get_inheritable(browser._native_timing.fd))
        with patch.object(product.WebDriverBrowser, 'is_alive', return_value=False), \
             patch.object(product.WebDriverBrowser, 'stop', return_value=False):
            self.assertFalse(browser.stop(force=True))
        self.assertTrue(browser._native_timing.closed)


class PhaseBoundaryTests(unittest.TestCase):
    def test_navigation_wrapper_preserves_body_timeout_and_exception(self):
        marker = RuntimeError('actual navigation failure')
        calls = []
        def send(*args):
            calls.append(args)
            raise marker
        session = SimpleNamespace(session_id='id', _ending=False, send_command=send)
        with self.assertRaises(RuntimeError) as caught:
            protocol.CheckedSession.send_session_command(session, 'POST', 'url', {'url': '/x'}, 7)
        self.assertIs(caught.exception, marker)
        self.assertEqual(calls, [('POST', 'session/id/url', {'url': '/x'}, 7)])

    def test_harness_wrappers_delegate_once_and_keep_upstream_return(self):
        parent = SimpleNamespace()
        part = object.__new__(protocol.OrynTestharnessProtocolPart)
        part.parent = parent
        sentinel = object()
        with patch.object(protocol.WebDriverTestharnessProtocolPart, 'load_runner', return_value=sentinel) as call:
            self.assertIs(part.load_runner('https'), sentinel)
            call.assert_called_once_with('https')
        with patch.object(protocol.WebDriverTestharnessProtocolPart, 'close_old_windows', side_effect=ValueError) as call:
            with self.assertRaises(ValueError):
                part.close_old_windows()
            call.assert_called_once_with()

    def test_thread_cpu_is_distinct_from_wall_and_nested_service_is_copied(self):
        rows = []
        logger = SimpleNamespace(info=lambda text: rows.append(json.loads(text.removeprefix('oryn-timing '))))
        service = {'service_id': 'a' * 32, 'pid': 17, 'kernel_birth': [2, 3]}
        with patch.object(timing.time, 'perf_counter_ns', side_effect=[10, 20, 50, 100]), \
             patch.object(timing.time, 'thread_time_ns', side_effect=[2, 3, 8, 12]):
            with timing.span(logger, 'outer', service=service, webdriver_session_id='session-id'):
                with timing.span(logger, 'inner'):
                    pass
        inner, outer = rows[2], rows[3]
        self.assertEqual(inner['elapsed_ns'], 30)
        self.assertEqual(inner['thread_cpu_elapsed_ns'], 5)
        self.assertEqual(outer['thread_cpu_elapsed_ns'], 10)
        self.assertEqual(inner['service']['pid'], 17)
        self.assertEqual(inner['webdriver_session_id'], 'session-id')
        self.assertEqual(outer['cpu_scope'], 'calling-python-thread')
        self.assertEqual(inner['parent_span_id'], outer['span_id'])

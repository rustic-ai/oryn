import os
import ctypes
import errno
import json
import importlib.util
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import completion_process_supervisor as supervisor


class MacOSKernelDiscoveryTests(unittest.TestCase):
    def candidates(self, api, **kwargs):
        library = unittest.mock.Mock()
        library.proc_listchildpids.side_effect = api
        with patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor, 'process_identity', return_value=((123, 456), 1)):
            return supervisor.kernel_child_candidates(10, **kwargs)

    def test_empty(self):
        self.assertEqual(self.candidates(lambda *_: 0), [])

    def test_grows_without_truncating(self):
        capacities = []
        def api(parent, buffer, size):
            capacity = size // ctypes.sizeof(ctypes.c_int)
            capacities.append(capacity)
            for index, pid in enumerate([20, 21, 22][:capacity]):
                buffer[index] = pid
            return min(capacity, 3)
        self.assertEqual(self.candidates(api, initial_capacity=2), [20, 21, 22])
        self.assertEqual(capacities, [2, 4])

    def test_saturation_is_not_silent_truncation(self):
        with self.assertRaises(OverflowError):
            self.candidates(lambda p, b, s: s // ctypes.sizeof(ctypes.c_int), initial_capacity=2, max_capacity=4)

    def test_permission_error_is_not_empty_inventory(self):
        def api(*unused):
            ctypes.set_errno(errno.EPERM)
            return 0
        with self.assertRaises(PermissionError): self.candidates(api)

    def test_negative_return_fails_closed(self):
        with self.assertRaises(OSError): self.candidates(lambda *_: -1)

    def test_failed_discovery_drops_branch_only_on_kernel_confirmed_absence(self):
        library = unittest.mock.Mock()
        library.proc_listchildpids.return_value = -1
        with patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor, 'process_identity', return_value=None):
            self.assertEqual(supervisor.kernel_child_candidates(10), [])

    def test_invalid_pid_fails_closed(self):
        with self.assertRaises(RuntimeError): self.candidates(lambda *_: 1)

    def test_duplicate_pid_fails_closed(self):
        def api(parent, buffer, size):
            buffer[0] = buffer[1] = 20
            return 2
        with self.assertRaises(RuntimeError): self.candidates(api)

    def test_descendant_discovery_does_not_spawn_ps_and_handles_cycles(self):
        rows = {10: [11, 12], 11: [13], 12: [], 13: [10]}
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, 'kernel_child_candidates', side_effect=rows.__getitem__) as query, \
             patch.object(supervisor.subprocess, 'check_output') as ps:
            self.assertEqual(supervisor.process_table([10]), {11: 10, 12: 10, 13: 11, 10: 13})
            self.assertEqual(query.call_count, 4)
            ps.assert_not_called()

    def test_linux_discovery_unchanged(self):
        with patch.object(supervisor.sys, 'platform', 'linux'), \
             patch.object(supervisor.subprocess, 'check_output', return_value='10 1\n11 10\n') as ps:
            self.assertEqual(supervisor.process_table([10]), {10: 1, 11: 10})
            ps.assert_called_once()


class MacOSBirthIdentityTests(unittest.TestCase):
    def identity(self, size, native_errno, probe):
        def pidinfo(*unused):
            ctypes.set_errno(native_errno)
            return size
        library = unittest.mock.Mock()
        library.proc_pidinfo.side_effect = pidinfo
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor.os, 'kill', side_effect=probe) as kill, \
             patch.object(supervisor, '_identity_context', return_value='fixture diagnostic'):
            result = supervisor.process_identity(12345)
            return result, kill.call_args_list

    def test_missing_identity_is_gone_only_when_independent_probe_proves_esrch(self):
        for size, native_errno in ((0, errno.EPERM), (0, 0), (1, errno.EPERM)):
            with self.subTest(size=size, native_errno=native_errno):
                result, calls = self.identity(size, native_errno, ProcessLookupError(errno.ESRCH, 'gone'))
                self.assertIsNone(result)
                self.assertEqual(calls, [unittest.mock.call(12345, 0)])

    def test_live_reused_or_unprobeable_identity_still_fails_closed_with_context(self):
        for size, native_errno in ((0, errno.EPERM), (0, 0), (1, errno.EPERM)):
            for probe in (None, PermissionError(errno.EPERM, 'denied'), OSError(errno.EIO, 'probe failed')):
                with self.subTest(size=size, native_errno=native_errno, probe=probe):
                    with self.assertRaises(OSError) as raised:
                        self.identity(size, native_errno, probe)
                    message = str(raised.exception)
                    self.assertIn('pid=12345', message)
                    self.assertIn('proc_pidinfo(PROC_PIDTBSDINFO)', message)
                    self.assertIn(f'returned_bytes={size}', message)
                    self.assertIn(f'native_errno={native_errno}', message)
                    self.assertIn('liveness=', message)

    def test_native_esrch_preserves_unreaped_zombie_absence_without_liveness_probe(self):
        def pidinfo(*unused):
            ctypes.set_errno(errno.ESRCH)
            return 0
        library = unittest.mock.Mock()
        library.proc_pidinfo.side_effect = pidinfo
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor.os, 'kill') as kill:
            self.assertIsNone(supervisor.process_identity(12345))
            kill.assert_not_called()

    def test_complete_kernel_identity_does_not_use_liveness_probe(self):
        def pidinfo(pid, flavor, arg, pointer, size):
            info = ctypes.cast(pointer, ctypes.POINTER(supervisor._BSDInfo)).contents
            info.seconds, info.microseconds, info.ids[4] = 123, 456, 7
            return size
        library = unittest.mock.Mock()
        library.proc_pidinfo.side_effect = pidinfo
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), patch.object(supervisor.os, 'kill') as kill:
            self.assertEqual(supervisor.process_identity(12345), ((123, 456), 7))
            kill.assert_not_called()

    def test_confirmed_exit_does_not_authorize_pid_only_signal(self):
        with patch.object(supervisor, 'birth', return_value=('original',)):
            tree = supervisor.OwnedTree(12345)
        library = unittest.mock.Mock()
        def pidinfo(*unused):
            ctypes.set_errno(errno.EPERM)
            return 0
        library.proc_pidinfo.side_effect = pidinfo
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor.os, 'kill', side_effect=ProcessLookupError(errno.ESRCH, 'gone')) as kill:
            events = []
            tree.send(signal.SIGKILL, {12345}, events)
            self.assertEqual(events, [])
            kill.assert_called_once_with(12345, 0)

    def test_live_identity_error_preserves_retained_tree_and_never_signals(self):
        with patch.object(supervisor, 'birth', return_value=('original',)):
            tree = supervisor.OwnedTree(12345)
        library = unittest.mock.Mock()
        def pidinfo(*unused):
            ctypes.set_errno(errno.EPERM)
            return 0
        library.proc_pidinfo.side_effect = pidinfo
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor, 'process_table', return_value={12345: 1}), \
             patch.object(supervisor.os, 'kill') as kill:
            with self.assertRaises(OSError): tree.observe()
            self.assertEqual(tree.owned, {12345: ('original',)})
            with self.assertRaises(OSError): tree.send(signal.SIGKILL, {12345}, [])
            self.assertTrue(all(call.args == (12345, 0) for call in kill.call_args_list))


class MacOSBirthRetryTests(unittest.TestCase):
    def library(self, outcomes):
        sequence = iter(outcomes)
        def pidinfo(pid, flavor, arg, pointer, size):
            outcome = next(sequence)
            if isinstance(outcome, int):
                ctypes.set_errno(outcome)
                return 0
            seconds, micros, parent = outcome
            info = ctypes.cast(pointer, ctypes.POINTER(supervisor._BSDInfo)).contents
            info.seconds, info.microseconds, info.ids[4] = seconds, micros, parent
            return size
        library = unittest.mock.Mock()
        library.proc_pidinfo.side_effect = pidinfo
        return library

    def test_transient_permission_returns_only_fresh_complete_kernel_identity(self):
        for identity in ((123, 456, 7), (999, 123, 8)):
            with self.subTest(identity=identity):
                library = self.library([errno.EPERM, identity])
                with patch.object(supervisor.sys, 'platform', 'darwin'), \
                     patch.object(supervisor, '_LIBPROC', library), \
                     patch.object(supervisor.os, 'kill') as kill, \
                     patch.object(supervisor.time, 'sleep') as sleep, \
                     patch.object(supervisor.time, 'monotonic', return_value=10.):
                    self.assertEqual(supervisor.process_identity(12345), (identity[:2], identity[2]))
                    kill.assert_called_once_with(12345, 0)
                    self.assertEqual(library.proc_pidinfo.call_count, 2)
                    sleep.assert_called_once_with(supervisor._IDENTITY_RETRY_DELAY)

    def test_retry_same_birth_can_signal_but_reused_birth_cannot(self):
        for seconds in (123, 999):
            with self.subTest(seconds=seconds):
                with patch.object(supervisor, 'birth', return_value=(123, 456)):
                    tree = supervisor.OwnedTree(12345)
                library = self.library([errno.EPERM, (seconds, 456, 7)])
                with patch.object(supervisor.sys, 'platform', 'darwin'), \
                     patch.object(supervisor, '_LIBPROC', library), \
                     patch.object(supervisor.os, 'kill') as kill, \
                     patch.object(supervisor.time, 'sleep'), \
                     patch.object(supervisor.time, 'monotonic', return_value=10.):
                    events = []
                    tree.send(signal.SIGTERM, {12345}, events)
                    expected = [unittest.mock.call(12345, 0)]
                    if seconds == 123: expected.append(unittest.mock.call(12345, signal.SIGTERM))
                    self.assertEqual(kill.call_args_list, expected)
                    self.assertEqual(len(events), int(seconds == 123))

    def test_transient_permission_then_native_or_independent_esrch_is_absence(self):
        for native in (True, False):
            with self.subTest(native=native):
                library = self.library([errno.EPERM, errno.ESRCH if native else errno.EPERM])
                probes = [None] if native else [None, ProcessLookupError(errno.ESRCH, 'gone')]
                with patch.object(supervisor.sys, 'platform', 'darwin'), \
                     patch.object(supervisor, '_LIBPROC', library), \
                     patch.object(supervisor.os, 'kill', side_effect=probes) as kill, \
                     patch.object(supervisor.time, 'sleep') as sleep, \
                     patch.object(supervisor.time, 'monotonic', return_value=10.):
                    self.assertIsNone(supervisor.process_identity(12345))
                    self.assertEqual(library.proc_pidinfo.call_count, 2)
                    self.assertEqual(kill.call_count, len(probes))
                    sleep.assert_called_once()

    def test_permanent_permission_retries_bounded_and_never_signals(self):
        with patch.object(supervisor, 'birth', return_value=(123, 456)):
            tree = supervisor.OwnedTree(12345)
        library = self.library([errno.EPERM] * supervisor._IDENTITY_ATTEMPTS)
        diagnostic = subprocess.CompletedProcess([], 0, '12345 7 501 /usr/bin/xcrun\n', '')
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor.os, 'kill') as kill, \
             patch.object(supervisor.time, 'sleep') as sleep, \
             patch.object(supervisor.time, 'monotonic', return_value=10.), \
             patch.object(supervisor.subprocess, 'run', return_value=diagnostic) as ps:
            with self.assertRaises(PermissionError) as error:
                tree.send(signal.SIGKILL, {12345}, [])
            self.assertIn(f'identity_attempts={supervisor._IDENTITY_ATTEMPTS}', str(error.exception))
            self.assertIn('pid=12345', str(error.exception))
            self.assertIn('liveness=present', str(error.exception))
            self.assertIn('/usr/bin/xcrun', str(error.exception))
            self.assertEqual(ps.call_count, 1)
            self.assertEqual(ps.call_args.kwargs['timeout'], 0.25)
            self.assertEqual(ps.call_args.args[0], ['ps', '-p', '12345', '-o', 'pid=,ppid=,uid=,comm='])
            self.assertEqual(library.proc_pidinfo.call_count, supervisor._IDENTITY_ATTEMPTS)
            self.assertEqual(sleep.call_count, supervisor._IDENTITY_ATTEMPTS - 1)
            self.assertTrue(all(call.args == (12345, 0) for call in kill.call_args_list))
            self.assertEqual(tree.owned, {12345: (123, 456)})

    def test_retry_window_does_not_retry_after_slow_failed_identity_query(self):
        library = self.library([errno.EPERM])
        with patch.object(supervisor.sys, 'platform', 'darwin'), \
             patch.object(supervisor, '_LIBPROC', library), \
             patch.object(supervisor.os, 'kill'), \
             patch.object(supervisor.time, 'monotonic', side_effect=[10., 10.2]), \
             patch.object(supervisor.time, 'sleep') as sleep, \
             patch.object(supervisor, '_identity_context', return_value='fixture diagnostic'):
            with self.assertRaises(PermissionError) as error:
                supervisor.process_identity(12345)
            self.assertIn('identity_attempts=1', str(error.exception))
            self.assertIn('retry_elapsed_ms=200.', str(error.exception))
            sleep.assert_not_called()


class BirthDiagnosticContextTests(unittest.TestCase):
    def test_context_failure_cannot_replace_primary_identity_error(self):
        library = MacOSBirthRetryTests().library([errno.EPERM])
        for failure in (subprocess.TimeoutExpired('ps', .25), PermissionError(errno.EPERM, 'ps denied')):
            with self.subTest(failure=failure), \
                 patch.object(supervisor.sys, 'platform', 'darwin'), \
                 patch.object(supervisor, '_LIBPROC', library), \
                 patch.object(supervisor.os, 'kill'), \
                 patch.object(supervisor, '_IDENTITY_ATTEMPTS', 1), \
                 patch.object(supervisor.subprocess, 'run', side_effect=failure):
                # Each case needs its own single-attempt kernel response.
                supervisor._LIBPROC = MacOSBirthRetryTests().library([errno.EPERM])
                with self.assertRaises(PermissionError) as raised:
                    supervisor.process_identity(12345)
                self.assertEqual(raised.exception.errno, errno.EPERM)
                self.assertIn('process birth identity unavailable: pid=12345', str(raised.exception))
                self.assertIn('ps_context_unavailable=', str(raised.exception))


class OwnedTreeTests(unittest.TestCase):
    def test_reparented_session_retained_and_unrelated_pid_excluded(self):
        identities = {10: ('a',), 11: ('b',), 99: ('other',)}
        with patch.object(supervisor, 'birth', side_effect=identities.get), \
             patch.object(supervisor, 'process_table', return_value={10: 1, 11: 10, 99: 1}), \
             patch.object(supervisor, 'process_identity', side_effect=lambda pid: (identities[pid], 10)):
            tree = supervisor.OwnedTree(10)
            self.assertEqual(tree.observe(), {10, 11})
            identities.pop(10)
            with patch.object(supervisor, 'process_table', return_value={11: 1, 99: 1}):
                self.assertEqual(tree.observe(), {11})
            with patch.object(os, 'kill') as kill:
                tree.send(signal.SIGTERM, tree.observe(), [])
                kill.assert_called_once_with(11, signal.SIGTERM)

    def test_stale_ps_child_row_cannot_adopt_reused_unrelated_pid(self):
        with patch.object(supervisor, 'birth', return_value=('parent',)), \
             patch.object(supervisor, 'process_table', return_value={10: 1, 11: 10}), \
             patch.object(supervisor, 'process_identity', return_value=(('replacement',), 99)):
            tree = supervisor.OwnedTree(10)
            self.assertEqual(tree.observe(), {10})
            self.assertNotIn(11, tree.owned)

    def test_changed_parent_identity_blocks_child_enrollment(self):
        with patch.object(supervisor, 'birth', side_effect=[('old',), ('old',), ('new',)]), \
             patch.object(supervisor, 'process_table', return_value={10: 1, 11: 10}), \
             patch.object(supervisor, 'process_identity', return_value=(('child',), 10)):
            tree = supervisor.OwnedTree(10)
            tree.observe()
            self.assertNotIn(11, tree.owned)

    def test_unobserved_original_group_member_fails_custody_without_blind_kill(self):
        process = unittest.mock.Mock(pid=10)
        process.poll.return_value = 0
        process.wait.return_value = 0
        tree = unittest.mock.Mock()
        tree.observe.return_value = set()
        with patch.object(supervisor, 'OwnedTree', return_value=tree), \
             patch.object(supervisor, 'group_exists', return_value=True):
            result = supervisor.supervise(process, supervisor.Cancellation())
        self.assertFalse(result['cleanup_complete'])
        self.assertIn('original process group', result['cleanup_errors'][0])
        tree.send.assert_not_called()

    def test_emergency_kill_failure_still_reaps_and_preserves_primary(self):
        process = unittest.mock.Mock(pid=10)
        process.poll.return_value = None
        process.kill.side_effect = PermissionError('denied')
        process.wait.side_effect = subprocess.TimeoutExpired('child', 1)
        with patch.object(supervisor, 'OwnedTree', side_effect=RuntimeError('observer')), \
             patch.object(supervisor, 'group_exists', return_value=True):
            result = supervisor.supervise(process, supervisor.Cancellation(), primary_error='first')
        self.assertEqual(result['error'], 'first')
        self.assertFalse(result['cleanup_complete'])
        process.wait.assert_called_once()
        self.assertTrue(any('emergency kill' in item for item in result['cleanup_errors']))

    def test_dead_history_pruned_but_living_orphan_retained(self):
        identities = {10: ('root',), 11: ('dead',), 12: ('orphan',), 13: ('reused',)}
        with patch.object(supervisor, 'birth', side_effect=identities.get):
            tree = supervisor.OwnedTree(10)
            tree.owned.update({11: ('dead',), 12: ('orphan',), 13: ('old',)})
            identities.pop(11)
            with patch.object(supervisor, 'process_table', return_value={10: 1, 12: 1, 13: 99}):
                self.assertEqual(tree.observe(), {10, 12})
                self.assertEqual(tree.owned, {10: ('root',), 12: ('orphan',)})
                self.assertEqual(tree.observe(), {10, 12})

    def test_reused_pid_is_not_signalled(self):
        with patch.object(supervisor, 'birth', return_value=('old',)):
            tree = supervisor.OwnedTree(10)
        with patch.object(supervisor, 'birth', return_value=('new',)), patch.object(os, 'kill') as kill:
            tree.send(signal.SIGKILL, {10}, [])
            kill.assert_not_called()

    def test_repeated_signal_does_not_reset_grace_and_handlers_restore(self):
        previous = {s: signal.getsignal(s) for s in (signal.SIGINT, signal.SIGTERM)}
        with supervisor.Cancellation() as latch:
            latch.receive(signal.SIGTERM, None)
            first = latch.when
            latch.receive(signal.SIGINT, None)
            self.assertEqual((latch.signum, latch.when), (signal.SIGTERM, first))
        self.assertEqual(previous, {s: signal.getsignal(s) for s in previous})

    def test_missing_kernel_identity_never_invents_pid_authority(self):
        with patch.object(supervisor, 'birth', return_value=None), \
             patch.object(supervisor, 'process_table', return_value={10: 1, 11: 10}):
            tree = supervisor.OwnedTree(10)
            self.assertEqual(tree.observe(), set())

    def test_primary_error_survives_cleanup_failure(self):
        process = unittest.mock.Mock(pid=10)
        process.poll.return_value = 1
        process.wait.return_value = 1
        with patch.object(supervisor, 'birth', return_value=('a',)), \
             patch.object(supervisor, 'process_table', side_effect=RuntimeError('observer failed')), \
             patch.object(os, 'kill'):
            result = supervisor.supervise(process, supervisor.Cancellation(), primary_error='receipt failed')
        self.assertEqual(result['error'], 'receipt failed')
        self.assertTrue(any('root_pid=10' in item for item in result['cleanup_errors']))
        self.assertFalse(result['cleanup_complete'])
        self.assertTrue(result['cleanup_errors'])


class NativeCancellationTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform == 'darwin', 'macOS nested discovery helpers')
    def test_nested_supervisors_observe_concurrently_without_ps_children(self):
        code = '''
import subprocess,sys,time
sys.path.insert(0,sys.argv[1])
from completion_process_supervisor import OwnedTree
child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(5)'],start_new_session=True)
try:
 tree=OwnedTree(child.pid)
 while child.poll() is None:
  tree.observe()
  time.sleep(.01)
finally:
 if child.poll() is None: child.kill()
 child.wait()
'''
        child = subprocess.Popen([sys.executable, '-c', code, str(Path(supervisor.__file__).parent)], start_new_session=True)
        try:
            result = supervisor.supervise(child, supervisor.Cancellation(), timeout=15, grace=2)
            self.assertIsNone(result['error'], result)
            self.assertEqual(result['returncode'], 0, result)
            self.assertTrue(result['cleanup_complete'], result)
        finally:
            if child.poll() is None: child.kill()
            child.wait(timeout=3)

    @unittest.skipUnless(sys.platform == 'darwin', 'macOS gone discovery candidate')
    def test_reaped_candidate_has_no_discoverable_children(self):
        child = subprocess.Popen(['/usr/bin/true'])
        child.wait(timeout=3)
        self.assertEqual(supervisor.kernel_child_candidates(child.pid), [])

    @unittest.skipUnless(sys.platform == 'darwin', 'macOS kernel child discovery')
    def test_live_child_discovery_growth_preserves_birth_and_parent(self):
        children = [subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(10)']) for _ in range(3)]
        try:
            pids = {child.pid for child in children}
            discovered = supervisor.kernel_child_candidates(os.getpid(), initial_capacity=2)
            self.assertTrue(pids.issubset(discovered))
            for pid in pids:
                record = supervisor.process_identity(pid)
                self.assertIsNotNone(record)
                self.assertEqual(record[1], os.getpid())
        finally:
            for child in children:
                if child.poll() is None: child.terminate()
                child.wait(timeout=3)

    @unittest.skipUnless(sys.platform == 'darwin', 'macOS proc_pidinfo exit race')
    def test_reaped_subprocess_churn_does_not_erase_live_sibling_identity(self):
        sibling = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True)
        try:
            original = supervisor.birth(sibling.pid)
            self.assertIsNotNone(original)
            for _ in range(40):
                child = subprocess.Popen(['/usr/bin/true'], start_new_session=True)
                try:
                    child.wait(timeout=3)
                    self.assertIsNone(supervisor.birth(child.pid))
                    self.assertEqual(supervisor.birth(sibling.pid), original)
                finally:
                    if child.poll() is None: child.kill()
                    child.wait(timeout=3)
        finally:
            if sibling.poll() is None: sibling.kill()
            sibling.wait(timeout=3)

    def test_nested_detached_runner_is_settled_before_wrapper_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker = root / 'worker.py'
            worker.write_text('''import signal,time,sys
from pathlib import Path
stop=False
def cancel(*args):
 global stop
 stop=True
signal.signal(signal.SIGTERM,cancel)
print('ready',flush=True)
while not stop: time.sleep(.01)
Path(sys.argv[1]).write_text('settled')
''')
            wrapper = root / 'wrapper.py'
            wrapper.write_text('''import sys,subprocess,json
from pathlib import Path
sys.path.insert(0,sys.argv[1])
from completion_process_supervisor import Cancellation,supervise,birth
root=Path(sys.argv[2])
with Cancellation() as latch:
 child=subprocess.Popen([sys.executable,str(root/'worker.py'),str(root/'settled')],stdout=subprocess.PIPE,text=True,start_new_session=True)
 (root/'worker-identity.json').write_text(json.dumps([child.pid,birth(child.pid)]))
 assert child.stdout.readline().strip()=='ready'
 print('ready',flush=True)
 result=supervise(child,latch,grace=1,reap_timeout=2)
 child.stdout.close()
 (root/'inner.json').write_text(json.dumps(result))
sys.exit(int(bool(result['error'])))
''')
            child = subprocess.Popen([sys.executable, str(wrapper),
                str(Path(supervisor.__file__).parent), str(root)],
                stdout=subprocess.PIPE, text=True, start_new_session=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), 'ready')
                latch = supervisor.Cancellation(); latch.receive(signal.SIGTERM, None)
                result = supervisor.supervise(child, latch, grace=3, reap_timeout=2)
                self.assertTrue(result['cleanup_complete'], result)
                self.assertFalse(result['forced_cleanup'])
                self.assertEqual((root/'settled').read_text(), 'settled')
                inner = json.loads((root/'inner.json').read_text())
                self.assertTrue(inner['cleanup_complete'])
                self.assertEqual(inner['cancellation_signal'], signal.SIGTERM)
            finally:
                if child.poll() is None: child.kill()
                child.wait(); child.stdout.close()
                identity = root/'worker-identity.json'
                if identity.exists():
                    pid, born = json.loads(identity.read_text())
                    if born is not None and supervisor.birth(pid) == tuple(born):
                        os.kill(pid, signal.SIGKILL)

    def test_cooperative_runner_writes_summary_before_wrapper_returns(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / 'summary'
            child = subprocess.Popen([sys.executable, '-c', '''
import signal,sys,time
from pathlib import Path
stop=False
def cancel(*args):
 global stop
 stop=True
signal.signal(signal.SIGTERM,cancel)
print('ready',flush=True)
while not stop: time.sleep(.01)
Path(sys.argv[1]).write_text('688 durable rows')
''', str(marker)], stdout=subprocess.PIPE, text=True, start_new_session=True)
            try:
                self.assertEqual(child.stdout.readline().strip(), 'ready')
                latch = supervisor.Cancellation()
                latch.receive(signal.SIGTERM, None)
                result = supervisor.supervise(child, latch, grace=2, reap_timeout=2)
                self.assertEqual(marker.read_text(), '688 durable rows')
                self.assertTrue(result['cleanup_complete'])
                self.assertFalse(result['forced_cleanup'])
                self.assertIn('cancelled', result['error'])
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait()
                child.stdout.close()

    def test_normal_exit_after_poll_reaps_root_is_success(self):
        process = unittest.mock.Mock(pid=10)
        process.poll.return_value = 0
        process.wait.return_value = 0
        tree = unittest.mock.Mock()
        tree.observe.side_effect = [{10}, set(), set()]
        with patch.object(supervisor, 'OwnedTree', return_value=tree), \
             patch.object(supervisor, 'group_exists', return_value=False):
            result = supervisor.supervise(process, supervisor.Cancellation())
        self.assertIsNone(result['error'])
        self.assertTrue(result['cleanup_complete'])
        tree.send.assert_not_called()

    def test_stubborn_runner_escalates_within_bound(self):
        child = subprocess.Popen([sys.executable, '-c', '''
import signal,time
signal.signal(signal.SIGTERM,signal.SIG_IGN)
print('ready',flush=True)
time.sleep(60)
'''], stdout=subprocess.PIPE, text=True, start_new_session=True)
        try:
            self.assertEqual(child.stdout.readline().strip(), 'ready')
            latch = supervisor.Cancellation()
            latch.receive(signal.SIGINT, None)
            start = time.monotonic()
            result = supervisor.supervise(child, latch, grace=.2, reap_timeout=2)
            self.assertLess(time.monotonic() - start, 5)
            self.assertTrue(result['forced_cleanup'])
            self.assertTrue(result['cleanup_complete'])
            self.assertEqual(result['returncode'], -signal.SIGKILL)
        finally:
            if child.poll() is None:
                child.kill()
            child.wait()
            child.stdout.close()


class WrapperContractTests(unittest.TestCase):
    def test_completion_receipt_keeps_cancellation_and_cleanup(self):
        import completion_execution
        receipt = dict(returncode=143, error='cancelled', timed_out=False,
                       cleanup_complete=True, cleanup_errors=[], cancellation_signal=15,
                       forced_cleanup=False, events=[])
        with tempfile.TemporaryDirectory() as output, \
             patch.object(completion_execution, 'GenerationLease'), \
             patch.object(completion_execution.subprocess, 'Popen'), \
             patch.object(completion_execution, 'supervise', return_value=receipt) as run:
            result = completion_execution.run_owned(['runner'], output, environment={
                'ORYN_GENERATION_GATE': 'gate', 'ORYN_GENERATION_GATE_SHA256': 'hash'})
        self.assertEqual(result['error'], 'cancelled')
        self.assertEqual(run.call_args.kwargs['grace'], 60)
        self.assertTrue(result['cleanup_complete'])

    def test_spec_loaded_custody_imports_helper_without_scripts_path(self):
        scripts = Path(__file__).resolve().parent
        spec = importlib.util.spec_from_file_location('isolated_qualification_custody',
                                                     scripts / 'qualification/custody.py')
        module = importlib.util.module_from_spec(spec)
        prior_path = list(sys.path)
        prior_helper = sys.modules.pop('completion_process_supervisor', None)
        try:
            sys.path[:] = [item for item in sys.path if Path(item or '.').resolve() != scripts]
            spec.loader.exec_module(module)
            self.assertTrue(callable(module.supervise))
            self.assertEqual(Path(sys.modules['completion_process_supervisor'].__file__).resolve(),
                             scripts / 'completion_process_supervisor.py')
        finally:
            sys.path[:] = prior_path
            sys.modules.pop('completion_process_supervisor', None)
            if prior_helper is not None:
                sys.modules['completion_process_supervisor'] = prior_helper

    def test_dispatch_start_receipt_failure_still_supervises_child(self):
        from qualification import custody
        receipt = dict(returncode=1, error='receipt failed', timed_out=False,
                       cleanup_complete=True, cleanup_errors=[], cancellation_signal=None,
                       forced_cleanup=False, events=[])
        with patch.object(custody.subprocess, 'Popen') as popen, \
             patch.object(custody, 'save', side_effect=OSError('receipt failed')), \
             patch.object(custody, 'supervise', return_value=receipt) as run:
            popen.return_value.pid = 123
            result = custody.run_owned_dispatch(['runner'], {}, start_receipt=Path('receipt'))
        self.assertIn('receipt failed', run.call_args.kwargs['primary_error'])
        self.assertEqual(run.call_args.kwargs['grace'], 90)
        self.assertEqual(result['execution_error'], 'receipt failed')
        self.assertTrue(result['group_settled'])


if __name__ == '__main__':
    unittest.main()

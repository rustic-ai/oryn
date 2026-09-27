"""Bounded resource API regressions; fake C functions do not inspect the host."""
import ctypes
import errno
import unittest
from unittest import mock
import workflow_macos_resources as mac
import workflow_resources as resources


class Function:
    def __init__(self, call): self.call = call
    def __call__(self, *args): return self.call(*args)


class Library:
    def __init__(self, *, byte_result=False):
        self.tree = {10: [11], 11: [12], 12: []}
        self.records = {10: (1, 1031, 10), 11: (10, 2055, 11), 12: (11, 3097, 12)}
        self.failures = {}; self.queried = []; self.capacities = []; self.byte_result = byte_result
        self.proc_listchildpids = Function(self.children)
        self.proc_pidinfo = Function(self.info)
    def children(self, pid, buffer, size):
        self.capacities.append((pid, size // 4))
        values = self.tree.get(pid, [])[:size // 4]
        for index, value in enumerate(values): buffer[index] = value
        return len(values) * (4 if self.byte_result else 1)
    def info(self, pid, flavor, argument, pointer, size):
        self.queried.append(pid)
        if pid in self.failures:
            result, failure = self.failures[pid]; ctypes.set_errno(failure); return result
        parent, rss, birth = self.records[pid]
        info = ctypes.cast(pointer, ctypes.POINTER(mac._TaskAllInfo)).contents
        info.bsd.pid = pid; info.bsd.ppid = parent; info.bsd.start_seconds = birth
        info.task.resident_size = rss
        return size


class MacOSResourceTests(unittest.TestCase):
    def test_sdk_layout_and_exact_byte_units(self):
        self.assertEqual((ctypes.sizeof(mac._BSDInfo), ctypes.sizeof(mac._TaskInfo),
                          ctypes.sizeof(mac._TaskAllInfo)), (136, 96, 232))
        lib = Library(); tree = mac.MacOSProcessTree(10, library=lib)
        self.assertEqual(tree(), [dict(pid=10, parent_pid=1, rss_bytes=1031, kernel_birth=[10, 0]),
                                 dict(pid=11, parent_pid=10, rss_bytes=2055, kernel_birth=[11, 0]),
                                 dict(pid=12, parent_pid=11, rss_bytes=3097, kernel_birth=[12, 0])])
        self.assertEqual(lib.queried, [10, 11, 12])
    def test_unrelated_inaccessible_process_is_never_queried(self):
        lib = Library(); lib.failures[99] = (0, errno.EPERM)
        mac.MacOSProcessTree(10, library=lib)()
        self.assertNotIn(99, lib.queried)
    def test_full_child_buffer_grows_without_interpreting_positive_units(self):
        for byte_result in (False, True):
            lib = Library(byte_result=byte_result); lib.tree = {10: list(range(100, 240))}
            for pid in lib.tree[10]: lib.records[pid] = (10, pid, pid)
            rows = mac.MacOSProcessTree(10, library=lib)()
            self.assertEqual(len(rows), 141)
            self.assertEqual(lib.capacities[:2], [(10, 128), (10, 256)])
    def test_owned_failure_never_creates_zero_rss(self):
        for result, failure in ((0, errno.EPERM), (128, 0), (0, 0)):
            lib = Library(); lib.failures[11] = (result, failure)
            with self.assertRaisesRegex(OSError, 'owned resource query failed'):
                mac.MacOSProcessTree(10, library=lib)()
    def test_vanished_child_and_root_are_not_fabricated(self):
        lib = Library(); lib.failures[11] = (0, errno.ESRCH)
        self.assertEqual([row['pid'] for row in mac.MacOSProcessTree(10, library=lib)()], [10])
        lib.failures[10] = (0, errno.ESRCH)
        self.assertEqual(mac.MacOSProcessTree(10, library=lib)(), [])
    def test_parent_reuse_and_reparenting_fail(self):
        lib = Library(); tree = mac.MacOSProcessTree(10, library=lib); tree()
        lib.records[10] = (1, 10, 999)
        with self.assertRaisesRegex(OSError, 'reused'): tree()
        lib = Library(); lib.records[11] = (999, 10, 11)
        with self.assertRaisesRegex(OSError, 'reparented'): mac.MacOSProcessTree(10, library=lib)()
    def test_duplicate_or_cyclic_children_and_capacity_fail(self):
        for children in ([11, 11], [10]):
            lib = Library(); lib.tree[10] = children
            with self.assertRaises(OSError): mac.MacOSProcessTree(10, library=lib)()
        lib = Library(); lib.tree[10] = [11, 12]
        with self.assertRaisesRegex(OSError, 'buffer exhausted'):
            mac.MacOSProcessTree(10, library=lib, max_processes=2)()
    def test_deadline_is_bounded(self):
        tree = mac.MacOSProcessTree(10, library=Library())
        with mock.patch.object(mac.time, 'monotonic', side_effect=[0, 4]):
            with self.assertRaisesRegex(OSError, 'deadline'): tree()


class SamplerBackendTests(unittest.TestCase):
    def one_sample(self, *, platform='darwin', direct=None, ps='10 1 2\n11 10 3\n'):
        sampler = resources.ProcessSampler(10)
        def wait(_timeout): sampler.stop.set(); return True
        with mock.patch.object(resources.sys, 'platform', platform), \
             mock.patch.object(mac, 'MacOSProcessTree', return_value=direct) as constructor, \
             mock.patch.object(resources.subprocess, 'check_output', return_value=ps) as command, \
             mock.patch.object(sampler.stop, 'wait', side_effect=wait):
            sampler.collect()
        return sampler, constructor, command
    def test_macos_fast_path_has_no_subprocess_and_preserves_schema(self):
        sampler, _, command = self.one_sample(direct=lambda: [dict(pid=10, parent_pid=1, rss_bytes=123)])
        command.assert_not_called()
        self.assertEqual(sampler.samples[0]['processes'][0]['rss_bytes'], 123)
        self.assertEqual(sampler.interval_ms, 50)
        self.assertEqual(sampler.backend_counts['macos_libproc'], 1)
    def test_owned_libproc_error_uses_original_ps_with_diagnostic(self):
        direct = mock.Mock(side_effect=OSError(errno.EPERM, 'exit transition'))
        sampler, _, command = self.one_sample(direct=direct)
        command.assert_called_once_with(['ps', '-axo', 'pid=,ppid=,rss='], text=True, timeout=3)
        self.assertEqual(sampler.errors, [])
        self.assertEqual(sampler.samples[0]['processes'][1]['rss_bytes'], 3072)
        self.assertEqual(sampler.backend_counts['macos_libproc_fallback'], 1)
        self.assertIn('exit transition', sampler.backend_fallbacks[0]['error'])
    def test_nonmacos_keeps_existing_collector(self):
        sampler, constructor, command = self.one_sample(platform='linux')
        constructor.assert_not_called(); command.assert_called_once()
        self.assertEqual(sampler.backend_counts['ps'], 1)
    def test_root_reuse_is_terminal_without_pid_only_fallback(self):
        direct = mock.Mock(side_effect=mac.ParentReusedError(errno.ESTALE, 'root reused'))
        sampler, _, command = self.one_sample(direct=direct)
        command.assert_not_called()
        self.assertEqual(sampler.samples, [])
        self.assertIn('root reused', sampler.errors[0])
    def test_failed_fallback_remains_sampling_error(self):
        sampler = resources.ProcessSampler(10)
        with mock.patch.object(resources.sys, 'platform', 'darwin'), \
             mock.patch.object(mac, 'MacOSProcessTree', side_effect=OSError(errno.EPERM, 'denied')), \
             mock.patch.object(resources.subprocess, 'check_output', side_effect=OSError('ps denied')):
            sampler.collect()
        self.assertEqual(sampler.samples, [])
        self.assertIn('ps denied', sampler.errors[0])


if __name__ == '__main__': unittest.main()

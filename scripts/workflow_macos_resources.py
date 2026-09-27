"""Read-only macOS descendant RSS snapshots through libproc.

ABI fields/constants are from SDK sys/proc_info.h and libproc.h. The child-list
return's positive units are deliberately unused: zero-initialized PID slots and
an unfilled trailing slot bound the returned population. No process is signalled.
"""
import ctypes
import errno
import time


class _BSDInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint32) for name in (
        'flags', 'status', 'xstatus', 'pid', 'ppid', 'uid', 'gid', 'ruid',
        'rgid', 'svuid', 'svgid', 'reserved')] + [
        ('comm', ctypes.c_char * 16), ('name', ctypes.c_char * 32),
        ('nfiles', ctypes.c_uint32), ('pgid', ctypes.c_uint32),
        ('jobc', ctypes.c_uint32), ('tdev', ctypes.c_uint32),
        ('tpgid', ctypes.c_uint32), ('nice', ctypes.c_int32),
        ('start_seconds', ctypes.c_uint64), ('start_microseconds', ctypes.c_uint64)]


class _TaskInfo(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        'virtual_size', 'resident_size', 'total_user', 'total_system',
        'threads_user', 'threads_system')] + [
        (name, ctypes.c_int32) for name in (
            'policy', 'faults', 'pageins', 'cow_faults', 'messages_sent',
            'messages_received', 'syscalls_mach', 'syscalls_unix', 'csw',
            'threadnum', 'numrunning', 'priority')]


class _TaskAllInfo(ctypes.Structure):
    _fields_ = [('bsd', _BSDInfo), ('task', _TaskInfo)]


class ParentReusedError(OSError):
    """Observed root identity drift cannot be replaced by PID-only sampling."""


class MacOSProcessTree:
    """One sampler's owned root; failed owned queries cannot certify RSS."""
    def __init__(self, parent_pid, *, library=None, max_processes=65536,
                 query_budget_seconds=3.0):
        if type(parent_pid) is not int or not 0 < parent_pid <= 0x7fffffff:
            raise ValueError('invalid resource parent PID')
        if type(max_processes) is not int or not 2 <= max_processes <= 65536:
            raise ValueError('invalid resource process bound')
        if query_budget_seconds <= 0:
            raise ValueError('invalid resource query deadline')
        if (ctypes.sizeof(ctypes.c_int), ctypes.sizeof(_BSDInfo),
                ctypes.sizeof(_TaskInfo), ctypes.sizeof(_TaskAllInfo)) != (4, 136, 96, 232):
            raise OSError(errno.ENOTSUP, 'unsupported libproc resource ABI')
        self.parent_pid = parent_pid
        self.max_processes = max_processes
        self.query_budget_seconds = query_budget_seconds
        self.root_birth = None
        self.library = library if library is not None else ctypes.CDLL(
            '/usr/lib/libproc.dylib', use_errno=True)
        self.library.proc_listchildpids.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_int]
        self.library.proc_listchildpids.restype = ctypes.c_int
        self.library.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int,
                                             ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        self.library.proc_pidinfo.restype = ctypes.c_int

    @staticmethod
    def _deadline(deadline):
        if time.monotonic() >= deadline:
            raise OSError(errno.ETIMEDOUT, 'libproc resource snapshot deadline')

    def _info(self, pid, deadline):
        self._deadline(deadline)
        info = _TaskAllInfo()
        ctypes.set_errno(0)
        size = self.library.proc_pidinfo(pid, 2, 0, ctypes.byref(info), ctypes.sizeof(info))
        failure = ctypes.get_errno()
        if size == 0 and failure == errno.ESRCH:
            return None
        if size != ctypes.sizeof(info):
            raise OSError(failure or errno.EIO,
                          f'owned resource query failed: pid={pid} '
                          f'proc_pidinfo(PROC_PIDTASKALLINFO) bytes={size} expected={ctypes.sizeof(info)}')
        if info.bsd.pid != pid or info.bsd.start_microseconds >= 1000000:
            raise OSError(errno.EIO, f'invalid owned libproc identity: pid={pid}')
        return info

    def _children(self, pid, deadline):
        capacity = min(128, self.max_processes)
        while True:
            self._deadline(deadline)
            buffer = (ctypes.c_int * capacity)()
            ctypes.set_errno(0)
            result = self.library.proc_listchildpids(pid, buffer, ctypes.sizeof(buffer))
            failure = ctypes.get_errno()
            if result < 0 or (result == 0 and failure):
                if result == 0 and failure == errno.ESRCH:
                    return []
                raise OSError(failure or errno.EIO, f'owned child enumeration failed: pid={pid}')
            # The ABI's PID array is authoritative. Do not guess whether result
            # expresses a byte count or PID count; use only its zero/error state.
            values = list(buffer)
            count = next((index for index, value in enumerate(values) if value == 0), capacity)
            if any(value != 0 for value in values[count:]):
                raise OSError(errno.EIO, 'noncontiguous libproc child PID buffer')
            children = values[:count]
            if any(value <= 0 for value in children) or len(set(children)) != len(children):
                raise OSError(errno.EIO, 'invalid or duplicate libproc child PID')
            if (result == 0) != (count == 0):
                raise OSError(errno.EIO, 'libproc child population/result disagree')
            if count < capacity:
                return children
            if capacity == self.max_processes:
                raise OSError(errno.EOVERFLOW, 'libproc child PID buffer exhausted')
            capacity = min(capacity * 2, self.max_processes)

    def __call__(self):
        deadline = time.monotonic() + self.query_budget_seconds
        root = self._info(self.parent_pid, deadline)
        if root is None:
            return []
        birth = (root.bsd.start_seconds, root.bsd.start_microseconds)
        if self.root_birth is not None and birth != self.root_birth:
            raise ParentReusedError(errno.ESTALE, 'resource parent PID was reused')
        self.root_birth = birth
        pending = [(self.parent_pid, root)]
        seen = {self.parent_pid}
        rows = []
        while pending:
            pid, info = pending.pop()
            rows.append(dict(pid=pid, parent_pid=int(info.bsd.ppid),
                             rss_bytes=int(info.task.resident_size),
                             kernel_birth=[int(info.bsd.start_seconds), int(info.bsd.start_microseconds)]))
            for child in self._children(pid, deadline):
                if child in seen:
                    raise OSError(errno.EIO, 'libproc descendant graph repeats PID')
                if len(seen) >= self.max_processes:
                    raise OSError(errno.EOVERFLOW, 'libproc descendant process bound exhausted')
                seen.add(child)
                child_info = self._info(child, deadline)
                if child_info is None:
                    continue
                if child_info.bsd.ppid != pid:
                    raise OSError(errno.ESTALE, f'owned process reparented during resource query: pid={child}')
                pending.append((child, child_info))
        self._deadline(deadline)
        return sorted(rows, key=lambda row: row['pid'])

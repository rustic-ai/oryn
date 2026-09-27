"""Cooperative cancellation and retained descendant custody for suite wrappers.

Only ancestry-observed PID/birth identities are signalled. This is not a general
orphan reaper: descendants which fork and reparent between observations cannot
be recovered, so nested launchers must also honour the cancellation contract.
"""
import ctypes
import errno
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


class _BSDInfo(ctypes.Structure):
    _fields_ = [('ids', ctypes.c_uint32 * 12), ('comm', ctypes.c_char * 16),
                ('name', ctypes.c_char * 32), ('files', ctypes.c_uint32),
                ('pgid', ctypes.c_uint32), ('jobc', ctypes.c_uint32),
                ('tdev', ctypes.c_uint32), ('tpgid', ctypes.c_uint32),
                ('nice', ctypes.c_int32), ('seconds', ctypes.c_uint64),
                ('microseconds', ctypes.c_uint64)]


_LIBPROC = None
_IDENTITY_ATTEMPTS = 6
_IDENTITY_RETRY_DELAY = 0.01
_IDENTITY_RETRY_WINDOW = 0.1

def _missing_bsd_identity(pid, size, failure):
    """Resolve only the gone-process race, never invent signal authority.

    macOS may deny proc_pidinfo while a process exits. A failed identity query
    alone does not prove exit. kill(pid, 0) is an independent liveness probe:
    only its ESRCH result authorizes discarding this missing identity. A live,
    reused, inaccessible or otherwise indeterminate PID still fails closed.
    """
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return None
    except OSError as error:
        liveness = f'indeterminate errno={error.errno}: {error.strerror}'
    else:
        liveness = 'present'
    raise OSError(failure or errno.EIO,
        f'process birth identity unavailable: pid={pid} '
        f'api=proc_pidinfo(PROC_PIDTBSDINFO) returned_bytes={size} '
        f'expected_bytes={ctypes.sizeof(_BSDInfo)} native_errno={failure} '
        f'liveness={liveness}')


def _identity_context(pid):
    """Bounded diagnostics only; ps output is never process/signal authority."""
    try:
        result = subprocess.run(['ps', '-p', str(pid), '-o', 'pid=,ppid=,uid=,comm='],
            capture_output=True, text=True, timeout=0.25, env=dict(os.environ, LC_ALL='C'))
        return (f'ps_returncode={result.returncode} ps_context={result.stdout.strip()[:2048]!r} '
                f'ps_stderr={result.stderr.strip()[:256]!r}')
    except BaseException as error:
        return f'ps_context_unavailable={repr(error)[:512]}'


def process_identity(pid):
    """Kernel birth identity; never fall back to a PID or second-resolution ps."""
    if sys.platform == 'darwin':
        global _LIBPROC
        if _LIBPROC is None:
            _LIBPROC = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
            _LIBPROC.proc_pidinfo.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
            _LIBPROC.proc_pidinfo.restype = ctypes.c_int
        started = time.monotonic()
        deadline = started + _IDENTITY_RETRY_WINDOW
        for attempt in range(1, _IDENTITY_ATTEMPTS + 1):
            # Never reuse a partly filled record from an earlier failed query.
            info = _BSDInfo()
            ctypes.set_errno(0)
            size = _LIBPROC.proc_pidinfo(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info))
            if size == ctypes.sizeof(info):
                # Returning this candidate identity does not admit/signal it:
                # callers still compare retained birth and same-record PPID.
                return ((info.seconds, info.microseconds), int(info.ids[4]))
            failure = ctypes.get_errno()
            if size == 0 and failure == errno.ESRCH:
                # proc_pidinfo can report ESRCH for an unreaped zombie while
                # kill(pid, 0) still succeeds. Preserve the kernel's absence
                # result; it does not confer any signalling authority.
                return None
            try:
                return _missing_bsd_identity(pid, size, failure)
            except OSError as unavailable:
                now = time.monotonic()
                if attempt == _IDENTITY_ATTEMPTS or now >= deadline:
                    raise OSError(unavailable.errno,
                        f'{unavailable.strerror} identity_attempts={attempt} '
                        f'retry_elapsed_ms={(now-started)*1000:.3f} '
                        f'{_identity_context(pid)}') from unavailable
                # A short start/exec/exit transition is allowed to settle. A
                # liveness result never substitutes for the next kernel record.
                time.sleep(min(_IDENTITY_RETRY_DELAY, deadline - now))
    if sys.platform.startswith('linux'):
        try:
            value = Path('/proc', str(pid), 'stat').read_text()
        except FileNotFoundError:
            return None
        fields = value[value.rfind(')') + 2:].split()
        return ((fields[19],), int(fields[1]))
    raise RuntimeError('process birth identity unsupported on this platform')


def birth(pid):
    record = process_identity(pid)
    return record[0] if record is not None else None


def group_exists(group):
    try:
        os.killpg(group, 0)
    except ProcessLookupError:
        return False
    return True


def process_table():
    raw = subprocess.check_output(['ps', '-axo', 'pid=,ppid='], text=True,
                                  timeout=3, env=dict(os.environ, LC_ALL='C'))
    return {int(pid): int(parent) for pid, parent in
            (line.split() for line in raw.splitlines() if line.strip())}


class OwnedTree:
    def __init__(self, pid):
        self.root = pid
        identity = birth(pid)
        if identity is None:
            # The direct Popen child may have already exited; its group is not
            # sufficient authority to signal some subsequently reused PID.
            self.owned = {}
        else:
            self.owned = {pid: identity}

    def observe(self):
        table = process_table()
        # Keep detached live descendants, but never scan accumulated dead
        # history across thousands of suite attempts. Identity errors raise
        # before pruning, preserving fail-closed observation custody.
        retained = {pid: identity for pid, identity in self.owned.items()
                    if birth(pid) == identity}
        self.owned = retained
        live = set(retained)
        frontier = set(live)
        while frontier:
            children = {pid for pid, parent in table.items()
                        if parent in frontier and pid not in live}
            frontier = set()
            for pid in children:
                record = process_identity(pid)
                parent = table[pid]
                # ps only discovers candidates. Kernel PPID and birth must be
                # from one record, and the admitted parent must still match.
                if (record is not None and record[1] == parent
                        and birth(parent) == self.owned.get(parent)):
                    self.owned[pid] = record[0]
                    live.add(pid)
                    frontier.add(pid)
        return live

    def send(self, signum, pids, events):
        for pid in sorted(pids):
            if birth(pid) != self.owned.get(pid):
                continue
            try:
                os.kill(pid, signum)
                events.append(dict(event='signal', pid=pid, signal=signum))
            except ProcessLookupError:
                pass


class Cancellation:
    def __init__(self):
        self.signum = None
        self.when = None
        self.previous = {}

    def receive(self, signum, frame):
        if self.signum is None:
            self.signum, self.when = signum, time.monotonic()

    def __enter__(self):
        try:
            for signum in (signal.SIGTERM, signal.SIGINT):
                self.previous[signum] = signal.signal(signum, self.receive)
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def __exit__(self, *unused):
        for signum, previous in self.previous.items():
            signal.signal(signum, previous)


def supervise(process, cancellation, *, timeout=None, grace=60, reap_timeout=15,
              primary_error=None):
    """Return a durable failed receipt on cancellation, timeout or custody loss."""
    events, errors = [], []
    error = primary_error
    started = time.monotonic()
    stopping = None
    forced = False
    timed_out = False
    tree = None
    live = set()
    sent = set()
    try:
        tree = OwnedTree(process.pid)
        while True:
            now = time.monotonic()
            live = tree.observe()
            code = process.poll()
            if code is not None:
                live = tree.observe()  # poll may just have reaped the root
            if cancellation.signum is not None and stopping is None:
                error = error or 'owned execution cancelled by signal ' + str(cancellation.signum)
                stopping = cancellation.when
            if timeout is not None and now - started >= timeout and stopping is None:
                timed_out = True
                error = error or 'owned execution timed out'
                stopping = now
            if error is not None and stopping is None:
                stopping = now
            if code is not None and live and stopping is None:
                error = error or 'descendants survived direct-child exit'
                stopping = now
            if code is not None and not live:
                break
            if stopping is not None:
                # Signal the direct launcher first: it owns nested runner
                # settlement. If it is gone, take over retained descendants.
                targets = ({process.pid} & live) if code is None else live
                tree.send(signal.SIGTERM, targets - sent, events)
                sent.update(targets)
                if now - stopping >= grace:
                    forced = True
                    tree.send(signal.SIGKILL, live, events)
                if now - stopping >= grace + reap_timeout:
                    errors.append('owned descendants did not settle within cleanup bound')
                    break
            time.sleep(0.25)
    except BaseException as failure:
        detail = f'process ownership observation failed: root_pid={process.pid}: {failure!r}'
        error = error or detail
        errors.append(detail)
        # Identity-checked retained descendants remain eligible for emergency
        # cleanup, but an incomplete observation can never become a pass.
        if tree is not None:
            try:
                tree.send(signal.SIGKILL, set(tree.owned), events)
            except BaseException as cleanup_failure:
                errors.append(repr(cleanup_failure))
        try:
            if process.poll() is None:
                process.kill()  # Popen still owns this unreaped child PID.
        except BaseException as cleanup_failure:
            errors.append('direct-child emergency kill failed: ' + repr(cleanup_failure))
    try:
        code = process.wait(timeout=reap_timeout)
    except BaseException as failure:
        code = None
        errors.append('direct-child reap failed: ' + repr(failure))
    try:
        live = tree.observe() if tree is not None else {process.pid}
    except BaseException as failure:
        errors.append('final ownership observation failed: ' + repr(failure))
        live = {process.pid}
    try:
        if group_exists(process.pid):
            errors.append('original process group still exists; unobserved members may remain')
    except BaseException as failure:
        errors.append('original group observation failed: ' + repr(failure))
    return dict(returncode=code, error=error, timed_out=timed_out,
                cleanup_complete=not live and not errors, cleanup_errors=errors,
                cancellation_signal=cancellation.signum, forced_cleanup=forced,
                events=events)

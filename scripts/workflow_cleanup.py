"""Owned cleanup shared by family fixtures and the qualification dispatcher."""
import json
import os
import signal
import threading
import time

from completion_process_supervisor import process_identity

_RETAINED = []
_LOCK = threading.Lock()
_MAX_IDENTITIES = 16384


class CleanupFailure(RuntimeError):
    def __init__(self, primary, errors, custody_lost=False, owners=()):
        self.primary = primary
        self.cleanup_errors = list(errors)
        self.custody_lost = custody_lost or getattr(primary, 'custody_lost', False) or (primary is not None and not isinstance(primary, Exception))
        self.owners = tuple(owners)
        super().__init__('; '.join(([str(primary)] if primary is not None else []) + self.cleanup_errors))
        # A failed constructor cannot transfer its owner to the caller. Keep it
        # reachable for retry/inspection; never silently discard uncertain custody.
        if owners:
            with _LOCK:
                _RETAINED.append(self)


class NativeExecutionFailure(RuntimeError):
    """Execution disposition stays distinct from any later cleanup failure."""
    def __init__(self, message, *, timed_out=False, interrupted=False):
        self.timed_out = timed_out
        self.interrupted = interrupted
        super().__init__(message)


def failure_details(error):
    cleanup_errors = []
    current = error
    seen = set()
    while isinstance(current, CleanupFailure) and id(current) not in seen:
        seen.add(id(current))
        cleanup_errors.extend(current.cleanup_errors)
        if current.primary is None:
            break
        current = current.primary
    return dict(primary_error=str(current), cleanup_errors=cleanup_errors,
                timed_out=getattr(current, 'timed_out', False) is True,
                interrupted=getattr(current, 'interrupted', False) is True)


class NativeCustody:
    """Retain kernel identities from the existing sampler, with no extra scans."""
    def __init__(self, process):
        self.process = process
        self.lock = threading.Lock()
        self.signalled = set()
        self.discovery_errors = []
        self.sealed = False
        self.pending = {}
        self.terminal_observations = []
        record = process_identity(process.pid)
        self.owned = {process.pid: record[0]} if record is not None else {}

    def mark_uncertain(self, error):
        with self.lock:
            if len(self.discovery_errors) < 32:
                self.discovery_errors.append(str(error))

    def seal(self):
        with self.lock:
            self.sealed = True

    def discovery_status(self):
        with self.lock:
            pending_errors = [f'unresolved sampled child pid={pid} birth={birth}'
                              for (pid,birth) in self.pending]
            return self.sealed, list(self.discovery_errors) + pending_errors

    def observe(self, rows):
        # Raw resource snapshots are not authority. Verify kernel PPID and
        # retained parent birth before admitting each newly discovered child.
        with self.lock:
            if self.sealed:
                self.discovery_errors.append('ownership publication after discovery sealed')
                raise OSError('ownership publication after discovery sealed')
            pending = {row['pid']: row for row in rows
                       if row['pid'] not in self.owned or
                       (row.get('kernel_birth') is not None and
                        tuple(row['kernel_birth']) != self.owned[row['pid']])}
            # Preserve a candidate even when its parent exits before admission.
            # This is uncertainty, not permission to signal that PID.
            for pid,row in pending.items():
                birth=row.get('kernel_birth')
                key=(pid,tuple(birth) if birth is not None else None)
                if key not in self.pending and len(self.pending)>=_MAX_IDENTITIES:
                    if 'pending ownership bound exceeded' not in self.discovery_errors:
                        self.discovery_errors.append('pending ownership bound exceeded')
                    continue
                self.pending[key] = dict(row)
            while pending:
                admitted = []
                for pid, row in pending.items():
                    parent = row['parent_pid']
                    if parent not in self.owned:
                        continue
                    captured = row.get('kernel_birth')
                    identity = ((tuple(captured), parent) if captured is not None
                                else process_identity(pid))
                    ancestor = process_identity(parent)
                    if (identity is not None and identity[1] == parent
                            and ancestor is not None and ancestor[0] == self.owned[parent]):
                        if pid not in self.owned and len(self.owned)>=_MAX_IDENTITIES:
                            if 'retained ownership bound exceeded' not in self.discovery_errors:
                                self.discovery_errors.append('retained ownership bound exceeded')
                            continue
                        self.owned[pid] = identity[0]
                        admitted.append(pid)
                        self.pending.pop((pid,tuple(captured) if captured is not None else None),None)
                if not admitted:
                    break
                for pid in admitted:
                    pending.pop(pid)

    def signal(self, signum, events):
        with self.lock:
            retained = dict(self.owned)
        failures = []
        for pid, identity in retained.items():
            if (pid, identity, signum) in self.signalled:
                continue
            try:
                current = process_identity(pid)
                if current is not None and current[0] == identity:
                    os.kill(pid, signum)
                    self.signalled.add((pid, identity, signum))
                    events.append(dict(event='signal', pid=pid, birth=list(identity), signal=signum))
            except ProcessLookupError:
                pass
            except Exception as error:
                failures.append(f'pid={pid}: {error!r}')
        if failures:
            raise RuntimeError('; '.join(failures))

    def live(self):
        with self.lock:
            retained = dict(self.owned)
        result = []
        terminal = []
        for pid, identity in retained.items():
            current = process_identity(pid)
            if current is not None and current[0] == identity:
                result.append(pid)
            else:
                terminal.append(dict(pid=pid,birth=list(identity),status='absent' if current is None else 'reused',
                                     observed_birth=None if current is None else list(current[0])))
        # Positive absence/reuse can resolve an unadmitted snapshot candidate.
        # A still-live candidate never gains signal authority merely by persisting.
        with self.lock:
            pending = dict(self.pending)
        resolved=[]
        for (pid,birth),row in pending.items():
            current=process_identity(pid)
            if current is None or (birth is not None and current[0]!=birth):
                terminal.append(dict(pid=pid,birth=None if birth is None else list(birth),
                    status='unadmitted_absent' if current is None else 'unadmitted_reused',
                    observed_birth=None if current is None else list(current[0])))
                resolved.append((pid,birth))
            else:
                result.append(pid)
        with self.lock:
            for key in resolved:self.pending.pop(key,None)
            # Retain positively resolved candidates for the final receipt.
            known={(row['pid'],tuple(row['birth']) if row['birth'] is not None else None) for row in self.terminal_observations}
            for row in terminal:
                key=(row['pid'],tuple(row['birth']) if row['birth'] is not None else None)
                if key not in known:
                    if len(self.terminal_observations)>=_MAX_IDENTITIES:
                        if 'terminal observation bound exceeded' not in self.discovery_errors:
                            self.discovery_errors.append('terminal observation bound exceeded')
                    else:self.terminal_observations.append(row);known.add(key)
        return result

    def evidence(self):
        with self.lock:
            return dict(discovery_sealed=self.sealed,
                retained_identities=[dict(pid=pid,birth=list(birth)) for pid,birth in sorted(self.owned.items())],
                unresolved_candidates=[dict(pid=pid,birth=None if birth is None else list(birth)) for pid,birth in self.pending],
                terminal_identity_observations=list(self.terminal_observations))

    def close(self):
        return settle_group(self.process, custody=self)


class PathOwner:
    def __init__(self, path):
        self.path = path

    def close(self):
        self.path.unlink(missing_ok=True)
        return dict(returncode=0, forced=False)


class StreamOwner:
    def __init__(self, stream):
        self.stream = stream

    def close(self):
        self.stream.close()
        return dict(returncode=0, forced=False)


def write_owned_stream(stream, value):
    primary = None
    try:
        stream.write(value)
    except BaseException as error:
        primary = error
    try:
        stream.close()
    except BaseException as error:
        raise CleanupFailure(primary, ['secret stream close: '+repr(error)],
                             True, (StreamOwner(stream),)) from primary
    if primary is not None:
        raise primary


class DescriptorOwner:
    def __init__(self, descriptor):
        self.descriptor = descriptor

    def close(self):
        os.close(self.descriptor)
        return dict(returncode=0, forced=False)


def retry_retained_cleanup():
    """Explicit bounded retry; original failures remain immutable evidence."""
    with _LOCK:
        failures = list(_RETAINED)
        _RETAINED.clear()
    remaining, receipts = [], []
    for failure in failures:
        pending = []
        for owner in failure.owners:
            if hasattr(owner, 'close'):
                receipt, errors, settled = close_owner(owner)
            elif hasattr(owner, 'finish'):
                try:
                    receipt = owner.finish()
                    errors = receipt.get('errors', [])
                    settled = not owner.thread.is_alive()
                except BaseException as error:
                    receipt, errors, settled = None, [repr(error)], False
            elif hasattr(owner, 'join'):
                try:
                    owner.join(timeout=1)
                    settled = not owner.is_alive()
                    receipt, errors = None, [] if settled else ['thread still alive']
                except BaseException as error:
                    receipt, errors, settled = None, [repr(error)], False
            elif hasattr(owner, 'pid'):
                receipt = settle_group(owner)
                errors, settled = receipt['errors'], receipt['group_settled'] and type(receipt['returncode']) is int
            else:
                receipt, errors, settled = None, ['unknown retained owner'], False
            receipts.append(dict(receipt=receipt, errors=errors, settled=settled))
            if not settled:
                pending.append(owner)
        if pending:
            failure.owners = tuple(pending)
            remaining.append(failure)
    with _LOCK:
        _RETAINED.extend(remaining)
    return receipts


def settle_group(process, timeout=15, custody=None):
    """Bounded settlement; permission failures are uncertain, never absence.

    Native executions retain descendant births. Legacy fixture callers can
    signal their still-unreaped session leader's group only while it is live.
    An exited leader alone never proves its descendants settled.
    """
    errors, observations, events = [], [], []
    deadline = time.monotonic() + timeout
    settled = False
    last_uncertainty = None
    try:
        if custody is not None:
            custody.signal(signal.SIGKILL, events)
        elif process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except Exception as error:
        observations.append('initial cleanup signal: ' + repr(error))
        last_uncertainty = repr(error)
    try:
        process.wait(timeout=max(.001, min(.05, deadline-time.monotonic())))
    except Exception as error:
        last_uncertainty = 'parent wait: ' + repr(error)
        observations.append(last_uncertainty)
    while True:
        if process.returncode is None:
            try:
                process.wait(timeout=max(.001, min(.02, deadline-time.monotonic())))
            except Exception as error:
                last_uncertainty = 'parent wait: ' + repr(error)
        absent = False
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            absent = True
        except Exception as error:
            last_uncertainty = 'group observation: ' + repr(error)
            if len(observations) < 32:
                observations.append(last_uncertainty)
        try:
            live = custody.live() if custody is not None else []
            if absent and not live and type(process.returncode) is int:
                sealed, discovery_errors = custody.discovery_status() if custody is not None else (True, [])
                if not sealed:
                    errors.append('ownership discovery is not sealed')
                errors.extend('ownership discovery: ' + error for error in discovery_errors)
                settled = sealed and not discovery_errors
                break
            if custody is not None and live:
                custody.signal(signal.SIGKILL, events)
        except Exception as error:
            last_uncertainty = 'retained identity observation: ' + repr(error)
            if len(observations) < 32:
                observations.append(last_uncertainty)
        if time.monotonic() >= deadline:
            errors.append('owned process group did not settle' +
                          (': ' + last_uncertainty if last_uncertainty else ''))
            break
        time.sleep(.02)
    if type(process.returncode) is not int:
        errors.append('parent returncode is not settled')
    return dict(parent_pid=process.pid, process_group=process.pid,
                returncode=process.returncode, group_settled=settled,
                errors=errors, observations=observations, events=events,
                **(custody.evidence() if isinstance(custody, NativeCustody) else {}))


def close_owner(owner):
    try:
        receipt = owner.close()
    except BaseException as error:
        return None, [repr(error)], False
    if (isinstance(receipt, dict) and type(receipt.get('returncode')) is int
            and receipt.get('forced') is False and receipt == dict(returncode=0, forced=False)):
        return receipt, [], True
    if not isinstance(receipt, dict):
        return receipt, ['malformed fixture cleanup receipt'], False
    settled = (receipt.get('group_settled') is True and type(receipt.get('returncode')) is int
               and receipt.get('resources_settled', True) is True)
    if isinstance(owner, NativeCustody) and settled and receipt.get('errors') == []:
        return receipt, [], True
    errors = ['fixture cleanup: ' + repr(receipt)]
    return receipt, errors, settled


def cleanup_cases(cases, probes, stop, directory, primary=None):
    errors, owners, receipts = [], [], []
    if isinstance(primary, CleanupFailure) and primary.custody_lost:
        stop.set()
    for probe in probes:
        path = probe.get('secret_path')
        if path is not None:
            try:
                path.unlink(missing_ok=True)
            except BaseException as error:
                errors.append('secret cleanup: ' + repr(error))
                owners.append(PathOwner(path))
    for case in cases:
        owner = case.oracle
        if owner is None:
            continue
        receipt, problems, settled = close_owner(owner)
        receipts.append(receipt)
        errors.extend(problems)
        if settled:
            case.oracle = None
        else:
            owners.append(owner)
    if owners:
        stop.set()
    try:
        (directory/'fixture-cleanup.json').write_text(json.dumps(dict(
            primary_error=str(primary) if primary is not None else None,
            errors=errors, custody_lost=bool(owners), receipts=receipts), indent=2)+'\n')
    except Exception as error:
        errors.append('cleanup receipt persistence: ' + repr(error))
    if errors:
        raise CleanupFailure(primary, errors, bool(owners), owners) from primary

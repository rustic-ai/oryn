"""Bounded per-service diagnostics; never process ownership or signal authority.

The append descriptor is inherited only over upstream's synchronous spawn
window. Native closes that inheritance window at entry and explicitly forwards
one duplicate to its workers. Reading uses pread so it cannot move the offset
shared with the child. No reader thread or per-command filesystem work exists.
"""
import errno
import hashlib
import json
import os
from pathlib import Path
import secrets
import tempfile
import threading

from .timing import _identity

_SPAWN_WINDOW = threading.Lock()

MAX_BYTES = 32 * 1024 * 1024
MAX_LINE = 64 * 1024
VERSIONS = {'oryn_native_timing': 1, 'oryn_context_rpc_progress': 1,
            'oryn_context_parent_summary': 2}


def process_record(pid):
    # load_contract schema2 already imports this pinned repository tools root.
    # Missing identity remains explicit; it never becomes process authority.
    result = {'pid': pid, 'kernel_birth': None, 'parent_pid': None,
              'identity_status': 'unavailable'}
    try:
        from completion_process_supervisor import process_identity
        observed = process_identity(pid)
        if observed is not None:
            result.update(kernel_birth=list(observed[0]), parent_pid=observed[1],
                          identity_status='observed')
        else:
            result['identity_status'] = 'gone_before_observation'
    except Exception as error:
        result['identity_error_type'] = type(error).__name__
    return result


class Collector:
    def __init__(self, logger, identity):
        self.logger = logger
        self.identity = _identity(identity)
        self.service = {'service_id': secrets.token_hex(16), 'pid': None,
                        'kernel_birth': None, 'parent_pid': None,
                        'identity_status': 'not_started'}
        self.directory = Path(tempfile.mkdtemp(prefix='oryn-native-timing-'))
        self.path = self.directory / 'events.jsonl'
        try:
            self.fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_APPEND,
                              0o600)
            os.set_inheritable(self.fd, False)
        except BaseException:
            if hasattr(self, "fd"):
                os.close(self.fd)
            self.path.unlink(missing_ok=True)
            self.directory.rmdir()
            raise
        self._file_identity = (os.fstat(self.fd).st_dev, os.fstat(self.fd).st_ino)
        self._fd_closed = False
        self._quarantine_error = None
        self._spawn_window = False
        self.closed = False
        self.log_failures = 0

    def _emit(self, event, **fields):
        record = {'schema_version': 1, 'event': event, 'service': dict(self.service),
                  'identity': self.identity, 'prior_log_failures': self.log_failures,
                  **fields}
        try:
            self.logger.info('oryn-native-timing ' + json.dumps(record, sort_keys=True,
                                                               separators=(',', ':')))
        except Exception:
            self.log_failures += 1

    def inherit_for_spawn(self, timeout=30):
        # WPT managers are threads. Without this short process-wide window,
        # mozprocess(close_fds=False) could inherit another service's FD.
        if not _SPAWN_WINDOW.acquire(timeout=max(0, timeout)):
            raise TimeoutError('native diagnostic spawn window did not settle')
        self._spawn_window = True
        try:
            os.set_inheritable(self.fd, True)
        except BaseException as primary:
            try:
                self.private()
            except OSError as cleanup_error:
                primary.add_note('native diagnostic descriptor quarantine: ' + type(cleanup_error).__name__)
            raise

    def private(self):
        if self._quarantine_error is not None:
            # An uncertain close may have freed/reused the numeric FD. Never
            # mutate that descriptor or release its inheritance slot on retry.
            raise self._quarantine_error
        if not self.closed and not self._fd_closed:
            try:
                os.set_inheritable(self.fd, False)
            except OSError as error:
                # Never unlock while an unproven inheritable parent FD exists.
                # Closing our copy does not close the already inherited child
                # descriptor. Keep the original file for same-inode pread later.
                try:
                    os.close(self.fd)
                except OSError as close_error:
                    if close_error.errno != errno.EBADF:
                        self._quarantine_error = close_error
                        self._emit('descriptor_reset_error', error_type=type(error).__name__,
                                   close_error_type=type(close_error).__name__,
                                   spawn_window_retained=True, raw_path=str(self.path))
                        raise
                self._fd_closed = True
                self._emit('descriptor_reset_error', error_type=type(error).__name__,
                           parent_descriptor_closed=True, raw_path=str(self.path))
        if self._spawn_window:
            self._spawn_window = False
            _SPAWN_WINDOW.release()

    def _open_snapshot(self):
        if not self._fd_closed:
            return
        descriptor = os.open(self.path, os.O_RDONLY | os.O_NOFOLLOW)
        try:
            observed = os.fstat(descriptor)
            if (observed.st_dev, observed.st_ino) != self._file_identity:
                raise ValueError('native diagnostic file identity changed')
            os.set_inheritable(descriptor, False)
        except BaseException:
            os.close(descriptor)
            raise
        self.fd = descriptor
        self._fd_closed = False

    def published(self, pid):
        self.private()
        self.service.update(process_record(pid))

    def finish(self, *, process_stopped):
        try:
            return self._finish(process_stopped=process_stopped)
        except Exception as error:
            self._emit('collection_error', error_type=type(error).__name__,
                       raw_path=str(self.path), raw_file_retained=True)
            if process_stopped and not self.closed:
                # Retry the safe reset/close path. If closure is still unknown,
                # preserve the lock and descriptor custody for explicit failure.
                try:
                    self.private()
                    if not self._fd_closed:
                        os.close(self.fd)
                        self._fd_closed = True
                    self.closed = True
                except OSError:
                    pass

    def _finish(self, *, process_stopped):
        """Snapshot only after upstream observes its process stopped.

        This condition is not a descendant-retirement assertion. Native worker
        terminal evidence may be missing even after the driver exits. Preserve
        the raw file on every path so late writes, losses and killed-process
        tails remain independently inspectable; the collector owns no child.
        """
        if self.closed:
            return
        self.private()
        if not process_stopped:
            self._emit('pending', reason='upstream_process_still_live', raw_path=str(self.path))
            return
        self._open_snapshot()
        invalid = emitted = oversized = 0
        offset = 0
        pending = b''
        digest = hashlib.sha256()
        size = os.fstat(self.fd).st_size
        limit = min(size, MAX_BYTES)
        read_error = None
        dropping_line = False
        try:
            while offset < limit:
                chunk = os.pread(self.fd, min(65536, limit - offset), offset)
                if not chunk:
                    break
                offset += len(chunk)
                digest.update(chunk)
                for part in chunk.splitlines(keepends=True):
                    if dropping_line:
                        if part.endswith(b'\n'):
                            dropping_line = False
                        continue
                    pending += part
                    if len(pending) > MAX_LINE:
                        oversized += 1
                        dropping_line = not pending.endswith(b'\n')
                        pending = b''
                        continue
                    if not pending.endswith(b'\n'):
                        continue
                    try:
                        value = json.loads(pending)
                        if (not isinstance(value, dict)
                                or type(value.get('version')) is not int
                                or VERSIONS.get(value.get('kind')) != value.get('version')
                                or type(value.get('pid')) is not int or value['pid'] <= 0):
                            raise ValueError('unrecognized native diagnostic record')
                    except (ValueError, UnicodeError, TypeError):
                        invalid += 1
                    else:
                        emitted += 1
                        self._emit('record', record=value)
                    pending = b''
        except Exception as error:
            read_error = type(error).__name__
        finally:
            os.close(self.fd)
            self._fd_closed = True
            self.closed = True
        summary = dict(raw_path=str(self.path), snapshot_bytes=size,
                   read_bytes=offset, read_sha256=digest.hexdigest(), records=emitted,
                   invalid_records=invalid, oversized_records=oversized,
                   partial_tail=bool(pending or dropping_line), file_cap_reached=size >= MAX_BYTES,
                   snapshot_complete=(offset == size and not read_error),
                   read_error_type=read_error, missing_native_records=emitted == 0,
                   driver_observed_stopped=True, descendants_proven_stopped=False,
                   terminal_cpu_coverage='per-record-only; absent killed tails are unknown',
                   raw_file_retained=True)
        self._emit('summary', **summary)
        summary.update(service=dict(self.service), identity=self.identity,
                       log_failures=self.log_failures)
        (self.directory / 'collector-summary.json').write_text(
            json.dumps(summary, sort_keys=True) + '\n')

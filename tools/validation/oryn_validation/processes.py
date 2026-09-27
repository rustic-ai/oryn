"""Durable launch receipts using the existing Oryn descendant supervisor."""
import fcntl
from contextlib import nullcontext
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
from types import SimpleNamespace
import uuid

from .retention import retention_lock, validate_retention_index

SOURCE_ROOT = Path(__file__).resolve().parents[3]


def supervisor():
    """Load one canonical implementation; do not duplicate process custody."""
    name = "completion_process_supervisor"
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(
            name, SOURCE_ROOT / "scripts/completion_process_supervisor.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name]


def _write(path, value, *, exclusive=False):
    data = json.dumps(value, sort_keys=True, indent=2) + "\n"
    if exclusive:
        with path.open("x") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        return
    temporary = path.with_name(path.name + ".tmp-" + uuid.uuid4().hex)
    try:
        with temporary.open("x") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def _index(store):
    path = store / "retention.json"
    if path.is_symlink():
        raise ValueError("retention index cannot be a symlink")
    index = json.loads(path.read_text()) if path.exists() else {
        "schema_version": 1, "objects": {}, "leases": []}
    validate_retention_index(index)
    return path, index


def _no_symlink_ancestors(path):
    for item in (path, *path.parents):
        if item.is_symlink():
            raise ValueError("owned output cannot traverse a symlink: " + str(item))


class ResourceLease:
    """Cross-process resource exclusion plus persistent fail-closed retention."""
    def __init__(self, store_root, resource_keys, *, objects=()):
        self.store = Path(store_root).resolve(strict=True)
        self.keys = sorted(set(resource_keys))
        if not self.keys or any(not isinstance(k, str) or not k for k in self.keys):
            raise ValueError("at least one resource key is required")
        self.objects = list(objects)
        self.lease_id = uuid.uuid4().hex
        self.handles = []
        self.record = None
        self.settled = False

    def __enter__(self):
        directory = self.store / "status/resources"
        _no_symlink_ancestors(directory)
        directory.mkdir(parents=True, exist_ok=True)
        try:
            with retention_lock(self.store):
                index_path, index = _index(self.store)
                if any(name not in index["objects"] for name in self.objects):
                    raise ValueError("lease object is not registered")
                if any("removed_ns" in index["objects"][name] for name in self.objects):
                    raise ValueError("deleted cache requires explicit re-registration")
                for key in self.keys:
                    name = hashlib.sha256(key.encode()).hexdigest()
                    path = directory / (name + ".lock")
                    fd = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
                    self.handles.append(fd)
                    try:
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError as error:
                        raise RuntimeError("resource is busy: " + key) from error
                    status = directory / (name + ".json")
                    if status.is_symlink():
                        raise ValueError("resource status cannot be a symlink")
                    previous = json.loads(status.read_text()) if status.exists() else None
                    if previous and previous.get("state") != "terminal":
                        raise RuntimeError("resource has unsettled prior custody: " + key)
                identity = supervisor().process_identity(os.getpid())
                if identity is None:
                    raise RuntimeError("owner birth identity unavailable")
                self.record = {"lease_id": self.lease_id, "state": "active",
                               "owner_pid": os.getpid(), "owner_birth": list(identity[0]),
                               "resource_keys": self.keys, "objects": self.objects}
                for key in self.keys:
                    name = hashlib.sha256(key.encode()).hexdigest()
                    _write(directory / (name + ".json"), self.record)
                index["leases"].append({"lease_id": self.lease_id,
                                        "state": "active", "objects": self.objects})
                _write(index_path, index)
            return self
        except BaseException:
            self._close()
            raise

    def mark_settled(self, result):
        self.settled = (result.get("cleanup_complete") is True
                        and result.get("cleanup_errors") == [])

    def attach_objects(self, objects):
        """Pin newly registered outputs/caches while the resource owner is active."""
        if self.record is None or self.record["state"] != "active":
            raise ValueError("objects require an active resource lease")
        with retention_lock(self.store):
            path, index = _index(self.store)
            for identifier in objects:
                if identifier not in index["objects"] or "removed_ns" in index["objects"][identifier]:
                    raise ValueError("cannot lease missing or removed object")
            self.objects = sorted(set(self.objects) | set(objects))
            self.record["objects"] = self.objects
            for row in index["leases"]:
                if row["lease_id"] == self.lease_id:
                    row["objects"] = self.objects
            for key in self.keys:
                name = hashlib.sha256(key.encode()).hexdigest()
                _write(self.store / "status/resources" / (name + ".json"), self.record)
            _write(path, index)

    def _close(self):
        for fd in reversed(self.handles):
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)
        self.handles.clear()

    def __exit__(self, error_type, primary, traceback):
        try:
            if self.record:
                with retention_lock(self.store):
                    self.record["state"] = "terminal" if self.settled else "unsettled"
                    for key in self.keys:
                        name = hashlib.sha256(key.encode()).hexdigest()
                        _write(self.store / "status/resources" / (name + ".json"), self.record)
                    path, index = _index(self.store)
                    for row in index["leases"]:
                        if row["lease_id"] == self.lease_id:
                            row["state"] = self.record["state"]
                    _write(path, index)
        except BaseException as cleanup_error:
            if primary is None:
                raise
            primary.add_note("resource settlement metadata failed: " + repr(cleanup_error))
        finally:
            self._close()


def assert_no_active_writers(store_root):
    """Called while holding retention_lock; uncertainty is never permission."""
    _, index = _index(Path(store_root).resolve(strict=True))
    if any(row["state"] != "terminal" for row in index["leases"]):
        raise RuntimeError("active or unsettled writers prevent cache deletion")
    return True


def run_owned(argv, *, cwd, output, env=None, timeout_seconds,
              store_root=None, resource_keys=(), objects=(), grace_seconds=60):
    """Run one command without a shell; never overwrite another attempt."""
    if not isinstance(argv, (list, tuple)) or not argv or any(
            not isinstance(x, str) or not x or "\0" in x for x in argv):
        raise ValueError("argv must contain nonempty strings")
    if not isinstance(timeout_seconds, (int, float)) or timeout_seconds <= 0:
        raise ValueError("positive finite timeout required")
    import math
    if not math.isfinite(timeout_seconds):
        raise ValueError("positive finite timeout required")
    output = Path(output).absolute()
    _no_symlink_ancestors(output)
    cwd = Path(cwd).resolve(strict=True)
    if output.exists() or output.is_symlink():
        raise ValueError("attempt output already exists")
    if store_root is None:
        if resource_keys or objects:
            raise ValueError("resource ownership requires a store")
        return _launch(argv, cwd, output, env, timeout_seconds, grace_seconds)
    with ResourceLease(store_root, resource_keys or ["attempt:" + str(output)], objects=objects) as lease:
        result = _launch(argv, cwd, output, env, timeout_seconds, grace_seconds)
        lease.mark_settled(result)
        return result


def _launch(argv, cwd, output, env, timeout, grace, *, cancellation=None):
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic_ns()
    preflight = {"argv": list(argv), "cwd": str(cwd), "timeout_seconds": timeout,
                 "owner_pid": os.getpid(), "started_ns": time.time_ns(), "started_monotonic_ns": started}
    _write(output / "preflight.json", preflight, exclusive=True)
    result = dict(returncode=None, error=None, timed_out=False,
                  cleanup_complete=True, cleanup_errors=[], forced_cleanup=False,
                  cancellation_signal=None, events=[])
    process = None
    supervisor_module = supervisor()
    try:
        with (output / "stdout.log").open("xb") as stdout, (output / "stderr.log").open("xb") as stderr:
            context = supervisor_module.Cancellation() if cancellation is None else nullcontext(cancellation)
            with context as cancellation:
                process = subprocess.Popen(list(argv), cwd=cwd, env=env,
                                           stdout=stdout, stderr=stderr, start_new_session=True)
                try:
                    preflight["child_pid"] = process.pid
                    preflight["child_identity"] = supervisor_module.process_identity(process.pid)
                    _write(output / "preflight.json", preflight)
                    result = supervisor_module.supervise(process, cancellation, timeout=timeout, grace=grace)
                except BaseException as error:
                    result = supervisor_module.supervise(process, cancellation, timeout=timeout,
                                                         grace=grace, primary_error=repr(error))
    except BaseException as error:
        result["error"] = result.get("error") or repr(error)
        if process is not None:
            result["cleanup_complete"] = False
            result["cleanup_errors"].append("supervisor failed; descendant custody is uncertain")
        if process is not None and process.poll() is None:
            try:
                process.kill()  # Popen owns this unreaped direct child, never a saved PID.
                result["returncode"] = process.wait(timeout=15)
            except BaseException as cleanup_error:
                result["cleanup_errors"].append("direct-child emergency settlement failed: " + repr(cleanup_error))
    result["argv"] = list(argv)
    result["elapsed_ns"] = time.monotonic_ns() - started
    _write(output / "process-result.json", result, exclusive=True)
    return result


class OwnedService:
    """Continuously supervise a service while its caller performs owned work.

    The outer coordinator holds the shared resource lease. The monitor uses the
    same supervisor from launch through teardown, retaining descendant identity.
    Shutdown is recorded as intentional cancellation, never rewritten as a pass.
    """
    def __init__(self, argv, *, cwd, output, env=None, timeout_seconds, grace_seconds=5):
        import math
        if (not isinstance(argv, (list, tuple)) or not argv
                or any(not isinstance(x, str) or not x or "\0" in x for x in argv)):
            raise ValueError("service argv must contain nonempty strings")
        if (not math.isfinite(timeout_seconds) or timeout_seconds <= 0
                or not math.isfinite(grace_seconds) or grace_seconds < 0):
            raise ValueError("service lifetime and cleanup bounds must be finite")
        self.argv, self.cwd, self.output = list(argv), Path(cwd).resolve(strict=True), Path(output).absolute()
        _no_symlink_ancestors(self.output)
        if self.output.exists():
            raise ValueError("service output already exists")
        self.env, self.timeout, self.grace = env, timeout_seconds, grace_seconds
        self.cancellation = SimpleNamespace(signum=None, when=None)
        self.result = None
        self.failure = None
        self.thread = None

    def _monitor(self):
        try:
            self.result = _launch(self.argv, self.cwd, self.output, self.env,
                                  self.timeout, self.grace, cancellation=self.cancellation)
        except BaseException as error:
            self.failure = error

    def __enter__(self):
        if self.thread is not None:
            raise ValueError("service context cannot be reused")
        self.thread = threading.Thread(target=self._monitor, name="oryn-service-owner", daemon=False)
        self.thread.start()
        return self

    def check(self):
        if self.failure is not None:
            raise RuntimeError("service monitor failed: " + repr(self.failure))
        if self.result is not None:
            raise RuntimeError("service exited before coordinator shutdown: " + repr(self.result))
        if self.thread is None or not self.thread.is_alive():
            raise RuntimeError("service owner is not active")

    def __exit__(self, error_type, primary, traceback):
        import signal
        stopping_active_service = self.thread.is_alive() and self.result is None and self.failure is None
        self.cancellation.when = time.monotonic()
        self.cancellation.signum = signal.SIGTERM
        self.thread.join(self.grace + 20)
        failure = None
        if self.thread.is_alive():
            failure = RuntimeError("service supervisor has not settled within its cleanup bound")
        elif self.failure is not None:
            failure = RuntimeError("service monitor failed: " + repr(self.failure))
        elif self.result is not None:
            self.result["service_stop_requested"] = stopping_active_service
            try:
                _write(self.output / "service-result.json", self.result, exclusive=True)
            except BaseException as error:
                failure = RuntimeError("service terminal receipt failed: " + repr(error))
            if (not self.result["cleanup_complete"] or self.result["cleanup_errors"]
                    or not stopping_active_service or self.result["timed_out"]
                    or self.result["error"] != "owned execution cancelled by signal " + str(signal.SIGTERM)
                    or self.result["cancellation_signal"] != signal.SIGTERM
                    or self.result["forced_cleanup"]):
                failure = RuntimeError("service failed or did not settle: " + repr(self.result))
        else:
            failure = RuntimeError("service supervisor returned no terminal receipt")
        if failure is not None:
            if primary is not None:
                primary.add_note("service teardown failed: " + str(failure))
            else:
                raise failure

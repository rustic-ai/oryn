"""Explicit input custody without discovering dependencies inside file contents.

All symlinks and special files fail closed. A configured root may be relocated;
no record embeds its absolute location. Stat checkpoints are drift alarms only:
admission and finalization always hash bytes and verify exact membership.
"""
from __future__ import annotations

import hashlib
import os
import stat
import time
from pathlib import Path
from typing import Iterable

from .manifests import ManifestError, manifest_digest, relative_path, validate_input


class InputError(ManifestError):
    """A declared input is absent, changed, unsafe, or not exactly populated."""


def resolve_input(root: str | Path, path: str) -> Path:
    """Resolve a declared relative path, refusing symlink traversal at any level."""
    relative_path(path)
    root = Path(root).absolute()
    cursor = Path(root.anchor)
    for part in root.parts[1:]:
        cursor /= part
        if cursor.is_symlink():
            raise InputError(f"symlink in configured root: {cursor}")
    if not root.is_dir():
        raise InputError(f"missing input root: {root}")
    cursor = root
    for part in path.split("/"):
        cursor /= part
        try:
            mode = cursor.lstat().st_mode
        except OSError as error:
            raise InputError(f"missing input: {cursor}") from error
        if stat.S_ISLNK(mode):
            raise InputError(f"symlink input is not permitted: {cursor}")
    if not cursor.resolve().is_relative_to(root.resolve()):
        raise InputError(f"input escapes configured root: {path}")
    return cursor


def _identity(st: os.stat_result) -> tuple:
    return (st.st_dev, st.st_ino, st.st_mode, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def _file(path: Path) -> dict:
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise InputError(f"input is not a regular file: {path}")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_NONBLOCK", 0)
    try:
        fd = os.open(path, flags)
        with os.fdopen(fd, "rb") as handle:
            opened = os.fstat(handle.fileno())
            if _identity(opened) != _identity(before):
                raise InputError(f"input changed before hashing: {path}")
            digest = hashlib.sha256()
            while block := handle.read(1024 * 1024):
                digest.update(block)
            after = os.fstat(handle.fileno())
    except OSError as error:
        raise InputError(f"cannot hash input: {path}: {error}") from error
    if _identity(before) != _identity(after) or _identity(after) != _identity(path.lstat()):
        raise InputError(f"input changed while hashing: {path}")
    return {"sha256": digest.hexdigest(), "size": after.st_size,
            "executable": bool(after.st_mode & 0o111)}


def _membership(path: Path, hash_files: bool = True) -> tuple[list[str], list[dict]]:
    if not stat.S_ISDIR(path.lstat().st_mode):
        raise InputError(f"input is not a directory: {path}")
    directories: list[str] = []
    members: list[dict] = []
    def visit(directory: Path, prefix: str = "") -> None:
        before = directory.lstat()
        if not stat.S_ISDIR(before.st_mode):
            raise InputError(f"directory changed or is a symlink: {directory}")
        # One no-follow stat per entry preserves the same complete metadata
        # fields without repeated Path construction and duplicate stat calls.
        with os.scandir(directory) as entries:
            for entry in entries:
                observed = entry.stat(follow_symlinks=False)
                mode = observed.st_mode
                name = prefix + entry.name
                if stat.S_ISDIR(mode):
                    directories.append(name)
                    visit(Path(entry.path), name + "/")
                elif stat.S_ISREG(mode):
                    members.append({"path": name, **(_file(Path(entry.path)) if hash_files else {"stat": _identity(observed)})})
                else:
                    raise InputError(f"symlink or special file in input membership: {entry.path}")
        if _identity(before) != _identity(directory.lstat()):
            raise InputError(f"directory membership changed during observation: {directory}")
    visit(path)
    directories.sort()
    members.sort(key=lambda item: item["path"])
    return directories, members


def snapshot_input(root: str | Path, path: str, *, scope: str = "store", kind: str = "file") -> dict:
    resolved = resolve_input(root, path)
    record = {"scope": scope, "path": path, "kind": kind}
    if kind == "file":
        record.update(_file(resolved))
    elif kind == "directory":
        directories, members = _membership(resolved)
        record.update(directories=directories, members=members)
        record["sha256"] = manifest_digest({"directories": directories, "members": members})
    else:
        raise InputError(f"unknown input kind: {kind}")
    validate_input(record)
    return record


def _root(record: dict, store_root: str | Path, bundle_root: str | Path | None) -> Path:
    if record["scope"] == "bundle":
        if bundle_root is None:
            raise InputError("bundle input requires an explicit bundle_root")
        return Path(bundle_root)
    return Path(store_root)


def verify_input(record: dict, *, store_root: str | Path, bundle_root: str | Path | None = None) -> dict:
    validate_input(record)
    actual = snapshot_input(_root(record, store_root, bundle_root), record["path"],
                            scope=record["scope"], kind=record["kind"])
    if actual != record:
        raise InputError(f"declared input content or membership changed: {record['scope']}:{record['path']}")
    return {"scope": record["scope"], "path": record["path"], "sha256": record["sha256"]}


def verify_inputs(records: Iterable[dict], *, store_root: str | Path,
                  bundle_root: str | Path | None = None) -> dict:
    started = time.monotonic()
    records = list(records)
    seen = set()
    verified = []
    for record in records:
        validate_input(record)
        key = (record["scope"], record["path"])
        if key in seen:
            raise InputError("duplicate declared input")
        seen.add(key)
        verified.append(verify_input(record, store_root=store_root, bundle_root=bundle_root))
    return {"valid": True, "input_count": len(verified), "inputs_sha256": manifest_digest(records),
            "verified": verified, "elapsed_seconds": time.monotonic() - started}


class InputLease:
    """Admission/final digest checks and complete metadata checkpoints.

    Context exit always verifies inputs, including exceptional exit. A cleanup
    drift error is attached to an existing primary exception rather than hiding
    it. Callers must inspect final_error when creating terminal custody evidence.
    """
    def __init__(self, records: Iterable[dict], *, store_root: str | Path,
                 bundle_root: str | Path | None = None):
        import copy
        self.records = copy.deepcopy(list(records))
        self.store_root = store_root
        self.bundle_root = bundle_root
        self.admission = None
        self.finalization = None
        self.final_error = None
        self._baseline = None
        self._pid = os.getpid()

    def verify_bound(self, records, *, store_root, bundle_root=None):
        """Reuse only this live process's exact admitted closure, never a receipt."""
        if (os.getpid() != self._pid or list(records) != self.records
                or Path(store_root).absolute() != Path(self.store_root).absolute()
                or (Path(bundle_root).absolute() if bundle_root is not None else None)
                != (Path(self.bundle_root).absolute() if self.bundle_root is not None else None)):
            raise InputError('input lease differs from requested closure or process')
        self.checkpoint()

    def verify_subset(self, records):
        """Require exact records already covered by this live lease's final sweep."""
        if os.getpid() != self._pid:
            raise InputError('input lease belongs to another process')
        admitted = {(item['scope'], item['path']): item for item in self.records}
        seen = set()
        for record in records:
            validate_input(record)
            key = record['scope'], record['path']
            if key in seen or admitted.get(key) != record:
                raise InputError('input absent from active lease closure: ' + record['path'])
            seen.add(key)
        self.checkpoint()

    def _observe(self) -> list:
        observations = []
        for record in self.records:
            path = resolve_input(_root(record, self.store_root, self.bundle_root), record["path"])
            if record["kind"] == "directory":
                directories, members = _membership(path, hash_files=False)
                observations.append((record["scope"], record["path"], _identity(path.lstat()), directories, members))
            else:
                state = path.lstat()
                if not stat.S_ISREG(state.st_mode):
                    raise InputError(f"input is not a regular file: {path}")
                observations.append((record["scope"], record["path"], _identity(state)))
        return observations

    def __enter__(self) -> "InputLease":
        if self._baseline is not None:
            raise InputError("input lease cannot be entered twice")
        before = self._observe()
        self.admission = verify_inputs(self.records, store_root=self.store_root, bundle_root=self.bundle_root)
        self._baseline = self._observe()
        if before != self._baseline:
            raise InputError("input changed during admission")
        return self

    def checkpoint(self) -> None:
        if self._baseline is None or self.finalization is not None or self.final_error is not None:
            raise InputError("input lease is not active")
        if self._observe() != self._baseline:
            raise InputError("input metadata or membership changed during execution")

    def check(self) -> None:
        """Compatibility name for the adapter's boundary check."""
        self.checkpoint()

    def finalize(self) -> dict:
        if self._baseline is None:
            raise InputError("input lease was not admitted")
        if self.final_error is not None:
            raise self.final_error
        try:
            self.finalization = verify_inputs(self.records, store_root=self.store_root, bundle_root=self.bundle_root)
            return self.finalization
        except BaseException as error:
            self.finalization = None
            self.final_error = error
            raise

    def final_check(self) -> dict:
        """Compatibility name for complete final digest verification."""
        return self.finalize()

    def __exit__(self, exc_type, exc, traceback) -> bool:
        try:
            self.finalize()
        except BaseException as error:
            if exc is None:
                raise
            if hasattr(exc, "add_note"):
                exc.add_note(f"Input finalization also failed: {error}")
        return False

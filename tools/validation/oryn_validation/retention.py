"""Preview explicit local retention objects; delete only unleased compiler caches.

All cache writers and index/lease publishers must hold retention_lock while
changing ownership. Historical evidence is never an executable dependency unless
its object is explicitly pinned. This first migration does not delete evidence.
"""
from __future__ import annotations

from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import time
import uuid


def _digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _path(root, name):
    if not isinstance(name, str) or not name or "\\" in name:
        raise ValueError("invalid retention path")
    relative = PurePosixPath(name)
    if relative.is_absolute() or any(part in ("", ".", "..") for part in name.split("/")):
        raise ValueError("retention path escapes data root")
    root = Path(root).resolve(strict=True)
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError("retention never follows symlinks")
    return current


@contextmanager
def retention_lock(store_root):
    """Shared ownership transaction lock, also required by every cache producer."""
    root = Path(store_root).resolve(strict=True)
    path = root / ".retention.lock"
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)


def _inventory(path):
    if not path.is_dir() or path.is_symlink():
        raise ValueError("cleanup objects must be real directories")
    result, size = [], 0
    for item in [path, *sorted(path.rglob("*"))]:
        info = item.lstat()
        if not (stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)):
            raise ValueError("cache contains a symlink or special file")
        result.append([item.relative_to(path).as_posix(), info.st_dev, info.st_ino,
                       info.st_size, info.st_mtime_ns, info.st_ctime_ns, info.st_mode])
        if stat.S_ISREG(info.st_mode):
            size += info.st_size
    return {"identity": _digest(result), "apparent_bytes": size, "entries": len(result)}


def validate_retention_index(index):
    if set(index) != {"schema_version", "objects", "leases"} or index["schema_version"] != 1:
        raise ValueError("unsupported retention index")
    objects = index["objects"]
    if not isinstance(objects, dict) or not isinstance(index["leases"], list):
        raise ValueError("invalid objects or leases")
    for identifier, obj in objects.items():
        required = {"path", "kind", "dependencies", "pinned", "last_used_ns"}
        if not identifier or not required <= set(obj) or set(obj) - required - {"removed_ns"}:
            raise ValueError("invalid retention object")
        if obj["kind"] not in {"cargo-cache", "engine", "environment", "dataset", "build", "run", "archive"}:
            raise ValueError("unknown retention class")
        if type(obj["pinned"]) is not bool or type(obj["last_used_ns"]) is not int:
            raise ValueError("invalid retention pin or last-use timestamp")
        if "removed_ns" in obj and (obj["kind"] != "cargo-cache" or type(obj["removed_ns"]) is not int
                                     or obj["removed_ns"] <= 0):
            raise ValueError("only explicitly removed compiler caches may have a removal timestamp")
        if not isinstance(obj["dependencies"], list) or any(dep not in objects for dep in obj["dependencies"]):
            raise ValueError("retention has an unknown dependency")
    for lease in index["leases"]:
        if set(lease) != {"lease_id", "state", "objects"} or lease["state"] not in {"active", "unsettled", "terminal"}:
            raise ValueError("invalid retention lease")
        if not isinstance(lease["objects"], list) or any(obj not in objects for obj in lease["objects"]):
            raise ValueError("lease refers to an unknown object")
    return objects


def _write_index(path, index):
    validate_retention_index(index)
    temporary = path.with_name(path.name + "." + uuid.uuid4().hex + ".tmp")
    try:
        with temporary.open("x") as output:
            json.dump(index, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def restore_cache_registration(store_root, index_path, object_id):
    """Explicitly register a recreated cache before any producer may lease it."""
    root = Path(store_root).resolve(strict=True)
    with retention_lock(root):
        index_path = Path(index_path).resolve(strict=True)
        index = json.loads(index_path.read_text())
        objects = validate_retention_index(index)
        obj = objects.get(object_id)
        if obj is None or obj["kind"] != "cargo-cache" or "removed_ns" not in obj:
            raise ValueError("cache must have a prior explicit removal record")
        if any(row["state"] != "terminal" and object_id in row["objects"] for row in index["leases"]):
            raise ValueError("cannot re-register a cache with unsettled prior ownership")
        _inventory(_path(root, obj["path"]))
        del obj["removed_ns"]
        obj["last_used_ns"] = time.time_ns()
        _write_index(index_path, index)
        return {"registered": object_id, "index_sha256": _digest(index)}


def preview_cleanup(store_root, index, *, cache_budget_bytes=120 * 1024**3):
    """Plan exact LRU cache paths, marking transitive pins and unsettled leases."""
    if type(cache_budget_bytes) is not int or cache_budget_bytes < 0:
        raise ValueError("cache budget must be a nonnegative integer")
    objects = validate_retention_index(index)
    paths = {key: _path(store_root, obj["path"]) for key, obj in objects.items()}
    # Overlapping object roots would let deleting one destroy another's pin.
    for key, path in paths.items():
        for other, other_path in paths.items():
            if key != other and (path == other_path or path in other_path.parents):
                raise ValueError("retention object roots overlap")
    live = {key for key, obj in objects.items() if obj["pinned"]}
    for lease in index["leases"]:
        if lease["state"] != "terminal":
            live.update(lease["objects"])
    pending = list(live)
    while pending:
        for dependency in objects[pending.pop()]["dependencies"]:
            if dependency not in live:
                live.add(dependency)
                pending.append(dependency)
    inventories = {}
    for key, obj in objects.items():
        if obj["kind"] == "cargo-cache":
            if not obj["path"].startswith("cache/cargo/") or len(PurePosixPath(obj["path"]).parts) != 3:
                raise ValueError("compiler caches must be immediate configuration children of cache/cargo")
            if "removed_ns" in obj:
                if paths[key].exists() or key in live:
                    raise ValueError("removed cache requires explicit re-registration before reuse")
                continue
            inventories[key] = _inventory(paths[key])
    total = sum(row["apparent_bytes"] for row in inventories.values())
    remaining, removals = total, []
    for key in sorted(inventories, key=lambda key: (objects[key]["last_used_ns"], key)):
        if remaining <= cache_budget_bytes:
            break
        if key in live:
            continue
        row = {"object_id": key, "path": objects[key]["path"], **inventories[key]}
        removals.append(row)
        remaining -= row["apparent_bytes"]
    plan = {"schema_version": 1, "kind": "cache-cleanup-preview", "index_sha256": _digest(index),
            "cache_budget_bytes": cache_budget_bytes, "cache_apparent_bytes": total,
            "remaining_apparent_bytes": remaining, "budget_satisfied": remaining <= cache_budget_bytes,
            "protected_objects": sorted(live), "removals": removals}
    return {**plan, "preview_sha256": _digest(plan)}


def execute_cleanup(store_root, index_path, preview, *, expected_preview_sha256, receipt_path,
                    confirm_no_active_writers):
    """Execute exactly an approved preview under ownership lock, never broad rm.

    Caller supplies a fresh ownership check from the process/resource supervisor.
    It runs under the same lock and must raise on uncertain or active writers.
    Index mutation or filesystem drift invalidates the preview. Receipts outside
    object roots are created exclusively even on partial failures.
    """
    root = Path(store_root).resolve(strict=True)
    raw_receipt = Path(receipt_path).absolute()
    if raw_receipt.exists() or raw_receipt.is_symlink():
        raise ValueError("cleanup cannot overwrite an existing receipt")
    receipt = raw_receipt.parent.resolve(strict=True) / raw_receipt.name
    index_path = Path(index_path).resolve(strict=True)
    body = {key: value for key, value in preview.items() if key != "preview_sha256"}
    if _digest(body) != expected_preview_sha256 or preview.get("preview_sha256") != expected_preview_sha256:
        raise ValueError("cleanup preview digest mismatch")
    result = {"schema_version": 1, "kind": "cache-cleanup-result", "preview_sha256": expected_preview_sha256,
              "started_ns": time.time_ns(), "removed": [], "errors": [], "complete": False}
    with retention_lock(root):
        index = json.loads(index_path.read_text())
        fresh = preview_cleanup(root, index, cache_budget_bytes=preview["cache_budget_bytes"])
        if fresh != preview:
            raise ValueError("ownership index or cache membership changed after preview")
        for obj in index["objects"].values():
            path = _path(root, obj["path"])
            if receipt == path or path in receipt.parents:
                raise ValueError("cleanup receipt must live outside all retention objects")
            if index_path == path or path in index_path.parents:
                raise ValueError("ownership index must live outside all retention objects")
        if confirm_no_active_writers([row["object_id"] for row in fresh["removals"]]) is not True:
            raise ValueError("fresh process-owner confirmation required before cleanup")
        free_before = shutil.disk_usage(root).free
        try:
            for row in fresh["removals"]:
                path = _path(root, row["path"])
                if _inventory(path)["identity"] != row["identity"]:
                    raise ValueError("cache changed immediately before removal")
                shutil.rmtree(path)
                result["removed"].append(row)
                index["objects"][row["object_id"]]["removed_ns"] = time.time_ns()
                _write_index(index_path, index)
            result["complete"] = True
        except BaseException as error:
            result["errors"].append(repr(error))
            raise
        finally:
            result.update(finished_ns=time.time_ns(), free_bytes_before=free_before,
                          free_bytes_after=shutil.disk_usage(root).free)
            result["index_sha256_after"] = _digest(index)
            result["actual_free_bytes_change"] = result["free_bytes_after"] - free_before
            with receipt.open("x") as output:
                json.dump(result, output, indent=2)
                output.write("\n")
    return result


def register_object(store_root, object_id, *, path, kind, dependencies=(), pinned=False):
    """Publish a new exact root; replacement and dependency cycles are forbidden."""
    root = Path(store_root).resolve(strict=True)
    if not isinstance(object_id, str) or not object_id:
        raise ValueError("object ID must be a nonempty string")
    with retention_lock(root):
        index_path = root / "retention.json"
        if index_path.is_symlink():
            raise ValueError("retention index cannot be a symlink")
        index = json.loads(index_path.read_text()) if index_path.exists() else {
            "schema_version": 1, "objects": {}, "leases": []}
        validate_retention_index(index)
        if object_id in index["objects"]:
            raise ValueError("object ID already registered; immutable identity cannot be replaced")
        target = _path(root, path)
        if not target.is_dir():
            raise ValueError("registered object must be an existing directory")
        if not isinstance(dependencies, (list, tuple)) or len(dependencies) != len(set(dependencies)):
            raise ValueError("dependencies must be unique registered object IDs")
        for dependency in dependencies:
            if dependency not in index["objects"] or "removed_ns" in index["objects"][dependency]:
                raise ValueError("dependency must be a registered present object")
        for current in index["objects"].values():
            other = _path(root, current["path"])
            if target == other or target in other.parents or other in target.parents:
                raise ValueError("registered object roots overlap")
        if target == index_path or target in index_path.parents:
            raise ValueError("registered root cannot own retention metadata")
        if kind == "cargo-cache" and (not path.startswith("cache/cargo/") or len(PurePosixPath(path).parts) != 3):
            raise ValueError("compiler cache must be one configuration root")
        index["objects"][object_id] = {"path": path, "kind": kind, "dependencies": list(dependencies),
                                        "pinned": pinned, "last_used_ns": time.time_ns()}
        validate_retention_index(index)
        _write_index(index_path, index)
        return index["objects"][object_id]


def touch_object(store_root, object_id):
    """Record actual use without changing an object's identity or dependencies."""
    root = Path(store_root).resolve(strict=True)
    with retention_lock(root):
        path = root / "retention.json"
        if path.is_symlink():
            raise ValueError("retention index cannot be a symlink")
        index = json.loads(path.read_text())
        objects = validate_retention_index(index)
        if object_id not in objects or "removed_ns" in objects[object_id]:
            raise ValueError("cannot touch an unregistered or deleted object")
        if not _path(root, objects[object_id]["path"]).is_dir():
            raise ValueError("registered object is missing")
        objects[object_id]["last_used_ns"] = time.time_ns()
        _write_index(path, index)

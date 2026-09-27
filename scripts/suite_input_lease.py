"""In-process metadata custody between full suite admission and final validation.

This is not content certification: the enclosing suite must deeply validate
before using the snapshot and again before accepting its result.
"""
import json
import os
from pathlib import Path
import stat
import subprocess
import threading


def source_members(root):
    root = Path(root).absolute()
    raw = subprocess.check_output(['git', '-C', str(root), 'ls-files', '--cached',
                                   '--others', '--exclude-standard', '-z'])
    excluded = {'docs/oryn2/IMPLEMENTATION_STATUS.md', 'docs/oryn2/CAPABILITY_MILESTONES.md'}
    names = {os.fsdecode(name) for name in raw.split(b'\0') if name}
    return tuple(sorted(str(root / name) for name in names
                        if not name.startswith('benchmarks/evidence/') and name not in excluded
                        and (root / name).is_file()))


def workflow_members(root):
    # Keep the authoritative non-Git discovery rules, including ignored fixtures.
    # input_paths rechecks only the small immutable baseline contract digest.
    from workflow_input_lock import input_paths
    return tuple(str(Path(root).absolute() / name) for name in input_paths(root))


def fingerprint(path, cache=None):
    """Include each symlink-bearing parent, final object and resolved identity.

    Directory times are intentionally excluded: creating unrelated output files
    must not invalidate an admitted sibling. Explicit trees check membership.
    """
    path = Path(path).absolute()
    cache = {} if cache is None else cache
    links = []
    for item in (*reversed(path.parents), path):
        key = ("lstat", str(item))
        if key not in cache:
            try:
                cache[key] = item.lstat()
            except FileNotFoundError:
                # Evidence may retain historical, no-longer-present inputs.
                # Deep admission decides which references are required; keep
                # absent references stable too rather than silently dropping them.
                return ('absent', str(path), tuple(links))
        info = cache[key]
        if stat.S_ISLNK(info.st_mode):
            links.append((str(item), info.st_dev, info.st_ino, info.st_mode,
                          info.st_size, info.st_mtime_ns, info.st_ctime_ns, os.readlink(item)))
        elif item != path:
            links.append((str(item), info.st_dev, info.st_ino, info.st_mode))
    key = ('resolved-parent', str(path.parent))
    if key not in cache:
        cache[key] = path.parent.resolve(strict=True)
    try:
        target = path.resolve(strict=True) if stat.S_ISLNK(info.st_mode) else cache[key] / path.name
    except FileNotFoundError:
        return ('absent', str(path), tuple(links))
    info = target.stat() if stat.S_ISLNK(info.st_mode) else info
    if not stat.S_ISREG(info.st_mode):
        raise ValueError('suite input is not a regular file: ' + str(path))
    return (str(target), info.st_dev, info.st_ino, info.st_mode, info.st_size,
            info.st_mtime_ns, info.st_ctime_ns, tuple(links))


def tree_members(root):
    root = Path(root).absolute()
    if root.is_symlink() or not root.is_dir():
        raise ValueError('suite input tree is missing or a symlink: ' + str(root))
    result = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs:
            if (Path(directory) / name).is_symlink():
                raise ValueError('suite input tree contains directory symlink')
        result.extend(str(Path(directory) / name) for name in files)
    return tuple(sorted(result))


class MetadataSnapshot:
    def __init__(self, paths, *, membership=None, trees=()):
        self.lock = threading.RLock()
        self.failure = None
        self.membership = membership
        self.members = tuple(membership()) if membership else ()
        self.trees = {str(Path(path).absolute()): tree_members(path) for path in trees}
        names = set(str(Path(path).absolute()) for path in paths)
        names.update(self.members)
        for items in self.trees.values():
            names.update(items)
        cache = {}
        self.files = {name: fingerprint(name, cache) for name in sorted(names)}
        self.check()

    def check(self):
        with self.lock:
            if self.failure is not None:
                raise ValueError('suite input admission revoked: ' + self.failure)
            try:
                if self.membership and tuple(self.membership()) != self.members:
                    raise ValueError('suite source membership changed')
                for root, expected in self.trees.items():
                    if tree_members(root) != expected:
                        raise ValueError('suite artifact tree membership changed: ' + root)
                cache = {}
                for name, expected in self.files.items():
                    if fingerprint(name, cache) != expected:
                        raise ValueError('suite input metadata changed: ' + name)
            except Exception as error:
                self.failure = str(error)
                raise
        return self


def candidate_snapshot(candidate_path, gate_path, root):
    """Capture source/declared input closure before the deep candidate read.

    JSON reference traversal only discovers inputs; it grants no authority.
    Full load_candidate and generation validation must follow this capture.
    """
    root = Path(root).absolute()
    pending = [Path(candidate_path).absolute(), Path(gate_path).absolute()]
    paths, seen, trees, discovered = set(), set(), set(), {}
    candidate_before = fingerprint(pending[0])
    candidate = json.loads(pending[0].read_text())
    if fingerprint(pending[0]) != candidate_before:
        raise ValueError("suite candidate changed during discovery")
    discovered[str(pending[0])] = candidate_before
    records = candidate.get('artifacts', {})
    worker = records.get('worker', {}).get('path')
    if worker and Path(worker).parents[2].suffix == '.app':
        trees.add(Path(worker).parents[2])

    def references(value, base, relative=False):
        if isinstance(value, dict):
            if isinstance(value.get('path'), str) and isinstance(value.get('sha256'), str):
                path = Path(value['path'])
                # Gate/source record paths are repository-relative, artifact
                # and TLS records are relative to their containing manifest.
                if path.is_absolute() or relative:
                    pending.append(path if path.is_absolute() else base / path)
            for key, item in value.items():
                if key.endswith('_path') and isinstance(item, str) and Path(item).is_absolute():
                    if Path(item).is_file():
                        pending.append(Path(item))
                elif key == 'files' and isinstance(item, dict):
                    for name, digest in item.items():
                        if isinstance(name, str) and isinstance(digest, str):
                            path = base / name
                            if path.is_file():
                                pending.append(path)
                if key == 'evidence_artifacts' and isinstance(item, dict):
                    for name, expected in item.items():
                        if isinstance(expected, str) and Path(name).is_absolute():
                            pending.append(Path(name))
                references(item, base, relative)
        elif isinstance(value, list):
            for item in value:
                references(item, base, relative)

    while pending:
        path = pending.pop().absolute()
        if path in seen:
            continue
        seen.add(path)
        paths.add(path)
        before = fingerprint(path)
        if before[0] == 'absent':
            discovered[str(path)] = before
            continue
        with path.open('rb') as stream:
            prefix = stream.read(256).lstrip()
        if prefix.startswith((b'{', b'[')):
            try:
                value = json.loads(path.read_text())
            except (UnicodeError, ValueError):
                value = None
            repository_relative = (path == Path(gate_path).absolute()
                                   or isinstance(value, dict) and 'base_contract_sha256' in value)
            base = root if repository_relative else path.parent
            relative = repository_relative or isinstance(value, dict) and 'tls' in value
            references(value, base, relative)
            if path == Path(records.get('architecture_prebuilt', {}).get('path', '')):
                # Full architecture validation requires exact legacy tree membership.
                trees.add(Path(value['legacy_source_root']))
            if path == Path(records.get('framework_build', {}).get('path', '')):
                for build in value.get('builds', {}).values():
                    trees.add(path.parent / build['artifact'])
            if fingerprint(path) != before:
                raise ValueError('suite manifest changed during discovery: ' + str(path))
        if str(path) in discovered and discovered[str(path)] != before:
            raise ValueError('suite candidate changed after initial discovery')
        discovered[str(path)] = before
    snapshot = MetadataSnapshot(paths, membership=lambda: tuple(sorted(
        set(source_members(root)) | set(workflow_members(root)))), trees=trees)
    for name, before in discovered.items():
        if snapshot.files[name] != before:
            raise ValueError('suite input changed before snapshot admission: ' + name)
    return snapshot

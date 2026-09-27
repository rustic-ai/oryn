"""Pinned Git dataset exports with explicit non-test symlink normalization.

The source repository is read only. Runtime exports contain regular files and
Git object proof, never .git or dependence on a developer checkout. Only the
exact reviewed upstream documentation/example links below may be materialized.
"""
from __future__ import annotations

import base64
import hashlib
import os
from pathlib import Path, PurePosixPath
import stat

from . import builds
from .generation import _tree_objects, _write_new, new_store_path
from .inputs import _identity as stat_identity, resolve_input, snapshot_input
from .manifests import relative_path, validate_input
from .production import validate_git_export

PINNED_WPT_COMMIT = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
_BASE = 'tools/third_party/websockets/'
APPROVED_NORMALIZATIONS = {
    _BASE + 'docs/_static/favicon.ico': '../../logo/favicon.ico',
    _BASE + 'docs/_static/tidelift.png': '../../logo/tidelift.png',
    _BASE + 'docs/_static/websockets.svg': '../../logo/vertical.svg',
    _BASE + 'experiments/authentication/favicon.ico': '../../logo/favicon.ico',
    **{_BASE + f'example/tutorial/step{step}/{name}': target
       for step in (1, 2, 3) for name, target in {
           'connect4.css': '../start/connect4.css',
           'connect4.js': '../start/connect4.js',
           'connect4.py': '../start/connect4.py',
           'favicon.ico': '../../../logo/favicon.ico',
       }.items()},
}


def _target(path, target):
    relative_path(path)
    if APPROVED_NORMALIZATIONS.get(path) != target:
        raise ValueError('unreviewed dataset symlink path or target: ' + path)
    if not isinstance(target, str) or target.startswith('/') or '\\' in target or '\0' in target:
        raise ValueError('unsafe dataset symlink target')
    parts = list(PurePosixPath(path).parent.parts)
    for part in target.split('/'):
        if part == '..':
            if not parts:
                raise ValueError('dataset symlink escapes export')
            parts.pop()
        elif part not in ('', '.'):
            parts.append(part)
    result = '/'.join(parts)
    relative_path(result)
    return result


def verify_dataset(identity, source, membership, *, expected_commit=PINNED_WPT_COMMIT):
    """Verify a normalized WPT export, including original Git links and bytes.

    Production callers use the default reviewed revision. Tests or a separately
    reviewed future dataset may explicitly supply its full expected Git commit.
    """
    fields = {'schema_version', 'kind', 'dataset', 'commit', 'commit_object', 'trees', 'normalizations'}
    if (not isinstance(identity, dict) or set(identity) != fields
            or type(identity['schema_version']) is not int or identity['schema_version'] != 1
            or identity['kind'] != 'git-dataset-export' or identity['dataset'] != 'wpt'
            or identity['commit'] != expected_commit):
        raise ValueError('pinned WPT dataset identity differs')
    validate_input(membership)
    if membership['kind'] != 'directory':
        raise ValueError('dataset must declare exact directory membership')
    source = Path(source).absolute()
    # The public verifier verifies bytes itself, not merely asserted inventory.
    actual = snapshot_input(source.parent, source.name, kind='directory', scope=membership['scope'])
    actual['path'] = membership['path']
    if actual != membership:
        raise ValueError('dataset bytes, mode or membership changed')
    rows = identity['normalizations']
    if not isinstance(rows, list):
        raise ValueError('dataset normalizations must be a sorted explicit list')
    normalized, by_path = {}, {}
    keys = {'path', 'blob', 'target', 'resolved_target', 'sha256', 'size', 'executable'}
    for row in rows:
        if not isinstance(row, dict) or set(row) != keys:
            raise ValueError('dataset normalization schema differs')
        name = row['path']
        relative_path(name)
        if name in by_path or _target(name, row['target']) != row['resolved_target']:
            raise ValueError('duplicate or substituted dataset normalization')
        try:
            raw = base64.b64decode(row['blob'], validate=True)
        except (ValueError, TypeError) as error:
            raise ValueError('invalid dataset link blob encoding') from error
        if raw != row['target'].encode('utf-8'):
            raise ValueError('dataset link blob differs from declared target')
        validate_input({'scope': 'store', 'path': name, 'kind': 'file',
                        **{key: row[key] for key in ('sha256', 'size', 'executable')}})
        by_path[name] = row
        normalized[name] = {'blob': row['blob'], 'executable': row['executable']}
    if list(by_path) != sorted(by_path):
        raise ValueError('dataset normalizations must be sorted')
    git_identity = {key: identity[key] for key in ('schema_version', 'commit', 'commit_object', 'trees')}
    git_identity['kind'] = 'git-source-export'
    proof = validate_git_export(git_identity, source, membership, normalized_symlinks=normalized)
    for name, row in by_path.items():
        target = row['resolved_target']
        if target in by_path or target not in proof['tracked']:
            raise ValueError('dataset link target must be a committed regular file')
        materialized = {key: row[key] for key in ('sha256', 'size', 'executable')}
        if proof['tracked'][name] != materialized or proof['tracked'][target] != materialized:
            raise ValueError('materialized dataset link differs from committed target')
    return {**proof, 'dataset': 'wpt', 'normalizations': len(by_path)}


def _clean_commit(source, expected):
    commit = builds._git(source, 'rev-parse', 'HEAD').decode().strip()
    if commit != expected:
        raise ValueError('dataset source differs from pinned commit')
    status = builds._git(source, 'status', '--porcelain=v1', '-z', '--untracked-files=all',
                         '--ignore-submodules=none')
    if status:
        raise ValueError('dataset source must be clean before and after export')
    return commit


def _entries(source, commit):
    entries = {}
    for row in builds._git(source, 'ls-tree', '-r', '-z', commit).split(b'\0'):
        if not row:
            continue
        metadata, raw_name = row.split(b'\t', 1)
        mode, kind, oid = metadata.decode().split()
        name = raw_name.decode('utf-8')
        relative_path(name)
        if name in entries or kind != 'blob' or mode not in ('100644', '100755', '120000'):
            raise ValueError('unsupported dataset Git entry')
        entries[name] = (mode, oid)
    return entries


def _copy_blob(original, target, oid, mode, algorithm):
    before = original.lstat()
    if not stat.S_ISREG(before.st_mode) or bool(before.st_mode & 0o111) != (mode == '100755'):
        raise ValueError('dataset source type/mode differs: ' + str(original))
    digest = hashlib.new(algorithm)
    digest.update(b'blob ' + str(before.st_size).encode() + b'\0')
    target.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(original, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
    with os.fdopen(fd, 'rb') as incoming, target.open('xb') as outgoing:
        opened = os.fstat(incoming.fileno())
        if stat_identity(opened) != stat_identity(before):
            raise ValueError('dataset source changed before copying')
        while block := incoming.read(1024 * 1024):
            outgoing.write(block); digest.update(block)
        if stat_identity(os.fstat(incoming.fileno())) != stat_identity(before):
            raise ValueError('dataset source changed while copying')
    if stat_identity(original.lstat()) != stat_identity(before) or digest.hexdigest() != oid:
        raise ValueError('dataset source blob differs from committed Git object')
    target.chmod(0o555 if mode == '100755' else 0o444)


def export_dataset(source_root, store_root, *, export_path, identity_path,
                   expected_commit=PINNED_WPT_COMMIT):
    """Export tracked entries only into a new store directory; no runtime jobs."""
    source = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    if source == store or source in store.parents:
        raise ValueError('dataset exports must be outside the source repository')
    destination = new_store_path(store, export_path)
    identity_file = new_store_path(store, identity_path)
    if (destination.exists() or identity_file.exists() or destination == identity_file
            or destination in identity_file.parents or identity_file in destination.parents):
        raise ValueError('dataset export and identity need new separate destinations')
    commit = _clean_commit(source, expected_commit)
    entries = _entries(source, commit)
    algorithm = 'sha1' if len(commit) == 40 else 'sha256'
    normalizations = []
    for name, (mode, oid) in entries.items():
        if mode != '120000':
            continue
        link = source / name
        # Resolve the parent without ever traversing the link itself.
        resolve_input(source, str(PurePosixPath(name).parent))
        if not link.is_symlink():
            raise ValueError('committed dataset symlink missing')
        target = os.readlink(link)
        resolved = _target(name, target)
        if resolved not in entries or entries[resolved][0] not in ('100644', '100755'):
            raise ValueError('dataset symlink target is missing or not a regular committed file')
        raw = target.encode()
        if hashlib.new(algorithm, b'blob ' + str(len(raw)).encode() + b'\0' + raw).hexdigest() != oid:
            raise ValueError('dataset symlink source differs from committed blob')
        normalizations.append({'path': name, 'blob': base64.b64encode(raw).decode(),
                               'target': target, 'resolved_target': resolved})
    identity = {'schema_version': 1, 'kind': 'git-dataset-export', 'dataset': 'wpt',
                'commit': commit,
                'commit_object': base64.b64encode(builds._git(source, 'cat-file', 'commit', commit)).decode(),
                'trees': _tree_objects(source, commit), 'normalizations': normalizations}
    destination.mkdir(parents=True)
    for name, (mode, oid) in entries.items():
        if mode != '120000':
            _copy_blob(resolve_input(source, name), destination / name, oid, mode, algorithm)
    for row in normalizations:
        mode, oid = entries[row['resolved_target']]
        _copy_blob(resolve_input(destination, row['resolved_target']), destination / row['path'], oid, mode, algorithm)
        row.update(builds._identity(destination / row['path']))
    normalizations.sort(key=lambda row: row['path'])
    membership = snapshot_input(store, export_path, kind='directory')
    proof = verify_dataset(identity, destination, membership, expected_commit=expected_commit)
    _clean_commit(source, expected_commit)
    identity_file.parent.mkdir(parents=True, exist_ok=True)
    _write_new(identity_file, identity)
    identity_file.chmod(0o444)
    # Enumerate only verified directories; never recursively copy arbitrary data.
    for name in sorted(membership['directories'], reverse=True):
        (destination / name).chmod(0o555)
    destination.chmod(0o555)
    return {'schema_version': 1, 'dataset': 'wpt', 'commit': proof['commit'],
            'tree_sha256': proof['tree_sha256'], 'files': len(proof['tracked']),
            'normalizations': len(normalizations), 'wpt_checkout': membership,
            'upstream_identity': snapshot_input(store, identity_path),
            'independent_review_required': True}

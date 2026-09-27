"""Read-only verification of offline Cargo package content against locked sources."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import stat
import tarfile
import tomllib

from .inputs import resolve_input
from .manifests import manifest_digest
from .production import _hash, _require, _git_object


def _stat_state(value):
    return [value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns, value.st_mode]


def _state(path):
    return _stat_state(path.stat())


def _registry_population(root):
    """No-follow complete metadata snapshot, including empty directories."""
    files, directories, states = set(), set(), {}
    def visit(directory):
        before = directory.lstat()
        _require(stat.S_ISDIR(before.st_mode), 'Cargo registry directory is nonregular')
        name = directory.relative_to(root).as_posix()
        states['directory:' + name] = _stat_state(before)
        with os.scandir(directory) as entries:
            for entry in entries:
                path = Path(entry.path); relative = path.relative_to(root).as_posix()
                observed = entry.stat(follow_symlinks=False)
                if stat.S_ISDIR(observed.st_mode):
                    directories.add(relative); visit(path)
                else:
                    _require(stat.S_ISREG(observed.st_mode), 'Cargo registry contains nonregular source')
                    files.add(relative); states['file:' + relative] = _stat_state(observed)
        _require(_stat_state(before) == _stat_state(directory.lstat()), 'Cargo registry directory changed')
    visit(root)
    return files, directories, states


def _registry(package, locked, cargo_home):
    root = Path(package['manifest_path']).resolve(strict=True).parent
    initial = _registry_population(root)
    label = package['name'] + '-' + package['version']
    archives = [path for path in (cargo_home / 'registry/cache').glob('*/' + label + '.crate')
                if _hash(path) == locked['checksum']]
    _require(bool(archives), 'locked Cargo archive unavailable or checksum differs: ' + label)
    files, modes, directories, members = {}, {}, set(), set()
    archive_path = archives[0]
    archive_state = _state(archive_path)
    descriptor = os.open(archive_path, os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0))
    with os.fdopen(descriptor, 'rb') as stream:
        _require(_stat_state(os.fstat(stream.fileno())) == archive_state, 'Cargo archive changed before open')
        _require(hashlib.file_digest(stream, 'sha256').hexdigest() == locked['checksum'], 'Cargo archive checksum changed')
        stream.seek(0)
        with tarfile.open(fileobj=stream, mode='r:gz') as archive:
            for member in archive.getmembers():
                name = member.name.rstrip('/') if member.isdir() else member.name
                parts = name.split('/')
                _require(parts and parts[0] == label and all(part not in {'', '..', '.'} for part in parts)
                         and '\\' not in name, 'Cargo archive member escapes package')
                path = '/'.join(parts[1:])
                _require(path not in members, 'Cargo archive duplicates package member')
                members.add(path)
                for parent in PurePosixPath(path).parents:
                    if str(parent) != '.': directories.add(str(parent))
                if member.isdir():
                    if path: directories.add(path)
                    continue
                _require(member.isfile() and path, 'Cargo archive contains nonregular source')
                _require(path not in {'.cargo-ok', '.cargo-checksum.json'}, 'Cargo archive shadows cache metadata')
                _require(not (member.mode & 0o7000) and stat.S_IFMT(member.mode) in {0, stat.S_IFREG},
                         'Cargo archive has unsupported source mode')
                data = archive.extractfile(member).read()
                files[path] = hashlib.sha256(data).hexdigest()
                selected = resolve_input(root, path); before = _state(selected)
                # Cargo's standard extraction masks group/other write permission.
                # Old archives can carry the regular-file type bits in mode.
                modes[path] = stat.S_IMODE(member.mode) & ~0o022
                _require(stat.S_IMODE(selected.lstat().st_mode) == modes[path],
                         'Cargo registry source mode differs from locked archive: ' + path)
                _require(_hash(selected) == files[path] and before == _state(selected),
                         'Cargo registry source differs from locked archive: ' + path)
        _require(_stat_state(os.fstat(stream.fileno())) == archive_state and _state(archive_path) == archive_state,
                 'Cargo archive changed during verification')
    actual, actual_directories, states = initial
    _require(actual - {'.cargo-ok', '.cargo-checksum.json'} == set(files)
             and actual_directories == directories, 'Cargo registry source population differs')
    states = dict(states, archive=archive_state)
    for name in ('.cargo-ok', '.cargo-checksum.json'):
        # These Cargo metadata files are not package sources. Pin both their
        # absence and their actual bytes, including between separate probes.
        states['metadata:' + name] = _hash(root / name) if name in actual else None
    if '.cargo-checksum.json' in actual:
        def unique(pairs):
            result = {}
            for key, value in pairs:
                _require(key not in result, 'Cargo checksum metadata repeats a key')
                result[key] = value
            return result
        checksum = json.loads((root / '.cargo-checksum.json').read_text(), object_pairs_hook=unique)
        _require(isinstance(checksum, dict) and checksum.get('package') == locked['checksum']
                 and checksum.get('files') == files, 'Cargo checksum metadata differs from actual locked archive')
    _require(_registry_population(root) == initial, 'Cargo registry changed during verification')
    return {'kind': 'registry', 'package_checksum': locked['checksum'],
            'files_sha256': manifest_digest({'files': files, 'modes': modes, 'directories': sorted(directories)}),
            'observed_state_sha256': manifest_digest(states)}


def _git(package, locked):
    manifest = Path(package['manifest_path']).resolve(strict=True)
    environment = dict(os.environ, GIT_OPTIONAL_LOCKS='0')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(manifest.parent), *args], env=environment)
    root = Path(git('rev-parse', '--show-toplevel').decode().strip()).resolve(strict=True)
    expected = locked['source'].rsplit('#', 1)[-1]
    _require(git('rev-parse', 'HEAD').decode().strip() == expected, 'Cargo Git source revision differs')
    raw_commit = git('cat-file', 'commit', expected)
    _require(_git_object('commit', raw_commit, 'sha1') == expected, 'Cargo Git commit object corrupt')
    files, states = {}, {}
    root_tree = raw_commit.split(b'\n', 1)[0].split()[1].decode()
    _require(_git_object('tree', git('cat-file', 'tree', root_tree), 'sha1') == root_tree, 'Cargo Git root tree corrupt')
    for row in git('ls-tree', '-rtz', '--full-tree', expected).split(b'\0'):
        if not row:
            continue
        meta, raw_path = row.split(b'\t', 1); mode, kind, oid = meta.split()
        if kind == b'tree':
            _require(_git_object('tree', git('cat-file', 'tree', oid.decode()), 'sha1') == oid.decode(), 'Cargo Git tree corrupt')
            continue
        _require(kind == b'blob' and mode in {b'100644', b'100755'}, 'Cargo Git source has unsupported special input')
        path = raw_path.decode(); source = resolve_input(root, path)
        before = _state(source)
        _require(_git_object('blob', source.read_bytes(), 'sha1') == oid.decode()
                 and bool(source.stat().st_mode & 0o111) == (mode == b'100755') and before == _state(source),
                 'Cargo Git source content/mode differs: ' + path)
        files[path] = {'sha256': _hash(source), 'executable': mode == b'100755'}
        states[path] = before
    actual = {str(path.relative_to(root)) for path in root.rglob('*')
              if '.git' not in path.relative_to(root).parts and (path.is_file() or path.is_symlink())}
    _require(actual - {'.cargo-ok'} == set(files), 'Cargo Git source has uncommitted effective inputs')
    return {'kind': 'git', 'commit': expected, 'files_sha256': manifest_digest(files),
            'observed_state_sha256': manifest_digest(states)}


def observe_packages(metadata, source, cargo_home):
    source, cargo_home = Path(source), Path(cargo_home)
    lock = tomllib.loads((source / 'Cargo.lock').read_text())
    locked = {(row['name'], row['version'], row.get('source')): row for row in lock['package']}
    observations = {}
    for package in metadata['packages']:
        key = (package['name'], package['version'], package.get('source'))
        _require(key in locked, 'Cargo graph includes package outside lockfile: ' + repr(key))
        if package.get('source') is None:
            _require(Path(package['manifest_path']).resolve(strict=True).is_relative_to(source),
                     'Cargo local source escapes immutable export')
            observation = {'kind': 'local', 'manifest_path': str(Path(package['manifest_path']).resolve().relative_to(source)),
                           'manifest_sha256': _hash(package['manifest_path'])}
        elif package['source'].startswith('registry+'):
            observation = _registry(package, locked[key], cargo_home)
        elif package['source'].startswith('git+'):
            observation = _git(package, locked[key])
        else:
            raise ValueError('unsupported offline Cargo source')
        observations[package['id']] = observation
    _require(observations, 'empty Cargo metadata cannot prove dependency graphs')
    return observations


def validate_observations(metadata, observations, source):
    """Retained cache paths are diagnostics; rejoin package IDs to pinned lock."""
    source = Path(source)
    lock = tomllib.loads((source / 'Cargo.lock').read_text())
    locked = {(row['name'], row['version'], row.get('source')): row for row in lock['package']}
    _require(set(observations) == {row['id'] for row in metadata['packages']}, 'Cargo package observation population differs')
    for package in metadata['packages']:
        key = (package['name'], package['version'], package.get('source'))
        _require(key in locked, 'retained Cargo package outside lockfile')
        row = observations[package['id']]
        if package.get('source') is None:
            _require(set(row) == {'kind', 'manifest_path', 'manifest_sha256'} and row['kind'] == 'local'
                     and _hash(resolve_input(source, row['manifest_path'])) == row['manifest_sha256'], 'Cargo local package changed')
        elif package['source'].startswith('registry+'):
            _require(set(row) == {'kind', 'package_checksum', 'files_sha256', 'observed_state_sha256'} and row['kind'] == 'registry'
                     and row['package_checksum'] == locked[key]['checksum'], 'Cargo registry package differs from locked archive')
        else:
            _require(set(row) == {'kind', 'commit', 'files_sha256', 'observed_state_sha256'} and row['kind'] == 'git'
                     and row['commit'] == locked[key]['source'].rsplit('#', 1)[-1], 'Cargo Git package differs from locked commit')
        if 'files_sha256' in row:
            for key in ('files_sha256', 'observed_state_sha256'):
                _require(isinstance(row[key], str) and len(row[key]) == 64
                         and all(c in '0123456789abcdef' for c in row[key]), 'Cargo package file inventory missing')

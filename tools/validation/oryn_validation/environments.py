"""Offline Python runtime staging with exact dependency bytes and system observations.

A legacy environment may be a one-time migration input. It is never an input to
verification or execution of the resulting environment. No pip/network/build is
performed: a real interpreter, its stdlib and the locked dependency payload are
copied. The current Oryn plugin comes from the separately pinned runner export.
"""
from __future__ import annotations

import email
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from .generation import new_store_path, _write_new
from .inputs import _file, resolve_input, snapshot_input, verify_input
from .manifests import manifest_digest, validate_input
from .processes import ResourceLease, run_owned

DEFAULT_LOCK = Path(__file__).resolve().parents[1] / 'locks/wpt-python-3.14.json'
MACHO = {b'\xfe\xed\xfa\xce', b'\xce\xfa\xed\xfe', b'\xfe\xed\xfa\xcf',
         b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca', b'\xca\xfe\xba\xbf', b'\xbf\xba\xfe\xca'}
SYSTEM_PREFIXES = ('/opt/homebrew/', '/usr/lib/', '/System/Library/')
OS_PREFIXES = ('/usr/lib/', '/System/Library/')
PROBE = """import hashlib,importlib.metadata,json,os,platform,sys,sysconfig
import ssl,ctypes,zlib,sqlite3
print(json.dumps({'version':platform.python_version(),'system':platform.system(),
 'architecture':platform.machine(),'executable':os.path.realpath(sys.executable),
 'prefix':sys.prefix,'base_prefix':sys.base_prefix,'stdlib':sysconfig.get_path('stdlib'),
 'interpreter_sha256':hashlib.sha256(open(sys.executable,'rb').read()).hexdigest(),
 'lock_sha256':hashlib.sha256(open(os.path.join(sys.prefix,'dependency-lock.json'),'rb').read()).hexdigest(),
 'path':sys.path,'distributions':sorted([d.metadata['Name'],d.version]
 for d in importlib.metadata.distributions())},sort_keys=True))
"""


def _lock(path=None):
    path = DEFAULT_LOCK if path is None else Path(path)
    if _file(path) != _file(DEFAULT_LOCK):
        raise ValueError('dependency lock differs from the source-owned pinned lock')
    value = json.loads(path.read_text())
    if (value.get('kind') != 'wpt-python-dependency-lock' or value.get('schema_version') != 1
            or value.get('python_version') != '3.14.7'
            or value.get('plugin_policy') != 'runner-source-export'):
        raise ValueError('unsupported Python dependency lock')
    files = value.get('files', [])
    if not files or [r['path'] for r in files] != sorted({r['path'] for r in files}):
        raise ValueError('dependency lock paths must be nonempty, sorted and unique')
    for row in files:
        validate_input({'scope': 'store', 'kind': 'file', **row})
        if _excluded(row['path']) or row['path'].endswith('.pth'):
            raise ValueError('startup hook or excluded plugin/cache in dependency lock')
    return value


def _excluded(name):
    return ('__pycache__' in Path(name).parts or name.endswith('.pyc')
            or name.startswith(('oryn_wptrunner/', 'oryn_wptrunner-')))


def _payload(root, *, stdlib=False):
    """Exact membership; the only skipped stdlib link is global site-packages."""
    root = Path(root).absolute()
    resolve_input(root.parent, root.name)
    rows = []
    def visit(directory):
        for path in sorted(directory.iterdir()):
            name = path.relative_to(root).as_posix()
            if (stdlib and name == 'site-packages') or _excluded(name):
                continue
            if path.is_symlink():
                raise ValueError('Python payload has a symlink: ' + name)
            if path.is_dir():
                visit(path)
            else:
                rows.append({'path': name, **_file(path)})
    visit(root)
    return sorted(rows, key=lambda r: r['path'])


def _dependencies(root, lock, *, allow_plugin=False):
    if not allow_plugin and any(p.name == 'oryn_wptrunner' or p.name.startswith('oryn_wptrunner-')
                                for p in Path(root).iterdir()):
        raise ValueError('installed Oryn plugin is forbidden in staged runtime')
    if _payload(root) != lock['files']:
        raise ValueError('dependency payload differs from exact locked membership/bytes')
    index = {r['path']: r for r in lock['files']}
    observed = []
    for path in sorted(Path(root).glob('*.dist-info/METADATA')):
        if path.parent.name.startswith('oryn_wptrunner-'):
            continue
        data = email.message_from_bytes(path.read_bytes())
        observed.append((data['Name'], data['Version'], path.relative_to(root).as_posix()))
    expected = [(r['name'], r['version'], r['metadata']) for r in lock['distributions']]
    if sorted(observed) != sorted(expected):
        raise ValueError('dependency distribution versions or membership differ')
    for row in lock['distributions']:
        if row['metadata'] not in index or row['wheel_metadata'] not in index:
            raise ValueError('dependency METADATA/WHEEL identity absent')


def _copy(root, target, rows):
    for row in rows:
        source = resolve_input(root, row['path'])
        expected = {k: row[k] for k in ('sha256', 'size', 'executable')}
        if _file(source) != expected:
            raise ValueError('Python source changed before copy')
        destination = target / row['path']
        destination.parent.mkdir(parents=True, exist_ok=True)
        resolve_input(destination.parent.parent, destination.parent.name)
        # Never follow a concurrently substituted source leaf or destination leaf.
        fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as incoming:
            fd_out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd_out, 'wb') as outgoing:
                shutil.copyfileobj(incoming, outgoing)
        destination.chmod(0o555 if row['executable'] else 0o444)
        if _file(destination) != expected or _file(source) != expected:
            raise ValueError('Python source changed while copying')


def _otool(path, flag):
    return subprocess.check_output(['/usr/bin/otool', flag, str(path)], text=True,
                                   stderr=subprocess.PIPE, timeout=15)


def _macho(path):
    with path.open('rb') as stream:
        return stream.read(4) in MACHO


def _system_libraries(root):
    """Observe transitive Mach-O dependencies; OS shared-cache names stay explicit.

    Homebrew libraries are byte-pinned and their symlink resolutions rechecked.
    System shared-cache libraries are tied to the observed macOS build, never
    represented as copied or individually hash-certified files.
    """
    root = Path(root).absolute()
    pending = [p for p in root.rglob('*') if p.is_file() and _macho(p)]
    seen, external, os_names = set(), {}, set()
    while pending:
        path = pending.pop()
        resolved = path.resolve(strict=True)
        if str(resolved) in seen:
            continue
        seen.add(str(resolved))
        loads = _otool(path, '-L').splitlines()[1:]
        rpaths = re.findall(r'\n\s*cmd LC_RPATH\n\s*cmdsize \d+\n\s*path (.*?) \(offset',
                           _otool(path, '-l'))
        for line in loads:
            if ' (compatibility version ' not in line:
                continue
            name = line.strip().split(' (compatibility version ', 1)[0]
            if Path(name).is_absolute() and ('..' in Path(name).parts or str(Path(name)) != name):
                raise ValueError('noncanonical Python library dependency: ' + name)
            if name.startswith(OS_PREFIXES):
                os_names.add(name)
                continue
            replacements = {'@loader_path': str(path.parent), '@executable_path': str(root / 'bin')}
            for prefix, value in replacements.items():
                if name.startswith(prefix + '/'):
                    name = value + name[len(prefix):]
            if name.startswith('@rpath/'):
                choices = []
                for base in rpaths:
                    for prefix, value in replacements.items():
                        base = base.replace(prefix, value)
                    candidate = Path(base) / name[len('@rpath/'):]
                    if candidate.is_file():
                        choices.append(candidate)
                if len({str(p.resolve()) for p in choices}) != 1:
                    raise ValueError('unresolved or ambiguous Python library rpath: ' + name)
                name = str(choices[0])
            if not Path(name).is_absolute():
                raise ValueError('relative Python library dependency: ' + name)
            dependency = Path(name)
            resolved_dependency = dependency.resolve(strict=True)
            if resolved_dependency.is_relative_to(root):
                resolve_input(root, resolved_dependency.relative_to(root).as_posix())
            else:
                if not name.startswith(SYSTEM_PREFIXES) or not str(resolved_dependency).startswith(SYSTEM_PREFIXES):
                    raise ValueError('Python library dependency is outside explicit system roots: ' + name)
                external[name] = {'selected_path': name, 'resolved_path': str(resolved_dependency),
                                  **_file(resolved_dependency)}
            pending.append(resolved_dependency)
    return {'libraries': [external[k] for k in sorted(external)], 'os_shared_cache_names': sorted(os_names),
            'macos_build': subprocess.check_output(['/usr/bin/sw_vers', '-buildVersion'],
                                                   text=True, timeout=15).strip()}


def _clean(result):
    return (type(result.get('returncode')) is int and result['returncode'] == 0 and result.get('error') is None
            and result.get('cleanup_complete') is True and result.get('cleanup_errors') == []
            and result.get('timed_out') is False and result.get('forced_cleanup') is False
            and result.get('cancellation_signal') is None)


def _runtime_env(root):
    return {'PYTHONHOME': str(root), 'PYTHONNOUSERSITE': '1', 'PYTHONDONTWRITEBYTECODE': '1'}


def _validate_probe(value, root, lock):
    root = Path(root).absolute()
    if (value.get('version') != lock['python_version'] or value.get('system') != 'Darwin'
            or value.get('architecture') not in ('arm64', 'aarch64')
            or value.get('executable') != str(root / 'bin/python')
            or value.get('prefix') != str(root) or value.get('base_prefix') != str(root)
            or value.get('stdlib') != str(root / 'lib/python3.14')
            or value.get('distributions') != sorted([r['name'], r['version']] for r in lock['distributions'])):
        raise ValueError('actual staged Python runtime/version/dependencies differ')
    for path in value['path']:
        if path and not Path(path).is_relative_to(root):
            raise ValueError('actual Python sys.path escapes staged environment')


def prepare_environment(donor_site_packages, system_python, stdlib, store_root, *,
                        environment_path, identity_path, diagnostics_path, lock_path=None):
    """Stage a fresh runtime, then run only a bounded owned Python identity probe.

    No installation, package build, dependency resolution or network operation.
    This is not WPT qualification. Failed output is retained and never reused.
    """
    lock = _lock(lock_path)
    store = Path(store_root).absolute()
    destination = new_store_path(store, environment_path)
    identity = new_store_path(store, identity_path)
    diagnostics = new_store_path(store, diagnostics_path)
    paths = [destination, identity, diagnostics]
    if (len(set(paths)) != 3 or any(p.exists() for p in paths)
            or any(a in b.parents for a in paths for b in paths if a != b)):
        raise ValueError('environment, identity and diagnostics require fresh separate paths')
    donor, stdlib = Path(donor_site_packages).absolute(), Path(stdlib).absolute()
    python = Path(system_python).absolute()
    resolve_input(python.parent, python.name)
    for source in (donor, stdlib, python):
        if any(p == source or p in source.parents or source in p.parents for p in paths):
            raise ValueError('environment outputs overlap preparation sources')
    _dependencies(donor, lock, allow_plugin=True)
    stdlib_rows = _payload(stdlib, stdlib=True)
    if not {'os.py', 'json/__init__.py', 'sysconfig/__init__.py'} <= {r['path'] for r in stdlib_rows}:
        raise ValueError('complete Python stdlib required')
    interpreter = _file(python)
    if not interpreter['executable']:
        raise ValueError('real Python interpreter must be executable')
    with ResourceLease(store, ['environment:' + environment_path]) as lease:
        destination.mkdir(parents=True)
        _copy(python.parent, destination / 'bin', [{'path': python.name, **interpreter}])
        (destination / 'bin' / python.name).rename(destination / 'bin/python')
        _copy(stdlib, destination / 'lib/python3.14', stdlib_rows)
        _copy(donor, destination / 'lib/python3.14/site-packages', lock['files'])
        _write_new(destination / 'dependency-lock.json', lock)
        system = _system_libraries(destination)
        before_probe = snapshot_input(store, environment_path, kind='directory')
        environment = {'PATH': '/usr/bin:/bin', **_runtime_env(destination)}
        argv = [str(destination / 'bin/python'), '-B', '-s', '-c', PROBE]
        result = run_owned(argv, cwd=destination, output=diagnostics,
                           env=environment, timeout_seconds=120, grace_seconds=15)
        lease.mark_settled(result)
        if not _clean(result):
            raise ValueError('Python environment probe failed or did not settle')
        actual = json.loads((diagnostics / 'stdout.log').read_text())
        _validate_probe(actual, destination, lock)
        verify_input(before_probe, store_root=store)
        _dependencies(donor, lock, allow_plugin=True)
        if _payload(stdlib, stdlib=True) != stdlib_rows or _file(python) != interpreter:
            raise ValueError('system Python source changed while staging')
        if system != _system_libraries(destination):
            raise ValueError('system Python libraries changed during probe')
        for p in sorted(destination.rglob('*'), reverse=True):
            if p.is_dir():
                p.chmod(0o555)
        destination.chmod(0o555)
        membership = snapshot_input(store, environment_path, kind='directory')
        value = {'schema_version': 1, 'kind': 'staged-wpt-python-environment',
                 'python_version': lock['python_version'], 'environment': membership,
                 'python': snapshot_input(store, environment_path + '/bin/python'),
                 'dependency_lock': snapshot_input(store, environment_path + '/dependency-lock.json'),
                 'system': system, 'stdlib_payload_sha256': manifest_digest(stdlib_rows),
                 'probe': snapshot_input(store, diagnostics_path, kind='directory'),
                 'plugin_policy': 'runner-source-export'}
        identity.parent.mkdir(parents=True, exist_ok=True)
        _write_new(identity, value)
        identity.chmod(0o444)
        verify_environment(value, store)
    return {'python': value['python'], 'python_environment': membership,
            'python_environment_identity': snapshot_input(store, identity_path),
            'python_dependency_lock': value['dependency_lock'], 'probe': value['probe'],
            'independent_review_required': True, 'qualified': False}


def verify_environment(identity, store_root):
    keys = {'schema_version', 'kind', 'python_version', 'environment', 'python', 'dependency_lock',
            'system', 'stdlib_payload_sha256', 'probe', 'plugin_policy'}
    if (not isinstance(identity, dict) or set(identity) != keys or identity['schema_version'] != 1
            or identity['kind'] != 'staged-wpt-python-environment'
            or identity['plugin_policy'] != 'runner-source-export'):
        raise ValueError('Python environment identity schema differs')
    for key in ('environment', 'python', 'dependency_lock', 'probe'):
        if identity[key].get('scope') != 'store' or identity[key].get('kind') != (
                'directory' if key in ('environment', 'probe') else 'file'):
            raise ValueError('Python environment asset scope/kind differs')
        verify_input(identity[key], store_root=store_root)
    root = resolve_input(store_root, identity['environment']['path'])
    if (identity['environment']['kind'] != 'directory'
            or identity['python']['path'] != identity['environment']['path'] + '/bin/python'
            or identity['dependency_lock']['path'] != identity['environment']['path'] + '/dependency-lock.json'):
        raise ValueError('Python environment asset paths differ')
    lock = _lock(root / 'dependency-lock.json')
    if identity['python_version'] != lock['python_version']:
        raise ValueError('Python version differs from dependency lock')
    _dependencies(root / 'lib/python3.14/site-packages', lock)
    if manifest_digest(_payload(root / 'lib/python3.14', stdlib=True)) != identity['stdlib_payload_sha256']:
        raise ValueError('staged stdlib content differs')
    if _system_libraries(root) != identity['system']:
        raise ValueError('observed system Python dependency changed')
    diagnostics = resolve_input(store_root, identity['probe']['path'])
    process = json.loads((diagnostics / 'process-result.json').read_text())
    preflight = json.loads((diagnostics / 'preflight.json').read_text())
    actual = json.loads((diagnostics / 'stdout.log').read_text())
    # Preserve original execution paths as evidence; relocation verifies identity,
    # then maps only that original environment prefix to this current root.
    original = Path(actual['prefix'])
    argv = [str(original / 'bin/python'), '-B', '-s', '-c', PROBE]
    birth = preflight.get('child_identity')
    if (not isinstance(birth, list) or len(birth) != 2
            or not isinstance(birth[0], list) or len(birth[0]) != 2
            or any(type(x) is not int for x in birth[0]) or birth[0][0] <= 0
            or not 0 <= birth[0][1] < 1_000_000
            or type(preflight.get('owner_pid')) is not int or preflight['owner_pid'] <= 0
            or birth[1] != preflight['owner_pid']
            or type(preflight.get('child_pid')) is not int or preflight['child_pid'] <= 0):
        raise ValueError('Python probe child/owner kernel birth custody missing')
    if (process.get('argv') != argv or not _clean(process)
            or preflight.get('argv') != argv or preflight.get('cwd') != str(original)
            or preflight.get('timeout_seconds') != 120
            or actual.get('interpreter_sha256') != identity['python']['sha256']
            or actual.get('lock_sha256') != identity['dependency_lock']['sha256']):
        raise ValueError('Python probe ownership or exact command differs')
    _validate_probe(actual, original, lock)
    return {'valid': True, 'python_version': lock['python_version'],
            'distributions': len(lock['distributions']), 'qualified': False}


def runtime_environment(identity, store_root):
    verify_environment(identity, store_root)
    return _runtime_env(resolve_input(store_root, identity['environment']['path']))

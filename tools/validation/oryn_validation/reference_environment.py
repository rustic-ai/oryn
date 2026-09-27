"""Exact signed reference Chrome app custody, including internal framework links.

Links are confined to this domain contract. Generic Input declarations continue
rejecting links; every physical app file and the driver are ordinary file Inputs.
No browser execution, signing, downloading or probing occurs here.
"""
from __future__ import annotations

import copy
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import subprocess

from .inputs import InputLease, _file, _identity, resolve_input, snapshot_input, verify_inputs
from .manifests import manifest_digest, relative_path, validate_input
from .generation import new_store_path, _write_new

KIND = 'reference-browser-environment'


def _require(value, message):
    if not value:
        raise ValueError(message)


def _safe_root(path):
    path = Path(path).absolute()
    resolve_input(path.parent, path.name)
    _require(stat.S_ISDIR(path.lstat().st_mode), 'reference app must be a physical directory')
    return path


def _target(root, name, target):
    _require(isinstance(target, str) and target and not target.startswith('/') and '\\' not in target
             and '\x00' not in target, 'reference link requires an explicit relative target')
    lexical = os.path.normpath(str(PurePosixPath(name).parent / target))
    _require(lexical != '..' and not lexical.startswith('../'), 'reference link escapes app')
    try:
        resolved = (root / name).resolve(strict=True)
    except (OSError, RuntimeError) as error:
        raise ValueError('reference link is dangling or cyclic: ' + name) from error
    _require(resolved.is_relative_to(root) and (resolved.is_file() or resolved.is_dir()),
             'reference link resolves outside app or to special file')


def _scan(root, *, full):
    root = _safe_root(root)
    files, directories, symlinks, states = [], [], [], []
    def visit(path, prefix=''):
        before = path.lstat()
        _require(stat.S_ISDIR(before.st_mode), 'reference directory changed type')
        states.append((prefix, _identity(before)))
        with os.scandir(path) as entries:
            for item in entries:
                name = prefix + item.name
                relative_path(name)
                entry = Path(item.path); observed = item.stat(follow_symlinks=False)
                mode = observed.st_mode
                if stat.S_ISDIR(mode):
                    directories.append({'path': name, 'mode': stat.S_IMODE(mode)})
                    visit(entry, name + '/')
                elif stat.S_ISREG(mode):
                    row = {'path': name, 'mode': stat.S_IMODE(mode)}
                    if full:
                        row.update(_file(entry))
                    files.append(row); states.append((name, _identity(observed)))
                    _require(_identity(observed) == _identity(entry.lstat()), 'reference file changed during scan')
                elif stat.S_ISLNK(mode):
                    target = os.readlink(entry)
                    symlinks.append({'path': name, 'target': target})
                    states.append((name, _identity(observed), target))
                    _require(_identity(observed) == _identity(entry.lstat()), 'reference link changed during scan')
                else:
                    raise ValueError('special file in reference app: ' + name)
        _require(_identity(before) == _identity(path.lstat()), 'reference directory changed during scan')
    visit(root)
    for row in symlinks:
        _target(root, row['path'], row['target'])
    return {'mode': stat.S_IMODE(root.lstat().st_mode),
            'files': sorted(files, key=lambda r: r['path']),
            'directories': sorted(directories, key=lambda r: r['path']),
            'symlinks': sorted(symlinks, key=lambda r: r['path'])}, sorted(states)


def _signature(path, *, app, publisher_archive=False):
    command = ['/usr/bin/codesign', '--verify', '--strict']
    if app:
        command.append('--deep')
    checked = subprocess.run([*command, str(path)], check=False, capture_output=True, text=True, timeout=60)
    verified = checked.returncode == 0
    missing = 'code has no resources but signature indicates they must be present'
    if not verified and not (app and publisher_archive and checked.returncode == 1
                             and checked.stderr.strip() == str(path) + ': ' + missing):
        raise ValueError('reference signature verification failed: ' + checked.stderr.strip())
    raw = subprocess.run(['/usr/bin/codesign', '-d', '--verbose=4', str(path)],
                         check=True, capture_output=True, text=True, timeout=30).stderr
    def one(key):
        values = [line[len(key) + 1:] for line in raw.splitlines() if line.startswith(key + '=')]
        _require(len(values) == 1 and values[0], 'missing/ambiguous reference signature field: ' + key)
        return values[0]
    result = {'identifier': one('Identifier'), 'team': one('TeamIdentifier'), 'cdhash': one('CDHash'),
              'authorities': [line.split('=', 1)[1] for line in raw.splitlines() if line.startswith('Authority=')]}
    _require(re.fullmatch('[a-fA-F0-9]{40,64}', result['cdhash']), 'invalid reference signature CDHash')
    result['verified'] = verified
    if not verified:result['failure'] = missing
    return result


def _native_libraries(app, driver, browser, files):
    """All local Mach-O operands resolve inside declared app bytes; OS names
    remain explicit shared-cache observations tied to the current macOS build.
    """
    from .environments import _macho, _otool, _dylib_loads, OS_PREFIXES
    physical = {str(app / row['path']) for row in files}
    native = [app / row['path'] for row in files if _macho(app / row['path'])]
    if _macho(driver):
        native.append(driver)
    # -m disables archive(member) parsing for Chrome helper names such as
    # "Google Chrome for Testing Helper (Alerts)".
    commands = {str(path): _otool(path, '-mhl') for path in native}
    def rpaths(raw):
        return re.findall(r'\n\s*cmd LC_RPATH\n\s*cmdsize \d+\n\s*path (.*?) \(offset', raw)
    browser_rpaths = rpaths(commands.get(str(browser), ''))
    os_names, edges = set(), []
    for path in native:
        label = 'driver' if path == driver else path.relative_to(app).as_posix()
        raw = commands[str(path)]
        header_types = {int(line.split()[4]) for line in raw.splitlines()
                        if re.match(r'^\s*0x[0-9a-fA-F]+\s+\d+\s+\d+\s+0x', line)}
        _require(len(header_types) == 1 and header_types <= {2, 6, 8}, 'reference Mach-O filetype is missing or ambiguous')
        executable_dir = path.parent if header_types == {2} else browser.parent
        def expand(value, loader_dir=path.parent, executable=executable_dir):
            return value.replace('@loader_path', str(loader_dir)).replace('@executable_path', str(executable))
        for name in _dylib_loads(raw):
            if name.startswith(OS_PREFIXES):
                _require('..' not in Path(name).parts and str(Path(name)) == name, 'noncanonical reference OS dependency')
                os_names.add(name); continue
            if name.startswith('@rpath/'):
                candidates = []
                # Inherited LC_RPATH tokens retain their declaring image.
                search = [(base, path.parent, executable_dir) for base in rpaths(raw)]
                search += [(base, browser.parent, browser.parent) for base in browser_rpaths]
                for base, loader, executable in search:
                    candidate = Path(expand(base, loader, executable)) / name[len('@rpath/'):]
                    if candidate.is_file():
                        candidates.append(candidate.resolve(strict=True))
                _require(len(set(candidates)) == 1, 'unresolved/ambiguous reference rpath: ' + name)
                target = candidates[0]
            else:
                operand = expand(name)
                _require(Path(operand).is_absolute(), 'relative reference library operand')
                target = Path(operand).resolve(strict=True)
            _require(str(target) in physical, 'reference library outside declared app: ' + name)
            edges.append({'from': label, 'to': target.relative_to(app).as_posix(), 'load_name': name})
    return {'macos_build': subprocess.check_output(['/usr/bin/sw_vers', '-buildVersion'], text=True, timeout=15).strip(),
            'os_shared_cache_names': sorted(os_names),
            'internal_loads': sorted(edges, key=lambda r: (r['from'], r['to'], r['load_name']))}


def _schema(identity):
    _require(set(identity) == {'schema_version', 'kind', 'app', 'browser', 'driver', 'signatures', 'system', 'authentication'},
             'invalid reference environment fields')
    _require(identity['schema_version'] == 1 and identity['kind'] == KIND, 'unsupported reference environment')
    app = identity['app']
    _require(set(app) == {'path', 'mode', 'directories', 'files', 'symlinks', 'file_modes'}, 'invalid reference app fields')
    relative_path(app['path'])
    _require(app['path'].endswith('.app'), 'reference root must be an app bundle')
    def mode(value):
        _require(type(value) is int and 0 <= value <= 0o777, 'unsupported reference mode')
    mode(app['mode'])
    names = set()
    for category, keys in [('directories', {'path', 'mode'}), ('symlinks', {'path', 'target'})]:
        rows = app[category]
        _require(isinstance(rows, list) and [r['path'] for r in rows] == sorted({r['path'] for r in rows}),
                 'reference membership must be sorted and unique')
        for row in rows:
            _require(set(row) == keys, 'invalid reference membership row')
            relative_path(row['path']); _require(row['path'] not in names, 'overlapping reference membership')
            names.add(row['path'])
            if category == 'directories':mode(row['mode'])
    _require(isinstance(app['files'], list) and app['files'], 'reference physical file population required')
    _require([r['path'] for r in app['files']] == sorted({r['path'] for r in app['files']}), 'reference files must be sorted and unique')
    file_names = set()
    for row in app['files']:
        validate_input(row)
        _require(row['scope'] == 'store' and row['kind'] == 'file' and row['path'].startswith(app['path'] + '/'),
                 'reference physical file outside app')
        name = row['path'][len(app['path']) + 1:]
        _require(name not in names, 'overlapping reference membership'); names.add(name); file_names.add(name)
    _require(set(app['file_modes']) == file_names, 'reference file modes differ from membership')
    for value in app['file_modes'].values():mode(value)
    for role in ('browser', 'driver'):
        validate_input(identity[role])
        _require(identity[role]['scope'] == 'store' and identity[role]['kind'] == 'file'
                 and identity[role]['executable'], 'reference executable must be a physical store file')
    _require(identity['browser'] in app['files'], 'reference browser outside declared app files')
    _require(not identity['driver']['path'].startswith(app['path'] + '/') and identity['driver']['path'] != app['path'],
             'reference driver must be separately declared')
    _require(set(identity['signatures']) == {'app', 'driver'}, 'reference signature roles differ')
    authentication = identity['authentication']
    _require(isinstance(authentication, dict), 'reference authentication policy required')
    if authentication.get('method') == 'publisher-archive':
        from .reference_archive import PACKAGE_SHA256
        _require(set(authentication) == {'method', 'archive', 'members_sha256'}, 'invalid reference archive policy')
        archive = authentication['archive']; validate_input(archive)
        _require(archive['scope'] == 'store' and archive['kind'] == 'file' and archive['sha256'] == PACKAGE_SHA256,
                 'reference archive differs from pinned publisher package')
    else:
        _require(authentication == {'method': 'codesign'}, 'unknown reference authentication policy')
    return identity


def physical_inputs(identity):
    _schema(identity)
    records = [*identity['app']['files'], identity['driver']]
    if identity['authentication']['method'] == 'publisher-archive':records.append(identity['authentication']['archive'])
    return copy.deepcopy(records)


def _browser(app):
    metadata = resolve_input(app, 'Contents/Info.plist')
    plist = plistlib.loads(metadata.read_bytes())
    executable = plist.get('CFBundleExecutable')
    _require(isinstance(executable, str) and executable and '/' not in executable
             and executable not in ('.', '..'), 'invalid reference executable')
    browser = resolve_input(app, 'Contents/MacOS/' + executable)
    _require(_file(browser)['executable'], 'reference browser is not executable')
    return browser


def _verify(identity, store_root, *, full):
    _schema(identity)
    app = resolve_input(store_root, identity['app']['path'])
    driver = resolve_input(store_root, identity['driver']['path'])
    browser = resolve_input(store_root, identity['browser']['path'])
    _require(browser == _browser(app), 'reference browser differs from CFBundleExecutable')
    observed, states = _scan(app, full=full)
    expected = identity['app']
    _require(observed['mode'] == expected['mode'] and observed['directories'] == expected['directories']
             and observed['symlinks'] == expected['symlinks'], 'reference directory/link membership changed')
    actual = []
    for row in observed['files']:
        name = row['path']
        _require(row['mode'] == expected['file_modes'].get(name), 'reference physical file mode changed')
        actual.append({'scope': 'store', 'kind': 'file', 'path': expected['path'] + '/' + name,
                       **({key: row[key] for key in ('sha256', 'size', 'executable')} if full else {})})
    if full:
        _require(actual == expected['files'], 'reference physical file bytes/population changed')
        verify_inputs([identity['driver']], store_root=store_root)
        publisher = identity['authentication']['method'] == 'publisher-archive'
        if publisher:
            from .reference_archive import verify_archive
            archive = identity['authentication']['archive']
            proof = verify_archive(resolve_input(store_root, archive['path']))
            _require(proof['app'] == observed and proof['members_sha256'] == identity['authentication']['members_sha256'],
                     'reference app differs from exact publisher archive')
        _require(_signature(app, app=True, **({'publisher_archive': True} if publisher else {})) == identity['signatures']['app'],
                 'reference app signature observation changed')
        _require(_signature(driver, app=False) == identity['signatures']['driver'], 'reference driver signature changed')
        _require(_native_libraries(app, driver, browser, observed['files']) == identity['system'], 'reference system/load graph changed')
    else:
        _require([r['path'] for r in actual] == [r['path'] for r in expected['files']], 'reference file population changed')
    return states


class ReferenceLease:
    def __init__(self, identity, store_root):
        self.identity = copy.deepcopy(identity); self.store_root = Path(store_root).absolute()
        self.inputs = InputLease(physical_inputs(self.identity), store_root=self.store_root)
        self.admission = self.finalization = self.final_error = self._baseline = None

    def _receipt(self, inputs):
        return {'identity_sha256': manifest_digest(self.identity),
                'inputs_sha256': manifest_digest(self.inputs.records), 'inputs': inputs,
                'signatures': copy.deepcopy(self.identity['signatures']),
                'system': copy.deepcopy(self.identity['system'])}

    def __enter__(self):
        _require(self._baseline is None, 'reference lease cannot be entered twice')
        before = _verify(self.identity, self.store_root, full=False)
        self.inputs.__enter__()
        _verify(self.identity, self.store_root, full=True)
        self._baseline = _verify(self.identity, self.store_root, full=False)
        _require(before == self._baseline, 'reference app changed during admission')
        self.admission = self._receipt(self.inputs.admission)
        return self

    def checkpoint(self):
        _require(self._baseline is not None and self.finalization is None and self.final_error is None, 'reference lease is not active')
        try:
            self.inputs.checkpoint()
            _require(_verify(self.identity, self.store_root, full=False) == self._baseline,
                     'reference app metadata changed during execution')
        except BaseException as error:
            self.final_error = error
            raise

    def finalize(self):
        _require(self._baseline is not None, 'reference lease was not admitted')
        self.finalization = None
        errors = [self.final_error] if self.final_error is not None else []
        # Never skip full bytes/signatures/system observations just because a
        # checkpoint or the caller already failed. Preserve the first error.
        for check in (lambda: _verify(self.identity, self.store_root, full=True),
                      self.inputs.finalize,
                      lambda: _require(_verify(self.identity, self.store_root, full=False) == self._baseline,
                                       'reference app metadata changed during execution')):
            try:check()
            except BaseException as error:errors.append(error)
        if errors:
            self.final_error = errors[0]
            raise self.final_error
        self.finalization = self._receipt(self.inputs.finalization)
        return self.finalization

    def __exit__(self, exc_type, exc, traceback):
        try:self.finalize()
        except BaseException as error:
            if exc is None:raise
            if hasattr(exc, 'add_note'):exc.add_note('Reference environment finalization also failed: ' + str(error))
        return False


def verify_reference_environment(identity, store_root):
    with ReferenceLease(identity, store_root) as lease:
        pass
    return lease.finalization


def stage_reference_environment(source_app, source_driver, store_root, output_directory, *, source_archive=None):
    """Preserve app bytes, mode and link spellings; never re-sign or launch it."""
    app = _safe_root(source_app); driver = Path(source_driver).absolute()
    resolve_input(driver.parent, driver.name)
    store = Path(store_root).resolve(strict=True)
    target = new_store_path(store, output_directory)
    _require(not target.exists() and not target.is_relative_to(app), 'reference staging requires new external directory')
    observed, before = _scan(app, full=True)
    driver_identity = _file(driver)
    _require(driver_identity['executable'], 'reference driver is not executable')
    browser = _browser(app)
    executable = browser.name
    archive_proof = None
    if source_archive is not None:
        from .reference_archive import verify_archive
        source_archive = Path(source_archive).absolute()
        archive_proof = verify_archive(source_archive)
        _require(archive_proof['app'] == observed, 'source app differs from pinned publisher archive')
    signature_options = {'publisher_archive': True} if archive_proof else {}
    signatures = {'app': _signature(app, app=True, **signature_options), 'driver': _signature(driver, app=False)}
    system = _native_libraries(app, driver, browser, observed['files'])
    _require(_scan(app, full=False)[1] == before, 'source app changed before staging')
    target.mkdir(parents=True)
    copied = target / app.name; copied.mkdir()
    for row in observed['directories']:
        (copied / row['path']).mkdir()
    def copy_file(original, destination, expected, mode):
        _require(_file(original) == expected, 'reference source changed before copy')
        fd = os.open(original, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as incoming:
            out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(out, 'wb') as outgoing:shutil.copyfileobj(incoming, outgoing)
        destination.chmod(mode)
        _require(_file(destination) == expected and _file(original) == expected, 'reference bytes changed during copy')
    for row in observed['files']:
        copy_file(resolve_input(app, row['path']), copied / row['path'],
                  {key: row[key] for key in ('sha256', 'size', 'executable')}, row['mode'])
    for row in observed['symlinks']:
        os.symlink(row['target'], copied / row['path'])
    for row in sorted(observed['directories'], key=lambda r: r['path'], reverse=True):
        (copied / row['path']).chmod(row['mode'])
    copied.chmod(observed['mode'])
    copied_driver = target / 'chromedriver'
    copy_file(driver, copied_driver, driver_identity, stat.S_IMODE(driver.stat().st_mode))
    authentication = {'method': 'codesign'}
    if archive_proof:
        archive_copy = target / 'publisher.zip'
        copy_file(source_archive, archive_copy, _file(source_archive), 0o444)
        authentication = {'method': 'publisher-archive', 'archive': snapshot_input(store, archive_copy.relative_to(store).as_posix()),
                          'members_sha256': archive_proof['members_sha256']}
    app_path = copied.relative_to(store).as_posix()
    identity = {'schema_version': 1, 'kind': KIND,
        'app': {'path': app_path, 'mode': observed['mode'], 'directories': observed['directories'],
                'files': [snapshot_input(store, app_path + '/' + row['path']) for row in observed['files']],
                'symlinks': observed['symlinks'], 'file_modes': {row['path']: row['mode'] for row in observed['files']}},
        'browser': snapshot_input(store, app_path + '/Contents/MacOS/' + executable),
        'driver': snapshot_input(store, copied_driver.relative_to(store).as_posix()), 'signatures': signatures, 'system': system, 'authentication': authentication}
    verify_reference_environment(identity, store)
    _require(_scan(app, full=True)[0] == observed and _scan(app, full=False)[1] == before
             and _file(driver) == driver_identity, 'reference source changed during staging')
    _require(_signature(app, app=True, **signature_options) == signatures['app'] and _signature(driver, app=False) == signatures['driver'],
             'reference source signature changed during staging')
    _write_new(target / 'identity.json', identity)
    return identity

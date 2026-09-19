#!/usr/bin/env python3
"""Read-only Python11 environment custody; writes a new result artifact only."""
import argparse
import datetime
import email.parser
import hashlib
import json
import os
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
ARTIFACTS = ROOT / 'artifacts/completion-validation-2026-09-18'
PINS = {
    'repair40-python09-environment.json': '5081d2bc2cbb85974794de5ca593c7fd1a089cb7c6a24eac73fab013f06754cb',
    'repair40-python-dependency-preparation.json': '7d6425c8d07261cc262ea9eac1afc0cd6c267f62e23cf22d46c9a7e1b425b922',
}

def stable_bytes(path):
    path = Path(path)
    first = path.read_bytes()
    if first != path.read_bytes():
        raise ValueError('unstable file: ' + str(path))
    return first

def digest(raw):
    return hashlib.sha256(raw).hexdigest()

def name_key(name):
    return re.sub(r'[-_.]+', '-', name).lower()

def check_environment(check_process_environment=False):
    errors, identities, records = [], [], {}
    for name, expected in PINS.items():
        raw = stable_bytes(ARTIFACTS / name)
        if digest(raw) != expected:
            raise ValueError('record changed: ' + name)
        records[name] = json.loads(raw)
    env = records['repair40-python09-environment.json']
    dependency = records['repair40-python-dependency-preparation.json']
    if env['dependency_receipt']['sha256'] != PINS['repair40-python-dependency-preparation.json']:
        raise ValueError('dependency receipt binding changed')
    def file_check(path, expected, kind, **extra):
        try:
            path = Path(path)
            actual = digest(stable_bytes(path))
            identities.append(dict(kind=kind, path=str(path), sha256=expected, observed_sha256=actual, **extra))
            if actual != expected:
                errors.append('file identity changed: ' + str(path))
        except (OSError, ValueError) as error:
            errors.append(str(error))
    for name, row in env['executables'].items():
        file_check(row['path'], row['sha256'], 'executable', name=name)
        if not os.access(row['path'], os.X_OK):
            errors.append('executable permission missing: ' + row['path'])
        if 'invocation_path' in row:
            try:
                invocation = Path(row['invocation_path'])
                if not invocation.is_symlink():
                    raise ValueError('invocation is no longer a symlink: ' + str(invocation))
                resolved = invocation.resolve(strict=True)
                if str(resolved) != row['path']:
                    raise ValueError('invocation target changed: ' + str(invocation))
                current, seen, chain = invocation, set(), []
                while current.is_symlink():
                    if str(current) in seen:
                        raise ValueError('invocation symlink cycle')
                    seen.add(str(current))
                    target = os.readlink(current)
                    chain.append(dict(path=str(current), target=target))
                    current = Path(os.path.normpath(str(current.parent / target))) if not os.path.isabs(target) else Path(target)
                if invocation.resolve(strict=True) != resolved:
                    raise ValueError('unstable invocation target')
                identities.append(dict(kind='invocation_symlink', path=str(invocation), resolved_path=str(resolved), links=chain))
            except (OSError, ValueError, RuntimeError) as error:
                errors.append(str(error))
            config = row['venv_config']
            file_check(config['path'], config['sha256'], 'venv_config', name=name)
    total = 0
    for environment in dependency['environments']:
        base = Path(environment['path'])
        expected = {name_key(row['name']): row for row in environment['packages']}
        if len(expected) != len(environment['packages']):
            raise ValueError('duplicate recorded package identity')
        found, metadata_paths = {}, []
        for site in sorted(base.glob('lib/python*/site-packages')):
            for distribution in sorted(site.glob('*.dist-info')):
                path = distribution / 'METADATA'
                try:
                    raw = stable_bytes(path)
                    header = email.parser.BytesParser().parsebytes(raw, headersonly=True)
                    key = name_key(header['Name'] or '')
                    version = header['Version']
                    if not key or key in found:
                        raise ValueError('missing or duplicate package name: ' + str(path))
                    found[key] = (path, version, digest(raw))
                    metadata_paths.append(str(path))
                except (OSError, ValueError) as error:
                    errors.append(str(error))
            for path in site.glob('*.egg-info'):
                errors.append('unrecorded egg-info distribution: ' + str(path))
        if set(found) != set(expected):
            errors.append('package population changed: ' + str(base) + '; missing=' + repr(sorted(set(expected)-set(found))) + '; added=' + repr(sorted(set(found)-set(expected))))
        for key, row in expected.items():
            total += 1
            if key not in found:
                continue
            path, version, actual = found[key]
            identities.append(dict(kind='package_metadata', environment=str(base), path=str(path), name=row['name'], version=row['version'], sha256=row['metadata_sha256'], observed_sha256=actual))
            if version != row['version'] or actual != row['metadata_sha256']:
                errors.append('package metadata changed: ' + str(path))
        identities.append(dict(kind='metadata_population', environment=str(base), paths=sorted(metadata_paths), expected_count=len(expected)))
    if total != 82 or len(dependency['environments']) != 2:
        raise ValueError('recorded package/environment denominator changed')
    if check_process_environment:
        for key, value in env['environment'].items():
            if os.environ.get(key) != value:
                errors.append('launcher environment differs: ' + key)
        for key in env['unset']:
            if key in os.environ:
                errors.append('launcher environment must unset: ' + key)
        if os.environ.get('PATH', '').split(os.pathsep)[0] != env['path_prepend']:
            errors.append('launcher PATH prefix differs')
    for name, expected in PINS.items():
        if digest(stable_bytes(ARTIFACTS / name)) != expected:
            errors.append('record changed during verification: ' + name)
    return dict(status='passed' if not errors else 'failed', errors=errors, identities=identities,
                package_metadata_obligations=82, environment_count=2,
                expected_records=[dict(path=str(ARTIFACTS/name), sha256=value) for name,value in PINS.items()],
                recorded_launcher_environment=env['environment'], recorded_path_prepend=env['path_prepend'], recorded_unset=env['unset'],
                process_environment_checked=check_process_environment,
                scope='Executable bytes, recorded Python invocation targets and link chains, venv configs, exact82 METADATA identities and distribution population; no package imports, executable invocation, or test/build execution.')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--phase', choices=('before', 'after'), required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--before', type=Path)
    parser.add_argument('--before-sha256')
    parser.add_argument('--check-process-environment', action='store_true')
    args = parser.parse_args()
    if args.phase == 'after' and (not args.before or not args.before_sha256):
        parser.error('after requires --before and its original --before-sha256')
    if args.phase == 'before' and (args.before or args.before_sha256):
        parser.error('before does not accept prior result arguments')
    helper_sha = digest(stable_bytes(__file__))
    try:
        result = check_environment(args.check_process_environment)
        if args.phase == 'after':
            raw = stable_bytes(args.before)
            if digest(raw) != args.before_sha256:
                raise ValueError('before result changed')
            before = json.loads(raw)
            if before['status'] != 'passed' or before['phase'] != 'before':
                raise ValueError('before environment check did not pass')
            if before['helper_sha256'] != helper_sha or before['identities'] != result['identities']:
                raise ValueError('environment identity or checker changed since before launch')
            if before['process_environment_checked'] != result['process_environment_checked']:
                raise ValueError('before/after process environment check modes differ')
            result['before_result'] = dict(path=str(args.before), sha256=args.before_sha256)
    except Exception as error:
        result = dict(status='failed', errors=[str(error)])
    result.update(phase=args.phase, helper_sha256=helper_sha, captured_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), tests_or_builds_executed=False)
    if digest(stable_bytes(__file__)) != helper_sha:
        result['status'] = 'failed'; result.setdefault('errors', []).append('checker changed during verification')
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2); stream.write('\n')
    print(json.dumps(dict(status=result['status'], output=str(args.output), sha256=digest(stable_bytes(args.output)), errors=result['errors'])))
    return 0 if result['status'] == 'passed' else 1

if __name__ == '__main__':
    sys.exit(main())

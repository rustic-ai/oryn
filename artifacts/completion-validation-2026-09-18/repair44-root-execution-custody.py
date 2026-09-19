"""Reviewed repair44 dispatch; actual declaration/whole-lease digests are required at launch."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'artifacts/completion-validation-2026-09-18'
GATE = ROOT / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-44-2026-09-18.json'
PREVIOUS_LEASE = BASE / 'repair43-whole-source-lease-inputs-01.json'
PREVIOUS_LEASE_SHA = '4e8ba03f573bec8c09aefdf7eb699f1f95b7df1adcecadc7b63f1d2bc7fd3f0d'
ENVIRONMENT = BASE / 'repair40-python09-environment.json'
ENVIRONMENT_SHA = '5081d2bc2cbb85974794de5ca593c7fd1a089cb7c6a24eac73fab013f06754cb'
ENVIRONMENT_CHECKER = BASE / 'check-python11-environment.py'
ENVIRONMENT_CHECKER_SHA = 'abb2e1fb93ded5a47025b062b015278ccbd398c5c3ba5568654c3d762e9822c6'


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def included(path):
    # Same authored discovery population as repair43; do not shrink exclusions.
    p = Path(path)
    return (p.parts[0] in ('crates', 'vendor', 'scripts', 'tools', 'docs')
            and not any(x in ('target', 'node_modules', '__pycache__', '.git') for x in p.parts[:-1])
            and not p.name.endswith(('.pyc', '.pyo', '.orig', '.rej', '.bak'))
            and not p.name.startswith('GENERATION-COMPLETE'))


def check(lease_path, lease_sha, gate_sha):
    errors, count = [], None
    try:
        if sha(PREVIOUS_LEASE) != PREVIOUS_LEASE_SHA:
            raise ValueError('historical whole-source lease changed')
        if sha(lease_path) != lease_sha or sha(GATE) != gate_sha:
            raise ValueError('whole lease or declaration changed')
        data = json.loads(lease_path.read_bytes())
        previous = json.loads(PREVIOUS_LEASE.read_bytes())
        declaration = json.loads(GATE.read_bytes())
        rows = data['sources']
        count = len(rows)
        paths = [row['path'] for row in rows]
        if len(set(paths)) != count or data['source_count'] != count:
            raise ValueError('inconsistent whole-source population')
        if not {row['path'] for row in previous['sources']}.issubset(paths):
            raise ValueError('whole lease dropped historical source paths')
        if data['authored_discovery_roots'] != previous['authored_discovery_roots']:
            raise ValueError('authored discovery root denominator changed')
        if data['expected_declaration_path'] != str(GATE.relative_to(ROOT)):
            raise ValueError('whole lease names a different generation')
        for row in rows + declaration['review_sources']:
            p = ROOT / row['path']
            if Path(row['path']).is_absolute() or '..' in Path(row['path']).parts:
                raise ValueError('source path escapes repository')
            try:
                before = p.stat()
                actual = sha(p)
                after = p.stat()
                if actual != row['sha256'] or (before.st_size, before.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                    errors.append(row['path'])
            except OSError as error:
                errors.append(str(error))
        expected = {path for path in paths if included(path)}
        found = set()
        for top in data['authored_discovery_roots']:
            for folder, dirs, files in os.walk(ROOT / top):
                dirs[:] = [x for x in dirs if x not in ('target', 'node_modules', '__pycache__', '.git')]
                for name in files:
                    path = str((Path(folder) / name).relative_to(ROOT))
                    if included(path):
                        found.add(path)
        errors.extend('added:' + path for path in sorted(found - expected))
        errors.extend('missing:' + path for path in sorted(expected - found))
        if sha(lease_path) != lease_sha or sha(GATE) != gate_sha:
            raise ValueError('whole lease/declaration changed during check')
    except Exception as error:
        errors.append(repr(error))
    return dict(errors=errors, source_count=count, declaration_sha256=gate_sha,
                lease_sha256=lease_sha, checked_at=time.time())


def bound_harness(launch, python):
    declaration = json.loads(GATE.read_bytes())
    bound = {}
    for requirement in declaration['requirements'].values():
        for field in ('implementation_sources', 'test_sources'):
            for row in requirement[field]:
                if row['path'] in bound and bound[row['path']] != row['sha256']:
                    raise ValueError('conflicting source binding: ' + row['path'])
                bound[row['path']] = row['sha256']
    required = [Path(__file__), launch]
    if python:
        required.extend((launch.parent / 'runner.py', ENVIRONMENT_CHECKER))
    for path in required:
        if bound.get(str(path.relative_to(ROOT))) != sha(path):
            raise ValueError('execution helper is not bound: ' + str(path))


def group_exists(group_id):
    try:
        os.killpg(group_id, 0)
    except ProcessLookupError:
        return False
    return True


def run_owned_dispatch(command, env, start_receipt=None):
    """One new session covers launcher and children; settle it before source checks."""
    process, code, execution_error = None, None, None
    cleanup_errors, events = [], []
    group_settled = False
    def terminate(signum, frame):
        raise InterruptedError('owned dispatch received signal ' + str(signum))
    previous_terminate = signal.signal(signal.SIGTERM, terminate)
    try:
        process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
        events.append(dict(event='started', pid=process.pid, group_id=process.pid))
        if start_receipt is not None:
            save(start_receipt, dict(pid=process.pid, group_id=process.pid, command=command,
                 generation_gate=env.get('ORYN_GENERATION_GATE'),
                 generation_gate_sha256=env.get('ORYN_GENERATION_GATE_SHA256'),
                 supervision=env.get('ORYN_REPAIR44_SUPERVISED_GROUP')))
        code = process.wait()
    except BaseException as error:
        execution_error = repr(error)
    finally:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        if process is not None:
            previous_interrupt = signal.signal(signal.SIGINT, signal.SIG_IGN)
            try:
                if process.poll() is None:
                    events.append(dict(event='terminate_running_group', group_id=process.pid))
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                code = process.wait(timeout=15)
                if group_exists(process.pid):
                    cleanup_errors.append('descendant group survived direct-child exit')
                    events.append(dict(event='terminate_leftover_group', group_id=process.pid))
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    deadline = time.monotonic() + 5
                    while group_exists(process.pid) and time.monotonic() < deadline:
                        time.sleep(0.05)
                group_settled = not group_exists(process.pid)
                if not group_settled:
                    cleanup_errors.append('owned process group did not settle')
            except BaseException as error:
                cleanup_errors.append(repr(error))
            finally:
                signal.signal(signal.SIGINT, previous_interrupt)
        else:
            group_settled = True
        signal.signal(signal.SIGTERM, previous_terminate)
    return dict(returncode=code, execution_error=execution_error, cleanup_errors=cleanup_errors,
                group_settled=group_settled, pid=process.pid if process is not None else None,
                events=events, cleanup_completed_at=time.time())


def environment_check(env, phase, before_sha=None):
    if sha(ENVIRONMENT) != ENVIRONMENT_SHA or sha(ENVIRONMENT_CHECKER) != ENVIRONMENT_CHECKER_SHA:
        raise ValueError('historical environment/checker changed')
    output = BASE / ('python12-environment-' + phase + '.json')
    command = [env['ORYN_WPT_PYTHON'], str(ENVIRONMENT_CHECKER), '--phase', phase,
               '--output', str(output), '--check-process-environment']
    if phase == 'after':
        command.extend(('--before', str(BASE / 'python12-environment-before.json'),
                        '--before-sha256', before_sha))
    child_env = dict(env)
    child_env.pop('ORYN_REPAIR44_SUPERVISED_GROUP', None)
    process = run_owned_dispatch(command, child_env)
    receipt = BASE / ('python12-environment-' + phase + '-process.json')
    save(receipt, process)
    code = process['returncode']
    if process['execution_error'] or process['cleanup_errors'] or not process['group_settled']:
        raise ValueError('Python environment process custody failed: ' + repr(process))
    result = json.loads(output.read_bytes())
    if code or result.get('status') != 'passed':
        raise ValueError('Python environment custody failed: ' + repr(result.get('errors')))
    return dict(path=str(output.relative_to(ROOT)), sha256=sha(output),
                process_receipt=dict(path=str(receipt.relative_to(ROOT)), sha256=sha(receipt)))


def build_environment(admission, python, launch, gate_sha):
    env = dict(os.environ)
    env['PYTHONDONTWRITEBYTECODE'] = '1'
    if python:
        if sha(ENVIRONMENT) != ENVIRONMENT_SHA:
            raise ValueError('historical Python environment record changed')
        config = json.loads(ENVIRONMENT.read_bytes())
        env.update(config['environment'])
        env['PATH'] = config['path_prepend'] + os.pathsep + env.get('PATH', '')
        for key in config['unset']:
            env.pop(key, None)
        command = [config['environment']['ORYN_WPT_PYTHON'], str(launch)]
    else:
        command = [sys.executable, str(launch)]
    if python:
        env['ORYN_REPAIR44_SUPERVISED_GROUP'] = 'outer-owned-session'
    else:
        env.pop('ORYN_REPAIR44_SUPERVISED_GROUP', None)
    # Bind last: historical environment dictionaries must never restore an older gate.
    env = admission.environment(env)
    if env['ORYN_GENERATION_GATE'] != str(GATE.resolve()) or env['ORYN_GENERATION_GATE_SHA256'] != gate_sha:
        raise ValueError('final child environment is not bound to repair44')
    return env, command


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('attempt', choices=('native30', 'python12'))
    parser.add_argument('--gate-sha256', required=True)
    parser.add_argument('--whole-lease', type=Path, required=True)
    parser.add_argument('--whole-lease-sha256', required=True)
    args = parser.parse_args()
    for value in (args.gate_sha256, args.whole_lease_sha256):
        if not re.fullmatch('[0-9a-f]{64}', value):
            parser.error('freeze digests must be actual lowercase SHA256 values')
    lease_path = args.whole_lease.resolve()
    if not lease_path.is_relative_to(BASE):
        parser.error('whole lease must be a reviewed completion artifact')
    python = args.attempt == 'python12'
    launch = BASE / ('fq-python-regressions-12/launch.py' if python else 'run-native-js-tests-30.py')
    output = BASE / ('repair44-' + args.attempt + '-whole-custody.json')
    preflight = BASE / ('repair44-' + args.attempt + '-whole-preflight.json')
    process_start = BASE / ('repair44-' + args.attempt + '-process-start.json')
    process_receipt = BASE / ('repair44-' + args.attempt + '-process.json')
    protected = [output, preflight, process_start, process_receipt]
    if python:
        protected.extend(BASE / ('python12-environment-' + phase + suffix) for phase in ('before', 'after') for suffix in ('.json', '-process.json'))
        protected.extend(launch.parent / name for name in ('preflight.json', 'process.json', 'full.log', 'test-results.json', 'execution.json'))
    else:
        protected.extend(BASE / name for name in ('native-js-tests-30.json', 'native-js-tests-30.preflight.json', 'native-js-tests-30.log', 'webidl-check-30.json', 'webidl-check-30.log'))
    if any(path.exists() for path in protected):
        raise ValueError('attempt output already exists; preserve it and prepare a new reviewed attempt')
    record = dict(operation_sha256=sha(__file__), gate=str(GATE), whole_lease=str(lease_path),
                  before=check(lease_path, args.whole_lease_sha256, args.gate_sha256),
                  returncode=None, execution_error=None)
    if record['before']['errors']:
        save(output, record)
        return 1
    env = None
    try:
        sys.path.insert(0, str(ROOT / 'scripts'))
        from generation_gate import GenerationLease
        admission = GenerationLease(GATE, expected=args.gate_sha256)
        bound_harness(launch, python)
        env, command = build_environment(admission, python, launch, args.gate_sha256)
        record.update(command=command, final_generation_environment={key: env[key] for key in ('ORYN_GENERATION_GATE', 'ORYN_GENERATION_GATE_SHA256')})
        if python:
            record['environment_before'] = environment_check(env, 'before')
        save(preflight, record)
        admission.check()
        record['process'] = run_owned_dispatch(command, env, start_receipt=process_start)
        save(process_receipt, record['process'])
        record['process_receipt'] = dict(path=str(process_receipt.relative_to(ROOT)), sha256=sha(process_receipt))
        record['returncode'] = record['process']['returncode']
        record['execution_error'] = record['process']['execution_error']
    except BaseException as error:
        record['execution_error'] = repr(error)
    finally:
        if python and env is not None and 'environment_before' in record:
            try:
                record['environment_after'] = environment_check(env, 'after', record['environment_before']['sha256'])
            except BaseException as error:
                record['environment_error'] = repr(error)
        record['after'] = check(lease_path, args.whole_lease_sha256, args.gate_sha256)
        record['custody_valid'] = not (record['before']['errors'] or record['after']['errors']
            or record.get('environment_error') or record['execution_error']
            or record.get('process', {}).get('cleanup_errors')
            or not record.get('process', {}).get('group_settled', False))
        save(output, record)
    print(json.dumps(record), flush=True)
    return record['returncode'] or int(not record['custody_valid'] or record['execution_error'] is not None)


if __name__ == '__main__':
    sys.exit(main())

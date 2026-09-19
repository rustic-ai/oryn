"""Collect every frozen Python regression; incomplete execution never passes."""
import hashlib
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import time
import uuid

HISTORICAL_SHA256 = 'b771fe6c9c532bc6cfaecdc00b44612c960386c01d30f38c68da3e850a09d687'
SUPERVISOR_KEY = 'ORYN_REPAIR44_SUPERVISED_GROUP'
SUPERVISOR_VALUE = 'outer-owned-session'
OUTPUT_NAMES = ('preflight.json', 'process.json', 'full.log', 'test-results.json', 'execution.json')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def persist(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def regression_sources(root, previous_path, bound):
    if digest(previous_path) != HISTORICAL_SHA256:
        raise ValueError('historical regression preflight changed')
    previous = json.loads(previous_path.read_text())
    historical = previous['test_sources']
    if len(historical) != 45 or len(set(historical)) != 45:
        raise ValueError('all 45 historical regression sources are required')
    sources = set(historical)
    for directory in (root / 'scripts', root / 'tools/wptrunner-oryn/tests'):
        sources.update(str(path.relative_to(root)) for path in directory.rglob('test*.py'))
    sources.update(path for path in bound if Path(path).suffix == '.py' and Path(path).name.startswith('test'))
    identities = []
    for source in sorted(sources):
        path = (root / source).resolve()
        if not path.is_relative_to(root) or not path.is_file():
            raise ValueError('regression source missing or outside repository: ' + source)
        actual = digest(path)
        if bound.get(source) != actual:
            raise ValueError('regression source is not bound to this candidate: ' + source)
        identities.append(dict(path=source, sha256=actual))
    return identities


def bind_native_inputs(environment):
    """Bind dispatch to resolved CLI/worker paths; release evidence proves their origin."""
    identities, errors = {}, []
    for name in ('ORYN_TEST_NATIVE_BIN', 'ORYN_PAGE_WORKER'):
        value = environment.get(name)
        if not value:
            errors.append('signed CLI prerequisite missing: ' + name)
            continue
        path = Path(value).resolve()
        if not path.is_file() or not os.access(path, os.X_OK):
            errors.append('signed CLI prerequisite is not executable: ' + name)
            continue
        identities[name] = dict(path=str(path), sha256=digest(path))
        environment[name] = identities[name]['path']
    return identities, errors


def validate_results(record, preflight):
    if not isinstance(record, dict):
        raise ValueError('regression result is not an object')
    if record.get('run_id') != preflight['run_id'] or record.get('generation_gate_sha256') != preflight['generation_gate_sha256']:
        raise ValueError('regression results belong to a different invocation or generation')
    if record.get('source_identities') != preflight['source_identities']:
        raise ValueError('regression results omit or change source identities')
    sources = record['sources']
    if [item['path'] for item in sources] != preflight['test_sources']:
        raise ValueError('regression collection does not cover the exact source population')
    ids = [test for item in sources for test in item['test_ids']]
    if not ids or len(ids) != len(set(ids)) or any(item['count'] != len(item['test_ids']) or item['count'] <= 0 for item in sources):
        raise ValueError('empty, duplicate, or inconsistent source collection')
    outcomes = record['test_outcomes']
    if [item['test'] for item in outcomes] != ids or record['discovered_tests'] != len(ids):
        raise ValueError('regression outcome ledger does not cover collected tests')
    if record['tests'] != len(ids) or record.get('execution_complete') is not True:
        raise ValueError('not every collected regression completed')
    # Skips and expected failures remain visible, but do not certify full coverage.
    if any(record[key] for key in ('failures', 'errors', 'skipped', 'expected_failures', 'unexpected_successes', 'collection_errors', 'execution_errors')):
        raise ValueError('regressions contain failures, skips, or incomplete collection')
    if any(item['status'] != 'passed' for item in outcomes):
        raise ValueError('not every collected regression passed')


def read_results(path, preflight):
    record = json.loads(Path(path).read_text())
    validate_results(record, preflight)
    return record


def supervision(environment):
    value = environment.get(SUPERVISOR_KEY)
    if value is None:
        return dict(mode='standalone', group_id=None)
    if value != SUPERVISOR_VALUE or os.getpgrp() != os.getpid():
        raise ValueError('supervised Python launcher must lead the outer-owned process group')
    return dict(mode='outer-owned', group_id=os.getpgrp())


def run_child(argv, environment, output, preflight, admission):
    """Own the entire child process group even if bookkeeping or waiting fails."""
    owner = supervision(environment)
    child_environment = dict(environment)
    child_environment.pop(SUPERVISOR_KEY, None)
    process, code, execution_error = None, None, None
    cleanup_errors = []
    try:
        admission.check()
        with (output / 'full.log').open('x') as log:
            process = subprocess.Popen(argv, env=child_environment, stdout=log,
                                       stderr=subprocess.STDOUT, start_new_session=owner['mode'] == 'standalone')
            persist(output / 'process.json', dict(pid=process.pid, argv=argv,
                    run_id=preflight['run_id'], generation_gate_sha256=admission.sha256, supervision=owner))
            code = process.wait()
    except BaseException as error:
        # In particular, a KeyboardInterrupt after Popen still owns the child.
        execution_error = repr(error)
    finally:
        if process is not None and owner['mode'] == 'outer-owned':
            # The outer wrapper must reap this complete group before after-custody.
            # Do not kill our own group or wait indefinitely after interruption.
            if code is None:
                code = process.poll()
        elif process is not None:
            previous_interrupt = signal.signal(signal.SIGINT, signal.SIG_IGN)
            try:
                if process.poll() is None:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    except OSError as error:
                        cleanup_errors.append('process-group kill: ' + repr(error))
                        process.kill()
                    code = process.wait(timeout=15)
                else:
                    code = process.wait(timeout=15)
                try:
                    os.killpg(process.pid, 0)
                except ProcessLookupError:
                    pass
                else:
                    # A leftover descendant is failed custody even if force-reaped.
                    cleanup_errors.append('child process group survived direct-child exit')
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            except BaseException as error:
                cleanup_errors.append(repr(error))
            finally:
                signal.signal(signal.SIGINT, previous_interrupt)
    return dict(returncode=code, execution_error=execution_error, cleanup_errors=cleanup_errors,
                pid=process.pid if process is not None else None, supervision=owner)


def main():
    output = Path(__file__).resolve().parent
    root = output.parents[2]
    os.chdir(root)
    sys.path.insert(0, str(root / 'scripts'))
    from generation_gate import inherited_generation_lease
    lease = inherited_generation_lease()
    gate = root / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-44-2026-09-18.json'
    if lease.path != gate.resolve():
        raise ValueError('Python regression execution requires the repair44 declaration')
    declaration = json.loads(gate.read_text())
    bound = {source['path']: source['sha256'] for requirement in declaration['requirements'].values()
             for field in ('implementation_sources', 'test_sources') for source in requirement[field]}
    identities = regression_sources(root, output.parent / 'fq-python-regressions-07/preflight.json', bound)
    for source in (Path(__file__).relative_to(root), (output / 'runner.py').relative_to(root)):
        if bound.get(str(source)) != digest(root / source):
            raise ValueError('regression harness is not bound to this candidate: ' + str(source))
    if any((output / name).exists() for name in OUTPUT_NAMES):
        raise ValueError('regression output already exists; preserve this attempt and use a new bound run directory')
    upstream = root.parent / 'oryn/artifacts/upstream-conformance/wpt-719d5e38fdd0903a18ed9007aba816c98cc491e0'
    python = output.parent / 'fq-python-venv/bin/python'
    sources = [source['path'] for source in identities]
    argv = [str(python), str(output / 'runner.py')] + [str(root / source) for source in sources]
    env = lease.environment()
    inputs, prerequisite_errors = bind_native_inputs(env)
    if prerequisite_errors:
        # Never let a half-configured CLI select an implicit, potentially stale worker.
        env.pop('ORYN_TEST_NATIVE_BIN', None)
    run_id = uuid.uuid4().hex
    env.update(PYTHONDONTWRITEBYTECODE='1', PYTHONUNBUFFERED='1', ORYN_PYTHON_RUN_ID=run_id,
               PYTHONPATH=os.pathsep.join(map(str, [root, root / 'scripts', root / 'tools/wptrunner-oryn', upstream, upstream / 'tools'])))
    preflight = dict(run_id=run_id, argv=argv, test_sources=sources, source_count=len(sources),
                     source_identities=identities, supervision=supervision(env), generation_gate=str(gate), generation_gate_sha256=lease.sha256,
                     pythonpath=env['PYTHONPATH'], native_inputs=inputs, prerequisite_errors=prerequisite_errors)
    persist(output / 'preflight.json', preflight)
    start = time.monotonic()
    errors = []
    child = run_child(argv, env, output, preflight, lease)
    try:
        lease.check()
        for identity in inputs.values():
            if digest(identity['path']) != identity['sha256']:
                raise ValueError('selected native executable changed during Python regressions')
    except Exception as error:
        errors.append(str(error))
    result_errors = list(prerequisite_errors)
    try:
        read_results(output / 'test-results.json', preflight)
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        result_errors.append(str(error))
    passed = child['returncode'] == 0 and not errors and child['execution_error'] is None and not result_errors and not child['cleanup_errors']
    record = dict(run_id=run_id, elapsed_seconds=time.monotonic() - start,
                  generation_gate_sha256=lease.sha256, generation_errors=errors,
                  **child, result_errors=result_errors,
                  full_regression_pass=passed)
    persist(output / 'execution.json', record)
    print(json.dumps(record), flush=True)
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())

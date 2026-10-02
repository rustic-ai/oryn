"""Ordinary developer WPT: run a selected population using an existing build.

No source export, review receipt, build requalification, or recursive input audit
is required. --audit-inputs explicitly opts into one expensive pre-run audit.
The upstream tests, fixture behavior, outcomes and owned cleanup are unchanged.
"""
import argparse
import copy
import datetime
import json
import math
import os
from pathlib import Path
import sys
import threading
import time
import traceback

from oryn_validation.admission import SOURCE_ROOT, read_object
from oryn_validation.developer_inputs import load_developer_inputs, select_fixture, python_environment, audit_inputs, source_info, serialize_inputs, configured_inputs, bundle_owner
from oryn_validation.wpt import (
    _digest, plan_command, fixture_services, verify_upstream_completion,
)
from oryn_validation.worker_environment import worker_environment, assert_worker_environment
from oryn_validation.processes import ResourceLease, run_owned
from oryn_validation.fixture_startup import settlement
from oryn_validation.fixture_tls import verify_receipts
from oryn_validation.results import reconcile_wpt, read_mozlog

class IntegrityError(RuntimeError):
    pass


def write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, default=str)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def phase(output, name):
    with (Path(output) / 'phases.jsonl').open('a') as stream:
        stream.write(json.dumps({'phase': name, 'time': datetime.datetime.now(
            datetime.timezone.utc).isoformat(), 'monotonic_ns': time.monotonic_ns()}) + '\n')
        stream.flush()


def safe_path(value, root):
    path, root = Path(value).absolute(), Path(root).absolute()
    if '..' in Path(value).parts or not path.is_relative_to(root):
        raise ValueError('developer output must stay under ' + str(root))
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise ValueError('developer output cannot traverse symlinks')
    return path


def urls(path):
    rows = Path(path).read_text().splitlines()
    if not rows or len(rows) != len(set(rows)) or any(not r.startswith('/') or r.startswith('//') for r in rows):
        raise ValueError('include file must contain unique, absolute WPT URL paths')
    return rows


def summarize_events(population, events, execution):
    """Reconcile in one pass without retaining multi-gigabyte diagnostic logs."""
    first_start = last_end = None
    failures = []

    def observe():
        nonlocal first_start, last_end
        for event in events:
            action = event.get('action')
            if action == 'test_start':
                first_start = event['time'] if first_start is None else min(first_start, event['time'])
            elif action == 'test_end':
                last_end = event['time'] if last_end is None else max(last_end, event['time'])
                if event.get('status') not in ('OK', 'PASS'):
                    failures.append({k: event[k] for k in ('test', 'status', 'message') if k in event})
            yield event

    reconciliation = reconcile_wpt(population, observe(), execution=execution, assessment_total=0)
    interval = None if first_start is None or last_end is None else (last_end - first_start) / 1000
    return reconciliation, interval, failures


class DeveloperLookup:
    """Opaque process-local projection with sticky drift and deep-copy returns."""
    def __init__(self, bound, contract):
        self.bound = bound
        self._contract = copy.deepcopy(contract)
        self.pid = os.getpid()
        self.lock = threading.Lock()
        self.failure = None
        self.closed = False

    def __call__(self, requested=None, binary=None, webdriver_binary=None):
        if os.getpid() != self.pid:
            raise IntegrityError('developer lookup cannot cross process boundary')
        with self.lock:
            if self.failure is not None or self.closed:
                raise IntegrityError('developer lookup is revoked or closed')
            try:
                for actual, expected in ((requested, self.bound['path']),
                        (binary, self.bound['assets']['runtime']),
                        (webdriver_binary, self.bound['assets']['driver'])):
                    if actual is not None and Path(actual).absolute() != expected:
                        raise IntegrityError('developer lookup identity substitution')
                expected_environment = {
                    'ORYN_CORE_COLLECTION': 'core',
                    'ORYN_VALIDATION_STORE_ROOT': str(self.bound['store_root']),
                    'ORYN_VALIDATION_BUNDLE_ROOT': str(self.bound['bundle_root']),
                    'ORYN_WPT_RUN_CONTRACT': str(self.bound['path']),
                    'ORYN_WPT_RUN_CONTRACT_SHA256': self.bound['sha256'],
                }
                if any(os.environ.get(key) != value for key, value in expected_environment.items()):
                    raise IntegrityError('developer collection/roots/run environment changed')
                assert_worker_environment(self.bound)
                return copy.deepcopy(self._contract)
            except BaseException as error:
                self.failure = type(error).__name__ + ': ' + str(error)
                raise

    def finish(self):
        self.closed = True
        if self.failure is not None:
            raise IntegrityError('sticky developer lookup failure: ' + self.failure)


def configured(config_path, expected_sha256):
    bound, config = configured_inputs(config_path, expected_sha256)
    if _digest(config['include']) != config['include_sha256']:
        raise IntegrityError('selected URLs changed')
    selected = select_fixture(bound)
    selected['include'] = Path(config['include'])
    selected['urls'] = urls(selected['include'])
    return bound, selected


def argv_for(bound, selected, output):
    argv = plan_command(bound, selected, output)
    argv = argv[argv.index('--') + 1:]
    argv[argv.index('--include-file') + 1] = str(selected['include'])
    return argv


def runtime_environment(bound, output, config_path):
    env = {k: v for k, v in os.environ.items() if not k.startswith(('ORYN_', 'PYTHON', 'DYLD_'))}
    env.update(python_environment(bound)); env.update(worker_environment(bound))
    env.update(ORYN_WPT_DEVELOPER_CONFIG=str(config_path),
        ORYN_WPT_DEVELOPER_CONFIG_SHA256=_digest(config_path), ORYN_VALIDATION_STORE_ROOT=str(bound['store_root']),
        ORYN_VALIDATION_BUNDLE_ROOT=str(bound['bundle_root']),
        ORYN_WPT_RUN_CONTRACT=str(bound['path']), ORYN_WPT_RUN_CONTRACT_SHA256=bound['sha256'],
        ORYN_FIXTURE_TLS_CONTRACT=str(bound['assets']['fixture_tls_policy']),
        ORYN_FIXTURE_TLS_CONTRACT_SHA256=_digest(bound['assets']['fixture_tls_policy']),
        ORYN_FIXTURE_TLS_RECEIPTS=str(output / 'tls-activation'), ORYN_CORE_COLLECTION='core',
        ORYN_WPT_CANDIDATE_SHA256=bound['run']['build']['manifest_sha256'],
        ORYN_WPT_PROTOCOL_CUSTODY=str(output / 'protocol-custody.jsonl'), PYTHONDONTWRITEBYTECODE='1',
        PYTHONPATH=os.pathsep.join([str(bound['assets']['wpt_checkout']), *[str(SOURCE_ROOT / p)
            for p in ('tools/validation/bootstrap', 'tools/validation', 'tools/wptrunner-oryn')]]))
    assert_worker_environment(bound, env)
    return env


def child(config_path, expected_sha256):
    output = Path(config_path).parent
    phase(output, 'child_start')
    bound, selected = configured(config_path, expected_sha256)
    argv = argv_for(bound, selected, output / 'output')
    sys.path.insert(0, str(bound['assets']['wpt_checkout']))
    sys.path.insert(0, str(bound['assets']['wpt_checkout'] / 'tools'))
    import localpaths
    from oryn_wptrunner import product, provenance
    from oryn_validation.typed_provenance import _project
    from wptrunner import wptcommandline, wptrunner
    from oryn_wptrunner.recovery import manager_recovery
    from unittest.mock import patch
    contract = _project(bound, selected)
    contract.update(developer_only=True, qualification='not-evaluated')
    lookup = DeveloperLookup(bound, contract)
    try:
        with patch.object(product, 'load_contract', side_effect=lookup), \
             patch.object(provenance, 'load_run_contract', side_effect=lookup), manager_recovery():
            sys.argv = [str(Path(__file__)), *argv]
            kwargs = wptcommandline.parse_args()
            if kwargs['prefs_root'] is None:
                kwargs['prefs_root'] = str(Path(wptrunner.__file__).parent / 'prefs')
            with wptrunner.GlobalLogger(kwargs, {'raw': sys.stdout}):
                phase(output, 'upstream_execution_start')
                code = wptrunner.start(**kwargs)
                phase(output, 'upstream_execution_end')
        return code
    finally:
        lookup.finish()
        phase(output, 'child_complete')


def execute(run, store_root, bundle_root, output, include_file, *, workers=6, timeout_seconds=7200, audit=False):
    if type(workers) is not int or not 1 <= workers <= 6:
        raise ValueError('developer WPT supports 1 to 6 workers')
    if not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
        raise ValueError('timeout must be positive')
    started = time.monotonic_ns()
    population = urls(include_file)
    store = Path(store_root).absolute()
    output = safe_path(output, store / 'development')
    output.mkdir(parents=True, exist_ok=False)
    include = output / 'include.txt'
    include.write_text('\n'.join(population) + '\n')
    phase(output, 'timed_run_start')
    bound = load_developer_inputs(run, store_root=store, bundle_root=bundle_root)
    bound['run'].update(workers=workers, run_id=output.name, original_run_id=output.name, attempt=1)
    source = source_info()
    bound['run']['runner'] = dict(commit=source['commit'], working_tree_changes=source['working_tree_changes'])
    config_path = output / 'developer-config.json'
    config = dict(kind='developer-wpt', qualification='not-evaluated', workers=workers,
        timeout_seconds=timeout_seconds, include=str(include), include_sha256=_digest(include),
        run=str(Path(run).absolute()), run_sha256=bound['sha256'],
        store_root=str(store), bundle_root=str(Path(bundle_root).absolute()),
        source=source, inputs=serialize_inputs(bound), output=str(output), audit_inputs=audit)
    write(config_path, config)
    errors, process, settled, bound = [], None, None, None
    result = dict(kind='developer-wpt', qualification='not-evaluated', full_original=False,
        selected_count=len(population), workers=workers, input_audit='not-requested',
        source=config['source'], native_build='recorded below; not requalified')
    try:
        bound, selected = configured(config_path, _digest(config_path))
        result['native_build'] = bound['run']['build']
        if audit:
            result['input_audit'] = 'failed-or-interrupted'
            audit_bound = dict(bound, path=Path(run).absolute())
            audit_inputs(audit_bound)
            result['input_audit'] = 'passed-before-run'
        phase(output, 'parent_setup_complete')
        actual = output / 'output'; actual.mkdir(); (actual / 'tls-activation').mkdir()
        from oryn_wptrunner.custody import create
        create(actual / 'protocol-custody.jsonl')
        env = runtime_environment(bound, actual, config_path)
        write(output / 'upstream-argv.json', argv_for(bound, selected, actual))
        services, attempted = [], False
        with ResourceLease(store, ['native-focus', 'wpt-fixed-ports'],
                           objects=[bundle_owner(bound)]) as resource:
            try:
                with fixture_services(bound, selected, output=actual, environment=env,
                        lease=None, timeout_seconds=timeout_seconds, services=services, verify_environment=False):
                    phase(output, 'relays_ready'); attempted = True
                    process = run_owned([str(bound['assets']['python']), '-B', str(Path(__file__).resolve()),
                        'child', '--config', str(config_path), '--config-sha256', _digest(config_path)],
                        cwd=bound['assets']['wpt_checkout'], output=actual / 'process', env=env,
                        timeout_seconds=timeout_seconds)
                    phase(output, 'child_process_settled')
                    for service in services:
                        service.check()
                    ports = [v for k, values in selected['fixture']['ports'].items()
                             if k.startswith('https') for v in values]
                    verify_upstream_completion(process, actual, lambda: verify_receipts(
                        actual / 'tls-activation', bound['assets']['fixture_tls_policy'],
                        _digest(bound['assets']['fixture_tls_policy']), ports, require_live=False, bound=bound))
            finally:
                settled = settlement(process, services, process_attempted=attempted)
                write(actual / 'resource-settlement.json', settled)
                resource.mark_settled(settled)
    except BaseException as error:
        errors.append(type(error).__name__ + ': ' + str(error)); traceback.print_exc()
    finally:
        result.update(elapsed_seconds=(time.monotonic_ns() - started) / 1e9,
                      process=process, resource_settlement=settled, errors=errors)
        try:
            actual = output / 'output'
            if process is not None and (actual / 'mozlog.jsonl').exists():
                execution = dict(process)
                if settled:
                    execution.update(cleanup_complete=settled['cleanup_complete'], cleanup_errors=settled['cleanup_errors'])
                reconciliation, interval, failures = summarize_events(
                    population, read_mozlog(actual / 'mozlog.jsonl'), execution)
                result['reconciliation'] = reconciliation
                if interval is not None:
                    result['first_test_start_to_last_test_end_seconds'] = interval
                write(actual / 'non-ok-tests.json', failures)
                from wpt_execution_evidence import protocol_custody
                result['protocol_completed_sessions'] = protocol_custody(actual / 'protocol-custody.jsonl',
                    bound['run']['build']['manifest_sha256'])
        except BaseException as error:
            errors.append('result parsing: ' + type(error).__name__ + ': ' + str(error)); traceback.print_exc()
        phase(output, 'timed_run_end')
        result['elapsed_seconds'] = (time.monotonic_ns() - started) / 1e9
        write(output / 'timing-result.json', result)
    return 0 if not errors and result.get('reconciliation', {}).get('compatibility_complete') else 1


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    commands = parser.add_subparsers(dest='mode', required=True)
    parent = commands.add_parser('execute', allow_abbrev=False)
    for name in ('run', 'store-root', 'bundle-root', 'output', 'include-file'):
        parent.add_argument('--' + name, required=True, type=Path)
    parent.add_argument('--workers', type=int, default=6)
    parent.add_argument('--timeout-seconds', type=float, default=7200)
    parent.add_argument('--audit-inputs', action='store_true', help='Expensive: hash all declared inputs and verify the native build once before running')
    worker = commands.add_parser('child', allow_abbrev=False)
    worker.add_argument('--config', required=True, type=Path)
    worker.add_argument('--config-sha256', required=True)
    args = parser.parse_args()
    try:
        if args.mode == 'child':
            return child(args.config, args.config_sha256)
        return execute(args.run, args.store_root, args.bundle_root, args.output, args.include_file,
                       workers=args.workers, timeout_seconds=args.timeout_seconds, audit=args.audit_inputs)
    except BaseException:
        traceback.print_exc()
        return 65


if __name__ == '__main__':
    sys.exit(main())

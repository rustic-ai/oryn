"""Original-run candidate custody for existing completion producers.

Commands are fixed by producer kind. Preflight is persisted before starting any
producer; exit, process-group cleanup, source/artifact drift and raw output hashes
are retained even when a producer fails. This module does not rewrite old reports.
"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from completion_provenance import ROOT, digest, load_candidate, read_record
from generation_gate import generation_gate, GenerationLease

KINDS = {'frameworks', 'security', 'architecture', 'reliability', 'booking-regression', 'native-prerequisites', 'fixture-topology', 'native-media'}


def reference(path):
    path = Path(path).resolve()
    return dict(path=str(path), sha256=digest(path))


def persist(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def source_hash():
    return subprocess.check_output([str(ROOT / 'scripts/source-tree-hash.sh')], cwd=ROOT, text=True).strip()


def command(kind, candidate, gate, output):
    artifacts = candidate['artifacts']
    raw = Path(output) / 'raw'
    if kind == 'fixture-topology':
        controls = json.loads(artifacts['native_controls_contract'].read_text())
        python = read_record(controls['upstream_python'])
        return [str(python), str(ROOT / 'scripts/run-wpt-topology-probe.py'),
                '--generation-gate', str(Path(gate).resolve()), '--candidate', str(candidate['path']), '--output', str(raw)]
    if kind == 'native-media':
        return [sys.executable, str(ROOT / 'scripts/run-native-media-qualification.py'),
                '--generation-gate', str(Path(gate).resolve()), '--candidate', str(candidate['path']), '--output', str(raw)]
    if kind == 'native-prerequisites':
        return [sys.executable, str(ROOT / 'scripts/run-wpt-native-controls.py'),
                '--generation-gate', str(Path(gate).resolve()), '--candidate', str(candidate['path']), '--output', str(raw)]
    if kind == 'booking-regression':
        plan = json.loads(artifacts['qualification_plan'].read_text())
        name = plan.get('booking_regression_name')
        if not isinstance(name, str) or not name or any(not (char.isascii() and (char.isalnum() or char in '_:')) for char in name):
            raise ValueError('exact booking mechanism regression name required')
        return [str(artifacts['booking_regression']), '--exact', name, '--nocapture']
    if kind == 'frameworks':
        return [sys.executable, str(ROOT / 'scripts/run-oil-framework-outcomes.py'),
                '--generation-gate', str(Path(gate).resolve()), '--binary', str(artifacts['runtime']), '--worker', str(artifacts['worker']),
                '--build', str(artifacts['framework_build']), '--samples', '3', '--jobs', '1', '--output', str(raw)]
    if kind == 'reliability':
        return [sys.executable, str(ROOT / 'scripts/run-oil-full-contract.py'),
                '--binary', str(artifacts['workflow_executor']), '--worker', str(artifacts['worker']),
                '--candidate', str(candidate['path']), '--generation-gate', str(Path(gate).resolve()), '--output', str(raw)]
    if kind == 'architecture':
        return [sys.executable, str(ROOT / 'scripts/validate_runtime_architecture.py'), '--require-cutover',
                '--candidate', str(candidate['path']), '--candidate-sha256', candidate['sha256'],
                '--output', str(raw)]
    if kind == 'security':
        # This existing suite selects these release paths itself. Explicitly
        # reject a differently located candidate instead of running another one.
        expected = {'runtime': ROOT / 'target/release/oryn',
                    'worker': ROOT / 'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'}
        if any(artifacts[name] != path.resolve() for name, path in expected.items()):
            raise ValueError('security suite fixed paths differ from candidate')
        return [str(ROOT / 'scripts/run-security-suite.sh'), str(raw),
                '--prebuilt-candidate', str(candidate['path']),
                '--candidate-sha256', candidate['sha256']]
    raise ValueError('unknown completion producer')


def run_owned(argv, output, timeout=43200, environment=None, cwd=None):
    """One owned process group; a successful parent exit alone is insufficient."""
    child_environment = dict(os.environ if environment is None else environment)
    gate_path = child_environment.get('ORYN_GENERATION_GATE')
    gate_digest = child_environment.get('ORYN_GENERATION_GATE_SHA256')
    if not gate_path or not gate_digest:
        raise ValueError('child execution requires frozen generation admission')
    admission = GenerationLease(gate_path, validator=generation_gate, expected=gate_digest)
    output = Path(output)
    started = time.monotonic_ns()
    code = None
    error = None
    timed_out = False
    cleanup = False
    with (output / 'stdout.log').open('xb') as stdout, (output / 'stderr.log').open('xb') as stderr:
        process = None
        try:
            admission.check()
            process = subprocess.Popen(argv, cwd=ROOT if cwd is None else cwd, stdout=stdout, stderr=stderr, start_new_session=True, env=child_environment)
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                os.killpg(process.pid, signal.SIGKILL)
                code = process.wait(timeout=15)
        except Exception as failure:
            error = str(failure)
        finally:
            if process is not None:
                try:
                    os.killpg(process.pid, 0)
                except ProcessLookupError:
                    cleanup = True
                except PermissionError:
                    pass
                else:
                    # Surviving descendants stay a failure even after this kill.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if process.poll() is None:
                    try:
                        process.kill()
                        process.wait(timeout=15)
                    except Exception as failure:
                        error = (error + '; ' if error else '') + 'child reap failed: ' + str(failure)
    return dict(argv=argv, returncode=code, timed_out=timed_out, error=error,
                cleanup_complete=cleanup, elapsed_ns=time.monotonic_ns() - started)


def produce(kind, candidate_path, gate, output):
    admission = GenerationLease(gate, validator=generation_gate)
    frozen_gate = reference(gate)
    candidate = load_candidate(candidate_path)
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('producer source differs from candidate')
    argv = command(kind, candidate, gate, output)
    output = Path(output).resolve()
    admission.check()
    output.mkdir(parents=True, exist_ok=False)
    before = dict(schema_version=1, kind=kind, candidate_manifest_sha256=candidate['sha256'],
                  candidate=reference(candidate_path), generation_gate=frozen_gate, argv=argv,
                  producer_source=reference(ROOT / 'scripts/completion_execution.py'),
                  source_tree_sha256=candidate['record']['source_tree_sha256'])
    persist(output / 'preflight.json', before)
    environment = admission.environment(dict(os.environ, ORYN_PAGE_WORKER=str(candidate['artifacts']['worker'])))
    if kind == 'security':
        environment['ORYN_V8_PROFILE_MANIFEST'] = str(candidate['artifacts']['engine_profile'])
        # The explicit prebuilt route allocates its own temporary build tree
        # for full software checks; never inherit a frozen release target.
        environment.pop('CARGO_TARGET_DIR', None)
    # Candidate/source inspection and preflight persistence can be lengthy.
    # Admit the child against the same declaration immediately before launch,
    # including replacements that are independently valid but not this run's.
    generation_gate(gate)
    read_record(frozen_gate)
    if load_candidate(candidate_path)['sha256'] != candidate['sha256'] or source_hash() != before['source_tree_sha256']:
        raise ValueError('candidate drift before producer launch')
    admission.check()
    execution = run_owned(argv, output, environment=environment)
    errors = []
    if execution['returncode'] != 0 or execution['timed_out'] or execution['error'] or not execution['cleanup_complete']:
        errors.append('producer execution or cleanup failed')
    try:
        generation_gate(gate)
        read_record(frozen_gate)
        if load_candidate(candidate_path)['sha256'] != candidate['sha256'] or source_hash() != before['source_tree_sha256']:
            raise ValueError('candidate drift')
    except Exception as error:
        errors.append(str(error))
    raw = [reference(path) for path in sorted(output.rglob('*')) if path.is_file()]
    receipt = dict(schema_version=1, kind=kind, candidate_manifest_sha256=candidate['sha256'],
                   preflight=reference(output / 'preflight.json'), execution=execution,
                   artifacts=raw, errors=errors, producer_completed=not errors, qualified=False)
    persist(output / 'receipt.json', receipt)
    return receipt


def verify_receipt(path, kind, candidate, *, expected_regression_failure=False):
    path = Path(path).resolve()
    receipt = json.loads(path.read_text())
    if not isinstance(receipt, dict) or type(receipt.get('schema_version')) is not int or receipt.get('schema_version') != 1 or receipt.get('kind') != kind or receipt.get('candidate_manifest_sha256') != candidate['sha256']:
        raise ValueError('original-run receipt identity mismatch: ' + kind)
    preflight = json.loads(read_record(receipt.get('preflight')).read_text())
    if not isinstance(preflight, dict):
        raise ValueError('preflight evidence must be an object')
    if preflight.get('candidate_manifest_sha256') != candidate['sha256'] or preflight.get('kind') != kind:
        raise ValueError('preflight candidate mismatch')
    if preflight.get('source_tree_sha256') != candidate['record']['source_tree_sha256']:
        raise ValueError('preflight source snapshot differs from candidate')
    if read_record(preflight.get('candidate')) != candidate['path']:
        raise ValueError('preflight bound another candidate')
    read_record(preflight.get('generation_gate'))
    if read_record(preflight.get('producer_source')) != ROOT / 'scripts/completion_execution.py':
        raise ValueError('receipt producer differs')
    expected = command(kind, candidate, preflight['generation_gate']['path'], path.parent)
    execution = receipt.get('execution')
    if not isinstance(execution, dict):
        raise ValueError('producer execution evidence must be an object')
    if preflight.get('argv') != expected or execution.get('argv') != expected:
        raise ValueError('producer command was substituted')
    if expected_regression_failure:
        valid_outcome = (kind == 'booking-regression' and receipt.get('producer_completed') is False
                         and receipt.get('errors') == ['producer execution or cleanup failed']
                         and type(execution.get('returncode')) is int and execution['returncode'] > 0)
    else:
        valid_outcome = (receipt.get('producer_completed') is True and receipt.get('errors') == []
                         and type(execution.get('returncode')) is int and execution['returncode'] == 0)
    if not valid_outcome or execution.get('timed_out') is not False or 'error' not in execution or execution.get('error') is not None or execution.get('cleanup_complete') is not True:
        raise ValueError('original producer did not complete cleanly')
    artifacts = receipt.get('artifacts')
    if not isinstance(artifacts, list) or not artifacts:
        raise ValueError('producer artifact inventory must be a nonempty list')
    paths = [read_record(item) for item in artifacts]
    if len(paths) != len(set(paths)) or not paths or any(path.parent not in item.parents for item in paths):
        raise ValueError('duplicate or foreign producer artifact')
    actual = {item.resolve() for item in path.parent.rglob('*') if item.is_file() and item.resolve() != path}
    if set(paths) != actual:
        raise ValueError('producer artifact inventory changed')
    if kind == 'security':
        # Revalidate live bundle state when consuming original evidence. The
        # preparation map also binds every sidecar into final candidate hashes.
        from security_prebuilt import verify_consumed_security
        verify_consumed_security(candidate)
    if kind == 'architecture':
        from architecture_prebuilt import validate_preparation
        preparation = validate_preparation(candidate)
        if preparation['generation_gate'] != preflight['generation_gate']:
            raise ValueError('architecture receipt generation declaration differs from preparation')
    return receipt

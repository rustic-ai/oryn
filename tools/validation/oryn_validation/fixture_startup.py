"""Bound topology admission handshake and complete fixture-owner settlement."""
import json
import os
from pathlib import Path
import re
import time

from .inputs import snapshot_input
from .manifests import manifest_digest

NONCE = 'ORYN_TOPOLOGY_ADMISSION_NONCE'
BIND_SECONDS = 30
TOPOLOGY_LIFETIME_SECONDS = 600


def _read(path):
    if Path(path).is_symlink():
        raise ValueError('fixture readiness receipt cannot be a symlink')
    return json.loads(Path(path).read_text())


def _write(path, value):
    path = Path(path)
    if path.exists() or path.is_symlink():
        raise ValueError('fixture readiness receipt already exists')
    temporary = path.with_name(path.name + '.pending')
    with temporary.open('x') as stream:
        json.dump(value, stream, sort_keys=True)
        stream.write('\n')
    os.link(temporary, path)  # Atomic, and never replaces a preexisting signal.
    temporary.unlink()


def _inputs_digest(bound):
    # Retained callers may also hold later-stage evidence; only this original's
    # exact declared input population plus its own manifest belongs here.
    run_record = snapshot_input(bound['store_root'],
                                str(Path(bound['path']).relative_to(bound['store_root'])))
    return manifest_digest([*bound['run']['immutable_inputs'], run_record])


def publish_admission(bound, lease):
    """Called only inside the admitted service InputLease, before serve.run."""
    from .processes import supervisor
    nonce = os.environ.get(NONCE, '')
    if not re.fullmatch('[0-9a-f]{64}', nonce):
        raise ValueError('topology admission nonce missing or malformed')
    lease.checkpoint()
    if not lease.admission or lease.admission.get('inputs_sha256') != _inputs_digest(bound):
        raise ValueError('topology service admission custody differs')
    identity = supervisor().process_identity(os.getpid())
    if identity is None:
        raise ValueError('topology service birth unavailable')
    value = {'schema_version': 1, 'kind': 'topology-service-admitted',
             'run_manifest_sha256': bound['sha256'], 'inputs_sha256': _inputs_digest(bound),
             'nonce': nonce, 'child_pid': os.getpid(), 'child_identity': identity,
             'admitted_monotonic_ns': time.monotonic_ns()}
    path = Path(bound['store_root']) / bound['run']['outputs']['results'] / 'services/upstream/admission-ready.json'
    _write(path, value)
    return value


def service_deadline(preflight):
    started, timeout = preflight.get('started_monotonic_ns'), preflight.get('timeout_seconds')
    if (type(started) is not int or started <= 0 or type(timeout) not in (int, float)
            or timeout != TOPOLOGY_LIFETIME_SECONDS):
        raise ValueError('topology service monotonic lifetime missing or invalid')
    return started + int(timeout * 1_000_000_000)


def verify_admission(bound, value, preflight, *, nonce=None):
    deadline = service_deadline(preflight)
    birth = preflight.get('child_identity')
    if (not isinstance(birth, list) or len(birth) != 2 or not isinstance(birth[0], list)
            or len(birth[0]) not in (1, 2) or type(birth[1]) is not int
            or birth[1] != preflight.get('owner_pid')):
        raise ValueError('topology service preflight birth identity missing')
    if (value.get('schema_version') != 1 or value.get('kind') != 'topology-service-admitted'
            or value.get('run_manifest_sha256') != snapshot_input(bound['store_root'],
                str(Path(bound['path']).relative_to(bound['store_root'])))['sha256']
            or value.get('inputs_sha256') != _inputs_digest(bound)
            or not re.fullmatch('[0-9a-f]{64}', value.get('nonce', ''))
            or (nonce is not None and value['nonce'] != nonce)
            or value.get('child_pid') != preflight.get('child_pid')
            or value.get('child_identity') != preflight.get('child_identity')
            or type(value.get('child_pid')) is not int or value['child_pid'] <= 0
            or type(value.get('admitted_monotonic_ns')) is not int
            or not preflight['started_monotonic_ns'] <= value['admitted_monotonic_ns'] <= deadline - BIND_SECONDS * 1_000_000_000):
        raise ValueError('topology service admission signal differs from owned original')
    return deadline


def await_admission(bound, service, lease, nonce, *, started_ns, timeout_seconds):
    """Admission and bind share the original service lifetime; no budget reset."""
    root = Path(service.output)
    fallback_deadline = started_ns + int((timeout_seconds - BIND_SECONDS) * 1_000_000_000)
    while True:
        lease.checkpoint()
        service.check()
        preflight_path = root / 'preflight.json'
        preflight = _read(preflight_path) if preflight_path.exists() else None
        if preflight and (preflight.get('argv') != service.argv
                or preflight.get('cwd') != str(service.cwd)
                or preflight.get('timeout_seconds') != timeout_seconds
                or not started_ns <= preflight.get('started_monotonic_ns', 0) <= time.monotonic_ns()):
            raise ValueError('topology admission preflight differs from selected service')
        deadline = service_deadline(preflight) - BIND_SECONDS * 1_000_000_000 if preflight else fallback_deadline
        ready = root / 'admission-ready.json'
        if ready.exists() or ready.is_symlink():
            if not preflight:
                raise ValueError('topology admission appeared without owned preflight')
            value = _read(ready)
            overall = verify_admission(bound, value, preflight, nonce=nonce)
            service.check()
            if time.monotonic_ns() >= overall:
                raise TimeoutError('topology service lifetime exhausted during admission')
            return overall
        if time.monotonic_ns() >= deadline:
            raise TimeoutError('topology service admission did not complete within original lifetime')
        time.sleep(.05)


def publish_bound(service, *, nonce, started_ns, ports):
    value = {'schema_version': 1, 'kind': 'topology-service-bound', 'nonce': nonce,
             'bind_started_monotonic_ns': started_ns, 'bound_monotonic_ns': time.monotonic_ns(),
             'ports': sorted(set(ports))}
    _write(Path(service.output) / 'bind-ready.json', value)


def verify_startup(bound, service_root, ports):
    root = Path(service_root)
    preflight = _read(root / 'preflight.json')
    admitted = _read(root / 'admission-ready.json')
    deadline = verify_admission(bound, admitted, preflight)
    bound_receipt = _read(root / 'bind-ready.json')
    start, finish = bound_receipt.get('bind_started_monotonic_ns'), bound_receipt.get('bound_monotonic_ns')
    if (bound_receipt.get('schema_version') != 1 or bound_receipt.get('kind') != 'topology-service-bound'
            or bound_receipt.get('nonce') != admitted['nonce']
            or bound_receipt.get('ports') != sorted(set(ports))
            or type(start) is not int or type(finish) is not int
            or not admitted['admitted_monotonic_ns'] <= start <= finish <= min(deadline, start + BIND_SECONDS * 1_000_000_000)):
        raise ValueError('topology bind phase differs from admitted bounded service')
    return bound_receipt


def settlement(process, services, *, process_attempted):
    """A failed original may release resources only after proven clean teardown."""
    errors, rows = [], []
    if process is None:
        if process_attempted:
            errors.append('collector/upstream launch attempted without terminal result')
    elif process.get('cleanup_complete') is not True or process.get('cleanup_errors') != [] or process.get('forced_cleanup'):
        errors.append('collector/upstream cleanup incomplete or forced')
    for service in services:
        result = service.result
        rows.append(result)
        try:
            thread = service.thread
            if (thread is not None and thread.is_alive()) or service.failure is not None or not isinstance(result, dict):
                raise ValueError('service monitor not terminal')
            if result.get('cleanup_complete') is not True or result.get('cleanup_errors') != [] or result.get('forced_cleanup'):
                raise ValueError('service cleanup incomplete or forced')
            if _read(Path(service.output) / 'service-result.json') != result:
                raise ValueError('service terminal receipt missing or differs')
            raw = dict(result); raw.pop('service_stop_requested', None)
            if _read(Path(service.output) / 'process-result.json') != raw:
                raise ValueError('service canonical process receipt differs')
        except (OSError, ValueError, TypeError, AttributeError) as error:
            errors.append(str(service.output) + ': ' + str(error))
    return {'schema_version': 1, 'kind': 'fixture-resource-settlement',
            'process_attempted': process_attempted, 'process_result_present': process is not None,
            'fixture_services': rows, 'cleanup_complete': not errors, 'cleanup_errors': errors}


def require_settlement(value, primary=None):
    if value['cleanup_complete'] is not True:
        error = ValueError('fixture resource settlement failed: ' + '; '.join(value['cleanup_errors']))
        if primary is not None:
            primary.add_note(str(error))
        else:
            raise error


def collector_budget(services):
    deadlines = {Path(service.output).name: service_deadline(_read(Path(service.output) / 'preflight.json'))
                 for service in services}
    if not deadlines:
        raise ValueError('topology collector has no owned service lifetime')
    observed = time.monotonic_ns()
    timeout = (min(deadlines.values()) - observed) / 1_000_000_000
    if timeout <= 0:
        raise TimeoutError('topology service lifetime exhausted before collector')
    return {'schema_version': 1, 'kind': 'topology-collector-budget',
            'observed_monotonic_ns': observed, 'service_deadlines_ns': deadlines, 'timeout_seconds': timeout}


def verify_collector_budget(output, preflight, process):
    root = Path(output)
    value = _read(root / 'collector-budget.json')
    deadlines = {path.parent.name: service_deadline(_read(path))
                 for path in (root / 'services').glob('*/preflight.json')}
    observed = value.get('observed_monotonic_ns')
    timeout = value.get('timeout_seconds')
    started = preflight.get('started_monotonic_ns')
    elapsed = process.get('elapsed_ns')
    if (value.get('schema_version') != 1 or value.get('kind') != 'topology-collector-budget'
            or not deadlines or value.get('service_deadlines_ns') != deadlines
            or type(observed) is not int or type(started) is not int
            or not observed <= started < min(deadlines.values())
            or type(elapsed) is not int or elapsed < 0 or started + elapsed > min(deadlines.values())
            or type(timeout) not in (int, float) or not 0 < timeout <= 600
            or timeout != (min(deadlines.values()) - observed) / 1_000_000_000
            or preflight.get('timeout_seconds') != timeout):
        raise ValueError('topology collector budget differs from remaining service lifetime')
    return value

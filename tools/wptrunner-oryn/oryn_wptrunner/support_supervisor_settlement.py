"""Original-run settlement, invoked only by the canonical WPT finalizer.

The bound input object is returned by load_inputs while its InputLease is held.
No caller-supplied success flag or arbitrary observation file is accepted.
This adds a separate receipt; it never rewrites the original report/subtests.
"""
import hashlib
import json
from pathlib import Path

from .support_observation_join import PARENT, digest_object, join


def _digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _read(path, limit):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > limit:
        raise ValueError('original support custody artifact unavailable or oversized')
    with path.open('rb') as stream:
        value = json.load(stream)
    if not isinstance(value, dict):
        raise ValueError('original support custody artifact is not an object')
    return value


def settle_original(bound, output, descriptor):
    """Join the exact original report, typed execution, process and protocol.

    Called after canonical final input verification and protocol_custody, not
    from a runtime/author callback. Paths are derived from the bound run only.
    The ordinary WPT exit 1 remains valid custody, not successful assertions.
    """
    from wpt_execution_evidence import protocol_custody

    output = Path(output).absolute()
    # The finalizer owns resolution of this immutable run. Recheck its bytes;
    # a similarly-shaped saved execution dictionary cannot select the run.
    if _digest(Path(bound['path'])) != bound['sha256']:
        raise ValueError('support run contract changed')
    run = bound['run']
    declared = Path(bound['store_root']) / run['outputs']['results']
    if output != declared.absolute():
        raise ValueError('support settlement is outside the original run')
    execution = _read(output / 'execution.json', 16 * 1024 * 1024)
    candidate = run['build']['manifest_sha256']
    if (execution.get('schema_version') != 1
            or execution.get('kind') != 'typed-wpt-execution'
            or execution.get('run_id') != run['run_id']
            or execution.get('run_manifest_sha256') != bound['sha256']
            or execution.get('candidate_sha256') != candidate
            or execution.get('profile') != run['profile']
            or execution.get('custody_complete') is not True
            or execution.get('errors') != []):
        raise ValueError('support original execution identity or custody mismatch')
    selected = ('wptreport.json', 'protocol-custody.jsonl',
                'process/preflight.json', 'process/process-result.json')
    hashes = {}
    for name in selected:
        path = output / name
        # Check every path component, including the process directory.
        if any(part.is_symlink() for part in (path, *path.parents)):
            raise ValueError('support original artifact traverses a symlink')
        hashes[name] = _digest(path)
        if execution.get('artifacts', {}).get(name) != hashes[name]:
            raise ValueError('support artifact not bound to original execution')
    process = _read(output / 'process/process-result.json', 16 * 1024 * 1024)
    preflight = _read(output / 'process/preflight.json', 1024 * 1024)
    birth = preflight.get('child_identity')
    # Canonical process_identity is (birth tuple, parent PID), not two scalar
    # integers. Retain the complete birth and owner join from the actual launch.
    if (not isinstance(birth, list) or len(birth) != 2
            or not isinstance(birth[0], list) or len(birth[0]) not in (1, 2)
            or any(type(value) is not int or value < 0 for value in birth[0])
            or birth[1] != preflight.get('owner_pid')
            or type(preflight.get('child_pid')) is not int or preflight['child_pid'] <= 0
            or not isinstance(process.get('argv'), list) or not process['argv']
            or process['argv'] != preflight.get('argv')
            or preflight.get('cwd') != str(bound['assets']['wpt_checkout'])):
        raise ValueError('support original process birth or launch mismatch')
    for record, error_key in ((execution, 'launch_error'), (process, 'error')):
        if (type(record.get('returncode')) is not int or record['returncode'] not in (0, 1)
                or record.get('cleanup_complete') is not True
                or record.get('timed_out') is not False or record.get(error_key) is not None):
            raise ValueError('support original process is incomplete')
    if process.get('cleanup_errors') != [] or process['returncode'] != execution['returncode']:
        raise ValueError('support original process cleanup or terminal result mismatch')
    protocol_custody(output / 'protocol-custody.jsonl', candidate)
    report = _read(output / 'wptreport.json', 128 * 1024 * 1024)
    rows = [row for row in report.get('results', [])
            if isinstance(row, dict) and row.get('test') == PARENT]
    if len(rows) != 1:
        raise ValueError('support original parent result missing or duplicated')
    row = rows[0]
    observation = row.get('extra', {}).get('oryn_support_resource')
    if not isinstance(observation, dict):
        raise ValueError('support native observation missing from original parent result')
    # Reconstruct identity/multiplicity from the original report, not saved
    # capture-complete flags. The native snapshot remains separately bound by
    # its original report hash and the authenticated driver capture.
    assertions = row.get('subtests')
    if not isinstance(assertions, list):
        raise ValueError('support original subtest list missing')
    original = [{'name': item['name'], 'status': item['status']} for item in assertions]
    reconstructed = join(descriptor, observation.get('native'), row.get('status'),
                         original, candidate=candidate, process_complete=True)
    result = dict(observation)
    valid = (row.get('status') == 'OK' and result.get('parent') == PARENT
             and result.get('descriptor_sha256') == digest_object(descriptor)
             and result.get('native', {}).get('candidate') == candidate
             and result.get('native_observation_complete') is True
             and reconstructed.get('complete') is True
             and reconstructed.get('assertions') == result.get('assertions'))
    result.update(complete=valid, process_custody_complete=True,
                  qualification='not-evaluated', run_id=run['run_id'],
                  run_manifest_sha256=bound['sha256'], candidate_sha256=candidate,
                  original_artifacts=hashes, original_process_birth=birth,
                  original_child_pid=preflight['child_pid'])
    if not valid:
        result['reason'] = 'original-parent-or-native-observation-incomplete'
    # Recheck the selected immutable originals after parsing/protocol validation.
    if any(_digest(output / name) != value for name, value in hashes.items()):
        raise ValueError('support original artifacts changed during settlement')
    return result

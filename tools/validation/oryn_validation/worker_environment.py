"""Select the sealed worker explicitly; installation-layout fallback is not custody."""
import os
from pathlib import Path

from .inputs import resolve_input
from .manifests import validate_input

VARIABLE = 'ORYN_PAGE_WORKER'


def worker_binding(run, bundle_root):
    """Pure receipt projection, also usable against retained original roots."""
    record = run.get('assets', {}).get('worker')
    validate_input(record)
    if (record['scope'] != 'bundle' or record['kind'] != 'file'
            or record['executable'] is not True or record not in run['immutable_inputs']):
        raise ValueError('worker must be an executable sealed-bundle immutable input')
    root = Path(bundle_root)
    if not root.is_absolute():
        raise ValueError('worker requires an absolute declared bundle root')
    return {'variable': VARIABLE, 'path': str(root / record['path']),
            'input': record, 'build': run['build']}


def selected_worker(bound):
    """Join the resolved role to its receipt under the caller's active InputLease.

    Production admission verifies the app/signature/manifest relationship;
    InputLease owns full byte verification and each launch-boundary drift check.
    This check supplies no alternative worker discovery or signature policy.
    """
    receipt = worker_binding(bound['run'], bound['bundle_root'])
    path = resolve_input(bound['bundle_root'], receipt['input']['path'])
    if (path != bound['assets']['worker'] or str(path) != receipt['path']
            or not path.is_file() or not path.stat().st_mode & 0o111):
        raise ValueError('worker path differs from executable sealed-bundle input')
    return receipt


def worker_environment(bound):
    receipt = selected_worker(bound)
    return {VARIABLE: receipt['path']}


def assert_worker_environment(bound, environment=None):
    """Child and restart boundaries reject inherited substitution or fallback."""
    receipt = selected_worker(bound)
    environment = os.environ if environment is None else environment
    if environment.get(VARIABLE) != receipt['path']:
        raise ValueError('worker environment differs from selected sealed-bundle input')
    return receipt


def verify_worker_binding(receipt, run, original_bundle_root):
    if receipt != worker_binding(run, original_bundle_root):
        raise ValueError('recorded worker environment differs from original selected input')

"""Validate captured wire identities without upgrading historical evidence."""
from native_protocol_versions import runtime_matches

FIELDS = ('worker_protocol_version', 'context_service_protocol_version', 'agent_ipc_protocol_version')


def validate_protocols(value, root, *, fingerprint=False, historical=False):
    if historical and value is None:
        return
    if not isinstance(value, dict):
        raise ValueError('missing protocol identity object')
    captured = dict(value)
    if fingerprint:
        captured['worker_protocol_version'] = captured.get('protocol_version')
    if historical:
        # Absence stays absence. Historical admission never supplies current values.
        for field in FIELDS:
            if captured.get(field) is not None and (type(captured[field]) is not int or captured[field] <= 0):
                raise ValueError('invalid historical protocol identity: ' + field)
    elif not runtime_matches(captured, root):
        raise ValueError('captured worker/context/agent protocols do not match current source')


def validate_protocol_pair(runtime, fingerprint, root, *, historical=False):
    if not isinstance(runtime, dict) or not isinstance(fingerprint, dict):
        raise ValueError("raw protocol comparison requires both captured objects")
    validate_protocols(runtime, root, historical=historical)
    validate_protocols(fingerprint, root, fingerprint=True, historical=historical)
    for field in FIELDS:
        compact_field = 'protocol_version' if field == 'worker_protocol_version' else field
        if runtime.get(field) != fingerprint.get(compact_field):
            raise ValueError('runtime/fingerprint protocol disagreement: ' + field)

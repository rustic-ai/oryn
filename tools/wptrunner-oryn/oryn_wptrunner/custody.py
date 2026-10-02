"""Private supervisor log for native session connection and checked teardown."""
import json
import os
import secrets
import stat

# Shared with the exact runner generation, for a terminal failure record if
# that process exits before its own finally block. Never reused by a successor.
runner_session = None


def _private(descriptor):
    metadata = os.fstat(descriptor)
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
            or stat.S_IMODE(metadata.st_mode) != 0o600 or metadata.st_nlink != 1):
        raise ValueError('protocol custody log lacks private supervisor ownership')


def create(path):
    """Create one new supervisor-owned log, independent of the caller's umask.

    Never chmod/reuse an existing destination. The owner admits its output
    directory separately; O_EXCL and O_NOFOLLOW reject final-path substitution.
    """
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        _private(descriptor)
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def append(session, phase, error=None):
    path = os.environ.get('ORYN_WPT_PROTOCOL_CUSTODY')
    candidate = os.environ.get('ORYN_WPT_CANDIDATE_SHA256', '')
    if not path or len(candidate) != 64 or any(character not in '0123456789abcdef' for character in candidate):
        raise ValueError('official protocol custody configuration missing')
    record = {'schema_version': 1, 'candidate_sha256': candidate, 'session': session, 'phase': phase,
              'success': error is None, 'error_type': None if error is None else type(error).__name__}
    # No credentials, page content or error messages enter this supervisor log.
    encoded = (json.dumps(record, sort_keys=True) + '\n').encode()
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
    try:
        _private(descriptor)
        if os.write(descriptor, encoded) != len(encoded):
            raise OSError('incomplete protocol custody append')
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    if runner_session is not None and phase in ('teardown', 'connect-failed'):
        if runner_session.value == session.encode('ascii'):
            runner_session.value = b''


def begin():
    identity = secrets.token_hex(16)
    if runner_session is not None:
        runner_session.value = identity.encode('ascii')
    append(identity, 'connect')
    return identity


def finish_abandoned(session):
    # Only called after the exact runner has exited. Resolve the write/clear
    # race without manufacturing a second terminal record or a connection.
    with open(os.environ['ORYN_WPT_PROTOCOL_CUSTODY']) as stream:
        phases = [row['phase'] for line in stream
                  if (row := json.loads(line)).get('session') == session]
    if phases == ['connect']:
        append(session, 'teardown', RuntimeError('runner exited without protocol teardown'))

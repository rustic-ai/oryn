"""Private supervisor log for native session connection and checked teardown."""
import json
import os
import secrets
import stat


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
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise ValueError('protocol custody log lacks private supervisor ownership')
        if os.write(descriptor, encoded) != len(encoded):
            raise OSError('incomplete protocol custody append')
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def begin():
    identity = secrets.token_hex(16)
    append(identity, 'connect')
    return identity

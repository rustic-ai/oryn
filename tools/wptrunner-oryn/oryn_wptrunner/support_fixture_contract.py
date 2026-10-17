"""Select the exact pinned observation descriptor from admitted fixture inputs."""
import hashlib
import json
from pathlib import Path
from .support_observation_join import (
    REVISION, PARENT, CHILD, PARENT_HASH, CHILD_HASH, PINNED_IMPORTS, PINNED_NAMES,
    digest_object,
)


def pinned_descriptor():
    return {'schema': 1, 'revision': REVISION, 'parent': PARENT,
            'parent_sha256': PARENT_HASH, 'child': CHILD, 'child_sha256': CHILD_HASH,
            'imports': [dict(row) for row in PINNED_IMPORTS], 'assertions': list(PINNED_NAMES)}


def admitted_descriptor(fixture, fixture_path, candidate):
    record = fixture.get('parent_support')
    if record is None:
        return None
    if not isinstance(record, dict) or set(record) != {'path', 'sha256'}:
        raise ValueError('invalid admitted support fixture artifact')
    relative = Path(record['path'])
    if relative.is_absolute() or '..' in relative.parts:
        raise ValueError('support fixture artifact escapes immutable closure')
    path = Path(fixture_path).parent / relative
    if any(part.is_symlink() for part in (path, *path.parents)) or not path.is_file():
        raise ValueError('support fixture artifact is not an anchored regular file')
    if path.stat().st_size > 1024 * 1024:
        raise ValueError('support fixture artifact exceeds existing artifact bound')
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != record['sha256']:
        raise ValueError('support fixture artifact changed')
    contract = json.loads(data)
    descriptor = pinned_descriptor()
    if (contract.get('schema') != 1 or contract.get('revision') != REVISION
            or contract.get('candidate') != candidate
            or contract.get('descriptor') != digest_object(descriptor)):
        raise ValueError('support fixture differs from admitted candidate/pinned closure')
    return descriptor

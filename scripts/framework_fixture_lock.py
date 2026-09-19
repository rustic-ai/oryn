"""Validate the reviewed framework lock revision without rewriting history."""
import hashlib
import json
from pathlib import Path

V1_SHA256 = 'a7f4e8ef33279c2923a570767f9fd978072f982b13a9fe4665c4b81330c386be'
V2_SHA256 = '836b1a7620e590a6a2177960dce18f12081ca1155c20bc1d0e19932e1a6d7410'
ADMISSION = b'\n# Full-plan generation admission, including direct invocation.\npython3 "$(cd "$(dirname "$0")" && pwd)/generation_gate.py"\n'
CHANGES = {
    'controlled_regression_validator': 'validate-app-controlled-regressions.sh',
    'framework_validator': 'validate-frameworks.sh',
    'oauth_validator': 'validate-auth-flows.sh',
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def validate(root):
    root = Path(root)
    folder = root / 'benchmarks/app-compatibility'
    original = (folder / 'fixtures-v1.lock.json').read_bytes()
    previous = (folder / 'fixtures-v2.lock.json').read_bytes()
    if digest(original) != V1_SHA256 or digest(previous) != V2_SHA256:
        raise ValueError('historical framework fixture lock changed')
    baseline = json.loads(previous)
    current = json.loads((folder / 'fixtures.lock.json').read_bytes())
    if (baseline.get('revision') != 2 or baseline.get('previous_lock') != 'fixtures-v1.lock.json'
            or baseline.get('previous_lock_sha256') != V1_SHA256
            or current.get('revision') != 3 or current.get('previous_lock') != 'fixtures-v2.lock.json'
            or current.get('previous_lock_sha256') != V2_SHA256):
        raise ValueError('framework fixture lock revision chain changed')
    allowed = {'revision', 'revision_reason', 'previous_lock', 'previous_lock_sha256', 'harness'}
    if ({key: value for key, value in current.items() if key not in allowed}
            != {key: value for key, value in baseline.items() if key not in allowed}):
        raise ValueError('framework fixture behavior or dependency population changed')
    expected_harness = dict(baseline['harness'])
    for key, name in CHANGES.items():
        data = (root / 'scripts' / name).read_bytes()
        if data.count(ADMISSION) != 1 or digest(data.replace(ADMISSION, b'', 1)) != baseline['harness'][key]:
            raise ValueError('unreviewed framework harness change: ' + name)
        expected_harness[key] = digest(data)
    if current.get('harness') != expected_harness:
        raise ValueError('framework harness lock differs from reviewed generation admission')
    return current


if __name__ == '__main__':
    import sys
    validate(sys.argv[1])

"""Read-only, shared full-plan source-generation barrier.

A child checklist row is not a parent requirement completion declaration. All 51
parent IDs and their exact implementation/regression sources must be present.
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


REQUIREMENTS = {f'{package}-{number:02}' for package, count in
                [('G', 8), ('R', 7), ('P', 6), ('L', 5), ('W', 5), ('S', 5), ('V', 5), ('F', 4), ('Q', 6)]
                for number in range(1, count + 1)}


def generation_gate(path, root=None):
    root = Path(root) if root is not None else ROOT
    path = Path(path)
    declaration = path.read_bytes()
    declaration_sha256 = hashlib.sha256(declaration).hexdigest()
    gate = json.loads(declaration)
    plan = root / 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'
    if gate.get('schema_version') != 1 or gate.get('plan_sha256') != sha256(plan):
        raise ValueError('generation gate does not bind the current full completion plan')
    requirements = gate.get('requirements', {})
    if set(requirements) != REQUIREMENTS:
        raise ValueError('all 51 generation requirements must be accounted for')
    for identifier, item in requirements.items():
        if item.get('implementation_generated') is not True or item.get('tests_generated') is not True:
            raise ValueError('generation incomplete: ' + identifier)
        for field in ('implementation_sources', 'test_sources'):
            records = item.get(field) or []
            if not records:
                raise ValueError('generation source evidence missing: ' + identifier + ':' + field)
            for record in records:
                source = (root / record['path']).resolve()
                if root.resolve() not in source.parents or not source.is_file() or sha256(source) != record['sha256']:
                    raise ValueError('generation source evidence changed: ' + identifier)
    if sha256(path) != declaration_sha256:
        raise ValueError('generation declaration changed during source validation')
    return gate


class GenerationLease:
    """Freeze admission before setup and permanently reject observed drift."""
    def __init__(self, path, validator=None, expected=None):
        import threading
        self.path = Path(path).resolve()
        self.validator = validator or generation_gate
        try:
            self.sha256 = sha256(self.path) if expected is None else expected
        except OSError:
            self.validator(self.path)
            raise
        self.failure = None
        self.lock = threading.RLock()
        self.check()

    def check(self):
        with self.lock:
            if self.failure is not None:
                raise ValueError('generation admission revoked: ' + self.failure)
            try:
                if sha256(self.path) != self.sha256:
                    raise ValueError('generation declaration changed')
                self.validator(self.path)
                if sha256(self.path) != self.sha256:
                    raise ValueError('generation declaration changed')
            except Exception as error:
                self.failure = str(error)
                raise
        return self

    def environment(self, base=None):
        import os
        self.check()
        return dict(os.environ if base is None else base,
                    ORYN_GENERATION_GATE=str(self.path),
                    ORYN_GENERATION_GATE_SHA256=self.sha256)


def inherited_generation_lease():
    import os
    path = os.environ.get('ORYN_GENERATION_GATE')
    expected = os.environ.get('ORYN_GENERATION_GATE_SHA256')
    if not path or not expected:
        raise ValueError('execution requires the complete generation declaration and frozen digest')
    return GenerationLease(path, expected=expected)


if __name__ == '__main__':
    # Shell children inherit the exact declaration, never a boolean bypass.
    inherited_generation_lease().check()


_execution_lease = None
_execution_failure = None
_execution_lock = __import__('threading').RLock()


def check_execution_generation():
    """Admission for nested executors; observed revocation cannot be reset."""
    global _execution_lease, _execution_failure
    with _execution_lock:
        if _execution_failure is not None:
            raise ValueError('generation admission revoked: ' + _execution_failure)
        try:
            if _execution_lease is None:
                _execution_lease = inherited_generation_lease()
            return _execution_lease.check()
        except Exception as error:
            _execution_failure = str(error)
            raise

"""Exact staged socat identity and fresh macOS/Homebrew dependency custody.

The coordinator copies the regular executable to a new payload/bin/socat first.
This module observes files and Mach-O load commands; it never executes the relay.
OS shared-cache names are bound to macOS build, while Homebrew loads retain both
their selected paths and resolved byte identities. No dylib is silently copied.
"""
from pathlib import Path

from . import environments
from .generation import new_store_path, _write_new
from .inputs import resolve_input, snapshot_input, verify_input
from .manifests import manifest_digest, validate_input


def _payload(identity, store_root):
    keys = {'schema_version', 'kind', 'tool', 'environment', 'executable', 'system'}
    if (not isinstance(identity, dict) or set(identity) != keys
            or type(identity['schema_version']) is not int or identity['schema_version'] != 1
            or identity['kind'] != 'fixture-relay-environment' or identity['tool'] != 'socat'):
        raise ValueError('fixture relay identity schema differs')
    for name, kind in (('environment', 'directory'), ('executable', 'file')):
        record = validate_input(identity[name])
        if record['scope'] != 'store' or record['kind'] != kind:
            raise ValueError('fixture relay asset scope/kind differs')
        verify_input(record, store_root=store_root)
    directory, executable = identity['environment'], identity['executable']
    if (executable['path'] != directory['path'] + '/bin/socat'
            or not executable['executable'] or directory['directories'] != ['bin']
            or directory['members'] != [{key: value for key, value in executable.items()
                                         if key not in {'scope', 'kind', 'path'}} | {'path': 'bin/socat'}]):
        raise ValueError('fixture relay payload must contain exactly regular executable bin/socat')
    root = resolve_input(store_root, directory['path'])
    binary = resolve_input(store_root, executable['path'])
    if not environments._macho(binary):
        raise ValueError('fixture relay must be a macOS Mach-O executable')
    return root, binary


def verify_relay(identity, store_root):
    """Verify at admission and after all owned relay descendants have settled."""
    root, binary = _payload(identity, store_root)
    observed = environments._system_libraries(root, native_paths=[binary])
    if observed != identity['system']:
        raise ValueError('fixture relay macOS/Homebrew dependency identity changed')
    # Detect executable/membership changes during the external dependency walk.
    _payload(identity, store_root)
    return {'valid': True, 'identity_sha256': manifest_digest(identity)}


def prepare_relay_identity(store_root, *, executable_path, identity_path):
    """Create a new identity outside an already staged, otherwise empty payload.

    Paths are store-relative. The returned three Input records must be explicitly
    registered, pinned and included in run assets/immutable_inputs by the caller.
    This is preparation evidence, not a relay protocol or readiness test.
    """
    executable = snapshot_input(store_root, executable_path)
    binary = resolve_input(store_root, executable_path)
    if binary.name != 'socat' or binary.parent.name != 'bin':
        raise ValueError('fixture relay staging path must end in bin/socat')
    root = binary.parent.parent
    store = Path(store_root).absolute()
    environment = snapshot_input(store, root.relative_to(store).as_posix(), kind='directory')
    destination = new_store_path(store, identity_path)
    if destination == root or destination.is_relative_to(root) or destination.exists():
        raise ValueError('new fixture relay identity must be outside its payload')
    identity = {'schema_version': 1, 'kind': 'fixture-relay-environment', 'tool': 'socat',
                'environment': environment, 'executable': executable, 'system': None}
    _payload(identity, store)
    identity['system'] = environments._system_libraries(root, native_paths=[binary])
    verify_relay(identity, store)
    _write_new(destination, identity)
    destination.chmod(0o444)
    return {'identity': identity, 'assets': {'fixture_relay': executable,
            'fixture_relay_environment': environment,
            'fixture_relay_identity': snapshot_input(store, identity_path)}}

"""Bind one selected production bundle to its exact registered build owner."""
from pathlib import Path

from .admission import read_object
from .inputs import resolve_input, verify_inputs
from .manifests import load_manifest, manifest_digest
from .retention import retention_lock, validate_retention_index


def registered_bundle_owner(bound):
    """Return the unique build object owning this manifest and its runtime tree.

    Build artifacts remain subject to the enclosing full production/InputLease
    verification. This check binds their declared identities and physical roots
    to retention ownership, including a published child of builds/<build_id>.
    """
    store = Path(bound['store_root']).absolute()
    bundle = Path(bound['bundle_root']).absolute()
    run = bound['run']
    manifest_record = run['assets'].get('build_manifest')
    if not manifest_record or manifest_record.get('kind') != 'file' or manifest_record.get('scope') != 'store':
        raise ValueError('build owner requires an explicit store build manifest')
    manifest = resolve_input(store, manifest_record['path'])
    if Path(bound['assets']['build_manifest']) != manifest:
        raise ValueError('resolved build manifest differs from selected asset')
    verify_inputs([manifest_record], store_root=store, bundle_root=bundle)
    build = load_manifest(manifest, 'build')
    if run['build'] != {'build_id': build['build_id'], 'manifest_sha256': manifest_digest(build)}:
        raise ValueError('registered bundle build identity differs from selected manifest')
    relative = bundle.relative_to(store)
    expected_prefix = ('builds', build['build_id'])
    if (relative.parts[:2] != expected_prefix or len(relative.parts) < 3
            or relative.name != 'runtime' or manifest.parent / 'runtime' != bundle
            or resolve_input(store, relative.as_posix()) != bundle):
        raise ValueError('bundle must be the selected build manifest sibling runtime directory')
    artifacts = build['artifacts'] + build['immutable_inputs']
    for name, record in run['assets'].items():
        if record['scope'] != 'bundle':
            continue
        if record not in artifacts:
            raise ValueError('bundle asset absent from selected build identity: ' + name)
        if Path(bound['assets'][name]) != resolve_input(bundle, record['path']):
            raise ValueError('bundle asset resolves outside selected runtime: ' + name)
    with retention_lock(store):
        index = read_object(store / 'retention.json')
        objects = validate_retention_index(index)
        owners = [(name, item) for name, item in objects.items()
                  if relative.as_posix() == item['path'] or relative.as_posix().startswith(item['path'] + '/')]
        if len(owners) != 1:
            raise ValueError('bundle needs one unambiguous registered retention owner')
        identifier, owner = owners[0]
        owner_path = resolve_input(store, owner['path'])
        if (owner['kind'] != 'build' or 'removed_ns' in owner
                or owner_path.relative_to(store).parts[:2] != expected_prefix
                or not manifest.is_relative_to(owner_path)):
            raise ValueError('bundle owner does not own this selected build manifest')
        return identifier

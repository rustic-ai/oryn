"""Assemble one original successful native build into a sealed production bundle.

No build or browser process is started here. Original receipts are copied byte for byte;
manifest construction cannot turn preparation or failed builds into evidence.
"""
from pathlib import Path
import re

from . import builds, build_evidence, production, retention
from .generation import new_store_path, _write_new
from .inputs import InputLease, resolve_input, snapshot_input, verify_inputs
from .manifests import manifest_digest, validate_manifest, _id
from .processes import ResourceLease


EXECUTABLE_PATHS = {'runtime': 'bin/oryn', 'driver': 'bin/oryn-webdriver',
                    'worker_app': 'OrynPageWorker.app',
                    'worker': 'OrynPageWorker.app/Contents/MacOS/oryn-page-worker',
                    'worker_manifest': 'OrynPageWorker.app/Contents/Resources/worker-manifest.json'}


def _unique(records):
    result = {}
    for record in records:
        key = record['scope'], record['path']
        if key in result and result[key] != record:
            raise ValueError('conflicting assembly input identities')
        result[key] = record
    return list(result.values())


def _index(store):
    path = resolve_input(store, 'retention.json')
    index = production._read(path)
    retention.validate_retention_index(index)
    return path, index


def _owner(index, relative, *, required=True):
    owners = [name for name, row in index['objects'].items()
              if relative == row['path'] or relative.startswith(row['path'] + '/')]
    if len(owners) != 1:
        if not owners and not required:
            return None
        raise ValueError('assembly input needs exactly one registered retention root: ' + relative)
    if 'removed_ns' in index['objects'][owners[0]]:
        raise ValueError('assembly cannot use a removed retention object')
    return owners[0]


def _owners(store, records):
    with retention.retention_lock(store):
        _, index = _index(store)
        return sorted({_owner(index, record['path']) for record in records if record['scope'] == 'store'})


def _pin_output(store, relative, build_id, dependencies):
    with retention.retention_lock(store):
        _, index = _index(store)
        owner = _owner(index, relative, required=False)
    if owner is None:
        owner = 'published-build:' + build_id
        retention.register_object(store, owner, path=relative, kind='build',
                                  dependencies=dependencies, pinned=True)
    with retention.retention_lock(store):
        path, index = _index(store)
        if _owner(index, relative) != owner:
            raise ValueError('published build retention ownership changed')
        selected = sorted((set(index['objects'][owner]['dependencies']) | set(dependencies)) - {owner})
        pending, seen = list(selected), set()
        while pending:
            name = pending.pop()
            if name == owner:
                raise ValueError('published build retention dependency cycle')
            if name not in seen:
                seen.add(name)
                pending.extend(index['objects'][name]['dependencies'])
        index['objects'][owner].update(pinned=True, dependencies=selected)
        retention._write_index(path, index)
    return owner


def assemble_build(source_root, store_root, *, execution_path, source_export,
                   source_identity, generation_gate, generation_review,
                   output_directory, build_id):
    """Paths are store-relative; configuration and outputs come from the receipt."""
    _id(build_id, 'build_id')
    root = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    destination = new_store_path(store, output_directory)
    if (destination.exists() or not output_directory.startswith('builds/' + build_id + '/')
            or destination == root or root in destination.parents):
        raise ValueError('assembly requires a new published directory under the selected build ID')
    execution_file = resolve_input(store, execution_path)
    if execution_file.name != 'build-execution.json':
        raise ValueError('assembly requires the original canonical build execution receipt')
    execution = production._read(execution_file)
    if (execution.get('schema_version') != 1 or execution.get('kind') != 'typed-build-execution'
            or execution.get('integrity_errors') != []
            or set(execution.get('outputs', {})) != build_evidence.ROLES):
        raise ValueError('original successful complete build execution required')
    source_paths = {'source_export': source_export, 'source_identity': source_identity,
                    'build_generation_gate': generation_gate, 'build_generation_review': generation_review,
                    'build_execution': execution_path}
    for role, filename in {'build_process': 'process-result.json', 'build_stdout': 'stdout.log',
                           'build_stderr': 'stderr.log'}.items():
        source_paths[role] = (execution_file.parent / 'process' / filename).relative_to(store).as_posix()
    original_records = {name: snapshot_input(store, path, kind='directory' if name == 'source_export' else 'file')
                        for name, path in source_paths.items()}
    original_assets = {name: resolve_input(store, path) for name, path in source_paths.items()}
    process = production._read(original_assets['build_process'])
    if not build_evidence._successful(process):
        raise ValueError('failed or unsettled native builds cannot be assembled')
    for role, record in execution['outputs'].items():
        if record['scope'] != 'store':
            raise ValueError('original build output must belong to the store')
        original_records[role] = record
        original_assets[role] = resolve_input(store, record['path'])
    typed_directory = execution['literal_environment']['ORYN_TYPED_RUNTIME_OUTPUT']
    typed_record = snapshot_input(store, typed_directory + '/typed-runtime.json')
    typed = production._read(resolve_input(store, typed_record['path']))
    if (typed.get('schema_version') != 1 or typed.get('kind') != 'typed-runtime-preparation'
            or typed.get('source') != execution['source'] or not isinstance(typed.get('assets'), dict)):
        raise ValueError('typed engine preparation does not bind the original native source')
    for name, record in typed['assets'].items():
        if name not in {*production.ENGINE_ASSETS.values(), 'engine_profile', 'bridge_build_descriptor'} and not re.fullmatch(r'bridge_input_[0-9]{5}', name):
            raise ValueError('unknown typed engine asset role')
        if (record['scope'] != 'store' or record['kind'] != 'file'
                or not record['path'].startswith(typed_directory + '/')):
            raise ValueError('typed engine asset is outside its selected preparation directory')
        if name in original_records and {
                key: value for key, value in record.items() if key not in {'scope', 'path'}} != {
                key: value for key, value in original_records[name].items() if key not in {'scope', 'path'}}:
            raise ValueError('typed native output differs from actual build output: ' + name)
        original_records[name] = record
        original_assets[name] = resolve_input(store, record['path'])
    input_records = _unique([*execution['build_inputs'], *original_records.values(), typed_record,
                             *execution['outputs'].values()])
    for record in input_records:
        path = resolve_input(store, record['path'])
        if destination == path or (record['kind'] == 'directory' and path in destination.parents):
            raise ValueError('assembly output cannot mutate a declared input directory')
    owners = _owners(store, input_records)
    resources = ['production-signing', 'build:' + build_id,
                 'cargo-cache:' + builds.cargo_cache_key(execution['configuration'])]
    with ResourceLease(store, resources, objects=owners) as lease:
        with InputLease(input_records, store_root=store):
            source_admission = production.validate_source_export(
                production._read(original_assets['source_identity']), original_assets['source_export'],
                original_records['source_export'])
            if {key: source_admission[key] for key in ('commit', 'tree_sha256')} != {
                    key: execution['source'][key] for key in ('commit', 'tree_sha256')}:
                raise ValueError('exported source differs from original native build')
            # Join the original process before allocating/copying a large bundle.
            initial_build = {'source': execution['source'], 'configuration': execution['configuration'],
                'build_inputs': execution['build_inputs'], 'commands': [execution['command']],
                'immutable_inputs': input_records}
            build_evidence.verify_build_evidence({'assets': original_assets,
                'run': {'assets': original_records}, 'records': input_records, 'store_root': store}, initial_build)
            destination.mkdir(parents=True)
            bundle = destination / 'runtime'
            files, directories = {}, []
            app = original_assets['worker_app']
            app_inventory = builds.bundle_inventory(app)
            directories = ['OrynPageWorker.app', *('OrynPageWorker.app/' + name for name in app_inventory['directories'])]
            for name in app_inventory['files']:
                files['OrynPageWorker.app/' + name] = app / name
            for role in ('runtime', 'driver'):
                files[EXECUTABLE_PATHS[role]] = original_assets[role]
            final_paths = dict(EXECUTABLE_PATHS)
            for role in typed['assets']:
                if role in final_paths:
                    raise ValueError('typed engine role collides with executable role')
                final_paths[role] = 'engine/' + role
                files[final_paths[role]] = original_assets[role]
            for role in ('build_execution', 'build_process', 'build_stdout', 'build_stderr',
                         'build_generation_gate', 'build_generation_review'):
                final_paths[role] = 'evidence/' + role
                files[final_paths[role]] = original_assets[role]
            signature = lambda folder: [builds.verify_worker_signature(root, folder,
                worker_path=EXECUTABLE_PATHS['worker'], manifest_path=EXECUTABLE_PATHS['worker_manifest'])]
            sealed = builds.seal_bundle(root, bundle, files, directories=directories,
                                        signature_verifier=signature)
            records = {name: snapshot_input(bundle, path, scope='bundle',
                         kind='directory' if name == 'worker_app' else 'file')
                       for name, path in final_paths.items()}
            # Manifest files live outside runtime, so complete sealed membership
            # can be an immutable dependency without a self-referential digest.
            records['runtime_bundle'] = snapshot_input(store, bundle.relative_to(store).as_posix(), kind='directory')
            records.update({name: original_records[name] for name in ('source_export', 'source_identity')})
            immutable = _unique([*execution['build_inputs'], *records.values()])
            build = {'schema_version': 1, 'kind': 'build', 'build_id': build_id,
                'source': execution['source'], 'configuration': execution['configuration'],
                'build_inputs_sha256': builds.native_inputs_digest(source_admission,
                    execution['configuration'], external_inputs=execution['build_inputs']),
                'commands': [execution['command']], 'build_inputs': execution['build_inputs'],
                'immutable_inputs': immutable,
                'artifacts': [record for record in records.values() if record['scope'] == 'bundle'],
                'signatures': sealed['signatures'], 'development_only': False}
            validate_manifest(build, 'build')
            build_file = destination / 'build.json'
            _write_new(build_file, build)
            records['build_manifest'] = snapshot_input(store, build_file.relative_to(store).as_posix())
            bound_records = _unique([*immutable, *records.values()])
            assets = {name: resolve_input(bundle if record['scope'] == 'bundle' else store, record['path'])
                      for name, record in records.items()}
            build_ref = {'build_id': build_id, 'manifest_sha256': manifest_digest(build)}
            bound = {'assets': assets, 'run': {'assets': records, 'build': build_ref},
                     'records': bound_records, 'store_root': store, 'bundle_root': bundle}
            admission = production.validate_production(bound)
            builds.verify_sealed_bundle(bundle, sealed, signature_verifier=signature)
            verify_inputs(input_records, store_root=store)
            dependencies = _owners(store, [row for row in immutable if row['scope'] == 'store'
                and not row['path'].startswith(output_directory + '/')])
            owner = _pin_output(store, output_directory, build_id, dependencies)
            lease.attach_objects([owner, *dependencies])
            _write_new(destination / 'assets.json', {'schema_version': 1, 'kind': 'build-assets',
                'build': build_ref, 'bundle_root': bundle.relative_to(store).as_posix(), 'assets': records,
                'immutable_inputs': bound_records})
            _write_new(destination / 'sealed-bundle.json', sealed)
            receipt = {'schema_version': 1, 'kind': 'build-assembly', 'build': build_ref,
                'build_manifest': records['build_manifest'], 'bundle_root': bundle.relative_to(store).as_posix(),
                'assets_path': (destination / 'assets.json').relative_to(store).as_posix(),
                'production_validation': admission, 'retention_object': owner,
                'qualification_complete': False}
        # Only publish completion after the original InputLease's final check.
        _write_new(destination / 'assembly-receipt.json', receipt)
        for name in ('build.json', 'assets.json', 'sealed-bundle.json', 'assembly-receipt.json'):
            (destination / name).chmod(0o444)
        lease.settled = True  # Synchronous verification returned; no build/browser children exist.
        return receipt

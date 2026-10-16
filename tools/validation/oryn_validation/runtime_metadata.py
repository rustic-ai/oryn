"""Publish new typed engine metadata from freshly validated build outputs.

This is artifact preparation, not native build provenance or qualification.
The coordinator must retain the actual committed-source build process receipt.
Historical manifests and signed bundles are never rewritten by this module.
"""
from __future__ import annotations

import importlib.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys

from . import builds
from .generation import new_store_path, source_info, _write_new
from .inputs import snapshot_input
from .production import ENGINE_ASSETS, PATCHES, SOURCE_FIELDS, normalize_bridge_flags

PROFILE_FIELDS = {'profile', 'target', 'v8_revision', 'rusty_v8_revision', 'v8_version', 'icu_revision',
                  'v8_heap_sandbox', 'pointer_compression', 'gn_args', 'v8_regression_manifest_sha256',
                  'rusty_v8_archive_compatibility', *PATCHES, *SOURCE_FIELDS}


def _legacy_validator(source_root):
    spec = importlib.util.spec_from_file_location('oryn_producer_v8_profile',
                                                Path(source_root) / 'scripts/validate-v8-runtime-profile.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def observe_sdk(sdkroot):
    """Record the selected system SDK identity, without claiming a copied SDK.

    The platform/toolchain admission must recheck this observation before and
    after compilation. Metadata hashes alone do not attest every SDK header.
    """
    selected = Path(sdkroot)
    if not selected.is_absolute() or not selected.is_dir():
        raise ValueError('absolute existing system SDK required')
    resolved = selected.resolve(strict=True)
    markers = {}
    for name in ('SDKSettings.json', 'SDKSettings.plist', 'System/Library/CoreServices/SystemVersion.plist'):
        marker = resolved / name
        if marker.is_file():
            markers[name] = builds._identity(builds._file(resolved, name))
    if not markers:
        raise ValueError('system SDK has no supported identity metadata')
    return {'schema_version': 1, 'kind': 'system-sdk-observation',
            'selected_path': str(selected), 'resolved_path': str(resolved),
            'markers': markers, 'full_sdk_contents_attested': False}


def verify_sdk_observation(observation):
    if observe_sdk(observation['selected_path']) != observation:
        raise ValueError('selected system SDK identity changed')
    return observation


def stage_engine_inputs(source_root, store_root, *, legacy_profile, output_directory,
                        diagnostics_directory):
    """Copy the declared C++ input closure and remap only known compiler paths.

    This is prelaunch preparation. All emitted build_inputs must be leased by
    the build coordinator. No retained browser binary becomes a fresh product.
    V8 API headers remain the independently admitted committed vendor headers.
    """
    root = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    destination = new_store_path(store, output_directory)
    diagnostics = new_store_path(store, diagnostics_directory)
    if destination.exists() or destination == root or root in destination.parents:
        raise ValueError('staging requires a new external output directory')
    if (diagnostics == destination or destination in diagnostics.parents or
            diagnostics in destination.parents or root == diagnostics or root in diagnostics.parents):
        raise ValueError('compiler diagnostics must be outside immutable engine inputs and source')
    profile_path = Path(legacy_profile).absolute()
    profile_before = builds._identity(builds._file(profile_path.anchor,
                                                  profile_path.relative_to(profile_path.anchor).as_posix()))
    profile_bytes = profile_path.read_bytes()
    if hashlib.sha256(profile_bytes).hexdigest() != profile_before['sha256']:
        raise ValueError('engine profile changed before staging')
    profile = json.loads(profile_bytes)
    validator = _legacy_validator(root)
    validator.validate(profile, root, profile='core-full', allow_relocated_source_manifest=True)
    descriptor_path = Path(profile['bridge_abi_path'])
    descriptor_bytes = descriptor_path.read_bytes()
    if hashlib.sha256(descriptor_bytes).hexdigest() != profile['bridge_abi_sha256']:
        raise ValueError('engine descriptor changed before staging')
    descriptor = json.loads(descriptor_bytes)
    if descriptor.get('schema_version') != 1:
        raise ValueError('staging requires compile-time bridge schema1')
    sdk_before = observe_sdk(descriptor['sdkroot'])
    normalized = normalize_bridge_flags(descriptor['flags'])
    separate = ('-I', '-isystem', '-iquote', '-isysroot', '--sysroot', '-include')
    joined = ('-fcrash-diagnostics-dir=', '--sysroot=', '-isystem', '-iquote', '-I')
    compiler = Path(descriptor['compiler'])
    compiler_root = compiler.parent.parent if compiler.parent.name == 'bin' else compiler.parent
    candidate_roots = {compiler_root, *(Path(value) for value in descriptor['header_roots'])}
    pending = None
    for flag in descriptor['flags']:
        if pending:
            if pending in ('-I', '-isystem', '-iquote'):
                candidate_roots.add(Path(flag))
            pending = None
        elif flag in separate:
            pending = flag
        else:
            for prefix in ('-isystem', '-iquote', '-I'):
                if flag.startswith(prefix) and flag[len(prefix):].startswith('/'):
                    candidate_roots.add(Path(flag[len(prefix):]))
                    break
    if any(not path.is_absolute() or '..' in path.parts for path in candidate_roots):
        raise ValueError('unambiguous absolute compiler/header roots required')
    # Collapse include subdirectories to neutral labelled roots; preserving the
    # compiler bin/../lib resource relationship does not embed machine prefixes.
    minimal_roots = [path for path in candidate_roots
                     if not any(parent != path and parent in path.parents for parent in candidate_roots)]
    roots = {}
    for index, path in enumerate(sorted(minimal_roots, key=str)):
        label = 'compiler' if path == compiler_root else 'headers-' + str(index).zfill(3)
        roots[path] = destination / label
    artifacts = {Path(profile[role + '_path']): destination / 'engine-assets' / role
                 for role in validator.ARTIFACTS}
    artifacts[profile_path] = destination / 'historical-profile.json'
    standalone = {Path(record['path']): destination / 'descriptor-inputs' / str(index).zfill(5)
                  for index, record in enumerate(descriptor['inputs'])
                  if not any(Path(record['path']).is_relative_to(path) for path in roots)}
    destination.mkdir(parents=True)
    staged, directories = {}, set()

    def mapped(value):
        old = Path(value)
        if not old.is_absolute() or '..' in old.parts:
            raise ValueError('unambiguous absolute build input required')
        if old in artifacts:
            return artifacts[old]
        for original, replacement in roots.items():
            if old.is_relative_to(original):
                return replacement / old.relative_to(original)
        if old in standalone:
            return standalone[old]
        raise ValueError('path outside declared compiler/header roots: ' + str(old))

    def retain(path, expected_sha, expected_size):
        old = Path(path)
        old = builds._file(old.anchor, old.relative_to(old.anchor).as_posix())
        before = builds._identity(old)
        if before['sha256'] != expected_sha or before['size'] != expected_size:
            raise ValueError('retained engine input drift: ' + str(old))
        target = mapped(old)
        if str(old) in staged:
            if builds._identity(target) != before:
                raise ValueError('conflicting engine input identity')
            return target
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(old, target)
        target.chmod(0o555 if before['executable'] else 0o444)
        if builds._identity(old) != before or builds._identity(target) != before:
            raise ValueError('engine input changed during staging')
        staged[str(old)] = str(target)
        return target

    updated = dict(profile)
    retain(profile_path, profile_before['sha256'], profile_before['size'])
    for role in validator.ARTIFACTS:
        target = retain(profile[role + '_path'], profile[role + '_sha256'], profile[role + '_size_bytes'])
        updated[role + '_path'] = str(target)
    rewritten = dict(descriptor)
    rewritten['inputs'] = []
    for record in descriptor['inputs']:
        target = retain(record['path'], record['sha256'], record['size_bytes'])
        rewritten['inputs'].append(dict(record, path=str(target)))
    if descriptor['compiler'] not in staged:
        raise ValueError('compiler missing from exact descriptor inputs')
    rewritten['compiler'] = staged[descriptor['compiler']]
    for role in ('archive', 'binding'):
        rewritten[role] = dict(descriptor[role], path=updated[role + '_path'])

    def map_operand(value, switch):
        if switch in ('-isysroot', '--sysroot', '--sysroot='):
            if Path(value).resolve(strict=True) != Path(sdk_before['resolved_path']):
                raise ValueError('compiler flags select a different SDK')
            return value
        if switch == '-fcrash-diagnostics-dir=':
            return str(diagnostics)
        target = mapped(value)
        if switch == '-include':
            if value not in staged:
                raise ValueError('forced include not present in descriptor inputs')
        else:
            if target.exists() and not target.is_dir():
                raise ValueError('include directory collides with retained file')
            target.mkdir(parents=True, exist_ok=True)
            directories.add(str(target.relative_to(destination)))
        return str(target)

    flags, pending = [], None
    for flag in descriptor['flags']:
        if pending:
            flags.append(map_operand(flag, pending))
            pending = None
        elif flag in separate:
            flags.append(flag)
            pending = flag
        else:
            replacement = flag
            for prefix in joined:
                if flag.startswith(prefix) and flag[len(prefix):].startswith('/'):
                    replacement = prefix + map_operand(flag[len(prefix):], prefix)
                    break
            flags.append(replacement)
    if pending or normalize_bridge_flags(flags) != normalized:
        raise ValueError('compiler ABI policy changed during path remapping')
    rewritten['flags'] = flags
    rewritten['header_roots'] = [str(mapped(value)) for value in descriptor['header_roots']]
    for value in rewritten['header_roots']:
        if not Path(value).is_dir():
            raise ValueError('retained compiler/header hierarchy is incomplete')
    new_descriptor = destination / 'bridge-abi.json'
    _write_new(new_descriptor, rewritten)
    descriptor_identity = builds._identity(new_descriptor)
    updated.update(bridge_abi_path=str(new_descriptor),
                   bridge_abi_sha256=descriptor_identity['sha256'],
                   bridge_abi_size_bytes=descriptor_identity['size'])
    new_profile = destination / 'core-full-manifest.json'
    _write_new(new_profile, updated)
    _write_new(destination / 'sdk-observation.json', sdk_before)
    _write_new(destination / 'staging-provenance.json', {
        'schema_version': 1, 'kind': 'engine-input-staging',
        'original_profile_sha256': profile_before['sha256'],
        'original_bridge_sha256': profile['bridge_abi_sha256'],
        'normalized_compiler_flags': normalized,
        'remapped_include_directories': sorted(directories),
        'vendor_headers_source': 'vendor/v8-api/include',
        'historical_worker_is_build_output': False})
    validator.validate(updated, root, profile='core-full', allow_relocated_source_manifest=True)
    if observe_sdk(descriptor['sdkroot']) != sdk_before or builds._identity(profile_path) != profile_before:
        raise ValueError('engine source or SDK identity changed during staging')
    # Independent files and directories are sealed before forming the lease.
    for directory, children, files in os.walk(destination, topdown=False):
        for name in files:
            file = Path(directory) / name
            file.chmod(0o555 if file.stat().st_mode & 0o111 else 0o444)
        Path(directory).chmod(0o555)
    return {'schema_version': 1, 'kind': 'staged-engine-inputs',
            'profile_path': str(new_profile),
            'build_inputs': [snapshot_input(store, output_directory, kind='directory'),
                *(snapshot_input(store, Path(value).relative_to(store).as_posix()) for value in
                  dict.fromkeys([str(new_profile), str(new_descriptor),
                                 str(destination / 'sdk-observation.json'),
                                 str(destination / 'staging-provenance.json'), *staged.values()]))],
            'environment': {'ORYN_V8_PROFILE_MANIFEST': str(new_profile),
                'ORYN_V8_BRIDGE_ABI': str(new_descriptor),
                'RUSTY_V8_ARCHIVE': updated['archive_path'],
                'RUSTY_V8_SRC_BINDING_PATH': updated['binding_path'],
                'ORYN_ICU_DATA_FINGERPRINT_PATH': updated['icu_data_path'],
                'SDKROOT': descriptor['sdkroot']},
            'sdk_observation': sdk_before, 'independent_review_required': True}


def prepare_typed_runtime(source_root, store_root, *, legacy_profile, output_directory,
                          source_identity):
    """Copy explicit native assets, preserving original bridge descriptor bytes."""
    source = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    current_source = source_info(source, source_identity)
    profile_path = Path(legacy_profile).resolve(strict=True)
    profile = json.loads(profile_path.read_text())
    from .rust_source_rebind_marker import pending_rebind
    if pending_rebind(profile) is not None:
        raise ValueError('Rust source rebind requires fresh snapshot and worker before publication')
    _legacy_validator(source).validate(profile, source, profile='core-full', allow_relocated_source_manifest=True)
    descriptor_path = Path(profile['bridge_abi_path'])
    descriptor = json.loads(descriptor_path.read_text())
    if descriptor.get('schema_version') != 1:
        raise ValueError('fresh build-time schema1 bridge descriptor required')
    destination = new_store_path(store, output_directory)
    if destination.exists() or destination == source or source in destination.parents:
        raise ValueError('typed native metadata requires a new external output directory')
    destination.mkdir(parents=True)
    assets = {}

    def copy_asset(role, path, expected_sha, expected_size):
        original = Path(path).absolute()
        original = builds._file(original.anchor, original.relative_to(original.anchor).as_posix())
        before = builds._identity(original)
        if before['sha256'] != expected_sha or before['size'] != expected_size:
            raise ValueError('build output drift before typed publication: ' + role)
        target = destination / role
        shutil.copyfile(original, target)
        target.chmod(0o555 if before['executable'] else 0o444)
        if builds._identity(target) != before or builds._identity(original) != before:
            raise ValueError('build input changed during typed publication: ' + role)
        assets[role] = snapshot_input(store, target.relative_to(store).as_posix())

    for old_name, role in ENGINE_ASSETS.items():
        if old_name != 'bridge_abi':
            copy_asset(role, profile[old_name + '_path'], profile[old_name + '_sha256'],
                       profile[old_name + '_size_bytes'])
    copy_asset('bridge_build_descriptor', descriptor_path, profile['bridge_abi_sha256'],
               profile['bridge_abi_size_bytes'])
    bridge_roles, compiler = [], None
    for index, record in enumerate(descriptor['inputs']):
        role = 'bridge_input_' + str(index).zfill(5)
        copy_asset(role, record['path'], record['sha256'], record['size_bytes'])
        bridge_roles.append(role)
        if record['path'] == descriptor['compiler']:
            if compiler is not None:
                raise ValueError('duplicate compiler in build descriptor')
            compiler = role
    if compiler is None:
        raise ValueError('build descriptor lacks an exact compiler input')
    bridge = {key: descriptor[key] for key in
              ('target', 'libcxx_namespace', 'libcxx_abi_version', 'stdlib_linkage')}
    bridge.update(schema_version=2, kind='v8-bridge-abi', flags=normalize_bridge_flags(descriptor['flags']),
                  archive='v8_archive', binding='bindings_source', compiler=compiler,
                  inputs=bridge_roles, build_descriptor='bridge_build_descriptor')
    bridge_output = destination / 'bridge-abi.json'
    _write_new(bridge_output, bridge)
    bridge_output.chmod(0o444)
    assets['bridge_abi'] = snapshot_input(store, bridge_output.relative_to(store).as_posix())
    typed_profile = {key: profile[key] for key in PROFILE_FIELDS}
    typed_profile.update(schema_version=7, kind='v8-runtime-profile', assets=ENGINE_ASSETS)
    profile_output = destination / 'engine-profile.json'
    _write_new(profile_output, typed_profile)
    profile_output.chmod(0o444)
    assets['engine_profile'] = snapshot_input(store, profile_output.relative_to(store).as_posix())
    # Reject source drift between the producer's first read and completed copies.
    if source_info(source, source_identity) != current_source:
        raise ValueError('committed source changed during typed metadata publication')
    receipt = {'schema_version': 1, 'kind': 'typed-runtime-preparation', 'source': current_source,
               'assets': assets, 'build_process_evidence_required': True,
               'independent_review_required': True}
    _write_new(destination / 'typed-runtime.json', receipt)
    return receipt


def validate_profile_replacement(source_root, legacy_profile, typed_profile, source_identity):
    """Before signing, match the new profile's semantic fields to build inputs."""
    source = source_info(source_root, source_identity)
    original = json.loads(Path(legacy_profile).read_text())
    replacement = json.loads(Path(typed_profile).read_text())
    _legacy_validator(source_root).validate(original, Path(source_root).resolve(), profile='core-full', allow_relocated_source_manifest=True)
    expected = {key: original[key] for key in PROFILE_FIELDS}
    expected.update(schema_version=7, kind='v8-runtime-profile', assets=ENGINE_ASSETS)
    if replacement != expected:
        raise ValueError('typed profile differs from current compiled native profile')
    return source


def refresh_engine_profile(source_root, legacy_profile, output_path, *, worker=None,
                           runtime_info=None, startup_snapshot=None):
    """Rebind current native products while retaining validated C++ engine inputs.

    A snapshot-only profile is intermediate build input, not final native build
    proof. Final publication follows a fresh worker build and runtime identity
    check; the external process receipt binds that compilation to its source.
    """
    root = Path(source_root).resolve(strict=True)
    old = json.loads(Path(legacy_profile).read_text())
    validator = _legacy_validator(root)
    validator.validate(old, root, profile='core-full', allow_relocated_source_manifest=True)
    if worker is None and startup_snapshot is None:
        raise ValueError('profile refresh requires a newly produced native artifact')
    if (worker is None) != (runtime_info is None):
        raise ValueError('new raw worker requires its runtime-info result')
    updated = dict(old)
    from .rust_source_rebind_marker import pending_rebind
    pending = pending_rebind(updated)
    if pending is not None:
        if startup_snapshot is not None:
            pending["fresh_snapshot_required"] = False
        if worker is not None:
            pending["fresh_worker_required"] = False
        if pending["fresh_worker_required"] or pending["fresh_snapshot_required"]:
            updated["rust_source_rebind_intermediate"] = pending
        else:
            updated.pop("rust_source_rebind_intermediate")
    for role, value in (('worker', worker), ('startup_snapshot', startup_snapshot)):
        if value is not None:
            path = Path(value).resolve(strict=True)
            identity = builds._identity(path)
            updated[role + '_path'] = str(path)
            updated[role + '_sha256'] = identity['sha256']
            updated[role + '_size_bytes'] = identity['size']
    if worker is not None:
        old_path = list(sys.path)
        try:
            sys.path.insert(0, str(root / 'scripts'))
            spec = importlib.util.spec_from_file_location('oryn_refreshed_native_runtime',
                                                        root / 'scripts/validate-v8-rust-runtime.py')
            module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(module)
            runtime = json.loads(Path(runtime_info).read_text())
            module.validate_runtime(updated, runtime, Path(updated['binding_path']),
                                    Path(updated['archive_path']), root, allow_relocated_source_manifest=True)
            if runtime.get('v8_bridge_abi_sha256') != updated['bridge_abi_sha256']:
                raise ValueError('compiled worker bridge descriptor differs from selected build inputs')
        finally:
            sys.path[:] = old_path
    validator.validate(updated, root, profile='core-full', allow_relocated_source_manifest=True)
    _write_new(output_path, updated)
    return {'schema_version': 1, 'profile_sha256': builds._identity(Path(output_path))['sha256'],
            'updated_native_artifacts': [role for role, value in
                (('worker', worker), ('startup_snapshot', startup_snapshot)) if value is not None],
            'build_process_evidence_required': True}

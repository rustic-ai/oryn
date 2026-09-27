"""Typed production admission; retained success flags never admit an engine.

Engine schema 7 keeps schema 6's pinned runtime checks but replaces every path
with a named immutable asset. Bridge schema 2 does the same for compiler/header
inputs. These are new manifests, not edits to historical signed profiles.

This gate proves production provenance and fresh signing. Native/reference TLS,
focus, topology, canaries, placement, and release qualification are separate.
"""
from __future__ import annotations

import hashlib
import importlib.util
import json
import base64
from pathlib import Path
import re

from . import builds
from .admission import SOURCE_ROOT, legacy_algorithms
from .inputs import resolve_input, verify_inputs
from .manifests import load_manifest, manifest_digest


PATCHES = {
    'v8_patch_sha256': 'window-proxy-structural-semantics',
    'shared_memory_growth_patch_sha256': 'shared-memory-growth',
    'shared_memory_wait_patch_sha256': 'shared-memory-wait',
    'shared_wasm_size_patch_sha256': 'shared-wasm-size',
    'shared_wasm_backing_patch_sha256': 'shared-wasm-backing',
    'shared_memory_factory_patch_sha256': 'shared-memory-factory',
    'frame_context_reattachment_patch_sha256': 'frame-context-reattachment',
    'html_constructor_deferred_prototype_patch_sha256': 'html-constructor-deferred-prototype',
}
ENGINE_ASSETS = {
    'archive': 'v8_archive', 'binding': 'bindings_source', 'icu_data': 'icu_data',
    'startup_snapshot': 'startup_snapshot', 'external_references': 'external_references',
    'worker': 'engine_worker', 'bridge_abi': 'bridge_abi',
}
SOURCE_FIELDS = ('rusty_v8_source_manifest_sha256', 'rusty_v8_source_tree_sha256',
                 'rusty_v8_weak_teardown_patch_sha256', 'rusty_v8_patch_metadata_sha256')
REQUIRED_ASSETS = frozenset({
    'build_manifest', 'source_export', 'source_identity', 'engine_profile', 'worker_manifest',
    'worker', 'worker_app', 'runtime', 'driver', *ENGINE_ASSETS.values(),
})


def _require(condition, message):
    if not condition:
        raise ValueError(message)


def _object(value, fields, label):
    _require(isinstance(value, dict) and set(value) == set(fields), label + ' schema differs')
    return value


def _read(path):
    # Same duplicate-key and invalid-number policy as the shared manifests.
    from .manifests import _unique_object
    def invalid(value):
        raise ValueError('invalid JSON constant: ' + value)
    value = json.loads(Path(path).read_text(), object_pairs_hook=_unique_object,
                       parse_constant=invalid)
    _require(isinstance(value, dict), 'production declaration must be an object')
    return value


def _hash(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def _module(name, relative):
    # Load maintained code, never executable code from an asset/receipt.
    legacy_algorithms()
    spec = importlib.util.spec_from_file_location(name, SOURCE_ROOT / relative)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _asset_records(bound):
    records = bound['run'].get('assets', {})
    _require(REQUIRED_ASSETS <= records.keys(),
             'production admission lacks typed assets: ' + ', '.join(sorted(REQUIRED_ASSETS - records.keys())))
    for name, record in records.items():
        _require(record in bound['records'], 'asset outside immutable closure: ' + name)
        root = bound['bundle_root'] if record['scope'] == 'bundle' else bound['store_root']
        _require(resolve_input(root, record['path']) == Path(bound['assets'][name]),
                 'resolved production asset substituted: ' + name)
    for name in REQUIRED_ASSETS:
        kind = 'directory' if name in {'source_export', 'worker_app'} else 'file'
        _require(records[name]['kind'] == kind, 'production asset kind differs: ' + name)
    for name in ('worker', 'worker_app', 'worker_manifest', 'runtime', 'driver'):
        _require(records[name]['scope'] == 'bundle', 'production executable must be bundled: ' + name)
    for name in ('worker', 'runtime', 'driver'):
        _require(records[name]['executable'] is True, 'production executable permission missing: ' + name)
    return records


def _git_object(kind, raw, algorithm):
    return hashlib.new(algorithm, kind.encode() + b' ' + str(len(raw)).encode() + b'\0' + raw).hexdigest()


def validate_git_export(identity, source, membership, *, normalized_symlinks=None):
    """Prove exported bytes against raw Git commit/tree objects, without Git IO.

The producer exports tracked content and `git cat-file` commit/tree payloads
from the shared repository. Object IDs are recomputed here; neither an asserted
commit string nor a clean:true receipt proves source identity. Gitlinks and
symlinks are rejected unless a dataset adapter supplies explicit blob/mode
normalizations. Native source never supplies this opt-in mechanism.
    """
    _object(identity, {'schema_version', 'kind', 'commit', 'commit_object', 'trees'}, 'Git source identity')
    _require(type(identity['schema_version']) is int and identity['schema_version'] == 1
             and identity['kind'] == 'git-source-export', 'Git source identity schema differs')
    normalized_symlinks = {} if normalized_symlinks is None else normalized_symlinks
    _require(isinstance(normalized_symlinks, dict), 'Git normalizations must be an explicit mapping')
    consumed_normalizations = set()
    commit = identity['commit']
    _require(isinstance(commit, str) and re.fullmatch(r'[0-9a-f]{40}|[0-9a-f]{64}', commit),
             'Git source commit identity invalid')
    algorithm, oid_size = ('sha1', 20) if len(commit) == 40 else ('sha256', 32)
    def decode(value):
        _require(isinstance(value, str), 'Git object payload must be base64')
        try:
            return base64.b64decode(value, validate=True)
        except (ValueError, UnicodeError) as error:
            raise ValueError('invalid Git object encoding') from error
    commit_raw = decode(identity['commit_object'])
    _require(_git_object('commit', commit_raw, algorithm) == commit, 'Git commit object hash mismatch')
    first_line = commit_raw.split(b'\n', 1)[0]
    _require(re.fullmatch(b'tree [0-9a-f]{' + str(oid_size * 2).encode() + b'}', first_line),
             'Git commit lacks exact root tree')
    trees = identity['trees']
    _require(isinstance(trees, dict) and trees, 'Git tree objects missing')
    reached, active, files, directories = set(), set(), {}, set()
    def visit(oid, prefix=''):
        _require(oid in trees and oid not in active, 'Git tree missing or cyclic')
        raw = decode(trees[oid])
        _require(_git_object('tree', raw, algorithm) == oid, 'Git tree object hash mismatch')
        reached.add(oid); active.add(oid)
        pos, names, order = 0, set(), []
        while pos < len(raw):
            space, nul = raw.find(b' ', pos), raw.find(b'\0', pos)
            _require(pos < space < nul and nul + 1 + oid_size <= len(raw), 'malformed Git tree entry')
            mode, name_raw = raw[pos:space], raw[space + 1:nul]
            try:
                name = name_raw.decode('utf-8')
            except UnicodeError as error:
                raise ValueError('Git source paths must be UTF-8') from error
            _require(name not in {'', '.', '..'} and name.casefold() != '.git' and '/' not in name and '\\' not in name
                     and name not in names, 'unsafe or duplicate Git tree name')
            names.add(name)
            oid_child = raw[nul + 1:nul + 1 + oid_size].hex()
            pos = nul + 1 + oid_size
            relative = prefix + name
            _require(mode in {b'40000', b'100644', b'100755'} or
                     (mode == b'120000' and relative in normalized_symlinks),
                     'Git symlink/gitlink/special source forbidden')
            order.append(name_raw + (b'/' if mode == b'40000' else b''))
            if mode == b'40000':
                directories.add(relative)
                visit(oid_child, relative + '/')
            else:
                path = resolve_input(source, relative)
                _require(path.is_file(), 'Git source blob missing')
                # Bound memory even for large pinned upstream datasets.
                before = path.stat()
                content_hash = hashlib.sha256()
                git_hash = hashlib.new(algorithm)
                git_hash.update(b'blob ' + str(before.st_size).encode() + b'\0')
                with path.open('rb') as stream:
                    while block := stream.read(1024 * 1024):
                        content_hash.update(block); git_hash.update(block)
                after = path.stat()
                _require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
                         (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns),
                         'Git source changed during verification: ' + relative)
                executable = bool(after.st_mode & 0o111)
                expected_executable = mode == b'100755'
                if mode == b'120000':
                    normalization = normalized_symlinks[relative]
                    _object(normalization, {'blob', 'executable'}, 'Git symlink normalization')
                    _require(type(normalization['executable']) is bool, 'Git normalization mode must be boolean')
                    _require(_git_object('blob', decode(normalization['blob']), algorithm) == oid_child,
                             'Git symlink blob differs: ' + relative)
                    expected_executable = normalization['executable']
                    consumed_normalizations.add(relative)
                else:
                    _require(git_hash.hexdigest() == oid_child, 'Git source blob differs: ' + relative)
                _require(executable == expected_executable, 'Git executable mode differs: ' + relative)
                files[relative] = {'sha256': content_hash.hexdigest(),
                                   'size': after.st_size, 'executable': executable}
        _require(order == sorted(order), 'Git tree entries are not canonically ordered')
        active.remove(oid)
    visit(first_line[5:].decode())
    _require(reached == set(trees), 'unused Git tree objects are undeclared source evidence')
    _require(set(files) == {item['path'] for item in membership['members']}
             and directories == set(membership['directories']), 'Git export membership differs from committed tree')
    _require(consumed_normalizations == set(normalized_symlinks), 'unused Git normalization entries')
    _require(files, 'Git export has no files')
    return {'schema_version': 1, 'commit': commit, 'dirty': False, 'development_only': False,
            'tree_sha256': builds._digest(files), 'tracked': files,
            'effective_inputs': sorted(files), 'external_inputs': {}}


def validate_source_export(identity, source, membership):
    """Native source keeps its strict no-symlink, Cargo workspace contract."""
    result = validate_git_export(identity, source, membership)
    _require('Cargo.toml' in result['tracked'] and 'Cargo.lock' in result['tracked'],
             'Git export lacks workspace manifests')
    return result


def _engine(profile, source, assets, records, target):
    fields = {'schema_version', 'kind', 'profile', 'target', 'v8_revision',
              'rusty_v8_revision', 'v8_version', 'icu_revision', 'v8_heap_sandbox',
              'pointer_compression', 'gn_args', 'assets', 'v8_regression_manifest_sha256',
              'rusty_v8_archive_compatibility', *PATCHES, *SOURCE_FIELDS}
    _object(profile, fields, 'typed engine profile')
    _require(type(profile['schema_version']) is int and profile['schema_version'] == 7
             and profile['kind'] == 'v8-runtime-profile', 'typed engine profile schema 7 required')
    _require(profile['profile'] == 'core-full' and profile['target'] == target,
             'production requires current core-full target')
    for field, expected in {
        'v8_revision': 'ac1e23989121713ca642f6650b34deff7b686896',
        'rusty_v8_revision': '5c15a6995c9bb4bacd3e341b59fff32c909c80bf',
        'v8_version': '150.4.0', 'icu_revision': 'ee5f27adc28bd3f15b2c293f726d14d2e336cbd5',
    }.items():
        _require(profile[field] == expected, 'engine revision mismatch: ' + field)
    _require(profile['v8_heap_sandbox'] is True and profile['pointer_compression'] is True,
             'native sandbox and pointer compression are mandatory')
    _require(isinstance(profile['gn_args'], str), 'GN arguments must be a string')
    args = profile['gn_args'].split()
    for option in ('v8_drumbrake_bounds_checks=true', 'v8_enable_sandbox=true',
                   'v8_enable_pointer_compression=true'):
        key = option.split('=')[0] + '='
        _require(option in args and sum(item.startswith(key) for item in args) == 1,
                 'missing or conflicting required engine option: ' + option)
    _require(profile['assets'] == ENGINE_ASSETS, 'engine asset roles substituted')
    for field, name in PATCHES.items():
        patch = source / 'patches/v8' / (name + '.patch')
        metadata = _read(patch.with_suffix('.json'))
        _require(profile[field] == _hash(patch) == metadata['patch_sha256'],
                 'engine patch mismatch: ' + name)
        for prerequisite, expected in metadata.get('required_patch_sha256', {}).items():
            _require(prerequisite in PATCHES.values(), 'undeclared engine patch prerequisite')
            _require(_hash(source / 'patches/v8' / (prerequisite + '.patch')) == expected,
                     'engine patch prerequisite mismatch: ' + prerequisite)
    regression_path = source / 'patches/v8/tests/manifest.json'
    _require(profile['v8_regression_manifest_sha256'] == _hash(regression_path),
             'engine regression manifest mismatch')
    regression = _read(regression_path)
    for name, expected in regression.get('required_patch_sha256', {}).items():
        _require(name in PATCHES.values(), 'undeclared regression patch prerequisite')
        _require(_hash(source / 'patches/v8' / (name + '.patch')) == expected,
                 'engine regression prerequisite mismatch: ' + name)
    for name, expected in regression['sources'].items():
        _require(_hash(resolve_input(source, 'patches/v8/tests/' + name)) == expected,
                 'engine regression source mismatch: ' + name)
    for name, expected in regression.get('test_only_compatibility', {}).items():
        metadata_path = resolve_input(source, 'patches/v8/tests/' + name + '.json')
        _require(_hash(metadata_path) == expected, 'test compatibility metadata mismatch: ' + name)
        _require(_hash(metadata_path.with_suffix('.patch')) == _read(metadata_path)['patch_sha256'],
                 'test compatibility patch mismatch: ' + name)
    selected = _module('oryn_production_rust_v8', 'scripts/rusty_v8_source_provenance.py').validate(source)
    for field in SOURCE_FIELDS:
        _require(profile[field] == selected[field], 'Rust V8 selected source mismatch: ' + field)
    _require(profile['rusty_v8_archive_compatibility'] == {
        'cpp_changes': False, 'crate_version': '150.4.0',
        'archive_sha256': records['v8_archive']['sha256'],
        'binding_sha256': records['bindings_source']['sha256'],
    }, 'Rust V8 archive/binding compatibility mismatch')
    return _bridge(_read(assets['bridge_abi']), assets, records, target)


def _bridge(bridge, assets, records, target):
    _object(bridge, {'schema_version', 'kind', 'target', 'libcxx_namespace',
                    'libcxx_abi_version', 'stdlib_linkage', 'flags', 'archive',
                    'binding', 'compiler', 'inputs', 'build_descriptor'}, 'typed bridge ABI')
    _require(type(bridge['schema_version']) is int and bridge['schema_version'] == 2
             and bridge['kind'] == 'v8-bridge-abi', 'typed bridge ABI schema 2 required')
    _require(bridge['target'] == target and bridge['libcxx_namespace'] == '__Cr'
             and type(bridge['libcxx_abi_version']) is int and bridge['libcxx_abi_version'] == 2
             and bridge['stdlib_linkage'] == 'included-in-rusty-v8', 'bridge C++ ABI mismatch')
    _require(isinstance(bridge['flags'], list) and all(isinstance(x, str) for x in bridge['flags'])
             and '-nostdinc++' in bridge['flags'], 'bridge system C++ headers forbidden')
    _require(bridge['archive'] == 'v8_archive' and bridge['binding'] == 'bindings_source',
             'bridge selected engine artifacts differ')
    inputs = bridge['inputs']
    _require(isinstance(inputs, list) and inputs and all(isinstance(x, str) for x in inputs)
             and len(inputs) == len(set(inputs)) and bridge['compiler'] in inputs,
             'bridge compiler provenance missing or duplicate')
    for name in inputs:
        _require(name in assets and name in records and records[name]['kind'] == 'file',
                 'bridge input absent from typed closure: ' + name)
    descriptor_name = bridge['build_descriptor']
    _require(isinstance(descriptor_name, str) and descriptor_name in assets
             and descriptor_name in records and records[descriptor_name]['kind'] == 'file',
             'bridge compile-time descriptor absent from typed closure')
    # Compile-time absolute paths are retained as historical labels only. Never
    # resolve them; join every consumed byte through the explicit asset mapping.
    original = _read(assets[descriptor_name])
    _require(type(original.get('schema_version')) is int and original['schema_version'] == 1,
             'bridge compile-time descriptor schema differs')
    for name in ('target', 'libcxx_namespace', 'libcxx_abi_version', 'stdlib_linkage'):
        _require(original.get(name) == bridge[name], 'bridge build/runtime ABI differs: ' + name)
    _require(normalize_bridge_flags(original.get('flags')) == bridge['flags'],
             'bridge build/runtime policy flags differ')
    for field, name in [('archive', 'v8_archive'), ('binding', 'bindings_source')]:
        entry = original.get(field, {})
        _require(entry.get('sha256') == records[name]['sha256']
                 and entry.get('size_bytes') == records[name]['size'], 'bridge original artifact mismatch: ' + name)
    retained = original.get('inputs')
    _require(isinstance(retained, list) and len(retained) == len(inputs), 'bridge original input population differs')
    for entry, name in zip(retained, inputs):
        _require(isinstance(entry, dict) and entry.get('sha256') == records[name]['sha256']
                 and entry.get('size_bytes') == records[name]['size'], 'bridge original retained input differs: ' + name)
    compiler_index = inputs.index(bridge['compiler'])
    _require(retained[compiler_index].get('path') == original.get('compiler'),
             'bridge original compiler role differs')
    return [*inputs, descriptor_name]


def normalize_bridge_flags(flags):
    """Remove lexical build roots from schema2; exact argv stays in descriptor.

Placeholders are provenance labels, never compiler arguments. The immutable
original descriptor and its explicit input identities retain the exact build
invocation. Unknown path-bearing options fail closed rather than losing data.
    """
    _require(isinstance(flags, list) and all(isinstance(x, str) and x for x in flags),
             'bridge flags must be nonempty strings')
    path_options = {'-I', '-isystem', '-iquote', '-isysroot', '--sysroot', '-include'}
    result, pending = [], False
    for flag in flags:
        if pending:
            _require(Path(flag).is_absolute(), 'bridge path operand must be absolute')
            result.append('@build-path'); pending = False
        elif flag in path_options:
            result.append(flag); pending = True
        else:
            for prefix in ('-isystem', '-iquote', '--sysroot=', '-fcrash-diagnostics-dir=', '-I'):
                if flag.startswith(prefix) and Path(flag[len(prefix):]).is_absolute():
                    result.append(prefix + '@build-path')
                    break
            else:
                _require(not re.search(r'/[A-Za-z0-9_]', flag), 'unrecognized bridge path-bearing option')
                result.append(flag)
    _require(not pending, 'bridge flag path operand missing')
    return result


def validate_production(bound):
    """Require exact committed native inputs, engine profile and live signature.

    Source is an exact immutable export plus hash-checked Git objects; the
developer's shared repository and worktree metadata are never runtime inputs.
    """
    records = _asset_records(bound)
    roots = {'store_root': bound['store_root'], 'bundle_root': bound['bundle_root']}
    verify_inputs(bound['records'], **roots)
    assets, run = bound['assets'], bound['run']
    build = load_manifest(assets['build_manifest'], 'build')
    _require(run['build'] == {'build_id': build['build_id'], 'manifest_sha256': manifest_digest(build)},
             'run/build identity mismatch')
    _require(not build['development_only'] and not build['source']['dirty'],
             'production requires clean committed build provenance')
    _require(build['configuration']['profile'] == 'release'
             and build['configuration']['target'] == 'aarch64-apple-darwin',
             'only production macOS arm64 release admission is migrated')
    from .build_evidence import verify_build_evidence
    verify_build_evidence(bound, build)
    source = Path(assets['source_export'])
    source_admission = validate_source_export(_read(assets['source_identity']), source, records['source_export'])
    _require(source_admission['commit'] == build['source']['commit']
             and source_admission['tree_sha256'] == build['source']['tree_sha256'],
             'committed source content differs from build')
    _require(_hash(source / 'Cargo.lock') == build['configuration']['cargo_lock_sha256'],
             'committed Cargo.lock differs from build')
    verify_inputs(build['build_inputs'], **roots)
    _require(builds.native_inputs_digest(source_admission, build['configuration'],
                                        external_inputs=build['build_inputs']) == build['build_inputs_sha256'],
             'native build-input identity differs from committed source/configuration')
    for name in REQUIRED_ASSETS - {'build_manifest'}:
        _require(records[name] in build['immutable_inputs'] + build['artifacts'],
                 'production asset not bound to selected build: ' + name)
    profile = _read(assets['engine_profile'])
    bridge_inputs = _engine(profile, source, assets, records, build['configuration']['target'])
    for name in bridge_inputs:
        _require(records[name] in build['immutable_inputs'] + build['artifacts'],
                 'bridge input not bound to selected build: ' + name)
    worker, app = Path(assets['worker']), Path(assets['worker_app'])
    _require(worker == app / 'Contents/MacOS/oryn-page-worker'
             and Path(assets['worker_manifest']) == app / 'Contents/Resources/worker-manifest.json',
             'production worker bundle layout differs')
    _require(_hash(app / 'Contents/Resources/v8-profile-manifest.json') == records['engine_profile']['sha256'],
             'bundled engine profile differs')
    worker_manifest = _read(assets['worker_manifest'])
    _require(not any(str(key).endswith('_path') for key in worker_manifest),
             'new worker manifest must not carry legacy absolute-path dependencies')
    expected = {'source_tree_sha256': build['source']['tree_sha256'],
                'source_commit': build['source']['commit'],
                'worker_build_sha256': records['engine_worker']['sha256'],
                'worker_build_bytes': records['engine_worker']['size'],
                'v8_archive_sha256': records['v8_archive']['sha256'],
                'v8_profile_sha256': records['engine_profile']['sha256'],
                'v8_binding_sha256': records['bindings_source']['sha256'],
                **{name: profile[name] for name in SOURCE_FIELDS}}
    for field, value in expected.items():
        _require(worker_manifest.get(field) == value, 'signed worker manifest differs from build: ' + field)
    for field, symbol in [('render_contract_version', 'RENDER_CONTRACT_VERSION'),
                          ('local_effects_contract_version', 'LOCAL_EFFECTS_CONTRACT_VERSION')]:
        matches = re.findall(r'^pub const ' + symbol + r': u16 = ([0-9]+);',
                             (source / 'crates/oryn-common/src/v4.rs').read_text(), re.MULTILINE)
        _require(len(matches) == 1 and int(matches[0]) > 0 and type(worker_manifest.get(field)) is int
                 and worker_manifest[field] == int(matches[0]), 'signed rendering contract mismatch: ' + field)
    signature = builds.verify_worker_signature(SOURCE_ROOT, bound['bundle_root'],
                                               worker_path=records['worker']['path'],
                                               manifest_path=records['worker_manifest']['path'])
    _require(signature in build['signatures'], 'fresh signature differs from selected build')
    verify_inputs(bound['records'], **roots)
    return {'build_id': build['build_id'], 'build_manifest_sha256': manifest_digest(build),
            'source_commit': build['source']['commit'], 'runtime_profile': profile['profile'],
            'target': profile['target'], 'signature': signature}

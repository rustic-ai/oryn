"""Compiled runtime and parent dependency checks for a selected typed bundle."""
from __future__ import annotations

import hashlib
from pathlib import Path
import re

from .production import PATCHES, _hash, _read, _require, _module


PURITY_ARGUMENTS = {
    'default_parent': ['tree', '--locked', '-p', 'oryn', '-e', 'normal,build'],
    'release_parent': ['tree', '--locked', '-p', 'oryn', '--no-default-features',
                       '--features', 'native core-full webdriver-server', '-e', 'normal,build'],
    'worker_graph': ['tree', '--locked', '-p', 'oryn-native', '--features', 'v8-host', '-e', 'normal,build'],
}


def metadata_arguments(source):
    common = ['metadata', '--locked', '--offline', '--format-version', '1', '--manifest-path']
    return {'default_parent': common + [str(source / 'crates/oryn/Cargo.toml')],
            'release_parent': common + [str(source / 'crates/oryn/Cargo.toml'), '--no-default-features',
                                       '--features', 'native core-full webdriver-server'],
            'worker_graph': common + [str(source / 'crates/oryn-native/Cargo.toml'), '--features', 'v8-host']}


def validate_toolchain(bound, observations, version):
    build = _read(bound['assets']['build_manifest']); execution = _read(bound['assets']['build_execution'])
    _require(set(observations) == {'cargo', 'rustc'}, 'actual host toolchain observations missing')
    for name, observed in observations.items():
        expected = execution['toolchain_executables'][name]
        _require(set(observed) == {'path', 'sha256', 'size', 'executable'} and Path(observed['path']).is_absolute()
                 and observed['executable'] is True
                 and {k: v for k, v in observed.items() if k != 'path'} == {k: v for k, v in expected.items() if k != 'path'},
                 'readiness host toolchain differs from actual build: ' + name)
    lines = version.splitlines()
    _require(lines and lines[0] == build['configuration']['toolchain']
             and 'host: ' + build['configuration']['host'] in lines, 'readiness Rust toolchain version differs')


def validate_parent_purity(trees):
    """Same three actual Cargo graph assertions as the canonical shell gate."""
    _require(set(trees) == set(PURITY_ARGUMENTS), 'parent dependency graph population differs')
    pattern = re.compile(r'^[\s│├└─]*v8 v[0-9]', re.MULTILINE)
    for name in ('default_parent', 'release_parent'):
        _require(isinstance(trees[name], str) and trees[name].strip()
                 and not pattern.search(trees[name]), 'parent dependency graph contains V8 or is empty: ' + name)
    _require(isinstance(trees['worker_graph'], str) and pattern.search(trees['worker_graph']),
             'worker dependency graph does not own V8')


def patch_chain(source):
    digest = hashlib.sha256()
    for name in PATCHES.values():
        digest.update(name.encode()); digest.update(b'\0')
        digest.update((source / 'patches/v8' / (name + '.patch')).read_bytes()); digest.update(b'\0')
    return digest.hexdigest()


def validate_descriptors(bound, parent, worker):
    """Join actual runtime-info output to signed bytes and compile-time ABI."""
    assets = bound['assets']; source = Path(assets['source_export'])
    profile = _read(assets['engine_profile']); bridge = _read(assets['bridge_abi'])
    manifest = _read(assets['worker_manifest'])
    protocols = _module('typed_runtime_protocols', 'scripts/native_protocol_versions.py')
    _require(protocols.runtime_matches(parent, source), 'parent native wire protocol identity differs')
    versions = protocols.versions(source)
    for name, field in [('context', 'context_service_protocol_version'), ('agent', 'agent_ipc_protocol_version')]:
        _require(type(worker.get(field)) is int and worker[field] == versions[name], 'worker native wire identity differs')
    common = {'native_v8': True, 'build_profile': 'release', 'v8_archive_sha256': _hash(assets['v8_archive']),
              'v8_binding_sha256': _hash(assets['bindings_source']),
              'startup_snapshot_sha256': _hash(assets['startup_snapshot']),
              'v8_bridge_abi_sha256': _hash(assets[bridge['build_descriptor']]),
              'v8_patch_chain_sha256': patch_chain(source)}
    validator = _module('typed_compiled_v8_sources', 'scripts/validate-v8-rust-runtime.py')
    # Legacy source validator also expects derived path/size fields. These are
    # computed from the already pinned export, never traversed from a receipt.
    selected = dict(profile, **{key: value for key, value in validator.validate(source).items()
                                if key not in validator.FIELDS})
    for runtime in (parent, worker):
        _require(all(type(runtime.get(key)) is type(value) and runtime[key] == value for key, value in common.items()),
                 'compiled native engine or original bridge ABI differs')
        validator.validate_runtime(selected, runtime, assets['bindings_source'], assets['v8_archive'], source)
    expected = {'execution_mode': 'sandboxed_worker', 'sandboxed': True, 'sandbox_state': 'app_sandbox',
                'worker_bundle_id': 'ai.rustic.oryn.page-worker', 'worker_team_id': '5HVA9VFF8K',
                'worker_signing_cert_sha1': '9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90',
                'worker_binary_sha256': _hash(assets['worker'])}
    _require(all(type(parent.get(key)) is type(value) and parent[key] == value for key, value in expected.items()),
             'parent does not attest selected signed production worker')
    _require(manifest.get('v8_patch_chain_sha256') == common['v8_patch_chain_sha256'],
             'worker manifest patch chain differs from compiled source')
    return {'parent': parent, 'worker': worker}


def validate_profile_equivalence(bound):
    """Share configuration-independent probes only across the reviewed axes."""
    from .admission import validate_registry
    registry = _read(bound['assets']['core_profile_registry'])
    validate_registry(registry, bound['assets'], selected_profile=bound['run']['profile'])
    profiles = registry['profiles']; primary = profiles[registry['primary_profile_id']]
    def relevant(value):
        return {key: item for key, item in value.items() if key not in {'web_features', 'third_party_cookies'}}
    base = relevant(_read(bound['assets'][primary['fixture']]))
    for name, profile in profiles.items():
        _require({key: value for key, value in profile.items() if key != 'fixture'} ==
                 {key: value for key, value in primary.items() if key != 'fixture'}
                 and relevant(_read(bound['assets'][profile['fixture']])) == base,
                 'profile needs separate native/topology prerequisites: ' + name)
    return {'primary_profile_id': registry['primary_profile_id'], 'profiles': sorted(profiles),
            'excluded_semantic_axes': ['web_features', 'third_party_cookies']}

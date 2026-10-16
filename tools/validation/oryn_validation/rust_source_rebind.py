"""Narrow archive-watch source rebind; output is intermediate, never a product."""
import hashlib
import importlib.util
import json
from pathlib import Path

from .generation import _write_new
from .runtime_metadata import _legacy_validator


def _sources(root):
    spec = importlib.util.spec_from_file_location('rebind_sources', root / 'scripts/rusty_v8_source_provenance.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.validate(root)


def rebind_archive_watch(old_source_root, source_root, legacy_profile, output):
    old_root = Path(old_source_root).resolve(strict=True)
    root = Path(source_root).resolve(strict=True)
    profile_path = Path(legacy_profile).resolve(strict=True)
    if Path(output).resolve() == profile_path:
        raise ValueError('historical profile cannot be rewritten')
    old = json.loads(profile_path.read_text())
    _legacy_validator(old_root).validate(old, old_root, profile='core-full', allow_relocated_source_manifest=True)
    before = _sources(old_root)
    after = _sources(root)
    name = 'vendor/rusty-v8/oryn-source-manifest.json'
    a = json.loads((old_root / name).read_text())
    b = json.loads((root / name).read_text())
    changes = {key for key in a['files'].keys() | b['files'].keys() if a['files'].get(key) != b['files'].get(key)}
    if changes != {'build.rs'} or a['base_files'] != b['base_files']:
        raise ValueError('only reviewed archive-watch build.rs correction is permitted')
    old_build = (old_root / 'vendor/rusty-v8/build.rs').read_bytes()
    new_build = (root / 'vendor/rusty-v8/build.rs').read_bytes()
    removed = b'  println!("cargo:rerun-if-changed=.gn");\n'
    replacement = b'  // Archive-only subset has no upstream .gn; watching it forces perpetual rebuilds.\n'
    if old_build.count(removed) != 1 or new_build != old_build.replace(removed, replacement):
        raise ValueError('unreviewed Rust build-script change')
    updated = dict(old)
    updated.update(after)
    # Full current validator rechecks unchanged C++ patch/test/compiler/header
    # closure, archive, binding, sandbox options and bridge against current root.
    _legacy_validator(root).validate(updated, root, profile='core-full')
    updated['rust_source_rebind_intermediate'] = {
        'schema_version': 1,
        'old_profile_sha256': hashlib.sha256(profile_path.read_bytes()).hexdigest(),
        'old_rust_tree': before['rusty_v8_source_tree_sha256'],
        'new_rust_tree': after['rusty_v8_source_tree_sha256'],
        'fresh_snapshot_required': True,
        'fresh_worker_required': True,
        'qualification_complete': False,
    }
    _write_new(output, updated)
    return {'intermediate_only': True, 'qualification_complete': False,
            'profile_sha256': hashlib.sha256(Path(output).read_bytes()).hexdigest()}

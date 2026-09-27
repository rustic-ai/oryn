"""Source selection and drift regressions, without building or loading V8."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import rusty_v8_source_provenance as source


class RustV8SourceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vendor = self.root / source.VENDOR
        (self.vendor / 'src').mkdir(parents=True)
        (self.vendor / 'tests').mkdir()
        (self.root / 'patches/rusty-v8').mkdir(parents=True)
        (self.root / 'Cargo.toml').write_text('[patch.crates-io]\nv8 = { path = "vendor/rusty-v8" }\n')
        (self.root / 'Cargo.lock').write_text('version = 4\n[[package]]\nname = "v8"\nversion = "150.4.0"\n')
        (self.vendor / 'Cargo.toml').write_text('[package]\nname = "v8"\nversion = "150.4.0"\n')
        (self.vendor / 'src/handle.rs').write_text('old Rust weak callback')
        (self.vendor / 'build.rs').write_text('unchanged archive consumer')
        self.base = source.inventory(self.vendor)
        (self.vendor / 'src/handle.rs').write_text('new Rust weak callback')
        (self.vendor / 'tests/teardown.rs').write_text('regression')
        (self.root / source.PATCH).write_text('reviewed Rust-only patch')
        self.metadata = dict(schema_version=1, crate='v8', crate_version=source.VERSION,
            source_basis='authenticated upstream crate', cpp_changes=False,
            base_files={'src/handle.rs': self.base['src/handle.rs'], 'tests/teardown.rs': None},
            patched_files={name: source.digest(self.vendor / name) for name in ('src/handle.rs', 'tests/teardown.rs')},
            patch_sha256=source.digest(self.root / source.PATCH))
        (self.root / source.METADATA).write_text(json.dumps(self.metadata))
        files = source.inventory(self.vendor)
        self.manifest = dict(schema_version=1, crate='v8', version=source.VERSION,
            archive_only=True, registry_checksum=source.CHECKSUM, base_files=self.base, files=files,
            tree_sha256=source.tree_digest(files), supplemental_files={},
            patches=[dict(path=source.PATCH, sha256=self.metadata['patch_sha256'])])
        self.write_manifest()
        # This fixture has no binary ICU payload; its own approved supplemental
        # inventory is empty. Production pins the independently retained ICU.
        self.supplemental = patch.object(source, 'SUPPLEMENTAL', {})
        self.supplemental.start()
        self.addCleanup(self.supplemental.stop)

    def write_manifest(self):
        (self.root / source.MANIFEST).write_text(json.dumps(self.manifest))

    def test_complete_selected_source_and_new_regression_are_bound(self):
        result = source.validate(self.root)
        self.assertEqual(result['rusty_v8_source_tree_sha256'], source.tree_digest(self.manifest['files']))
        self.assertNotIn('tests/teardown.rs', self.manifest['base_files'])
        self.assertNotEqual(result['rusty_v8_weak_teardown_patch_sha256'], result['rusty_v8_source_manifest_sha256'])

    def test_source_drift_missing_and_unlisted_files_reject(self):
        path = self.vendor / 'src/handle.rs'
        original = path.read_bytes()
        for changed in (b'x' * len(original), None):
            if changed is None:
                path.unlink()
            else:
                path.write_bytes(changed)
            with self.assertRaisesRegex(ValueError, 'file set or content'):
                source.validate(self.root)
            path.write_bytes(original)
        (self.vendor / 'unlisted.rs').write_text('new source')
        with self.assertRaisesRegex(ValueError, 'file set or content'):
            source.validate(self.root)

    def test_rehashed_unclassified_file_does_not_become_reviewed_patch(self):
        (self.vendor / 'extra.rs').write_text('unexpected')
        self.manifest['files'] = source.inventory(self.vendor)
        self.manifest['tree_sha256'] = source.tree_digest(self.manifest['files'])
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            source.validate(self.root)

    def test_cargo_override_and_registry_lock_cannot_substitute_sources(self):
        (self.root / 'Cargo.toml').write_text('[patch.crates-io]\nv8 = { path = "other" }\n')
        with self.assertRaisesRegex(ValueError, 'Cargo does not select'):
            source.validate(self.root)
        (self.root / 'Cargo.toml').write_text('[patch.crates-io]\nv8 = { path = "vendor/rusty-v8" }\n')
        with (self.root / 'Cargo.lock').open('a') as f:
            f.write('source = "registry+https://github.com/rust-lang/crates.io-index"\n')
        with self.assertRaisesRegex(ValueError, 'Cargo lock'):
            source.validate(self.root)

    def test_supplemental_cannot_be_claimed_as_upstream_or_unpinned(self):
        name = 'supplemental.dat'
        (self.vendor / name).write_bytes(b'approved')
        receipt = {'sha256': source.digest(self.vendor / name), 'source_path': 'retained/source', 'icu_revision': 'pinned'}
        self.manifest['files'] = source.inventory(self.vendor)
        self.manifest['tree_sha256'] = source.tree_digest(self.manifest['files'])
        self.manifest['supplemental_files'] = {name: receipt}
        self.write_manifest()
        with self.assertRaisesRegex(ValueError, 'supplemental ICU provenance'):
            source.validate(self.root)
        with patch.object(source, 'SUPPLEMENTAL', {name: receipt}):
            source.validate(self.root)
            self.manifest['base_files'][name] = receipt['sha256']
            self.write_manifest()
            with self.assertRaisesRegex(ValueError, 'falsely claimed as upstream'):
                source.validate(self.root)

    def test_symlink_and_cpp_patch_claim_reject(self):
        (self.vendor / 'alias.rs').symlink_to(self.vendor / 'src/handle.rs')
        with self.assertRaisesRegex(ValueError, 'symlink'):
            source.validate(self.root)
        (self.vendor / 'alias.rs').unlink()
        self.metadata['cpp_changes'] = True
        (self.root / source.METADATA).write_text(json.dumps(self.metadata))
        with self.assertRaisesRegex(ValueError, 'Rust-only'):
            source.validate(self.root)

    def test_root_and_manifest_symlink_substitution_reject_even_identical_bytes(self):
        manifest_path = self.root / source.MANIFEST
        retained = self.root / 'retained-manifest.json'
        retained.write_bytes(manifest_path.read_bytes())
        manifest_path.unlink()
        manifest_path.symlink_to(retained)
        with self.assertRaisesRegex(ValueError, 'vendor or manifest symlink'):
            source.validate(self.root)
        manifest_path.unlink()
        manifest_path.write_bytes(retained.read_bytes())
        moved = self.root / 'actual-vendor'
        self.vendor.rename(moved)
        self.vendor.symlink_to(moved, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'vendor or manifest symlink'):
            source.validate(self.root)

    def test_candidate_snapshot_must_include_every_selected_vendor_input(self):
        import completion_provenance
        import completion_source_snapshot as snapshot
        selected = source.validate(self.root)
        paths = [path.relative_to(self.root).as_posix() for path in self.root.rglob('*') if path.is_file()]
        full = self.root / 'full.tar'
        full_index = snapshot.write(full, self.root, snapshot.inventory(self.root, paths))
        with patch.object(completion_provenance, 'ROOT', self.root):
            completion_provenance.validate_engine_source_snapshot(full, selected)
            validated = snapshot.validate(full, full_index['source_tree_sha256'])
            with patch('tarfile.open', side_effect=AssertionError('validated archive reopened')):
                completion_provenance.validate_engine_source_snapshot(full, selected, validated)
            missing = self.root / 'missing.tar'
            missing_index = snapshot.write(missing, self.root, snapshot.inventory(self.root,
                [name for name in paths if name != source.VENDOR + '/src/handle.rs']))
            with self.assertRaisesRegex(ValueError, 'omits or changes selected Rust V8'):
                completion_provenance.validate_engine_source_snapshot(missing, selected)
            validated_missing = snapshot.validate(missing, missing_index['source_tree_sha256'])
            with patch('tarfile.open', side_effect=AssertionError('validated archive reopened')):
                with self.assertRaisesRegex(ValueError, 'omits or changes selected Rust V8'):
                    completion_provenance.validate_engine_source_snapshot(missing, selected, validated_missing)

    def test_compiled_runtime_must_bind_all_rust_inputs_and_actual_binding(self):
        path = Path(__file__).with_name('validate-v8-rust-runtime.py')
        spec = importlib.util.spec_from_file_location('rust_runtime', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        selected = source.validate(self.root)
        binding = self.root / 'binding.rs'
        binding.write_bytes(b'exact generated API')
        archive = self.root / 'v8.a'
        archive.write_bytes(b'unchanged C++ archive')
        runtime = {field: selected[field] for field in source.FIELDS}
        runtime['v8_binding_sha256'] = source.digest(binding)
        runtime['v8_archive_sha256'] = source.digest(archive)
        module.validate_runtime(selected, runtime, binding, archive, self.root)
        for field in (*source.FIELDS, 'v8_binding_sha256', 'v8_archive_sha256'):
            changed = dict(runtime)
            changed[field] = '0' * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                module.validate_runtime(selected, changed, binding, archive, self.root)


if __name__ == '__main__':
    unittest.main()

"""Production ports retain engine semantics; fixtures never launch an engine."""
import json
import base64
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import production
from oryn_validation.inputs import snapshot_input


class ProductionContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.assets, self.records = {}, {}
        for name in (*production.ENGINE_ASSETS.values(), 'compiler', 'header', 'bridge_build_descriptor'):
            path = self.root / name
            path.write_text(name)
            self.assets[name] = path
            self.records[name] = snapshot_input(self.root, name)

    def bridge(self):
        bridge = {'schema_version': 2, 'kind': 'v8-bridge-abi',
                'target': 'aarch64-apple-darwin', 'libcxx_namespace': '__Cr',
                'libcxx_abi_version': 2, 'stdlib_linkage': 'included-in-rusty-v8',
                'flags': ['-nostdinc++'], 'archive': 'v8_archive',
                'binding': 'bindings_source', 'compiler': 'compiler',
                'inputs': ['compiler', 'header'], 'build_descriptor': 'bridge_build_descriptor'}
        original = {key: value for key, value in bridge.items() if key not in {'kind', 'build_descriptor'}}
        original.update(schema_version=1, compiler='/compile-time/compiler',
                        inputs=[{'path': '/compile-time/' + name, 'sha256': self.records[name]['sha256'],
                                 'size_bytes': self.records[name]['size']} for name in ['compiler', 'header']],
                        archive={'sha256': self.records['v8_archive']['sha256'], 'size_bytes': self.records['v8_archive']['size']},
                        binding={'sha256': self.records['bindings_source']['sha256'], 'size_bytes': self.records['bindings_source']['size']})
        self.assets['bridge_build_descriptor'].write_text(json.dumps(original))
        self.records['bridge_build_descriptor'] = snapshot_input(self.root, 'bridge_build_descriptor')
        return bridge

    def profile(self):
        source = production.SOURCE_ROOT
        selected = production._module('oryn_test_selected_v8', 'scripts/rusty_v8_source_provenance.py').validate(source)
        result = {'schema_version': 7, 'kind': 'v8-runtime-profile', 'profile': 'core-full',
                  'target': 'aarch64-apple-darwin',
                  'v8_revision': 'ac1e23989121713ca642f6650b34deff7b686896',
                  'rusty_v8_revision': '5c15a6995c9bb4bacd3e341b59fff32c909c80bf',
                  'v8_version': '150.4.0', 'icu_revision': 'ee5f27adc28bd3f15b2c293f726d14d2e336cbd5',
                  'v8_heap_sandbox': True, 'pointer_compression': True,
                  'gn_args': 'v8_drumbrake_bounds_checks=true v8_enable_sandbox=true v8_enable_pointer_compression=true',
                  'assets': dict(production.ENGINE_ASSETS),
                  'v8_regression_manifest_sha256': production._hash(source / 'patches/v8/tests/manifest.json'),
                  'rusty_v8_archive_compatibility': {'cpp_changes': False, 'crate_version': '150.4.0',
                      'archive_sha256': self.records['v8_archive']['sha256'],
                      'binding_sha256': self.records['bindings_source']['sha256']},
                  **{field: selected[field] for field in production.SOURCE_FIELDS},
                  **{field: production._hash(source / 'patches/v8' / (name + '.patch'))
                     for field, name in production.PATCHES.items()}}
        self.assets['bridge_abi'].write_text(json.dumps(self.bridge()))
        self.records['bridge_abi'] = snapshot_input(self.root, 'bridge_abi')
        return result

    def validate_engine(self, profile):
        return production._engine(profile, production.SOURCE_ROOT, self.assets,
                                  self.records, 'aarch64-apple-darwin')

    def test_current_typed_engine_and_tracked_patch_chain(self):
        self.assertEqual(self.validate_engine(self.profile()), ['compiler', 'header', 'bridge_build_descriptor'])

    def test_receipt_success_flag_is_not_an_engine_profile(self):
        with self.assertRaisesRegex(ValueError, 'schema differs'):
            self.validate_engine({'valid': True, 'passed': True})

    def test_legacy_paths_cannot_enter_profile(self):
        profile = self.profile()
        profile['archive_path'] = '/old/candidate/archive.a'
        with self.assertRaisesRegex(ValueError, 'schema differs'):
            self.validate_engine(profile)

    def test_conflicting_gn_flag_is_rejected(self):
        profile = self.profile()
        profile['gn_args'] += ' v8_enable_sandbox=false'
        with self.assertRaisesRegex(ValueError, 'conflicting'):
            self.validate_engine(profile)

    def test_core_lite_and_foreign_target_are_rejected(self):
        for field, value in [('profile', 'core-lite'), ('target', 'x86_64-unknown-linux-gnu')]:
            profile = self.profile()
            profile[field] = value
            with self.assertRaisesRegex(ValueError, 'core-full target'):
                self.validate_engine(profile)

    def test_forged_patch_or_selected_source_digest_rejected(self):
        for field in ('v8_patch_sha256', 'rusty_v8_source_tree_sha256'):
            profile = self.profile()
            profile[field] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'mismatch'):
                self.validate_engine(profile)

    def test_signed_and_unsigned_worker_roles_cannot_be_conflated(self):
        profile = self.profile()
        profile['assets']['worker'] = 'worker'
        with self.assertRaisesRegex(ValueError, 'roles substituted'):
            self.validate_engine(profile)

    def test_cpp_changes_cannot_reuse_archive(self):
        profile = self.profile()
        profile['rusty_v8_archive_compatibility']['cpp_changes'] = True
        with self.assertRaisesRegex(ValueError, 'compatibility mismatch'):
            self.validate_engine(profile)

    def test_bridge_compiler_must_be_in_declared_inputs(self):
        bridge = self.bridge()
        bridge['inputs'] = ['header']
        with self.assertRaisesRegex(ValueError, 'compiler provenance'):
            production._bridge(bridge, self.assets, self.records, bridge['target'])

    def test_bridge_undeclared_input_rejected(self):
        bridge = self.bridge()
        bridge['inputs'].append('old-candidate-header')
        with self.assertRaisesRegex(ValueError, 'absent from typed closure'):
            production._bridge(bridge, self.assets, self.records, bridge['target'])

    def test_bridge_system_headers_and_abi_drift_rejected(self):
        for field, value in [('flags', []), ('libcxx_namespace', 'std'), ('libcxx_abi_version', True)]:
            bridge = self.bridge()
            bridge[field] = value
            with self.assertRaises(ValueError):
                production._bridge(bridge, self.assets, self.records, bridge['target'])

    def test_relocated_asset_paths_do_not_change_engine_identity(self):
        profile = self.profile()
        expected = self.validate_engine(profile)
        relocated = self.root / 'relocated'
        relocated.mkdir()
        for name, path in list(self.assets.items()):
            destination = relocated / name
            destination.write_bytes(path.read_bytes())
            self.assets[name] = destination
        self.assertEqual(expected, self.validate_engine(profile))

    def test_duplicate_json_key_rejected(self):
        path = self.root / 'duplicate.json'
        path.write_text('{"valid":false,"valid":true}')
        with self.assertRaisesRegex(ValueError, 'duplicate JSON key'):
            production._read(path)

    def source_export(self, extra=None):
        source = self.root / 'source'
        source.mkdir()
        files = {'Cargo.lock': b'lock', 'Cargo.toml': b'workspace', **(extra or {})}
        for name, content in files.items():
            (source / name).parent.mkdir(parents=True, exist_ok=True)
            (source / name).write_bytes(content)
        trees = {}
        def encode_tree(directory):
            tree = b''
            for item in sorted(directory.iterdir(), key=lambda p: p.name.encode() + (b'/' if p.is_dir() else b'')):
                if item.is_dir():
                    oid, mode = encode_tree(item), b'40000'
                else:
                    oid, mode = production._git_object('blob', item.read_bytes(), 'sha1'), b'100644'
                tree += mode + b' ' + item.name.encode() + b'\0' + bytes.fromhex(oid)
            oid = production._git_object('tree', tree, 'sha1')
            trees[oid] = base64.b64encode(tree).decode()
            return oid
        tree_oid = encode_tree(source)
        commit = ('tree ' + tree_oid + '\nauthor Test <test@example.invalid> 0 +0000\n'
                  'committer Test <test@example.invalid> 0 +0000\n\nfixture\n').encode()
        identity = {'schema_version': 1, 'kind': 'git-source-export',
                    'commit': production._git_object('commit', commit, 'sha1'),
                    'commit_object': base64.b64encode(commit).decode(),
                    'trees': trees}
        return source, identity, snapshot_input(self.root, 'source', kind='directory')

    def production_bound(self):
        source, identity, member = self.source_export({'crates/oryn-common/src/v4.rs':
            b'pub const RENDER_CONTRACT_VERSION: u16 = 2;\npub const LOCAL_EFFECTS_CONTRACT_VERSION: u16 = 3;\n'})
        source_admission = production.validate_source_export(identity, source, member)
        self.assets['source_export'] = source
        self.records['source_export'] = member
        self.assets['source_identity'] = self.root / 'source-identity.json'
        self.assets['source_identity'].write_text(json.dumps(identity))
        bundle = self.root / 'bundle'
        app = bundle / 'Worker.app'
        app.mkdir(parents=True)
        for name, relative in [('runtime', 'bin/oryn'), ('driver', 'bin/driver'),
                               ('worker', 'Worker.app/Contents/MacOS/oryn-page-worker')]:
            p = bundle / relative
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(name)
            p.chmod(0o755)
            self.assets[name] = p
        resources = app / 'Contents/Resources'
        resources.mkdir()
        profile = {'profile': 'core-full', 'target': 'aarch64-apple-darwin',
                   **{field: 'd' * 64 for field in production.SOURCE_FIELDS}}
        self.assets['engine_profile'] = resources / 'v8-profile-manifest.json'
        self.assets['engine_profile'].write_text(json.dumps(profile))
        manifest = {'source_tree_sha256': source_admission['tree_sha256'],
                    'source_commit': identity['commit'],
                    'worker_build_sha256': self.records['engine_worker']['sha256'],
                    'worker_build_bytes': self.records['engine_worker']['size'],
                    'v8_archive_sha256': self.records['v8_archive']['sha256'],
                    'v8_profile_sha256': production._hash(self.assets['engine_profile']),
                    'v8_binding_sha256': self.records['bindings_source']['sha256'],
                    'render_contract_version': 2, 'local_effects_contract_version': 3,
                    **{field: profile[field] for field in production.SOURCE_FIELDS}}
        self.assets['worker_manifest'] = resources / 'worker-manifest.json'
        self.assets['worker_manifest'].write_text(json.dumps(manifest))
        self.assets['worker_app'] = app
        for name, path in self.assets.items():
            in_bundle = path.is_relative_to(bundle)
            root = bundle if in_bundle else self.root
            self.records[name] = snapshot_input(root, path.relative_to(root).as_posix(),
                scope='bundle' if in_bundle else 'store', kind='directory' if path.is_dir() else 'file')
        signature = {'path': self.records['worker']['path'], 'verifier': 'security_prebuilt.verify_signature',
                     'identity': 'F' * 40, 'valid': True}
        configuration = {'profile': 'release', 'target': 'aarch64-apple-darwin', 'host': 'aarch64-apple-darwin',
                         'toolchain': 'test rustc', 'cargo_lock_sha256': production._hash(source / 'Cargo.lock'),
                         'features': ['native', 'core-full'], 'flags': []}
        build = {'schema_version': 1, 'kind': 'build', 'build_id': 'test-production',
                 'source': {'commit': identity['commit'], 'tree_sha256': source_admission['tree_sha256'], 'dirty': False},
                 'configuration': configuration, 'commands': [['cargo', 'build', '--release']],
                 'immutable_inputs': [r for r in self.records.values() if r['scope'] == 'store'],
                 'artifacts': [r for r in self.records.values() if r['scope'] == 'bundle'],
                 'signatures': [signature], 'development_only': False}
        build['build_inputs'] = list(build['immutable_inputs'])
        build['build_inputs_sha256'] = production.builds.native_inputs_digest(source_admission, configuration,
                                                                              external_inputs=build['build_inputs'])
        self.assets['build_manifest'] = self.root / 'build.json'
        self.assets['build_manifest'].write_text(json.dumps(build))
        self.records['build_manifest'] = snapshot_input(self.root, 'build.json')
        bound = {'run': {'assets': dict(self.records), 'build': {'build_id': build['build_id'],
                    'manifest_sha256': production.manifest_digest(build)}}, 'assets': dict(self.assets),
                 'records': list(self.records.values()), 'store_root': self.root, 'bundle_root': bundle}
        return bound, signature

    def test_production_rechecks_signature_instead_of_accepting_saved_valid_flag(self):
        bound, signature = self.production_bound()
        # Engine byte/profile semantics have dedicated real-source tests above;
        # this fixture isolates the full source/build/app/signature integration.
        with patch.object(production, '_engine', return_value=['compiler']), \
             patch('oryn_validation.build_evidence.verify_build_evidence') as build_evidence, \
             patch.object(production.builds, 'verify_worker_signature', return_value=signature) as verify:
            self.assertEqual(production.validate_production(bound)['build_id'], 'test-production')
            verify.assert_called_once()
            build_evidence.assert_called_once()
        with patch.object(production, '_engine', return_value=['compiler']), \
             patch('oryn_validation.build_evidence.verify_build_evidence'), \
             patch.object(production.builds, 'verify_worker_signature', side_effect=ValueError('signature revoked')):
            with self.assertRaisesRegex(ValueError, 'signature revoked'):
                production.validate_production(bound)

    def test_production_rejects_wrong_run_build_identity(self):
        bound, _ = self.production_bound()
        bound['run']['build']['manifest_sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'run/build identity'):
            production.validate_production(bound)

    def test_production_rejects_same_size_worker_drift(self):
        bound, _ = self.production_bound()
        bound['assets']['worker'].write_bytes(b'broken')
        with self.assertRaisesRegex(ValueError, 'changed'):
            production.validate_production(bound)

    def test_source_export_proves_commit_without_git_repository(self):
        source, identity, membership = self.source_export()
        result = production.validate_source_export(identity, source, membership)
        self.assertEqual(identity['commit'], result['commit'])
        self.assertFalse(result['dirty'])
        self.assertEqual(set(result['tracked']), {'Cargo.lock', 'Cargo.toml'})

    def test_source_commit_claim_without_matching_object_rejected(self):
        source, identity, membership = self.source_export()
        identity['commit'] = 'a' * 40
        with self.assertRaisesRegex(ValueError, 'commit object hash'):
            production.validate_source_export(identity, source, membership)

    def test_source_blob_edit_rejected_even_with_updated_membership(self):
        source, identity, membership = self.source_export()
        (source / 'Cargo.lock').write_text('edit')
        membership = snapshot_input(self.root, 'source', kind='directory')
        with self.assertRaisesRegex(ValueError, 'source blob differs'):
            production.validate_source_export(identity, source, membership)

    def test_source_executable_mode_drift_rejected(self):
        source, identity, membership = self.source_export()
        (source / 'Cargo.lock').chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'executable mode'):
            production.validate_source_export(identity, source, membership)

    def test_extra_export_file_rejected(self):
        source, identity, membership = self.source_export()
        (source / 'ignored-build-source.rs').write_text('extra')
        membership = snapshot_input(self.root, 'source', kind='directory')
        with self.assertRaisesRegex(ValueError, 'membership differs'):
            production.validate_source_export(identity, source, membership)

    def test_extra_git_tree_object_rejected(self):
        source, identity, membership = self.source_export()
        identity['trees'][production._git_object('tree', b'', 'sha1')] = ''
        with self.assertRaisesRegex(ValueError, 'unused Git tree'):
            production.validate_source_export(identity, source, membership)

    def test_git_tree_mode_cannot_admit_symlink(self):
        source, identity, membership = self.source_export()
        old_oid, raw = next(iter(identity['trees'].items()))
        tree = base64.b64decode(raw).replace(b'100644 Cargo.lock', b'120000 Cargo.lock')
        new_oid = production._git_object('tree', tree, 'sha1')
        commit = base64.b64decode(identity['commit_object']).replace(old_oid.encode(), new_oid.encode())
        identity.update(commit=production._git_object('commit', commit, 'sha1'),
                        commit_object=base64.b64encode(commit).decode(),
                        trees={new_oid: base64.b64encode(tree).decode()})
        with self.assertRaisesRegex(ValueError, 'symlink/gitlink'):
            production.validate_source_export(identity, source, membership)


if __name__ == '__main__':
    unittest.main()

"""Runtime ABI and dependency proofs; no Cargo or native execution."""
import hashlib
import base64
import io
import json
import tarfile
import unittest
from unittest.mock import patch, MagicMock

from oryn_validation import runtime_readiness as subject, production
import test_production


class RuntimeReadinessTests(unittest.TestCase):
    def setUp(self):
        fixture = test_production.ProductionContracts()
        fixture.setUp(); self.addCleanup(fixture.doCleanups)
        self.fixture = fixture

    def descriptors(self):
        profile = self.fixture.profile()
        assets = self.fixture.assets
        for name, value in [('engine_profile', profile), ('worker_manifest',
                {'v8_patch_chain_sha256': subject.patch_chain(production.SOURCE_ROOT)})]:
            assets[name] = self.fixture.root / name; assets[name].write_text(json.dumps(value))
        assets['worker'] = self.fixture.root / 'signed-worker'; assets['worker'].write_text('signed')
        assets['source_export'] = production.SOURCE_ROOT
        common = {'native_v8': True, 'build_profile': 'release',
                  'v8_archive_sha256': production._hash(assets['v8_archive']),
                  'v8_binding_sha256': production._hash(assets['bindings_source']),
                  'startup_snapshot_sha256': production._hash(assets['startup_snapshot']),
                  'v8_bridge_abi_sha256': production._hash(assets['bridge_build_descriptor']),
                  'v8_patch_chain_sha256': subject.patch_chain(production.SOURCE_ROOT),
                  **{field: profile[field] for field in production.SOURCE_FIELDS}}
        protocols = production._module('test_runtime_versions', 'scripts/native_protocol_versions.py').versions()
        common.update(context_service_protocol_version=protocols['context'], agent_ipc_protocol_version=protocols['agent'])
        parent = dict(common, worker_protocol_version=protocols['worker'], execution_mode='sandboxed_worker',
                      sandboxed=True, sandbox_state='app_sandbox', worker_bundle_id='ai.rustic.oryn.page-worker',
                      worker_team_id='5HVA9VFF8K', worker_signing_cert_sha1='9A1405B3288A8DD06DE0D29CCCD02B8B45A33F90',
                      worker_binary_sha256=production._hash(assets['worker']))
        return {'assets': assets}, parent, common

    def test_actual_raw_descriptor_hash_and_compiled_sources_join(self):
        bound, parent, worker = self.descriptors()
        subject.validate_descriptors(bound, parent, worker)
        parent['v8_bridge_abi_sha256'] = production._hash(bound['assets']['bridge_abi'])
        with self.assertRaisesRegex(ValueError, 'original bridge ABI'):
            subject.validate_descriptors(bound, parent, worker)

    def test_wrong_worker_signed_bytes_protocol_or_source_is_rejected(self):
        for field, value in [('worker_binary_sha256', '0' * 64), ('worker_protocol_version', True),
                             ('rusty_v8_source_tree_sha256', '0' * 64), ('sandboxed', False),
                             ('startup_snapshot_sha256', '0' * 64)]:
            bound, parent, worker = self.descriptors(); parent[field] = value
            with self.assertRaises(ValueError):
                subject.validate_descriptors(bound, parent, worker)

    def test_all_three_parent_dependency_graphs_required(self):
        trees = {'default_parent': 'oryn v1\n', 'release_parent': 'oryn v1\n', 'worker_graph': 'oryn-native v1\n└── v8 v150.4.0\n'}
        subject.validate_parent_purity(trees)
        for name, value in [('default_parent', 'oryn v1\n└── v8 v150.4.0\n'), ('worker_graph', 'oryn-native v1\n'), ('release_parent', '')]:
            with self.assertRaises(ValueError):
                subject.validate_parent_purity(dict(trees, **{name: value}))

    def registry_fixture(self):
        from oryn_validation import cargo_custody
        home = self.fixture.root / 'cargo-home'
        root = home / 'registry/src/example/demo-1.0.0'; root.mkdir(parents=True)
        archive = home / 'registry/cache/example/demo-1.0.0.crate'; archive.parent.mkdir(parents=True)
        content = b'[package]\nname="demo"\nversion="1.0.0"\n'
        with tarfile.open(archive, 'w:gz') as stream:
            member = tarfile.TarInfo('demo-1.0.0/Cargo.toml'); member.size = len(content)
            stream.addfile(member, io.BytesIO(content))
        (root / 'Cargo.toml').write_bytes(content)
        checksum = {'package': production._hash(archive), 'files': {'Cargo.toml': hashlib.sha256(content).hexdigest()}}
        (root / '.cargo-checksum.json').write_text(json.dumps(checksum))
        package = {'name': 'demo', 'version': '1.0.0', 'manifest_path': str(root / 'Cargo.toml')}
        return cargo_custody, home, root, package, {'checksum': checksum['package']}

    def test_dirty_registry_manifest_cannot_hide_behind_edited_checksum_metadata(self):
        module, home, root, package, locked = self.registry_fixture()
        module._registry(package, locked, home)
        (root / 'Cargo.toml').write_text('forged dependency manifest')
        checksum = json.loads((root / '.cargo-checksum.json').read_text())
        checksum['files']['Cargo.toml'] = production._hash(root / 'Cargo.toml')
        (root / '.cargo-checksum.json').write_text(json.dumps(checksum))
        with self.assertRaisesRegex(ValueError, 'locked archive'):
            module._registry(package, locked, home)

    def test_untracked_registry_source_cannot_change_graph_inputs(self):
        module, home, root, package, locked = self.registry_fixture()
        (root / 'unexpected.rs').write_text('untracked source')
        with self.assertRaisesRegex(ValueError, 'population'):
            module._registry(package, locked, home)

    def test_archive_path_replacement_cannot_change_the_verified_stream(self):
        module, home, _, package, locked = self.registry_fixture()
        archive = next((home / 'registry/cache').glob('*/*.crate'))
        original_open = module.tarfile.open
        def replace_then_read(*args, **kwargs):
            replacement = archive.with_suffix('.new'); replacement.write_bytes(archive.read_bytes())
            replacement.replace(archive)
            return original_open(*args, **kwargs)
        with patch.object(module.tarfile, 'open', side_effect=replace_then_read):
            with self.assertRaisesRegex(ValueError, 'archive changed'):
                module._registry(package, locked, home)

    def test_locked_git_source_rejects_untracked_and_modified_files(self):
        from oryn_validation import cargo_custody
        source, identity, _ = self.fixture.source_export()
        rows = b''.join(b'100644 blob ' + production._git_object('blob', path.read_bytes(), 'sha1').encode()
                        + b'\t' + path.name.encode() + b'\0' for path in sorted(source.iterdir()))
        def git(command, **_):
            args = command[3:]
            if args == ['rev-parse', '--show-toplevel']: return str(source).encode()
            if args == ['rev-parse', 'HEAD']: return identity['commit'].encode()
            if args[:2] == ['cat-file', 'commit']: return base64.b64decode(identity['commit_object'])
            if args[:2] == ['cat-file', 'tree']: return base64.b64decode(identity['trees'][args[2]])
            if args[0] == 'ls-tree': return rows
            self.fail('unexpected Git command: ' + repr(command))
        package = {'manifest_path': str(source / 'Cargo.toml')}
        locked = {'source': 'git+https://example.invalid/source#' + identity['commit']}
        with patch.object(cargo_custody.subprocess, 'check_output', side_effect=git):
            cargo_custody._git(package, locked)
            extra = source / 'untracked.rs'; extra.write_text('extra')
            with self.assertRaisesRegex(ValueError, 'uncommitted effective'):
                cargo_custody._git(package, locked)
            extra.unlink(); (source / 'Cargo.toml').write_text('modified')
            with self.assertRaisesRegex(ValueError, 'content/mode differs'):
                cargo_custody._git(package, locked)

    def test_observed_toolchain_must_match_build_bytes_and_version(self):
        observed = {name: {'path': '/observed/bin/' + name, 'sha256': 'a' * 64, 'size': 10, 'executable': True}
                    for name in ('cargo', 'rustc')}
        assets = {}
        for name, value in [('build_manifest', {'configuration': {'toolchain': 'rustc fixture', 'host': 'test-host'}}),
                            ('build_execution', {'toolchain_executables': observed})]:
            assets[name] = self.fixture.root / name; assets[name].write_text(json.dumps(value))
        bound = {'assets': assets}; version = 'rustc fixture\nhost: test-host\n'
        subject.validate_toolchain(bound, observed, version)
        with self.assertRaisesRegex(ValueError, 'version differs'):
            subject.validate_toolchain(bound, observed, 'rustc another\nhost: test-host\n')
        observed['cargo']['sha256'] = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'differs from actual build'):
            subject.validate_toolchain(bound, observed, version)

    def test_runtime_collector_keeps_worker_descriptor_and_dependency_probe_distinct(self):
        from oryn_validation import prerequisites
        bound, parent, worker = self.descriptors()
        bound['assets']['runtime'] = self.fixture.root / 'runtime'
        from oryn_validation.inputs import snapshot_input
        bound['assets']['worker'].chmod(0o755)
        selected_worker = snapshot_input(self.fixture.root, 'signed-worker', scope='bundle')
        bound['bundle_root'] = self.fixture.root
        bound['run'] = {'assets': {'engine_worker': {'executable': True}, 'worker': selected_worker},
                        'immutable_inputs': [selected_worker],
                        'build': {'build_id': 'fixture', 'manifest_sha256': 'a' * 64}}
        output = self.fixture.root / 'runtime-output'; output.mkdir()
        observed = {name: {'path': '/observed/bin/' + name, 'sha256': 'a' * 64, 'size': 10, 'executable': True}
                    for name in ('cargo', 'rustc')}
        def owned(command, *, output, **kwargs):
            self.assertEqual(kwargs['env']['ORYN_PAGE_WORKER'], str(bound['assets']['worker']))
            output.mkdir()
            name = output.name
            if name in {'parent', 'worker'}:
                raw = json.dumps(parent if name == 'parent' else worker)
            elif name.startswith('metadata_'):
                raw = json.dumps({'packages': []})
            elif name == 'toolchain_version': raw = 'rustc fixture\nhost: test-host\n'
            elif name == 'worker_graph': raw = 'oryn-native v1\n└── v8 v150.4.0\n'
            else: raw = 'oryn v1\n'
            (output / 'stdout.log').write_text(raw)
            return {'argv': command, 'returncode': 0, 'timed_out': False, 'error': None,
                    'cleanup_complete': True, 'cleanup_errors': [], 'forced_cleanup': False, 'cancellation_signal': None}
        with patch('oryn_validation.build_evidence._toolchain', return_value=observed), \
             patch('oryn_validation.build_evidence._reject_implicit_cargo_configuration'), \
             patch.object(subject, 'validate_toolchain'), patch.object(subject, 'validate_profile_equivalence', return_value={}), \
             patch('oryn_validation.cargo_custody.observe_packages', return_value={'fixture': 'observed'}), \
             patch.object(prerequisites, 'run_owned', side_effect=owned):
            result = prerequisites._collect_runtime(bound, MagicMock(), output)
        self.assertEqual(result['errors'], [])
        self.assertEqual(len(result['processes']), 9)
        self.assertEqual(result['descriptors']['worker'], worker)
        self.assertIn('worker_graph', result['trees'])

    def test_profile_sharing_excludes_only_two_reviewed_axes(self):
        assets = self.fixture.assets
        for name, value in [('one', {'web_features': ['a'], 'third_party_cookies': 'block', 'tls': {'port': 443}}),
                            ('two', {'web_features': ['b'], 'third_party_cookies': 'allow', 'tls': {'port': 443}})]:
            assets[name] = self.fixture.root / name; assets[name].write_text(json.dumps(value))
        registry = {'primary_profile_id': 'primary', 'profiles': {'primary': {'fixture': 'one', 'platform': 'macos'},
                    'peer': {'fixture': 'two', 'platform': 'macos'}}}
        assets['core_profile_registry'] = self.fixture.root / 'registry'
        assets['core_profile_registry'].write_text(json.dumps(registry))
        bound = {'run': {'profile': 'primary'}, 'assets': assets}
        with patch('oryn_validation.admission.validate_registry'):
            subject.validate_profile_equivalence(bound)
            assets['two'].write_text(json.dumps({'tls': {'port': 444}}))
            with self.assertRaisesRegex(ValueError, 'separate native'):
                subject.validate_profile_equivalence(bound)


if __name__ == '__main__':
    unittest.main()

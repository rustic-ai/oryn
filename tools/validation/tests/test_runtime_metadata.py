"""Typed publication tests; native compilation and legacy semantic checks mocked."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from oryn_validation import production, runtime_metadata
from oryn_validation.inputs import verify_inputs


class RuntimeMetadataContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / 'source'
        self.source.mkdir()
        self.store = self.root / 'store'
        self.store.mkdir()
        self.raw = self.root / 'raw'
        self.raw.mkdir()
        self.source_info = {'commit': 'a' * 40, 'tree_sha256': 'b' * 64, 'dirty': False}
        self.source_patch = patch.object(runtime_metadata, 'source_info', return_value=self.source_info)
        self.source_patch.start()
        self.addCleanup(self.source_patch.stop)
        self.validator_patch = patch.object(runtime_metadata, '_legacy_validator', return_value=Mock())
        self.validator_patch.start()
        self.addCleanup(self.validator_patch.stop)
        self.profile = {key: 'fixture' for key in runtime_metadata.PROFILE_FIELDS}
        self.profile.update(schema_version=6, profile='core-full', target='aarch64-apple-darwin')
        for name in production.ENGINE_ASSETS:
            if name != 'bridge_abi':
                self.record_profile(name, self.file(name, b'artifact ' + name.encode()))
        compiler = self.file('compiler', b'compiler')
        compiler.chmod(0o755)
        header = self.file('header', b'header')
        descriptor = {'schema_version': 1, 'target': 'aarch64-apple-darwin',
                      'libcxx_namespace': '__Cr', 'libcxx_abi_version': 2,
                      'stdlib_linkage': 'included-in-rusty-v8', 'compiler': str(compiler),
                      'flags': ['-nostdinc++', '-isystem' + str(self.raw), '-isysroot', '/sdk',
                                '-fcrash-diagnostics-dir=' + str(self.raw / 'reports')],
                      'archive': self.record(self.raw / 'archive'),
                      'binding': self.record(self.raw / 'binding'),
                      'inputs': [self.record(compiler), self.record(header)]}
        self.descriptor = self.file('descriptor.json', json.dumps(descriptor).encode())
        self.record_profile('bridge_abi', self.descriptor)
        self.profile_file = self.file('profile.json', json.dumps(self.profile).encode())

    def file(self, name, data):
        path = self.raw / name
        path.write_bytes(data)
        return path

    @staticmethod
    def record(path):
        return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                'size_bytes': path.stat().st_size}

    def record_profile(self, name, path):
        for key, value in self.record(path).items():
            self.profile[name + '_' + key] = value

    def prepare(self):
        return runtime_metadata.prepare_typed_runtime(self.source, self.store,
                   legacy_profile=self.profile_file, output_directory='runtime/new', source_identity='mocked')

    def stage_fixture(self):
        sdk = self.root / 'sdk'
        sdk.mkdir()
        (sdk / 'SDKSettings.json').write_text('{"Version":"test"}')
        descriptor = json.loads(self.descriptor.read_text())
        descriptor['sdkroot'] = str(sdk)
        descriptor['flags'][descriptor['flags'].index('/sdk')] = str(sdk)
        descriptor['header_roots'] = [str(self.raw)]
        self.descriptor.write_text(json.dumps(descriptor))
        self.record_profile('bridge_abi', self.descriptor)
        self.record_profile('rusty_v8_source_manifest', self.file('rusty-source.json', b'source'))
        self.profile_file.write_text(json.dumps(self.profile))
        runtime_metadata._legacy_validator.return_value.ARTIFACTS = (*production.ENGINE_ASSETS,
                                                                     'rusty_v8_source_manifest')
        return descriptor

    def test_stage_preserves_compiler_layout_and_remaps_live_paths(self):
        descriptor = self.stage_fixture()
        result = runtime_metadata.stage_engine_inputs(self.source, self.store,
            legacy_profile=self.profile_file, output_directory='inputs/engine',
            diagnostics_directory='builds/new/crashreports')
        profile = json.loads(Path(result['profile_path']).read_text())
        bridge = json.loads(Path(profile['bridge_abi_path']).read_text())
        self.assertEqual(production.normalize_bridge_flags(bridge['flags']),
                         production.normalize_bridge_flags(descriptor['flags']))
        self.assertNotEqual(profile['bridge_abi_sha256'], self.profile['bridge_abi_sha256'])
        for role in runtime_metadata._legacy_validator.return_value.ARTIFACTS:
            self.assertTrue(Path(profile[role + '_path']).is_relative_to(self.store))
        for record in bridge['inputs']:
            self.assertTrue(Path(record['path']).is_relative_to(self.store))
        self.assertTrue(Path(bridge['compiler']).stat().st_mode & 0o111)
        self.assertNotEqual(Path(bridge['compiler']).stat().st_ino,
                            (self.raw / 'compiler').stat().st_ino)
        self.assertEqual(Path(bridge['compiler']).parent, Path(bridge['header_roots'][0]))
        self.assertEqual(result['environment']['SDKROOT'], descriptor['sdkroot'])
        self.assertFalse(result['sdk_observation']['full_sdk_contents_attested'])
        file_paths = {str(self.store / record['path']) for record in result['build_inputs']
                      if record['kind'] == 'file'}
        self.assertIn(result['profile_path'], file_paths)
        self.assertTrue(all(record['path'] in file_paths for record in bridge['inputs']))
        self.assertNotIn(self.root.name, json.dumps(result['build_inputs']))
        verify_inputs(result['build_inputs'], store_root=self.store)
        self.assertEqual(json.loads(self.descriptor.read_text()), descriptor)

    def test_stage_rejects_undeclared_forced_header_and_mutable_diagnostics_overlap(self):
        self.stage_fixture()
        with self.assertRaisesRegex(ValueError, 'diagnostics'):
            runtime_metadata.stage_engine_inputs(self.source, self.store,
                legacy_profile=self.profile_file, output_directory='inputs/engine',
                diagnostics_directory='inputs/engine/crashreports')
        descriptor = json.loads(self.descriptor.read_text())
        descriptor['flags'] += ['-include', str(self.raw / 'missing-header')]
        self.descriptor.write_text(json.dumps(descriptor))
        self.record_profile('bridge_abi', self.descriptor)
        self.profile_file.write_text(json.dumps(self.profile))
        with self.assertRaisesRegex(ValueError, 'forced include'):
            runtime_metadata.stage_engine_inputs(self.source, self.store,
                legacy_profile=self.profile_file, output_directory='inputs/other',
                diagnostics_directory='builds/new/crashreports')

    def test_staged_compiler_finds_its_relative_resource_headers(self):
        descriptor = self.stage_fixture()
        compiler = self.raw / 'toolchain/bin/clang'
        resource = self.raw / 'toolchain/lib/clang/23/include/stddef.h'
        header = self.raw / 'libcxx/include/memory'
        for path, data in ((compiler, b'compiler'), (resource, b'resource'), (header, b'header')):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        compiler.chmod(0o755)
        descriptor['compiler'] = str(compiler)
        descriptor['inputs'] = [self.record(path) for path in (compiler, resource, header)]
        descriptor['header_roots'] = [str(resource.parents[2]), str(header.parent)]
        descriptor['flags'][1] = '-isystem' + str(header.parent)
        self.descriptor.write_text(json.dumps(descriptor))
        self.record_profile('bridge_abi', self.descriptor)
        self.profile_file.write_text(json.dumps(self.profile))
        result = runtime_metadata.stage_engine_inputs(self.source, self.store,
            legacy_profile=self.profile_file, output_directory='inputs/toolchain',
            diagnostics_directory='builds/new/crashreports')
        profile = json.loads(Path(result['profile_path']).read_text())
        bridge = json.loads(Path(profile['bridge_abi_path']).read_text())
        self.assertEqual((Path(bridge['compiler']).parent.parent /
                         'lib/clang/23/include/stddef.h').read_bytes(), b'resource')
        runtime_metadata.verify_sdk_observation(result['sdk_observation'])
        (self.root / 'sdk/SDKSettings.json').write_text('{"Version":"drift"}')
        with self.assertRaisesRegex(ValueError, 'SDK identity changed'):
            runtime_metadata.verify_sdk_observation(result['sdk_observation'])

    def test_typed_copy_has_no_paths_and_retains_exact_descriptor_evidence(self):
        receipt = self.prepare()
        records = receipt['assets']
        assets = {name: self.store / value['path'] for name, value in records.items()}
        bridge = json.loads(assets['bridge_abi'].read_text())
        profile = json.loads(assets['engine_profile'].read_text())
        self.assertEqual(profile['schema_version'], 7)
        self.assertNotIn(str(self.root), json.dumps(profile) + json.dumps(bridge) + json.dumps(receipt))
        self.assertEqual(assets['bridge_build_descriptor'].read_bytes(), self.descriptor.read_bytes())
        production._bridge(bridge, assets, records, 'aarch64-apple-darwin')
        self.assertTrue(receipt['build_process_evidence_required'])
        self.assertNotEqual(assets['engine_worker'].stat().st_ino, (self.raw / 'worker').stat().st_ino)

    def test_changed_raw_artifact_and_reused_destination_rejected(self):
        self.prepare()
        with self.assertRaisesRegex(ValueError, 'new external'):
            self.prepare()
        (self.raw / 'archive').write_bytes(b'drift')
        with self.assertRaisesRegex(ValueError, 'drift'):
            runtime_metadata.prepare_typed_runtime(self.source, self.store,
                legacy_profile=self.profile_file, output_directory='runtime/other', source_identity='mocked')

    def test_profile_replacement_preserves_full_current_semantics(self):
        receipt = self.prepare()
        profile = self.store / receipt['assets']['engine_profile']['path']
        self.assertEqual(runtime_metadata.validate_profile_replacement(self.source, self.profile_file,
                         profile, 'mocked'), self.source_info)
        incomplete = self.root / 'incomplete.json'
        incomplete.write_text(json.dumps({'schema_version': 7, 'kind': 'v8-runtime-profile',
                                          'assets': production.ENGINE_ASSETS}))
        with self.assertRaisesRegex(ValueError, 'differs'):
            runtime_metadata.validate_profile_replacement(self.source, self.profile_file, incomplete, 'mocked')

    def test_refresh_uses_new_worker_and_rejects_bridge_or_runtime_mismatch(self):
        scripts = self.source / 'scripts'
        scripts.mkdir()
        (scripts / 'validate-v8-rust-runtime.py').write_text(
            "def validate_runtime(selected, runtime, binding, archive, root):\n"
            "    if runtime.get('reject'): raise ValueError('compiled source mismatch')\n")
        worker = self.file('fresh-worker', b'fresh compiled bytes')
        runtime = self.file('runtime.json', json.dumps({'v8_bridge_abi_sha256':
                                                       self.profile['bridge_abi_sha256']}).encode())
        output = self.root / 'refreshed.json'
        runtime_metadata.refresh_engine_profile(self.source, self.profile_file, output,
                                                 worker=worker, runtime_info=runtime)
        refreshed = json.loads(output.read_text())
        self.assertEqual(refreshed['worker_path'], str(worker))
        self.assertEqual(refreshed['worker_sha256'], hashlib.sha256(worker.read_bytes()).hexdigest())
        self.assertNotEqual(refreshed['worker_sha256'], self.profile['worker_sha256'])
        runtime.write_text(json.dumps({'v8_bridge_abi_sha256': 'f' * 64}))
        with self.assertRaisesRegex(ValueError, 'bridge descriptor'):
            runtime_metadata.refresh_engine_profile(self.source, self.profile_file, self.root / 'bad.json',
                                                     worker=worker, runtime_info=runtime)
        runtime.write_text(json.dumps({'reject': True}))
        with self.assertRaisesRegex(ValueError, 'compiled source'):
            runtime_metadata.refresh_engine_profile(self.source, self.profile_file, self.root / 'bad.json',
                                                     worker=worker, runtime_info=runtime)
        with self.assertRaisesRegex(ValueError, 'runtime-info'):
            runtime_metadata.refresh_engine_profile(self.source, self.profile_file, self.root / 'bad.json', worker=worker)


if __name__ == '__main__':
    unittest.main()

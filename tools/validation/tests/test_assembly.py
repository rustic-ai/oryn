"""Final assembly integration with real receipts/copies and mocked platform signing.

Fixture executables are text and are never launched. Engine semantic validation
has separate tests; original process/source/output joins execute here unchanged.
"""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import assembly, build_evidence, builds, generation, production, retention
from oryn_validation.inputs import snapshot_input, verify_inputs
from oryn_validation.manifests import load_manifest, manifest_digest


class AssemblyContracts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source, self.store = self.root / 'checkout', self.root / 'store'
        self.source.mkdir()
        self.store.mkdir()
        files = {'Cargo.lock': 'lock', 'Cargo.toml': '[workspace]\n', 'implementation': 'impl', 'regression': 'test',
            'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md': 'plan',
            'crates/oryn-common/src/v4.rs': 'pub const RENDER_CONTRACT_VERSION: u16 = 2;\n'
                'pub const LOCAL_EFFECTS_CONTRACT_VERSION: u16 = 3;\n'}
        for name, value in files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value)
        for args in (['init', '-q'], ['config', 'user.email', 'test@example.invalid'],
                     ['config', 'user.name', 'Test'], ['add', '.'],
                     ['-c', 'core.hooksPath=/dev/null', 'commit', '--no-gpg-sign', '-qm', 'fixture']):
            subprocess.run(['git', '-C', str(self.source), *args], check=True, capture_output=True)
        exported = generation.export_source(self.source, self.store,
            export_path='sources/test/export', identity_path='sources/test/identity.json')
        self.identity = exported['source']
        self.cfg = {'profile': 'release', 'host': 'aarch64-apple-darwin', 'target': 'aarch64-apple-darwin',
                    'toolchain': 'rustc test', 'cargo_lock_sha256': production._hash(self.source / 'Cargo.lock'),
                    'features': sorted(build_evidence.FEATURES), 'flags': []}
        self.gate = {'schema_version': 2, 'kind': 'committed-source-generation', 'source': self.identity,
            'plan_sha256': production._hash(self.source / generation.PLAN), 'requirements': {
                key: {'implementation_generated': True, 'tests_generated': True,
                      'implementation_sources': [{'path': 'implementation', 'sha256': production._hash(self.source / 'implementation')}],
                      'test_sources': [{'path': 'regression', 'sha256': production._hash(self.source / 'regression')}]}
                for key in generation.REQUIREMENTS}}
        self.write('builds/demo/generation.json', self.gate)
        self.write('builds/demo/review.json', {'schema_version': 1, 'kind': 'generation-review',
            'gate_sha256': self.record('builds/demo/generation.json')['sha256'], 'source': self.identity,
            'author': 'builder', 'reviewer': 'reviewer', 'decision': 'accepted'})
        profile = {'schema_version': 6, 'profile': 'core-full'}
        compiler = self.write('builds/demo/inputs/compiler', b'compiler')
        engine_files = [self.record(compiler)]
        for role in build_evidence.ENGINE_ROLES:
            value = (role + ' input').encode()
            if role == 'bridge_abi':
                row = self.record(compiler)
                value = {'compiler': str(self.store / compiler), 'inputs': [
                    {'path': str(self.store / compiler), 'sha256': row['sha256'], 'size_bytes': row['size']}]}
            path = self.write('builds/demo/inputs/' + role, value)
            row = self.record(path)
            engine_files.append(row)
            profile.update({role + '_path': str(self.store / path), role + '_sha256': row['sha256'],
                            role + '_size_bytes': row['size']})
        self.write('builds/demo/inputs/profile.json', profile)
        inputs = [self.record(path) for path in ('sources/test/identity.json', 'builds/demo/generation.json',
                   'builds/demo/review.json', 'builds/demo/inputs/profile.json')] + engine_files
        self.outputs = build_evidence.canonical_outputs(self.cfg, 'builds/demo/assembly')
        for role in ('runtime', 'driver', 'engine_worker', 'worker', 'startup_snapshot'):
            self.write(self.outputs[role], role.encode(), executable=role != 'startup_snapshot')
        typed_records = {}
        for role in production.ENGINE_ASSETS.values():
            content = role.encode()
            if role in ('engine_worker', 'startup_snapshot'):
                content = (self.store / self.outputs[role]).read_bytes()
            path = self.write('builds/demo/engine/' + role, content, executable=role == 'engine_worker')
            typed_records[role] = self.record(path)
        engine_profile = {'profile': 'core-full', 'target': self.cfg['target'],
                          **{field: 'a' * 64 for field in production.SOURCE_FIELDS}}
        path = self.write('builds/demo/engine/engine-profile.json', engine_profile)
        typed_records['engine_profile'] = self.record(path)
        resources = Path(self.outputs['worker_manifest']).parent
        self.write((resources / 'v8-profile-manifest.json').as_posix(), engine_profile)
        self.write(self.outputs['worker_manifest'], {
            'source_commit': self.identity['commit'], 'source_tree_sha256': self.identity['tree_sha256'],
            'worker_build_sha256': typed_records['engine_worker']['sha256'],
            'worker_build_bytes': typed_records['engine_worker']['size'],
            'v8_archive_sha256': typed_records['v8_archive']['sha256'],
            'v8_profile_sha256': typed_records['engine_profile']['sha256'],
            'v8_binding_sha256': typed_records['bindings_source']['sha256'],
            **{field: 'a' * 64 for field in production.SOURCE_FIELDS},
            'render_contract_version': 2, 'local_effects_contract_version': 3})
        (self.store / self.outputs['worker_app'] / 'Contents/Empty').mkdir()
        self.write('builds/demo/engine/typed-runtime.json', {'schema_version': 1,
            'kind': 'typed-runtime-preparation', 'source': self.identity, 'assets': typed_records})
        command = ['/bin/bash', 'scripts/build-sandboxed-worker.sh']
        self.write('runs/original/process/process-result.json', {'argv': command, 'returncode': 0,
            'error': None, 'timed_out': False, 'cleanup_complete': True, 'cleanup_errors': [],
            'cancellation_signal': None, 'forced_cleanup': False})
        self.write('runs/original/process/stdout.log', b'original stdout\n')
        self.write('runs/original/process/stderr.log', b'original stderr\n')
        environment = {'ORYN_V8_PROFILE_MANIFEST': self.record('builds/demo/inputs/profile.json'),
                       'ORYN_TYPED_SOURCE_IDENTITY': self.record('sources/test/identity.json')}
        literals = {'ORYN_CODESIGN_IDENTITY': 'fixture', 'ORYN_TYPED_RUNTIME_OUTPUT': 'builds/demo/engine'}
        self.execution = {'schema_version': 1, 'kind': 'typed-build-execution', 'source': self.identity,
            'configuration': self.cfg, 'command': command, 'build_inputs': inputs,
            'engine_input_files': engine_files, 'original_store_root': str(self.store),
            'environment_inputs': environment, 'literal_environment': literals,
            'environment_sha256': manifest_digest({'inputs': environment, 'literals': literals}),
            'toolchain_observation': 'rustc test\nhost: aarch64-apple-darwin\n',
            'generation_gate_sha256': self.record('builds/demo/generation.json')['sha256'],
            'generation_review_sha256': self.record('builds/demo/review.json')['sha256'],
            'outputs': {role: self.record(path, kind='directory' if role == 'worker_app' else 'file')
                        for role, path in self.outputs.items()}, 'integrity_errors': [],
            'evidence': {role: self.record('runs/original/process/' + filename)['sha256'] for role, filename in
                         [('process', 'process-result.json'), ('stdout', 'stdout.log'), ('stderr', 'stderr.log')]}}
        self.write('runs/original/build-execution.json', self.execution)
        for name, path, kind in [('source', 'sources/test', 'build'), ('build', 'builds/demo', 'build'),
                                 ('attempt', 'runs/original', 'run'),
                                 ('cache', 'cache/cargo/' + builds.cargo_cache_key(self.cfg), 'cargo-cache')]:
            retention.register_object(self.store, name, path=path, kind=kind)
        signature = lambda _root, _bundle, **kwargs: {'path': kwargs['worker_path'],
            'verifier': 'security_prebuilt.verify_signature', 'identity': 'fixture', 'valid': True}
        self.signature_patch = patch.object(builds, 'verify_worker_signature', side_effect=signature)
        self.signature_patch.start()
        self.addCleanup(self.signature_patch.stop)
        self.engine_patch = patch.object(production, '_engine', return_value=[])
        self.engine_patch.start()
        self.addCleanup(self.engine_patch.stop)

    def write(self, relative, value, executable=False):
        path = self.store / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(value if isinstance(value, bytes) else json.dumps(value).encode())
        if executable:
            path.chmod(0o755)
        return relative

    def record(self, path, kind='file'):
        return snapshot_input(self.store, path, kind=kind)

    def assemble(self):
        return assembly.assemble_build(self.source, self.store,
            execution_path='runs/original/build-execution.json', source_export='sources/test/export',
            source_identity='sources/test/identity.json', generation_gate='builds/demo/generation.json',
            generation_review='builds/demo/review.json', output_directory='builds/demo/published', build_id='demo')

    def test_original_build_assembles_and_pins_without_cache_dependency(self):
        result = self.assemble()
        bundle = self.store / result['bundle_root']
        build = load_manifest(self.store / result['build_manifest']['path'], 'build')
        self.assertFalse(result['qualification_complete'])
        self.assertTrue((bundle / 'OrynPageWorker.app/Contents/Empty').is_dir())
        self.assertEqual((bundle / 'evidence/build_stdout').read_bytes(), b'original stdout\n')
        self.assertNotEqual((bundle / 'bin/oryn').stat().st_ino,
                            (self.store / self.outputs['runtime']).stat().st_ino)
        verify_inputs(build['immutable_inputs'], store_root=self.store, bundle_root=bundle)
        index = production._read(self.store / 'retention.json')
        self.assertTrue(index['objects']['build']['pinned'])
        self.assertIn('source', index['objects']['build']['dependencies'])
        self.assertNotIn('cache', index['objects']['build']['dependencies'])
        self.assertEqual(index['leases'][-1]['state'], 'terminal')
        bundle.chmod(0o755)
        (bundle / 'undeclared-library.dylib').write_bytes(b'extra')
        with self.assertRaises(ValueError):
            verify_inputs(build['immutable_inputs'], store_root=self.store, bundle_root=bundle)

    def test_failed_original_and_changed_worker_reject_before_publication(self):
        self.execution['integrity_errors'] = ['failed']
        self.write('runs/original/build-execution.json', self.execution)
        with self.assertRaisesRegex(ValueError, 'successful complete'):
            self.assemble()
        self.execution['integrity_errors'] = []
        self.write('runs/original/build-execution.json', self.execution)
        self.write(self.outputs['engine_worker'], b'changed', executable=True)
        with self.assertRaises(ValueError):
            self.assemble()
        self.assertFalse((self.store / 'builds/demo/published').exists())

    def test_unchanged_completion_flags_cannot_hide_log_drift(self):
        self.write('runs/original/process/stdout.log', b'edited original stdout')
        with self.assertRaisesRegex(ValueError, 'evidence changed'):
            self.assemble()
        self.assertFalse((self.store / 'builds/demo/published').exists())

    def test_unregistered_source_root_rejects_without_inferred_ownership(self):
        index = production._read(self.store / 'retention.json')
        del index['objects']['source']
        self.write('retention.json', index)
        with self.assertRaisesRegex(ValueError, 'registered retention root'):
            self.assemble()
        self.assertFalse((self.store / 'builds/demo/published').exists())

    def test_signature_failure_never_publishes_completion_or_changes_original_logs(self):
        with patch.object(builds, 'verify_worker_signature', side_effect=ValueError('invalid signature')):
            with self.assertRaisesRegex(ValueError, 'invalid signature'):
                self.assemble()
        self.assertFalse((self.store / 'builds/demo/published/assembly-receipt.json').exists())
        self.assertEqual((self.store / 'runs/original/process/stdout.log').read_bytes(), b'original stdout\n')

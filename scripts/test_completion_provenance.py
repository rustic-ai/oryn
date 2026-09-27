"""Source regressions for exact-candidate joins; no signing or runtime execution."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
import completion_provenance as provenance


class CompletionProvenanceTests(unittest.TestCase):
    def test_candidate_identity_cannot_relabel_previously_parsed_manifest(self):
        original_digest = provenance.digest(self.candidate)
        def replace_candidate(*args):
            replacement = copy.deepcopy(self.record)
            replacement['source_tree_sha256'] = 'b' * 64
            self.candidate.write_text(json.dumps(replacement))
        with patch.object(provenance, 'validate_engine', side_effect=replace_candidate):
            with self.assertRaisesRegex(ValueError, 'manifest changed during validation'):
                provenance.load_candidate(self.candidate)
        self.assertNotEqual(provenance.digest(self.candidate), original_digest)

    def test_malformed_candidate_and_record_paths_reject_with_value_error(self):
        for value in (None, [], 1, dict(self.record, schema_version=True)):
            self.candidate.write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                provenance.load_candidate(self.candidate)
        for path in (None, [], {}, 1, ""):
            with self.assertRaises(ValueError):
                provenance.read_record(dict(path=path, sha256='a' * 64))

    def test_transaction_and_standalone_each_use_two_complete_inventories(self):
        original = provenance.read_record
        for operation in ('standalone', 'transaction'):
            with self.subTest(operation=operation), patch.object(provenance, 'read_record', wraps=original) as reads:
                if operation == 'standalone':
                    value = provenance.load_candidate(self.candidate)
                else:
                    with provenance.candidate_transaction(self.candidate) as value:
                        self.assertEqual(reads.call_count, len(self.record['artifacts']))
                        self.assertEqual(value['record'], self.record)
                self.assertEqual(reads.call_count, 2 * len(self.record['artifacts']))
                self.assertEqual(value['sha256'], provenance.digest(self.candidate))

    def test_transaction_body_drift_and_mutated_public_baseline_cannot_authorize_bytes(self):
        with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
            with provenance.candidate_transaction(self.candidate) as bound:
                worker = bound['artifacts']['worker']
                worker.write_text('changed after canonical semantics')
                bound['record']['artifacts']['worker']['sha256'] = provenance.digest(worker)

    def test_transaction_rejects_public_binding_mutation_without_disk_drift(self):
        with self.assertRaisesRegex(ValueError, 'binding changed during transaction'):
            with provenance.candidate_transaction(self.candidate) as bound:
                bound['record']['source_tree_sha256'] = 'b' * 64

    def test_transaction_candidate_symlink_retarget_with_identical_bytes_is_rejected(self):
        alternate = self.root / 'same-candidate.json'
        alternate.write_bytes(self.candidate.read_bytes())
        link = self.root / 'selected-candidate.json'
        link.symlink_to(self.candidate)
        with self.assertRaisesRegex(ValueError, 'manifest destination changed'):
            with provenance.candidate_transaction(link):
                link.unlink()
                link.symlink_to(alternate)

    def test_transaction_preserves_primary_error_and_reports_final_integrity_failure(self):
        primary = RuntimeError('consumer failed')
        with self.assertRaises(RuntimeError) as caught:
            with provenance.candidate_transaction(self.candidate) as bound:
                bound['artifacts']['worker'].write_text('drift on exceptional path')
                raise primary
        self.assertIs(caught.exception, primary)
        self.assertTrue(any('candidate final integrity check failed' in note
                            for note in primary.__notes__))

    def test_transaction_final_security_membership_rejects_added_resource(self):
        import hashlib
        worker = self.root / 'Worker.app/Contents/MacOS/oryn-page-worker'
        worker.parent.mkdir(parents=True)
        worker.write_text('signed worker fixture')
        self.record['artifacts']['worker'] = self.reference(worker)
        relative = 'Contents/MacOS/oryn-page-worker'
        inventory = {relative: self.reference(worker)}
        security = self.root / 'security-prebuilt.json'
        security.write_text(json.dumps({'bundle_files': inventory}))
        self.record['artifacts']['security_prebuilt'] = self.reference(security)
        self.record['artifacts']['security_bundle_' + hashlib.sha256(relative.encode()).hexdigest()] = self.reference(worker)
        self.persist()
        with self.assertRaisesRegex(ValueError, 'bundle file set or bytes changed'):
            with provenance.candidate_transaction(self.candidate):
                (worker.parent / 'unexpected-resource').write_text('added during plugin work')

    def test_transaction_does_not_skip_registry_or_architecture_semantics(self):
        for name in ('core_profile_registry', 'architecture_prebuilt'):
            path = self.root / name
            path.write_text('{}')
            self.record['artifacts'][name] = self.reference(path)
        self.persist()
        with patch('core_profile_contract.validate_registry') as registry, patch('architecture_prebuilt.validate_preparation') as architecture:
            with provenance.candidate_transaction(self.candidate):
                registry.assert_called_once()
                architecture.assert_called_once()

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name).resolve()
        contract = self.root / 'test-harness/workflows/five-family-contract-v1.json'
        for relative, symbol, version in [('crates/oryn-common/src/v4.rs', 'RENDER_CONTRACT_VERSION', 3), ('crates/oryn-common/src/v4.rs', 'LOCAL_EFFECTS_CONTRACT_VERSION', 1)]:
            source = self.root / relative
            source.parent.mkdir(parents=True, exist_ok=True)
            with source.open('a') as stream:
                stream.write(f'pub const {symbol}: u16 = {version};\n')
        contract.parent.mkdir(parents=True)
        contract.write_text('frozen workflow contract')
        self.record = dict(schema_version=1, source_tree_sha256='a' * 64, artifacts={})
        for name in provenance.REQUIRED_ARTIFACTS:
            path = contract if name == 'workflow_contract' else self.root / name
            if name != 'workflow_contract':
                path.write_text('artifact:' + name)
            self.record['artifacts'][name] = self.reference(path)
        self.candidate = self.root / 'candidate.json'
        self.persist()
        self.patch = patch.object(provenance, 'ROOT', self.root)
        self.patch.start()
        self.engine_patch = patch.object(provenance, 'validate_engine')
        self.engine_patch.start()
        self.snapshot_patch = patch.object(provenance, 'validate_source_snapshot')
        self.snapshot_patch.start()

    def test_render_protocol_is_available_without_renderer_sources(self):
        self.assertFalse((self.root / 'crates/oryn-render-core').exists())
        self.assertEqual(provenance.render_contracts(self.root), {
            'render_contract_version': 3,
            'local_effects_contract_version': 1,
        })

    def test_missing_duplicate_and_zero_render_protocol_are_rejected(self):
        source = self.root / 'crates/oryn-common/src/v4.rs'
        original = source.read_text()
        declaration = 'pub const RENDER_CONTRACT_VERSION: u16 = 3;\n'
        for invalid in [original.replace(declaration, ''),
                        original + declaration,
                        original.replace(declaration, declaration.replace('= 3', '= 0'))]:
            with self.subTest(source=invalid):
                source.write_text(invalid)
                with self.assertRaisesRegex(ValueError, 'ambiguous rendering contract: render_contract_version'):
                    provenance.render_contracts(self.root)

    def test_artifact_replacement_during_engine_validation_prevents_candidate_publication(self):
        worker = Path(self.record['artifacts']['worker']['path'])
        def replace_worker(*args):
            worker.write_text('replacement worker after initial identity check')
        with patch.object(provenance, 'validate_engine', side_effect=replace_worker):
            with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
                provenance.load_candidate(self.candidate)

    def test_same_bytes_symlink_retarget_during_validation_cannot_change_candidate_paths(self):
        worker = Path(self.record['artifacts']['worker']['path'])
        replacement = self.root / 'worker-copy'
        replacement.write_bytes(worker.read_bytes())
        link = self.root / 'worker-link'
        link.symlink_to(worker)
        self.record['artifacts']['worker'] = self.reference(link)
        self.persist()
        def retarget(*args):
            link.unlink()
            link.symlink_to(replacement)
        with patch.object(provenance, 'validate_engine', side_effect=retarget):
            with self.assertRaisesRegex(ValueError, 'artifact destination changed during validation: worker'):
                provenance.load_candidate(self.candidate)

    def tearDown(self):
        self.snapshot_patch.stop()
        self.engine_patch.stop()
        self.patch.stop()
        self.directory.cleanup()

    def reference(self, path):
        return dict(path=str(path), sha256=provenance.digest(path))

    def persist(self):
        self.candidate.write_text(json.dumps(self.record, sort_keys=True))

    def components(self):
        result = {}
        for name in provenance.COMPONENTS:
            report = self.root / (name + '-report.json')
            field = 'candidate_sha256' if name == 'core' else 'candidate_manifest_sha256'
            report.write_text(json.dumps({field: provenance.digest(self.candidate), 'qualified': False}))
            source = self.root / (name + '-producer.py')
            source.write_text('producer ' + name)
            raw = self.root / (name + '-raw.json')
            raw.write_text('raw ' + name)
            result[name] = dict(candidate_manifest_sha256=provenance.digest(self.candidate),
                                producer_sources=[self.reference(source)], inputs=[self.reference(raw)],
                                report=self.reference(report))
        return result

    def test_complete_join_preserves_unqualified_state(self):
        result = provenance.join(self.candidate, self.components())
        self.assertTrue(result['provenance_complete'])
        self.assertFalse(result['qualified'])
        self.assertEqual(set(result['components']), provenance.COMPONENTS)

    def test_engine_profile_validation_is_mandatory_before_identity_publication(self):
        with patch.object(provenance, 'validate_engine', side_effect=ValueError('engine mismatch')):
            with self.assertRaisesRegex(ValueError, 'engine mismatch'):
                provenance.load_candidate(self.candidate)

    def test_engine_profile_artifact_roles_and_worker_manifest_must_agree(self):
        # Exercise the join after the real profile validator's independently
        # tested validation boundary, without requiring a built engine fixture.
        profile = {}
        mapping = dict(archive='v8_archive', binding='bindings_source', icu_data='icu_data',
                       startup_snapshot='startup_snapshot', external_references='external_references',
                       worker='engine_worker')
        for engine, name in mapping.items():
            profile[engine + '_path'] = self.record['artifacts'][name]['path']
            profile[engine + '_sha256'] = self.record['artifacts'][name]['sha256']
        rust_fields = ('rusty_v8_source_manifest_sha256', 'rusty_v8_source_tree_sha256',
                       'rusty_v8_weak_teardown_patch_sha256', 'rusty_v8_patch_metadata_sha256')
        profile.update({field: 'c' * 64 for field in rust_fields})
        manifest_path = Path(self.record['artifacts']['worker_manifest']['path'])
        manifest = dict(render_contract_version=3, local_effects_contract_version=1, source_tree_sha256='a' * 64,
                        worker_build_sha256=self.record['artifacts']['engine_worker']['sha256'],
                        v8_archive_sha256=self.record['artifacts']['v8_archive']['sha256'],
                        v8_profile_sha256=self.record['artifacts']['engine_profile']['sha256'])
        manifest.update({field: profile[field] for field in rust_fields})
        manifest['v8_binding_sha256'] = self.record['artifacts']['bindings_source']['sha256']
        manifest_path.write_text(json.dumps(manifest))
        artifacts = {name: Path(record['path']) for name, record in self.record['artifacts'].items()}
        engine_path = artifacts['engine_profile']
        engine_path.write_text('{}')
        validator = SimpleNamespace(validate=lambda *args, **kwargs: profile)
        spec = SimpleNamespace(loader=SimpleNamespace(exec_module=lambda _: None))
        self.engine_patch.stop()
        try:
            with patch.object(provenance.importlib.util, 'spec_from_file_location', return_value=spec), patch.object(provenance.importlib.util, 'module_from_spec', return_value=validator), patch.object(provenance, 'validate_engine_source_snapshot'):
                provenance.validate_engine(self.record['artifacts'], artifacts, 'a' * 64)
                for field in (*rust_fields, 'v8_binding_sha256'):
                    original = manifest.pop(field)
                    manifest_path.write_text(json.dumps(manifest))
                    with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'signed worker manifest'):
                        provenance.validate_engine(self.record['artifacts'], artifacts, 'a' * 64)
                    manifest[field] = original
                manifest_path.write_text(json.dumps(manifest))
                profile['worker_sha256'] = self.record['artifacts']['worker']['sha256']
                with self.assertRaisesRegex(ValueError, 'engine profile'):
                    provenance.validate_engine(self.record['artifacts'], artifacts, 'a' * 64)
                profile['worker_sha256'] = self.record['artifacts']['engine_worker']['sha256']
                manifest['source_tree_sha256'] = 'b' * 64
                manifest_path.write_text(json.dumps(manifest))
                with self.assertRaisesRegex(ValueError, 'signed worker manifest'):
                    provenance.validate_engine(self.record['artifacts'], artifacts, 'a' * 64)
        finally:
            self.engine_patch.start()

    def test_missing_engine_or_signature_artifact_never_becomes_complete(self):
        for name in ('worker', 'v8_archive', 'bindings_source', 'icu_data', 'startup_snapshot', 'signature_evidence'):
            original = self.record['artifacts'].pop(name)
            self.persist()
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                provenance.load_candidate(self.candidate)
            self.record['artifacts'][name] = original
            self.persist()

    def test_changed_artifact_and_changed_workflow_contract_reject(self):
        path = Path(self.record['artifacts']['worker']['path'])
        path.write_text('worker replacement')
        with self.assertRaisesRegex(ValueError, 'changed'):
            provenance.load_candidate(self.candidate)

    def test_identical_bytes_at_unselected_path_do_not_replace_executable(self):
        bound = provenance.load_candidate(self.candidate)
        alternate = self.root / 'other-executor'
        alternate.write_bytes(bound['artifacts']['workflow_executor'].read_bytes())
        with self.assertRaisesRegex(ValueError, 'differs'):
            provenance.workflow_identity(bound, alternate, bound['artifacts']['worker'], 'a' * 64)
        result = provenance.workflow_identity(bound, bound['artifacts']['workflow_executor'], bound['artifacts']['worker'], 'a' * 64)
        self.assertEqual(result['candidate_manifest_sha256'], provenance.digest(self.candidate))

    def test_wrong_source_cannot_bind_workflow_to_candidate(self):
        bound = provenance.load_candidate(self.candidate)
        with self.assertRaisesRegex(ValueError, 'working source'):
            provenance.workflow_identity(bound, bound['artifacts']['workflow_executor'], bound['artifacts']['worker'], 'b' * 64)

    def test_index_cannot_relabel_report_from_another_candidate(self):
        components = self.components()
        report = Path(components['workflows']['report']['path'])
        report.write_text(json.dumps(dict(candidate_manifest_sha256='b' * 64)))
        components['workflows']['report'] = self.reference(report)
        with self.assertRaisesRegex(ValueError, 'raw report candidate'):
            provenance.join(self.candidate, components)

    def test_missing_component_and_missing_raw_inputs_reject(self):
        components = self.components()
        partial = copy.deepcopy(components)
        partial.pop('security')
        with self.assertRaisesRegex(ValueError, 'component set'):
            provenance.join(self.candidate, partial)
        components['q']['inputs'] = []
        with self.assertRaisesRegex(ValueError, 'raw inputs'):
            provenance.join(self.candidate, components)

    def test_changed_producer_or_raw_input_rejects_join(self):
        for field in ('inputs', 'producer_sources'):
            components = self.components()
            Path(components['q'][field][0]['path']).write_text('changed')
            with self.assertRaisesRegex(ValueError, 'changed'):
                provenance.join(self.candidate, components)

    def test_duplicate_inputs_and_relative_paths_reject(self):
        components = self.components()
        components['q']['inputs'] *= 2
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            provenance.join(self.candidate, components)
        with self.assertRaisesRegex(ValueError, 'missing or changed'):
            provenance.read_record(dict(path='relative', sha256='a' * 64))


    def test_transaction_reuses_only_its_freshly_validated_source_index(self):
        indexes = [dict(files=[{'path': 'first'}]), dict(files=[{'path': 'second'}])]
        with patch.object(provenance, 'validate_source_snapshot', side_effect=indexes) as snapshots, \
                patch.object(provenance, 'validate_engine') as engine:
            for index in indexes:
                with provenance.candidate_transaction(self.candidate):
                    self.assertIs(engine.call_args.args[3], index)
            self.assertEqual(snapshots.call_count, 2)
            self.assertEqual(engine.call_count, 2)
            self.assertIsNot(engine.call_args_list[0].args[3], engine.call_args_list[1].args[3])

    def test_semantic_timing_children_preserve_order_and_optional_validator_calls(self):
        from contextlib import contextmanager
        events = []
        @contextmanager
        def phase(name):
            events.append(('begin', name))
            try:
                yield
            finally:
                events.append(('end', name))
        for name in ('core_profile_registry', 'architecture_prebuilt', 'security_prebuilt'):
            path = self.root / name
            path.write_text('{}')
            self.record['artifacts'][name] = self.reference(path)
        self.persist()
        with patch('core_profile_contract.validate_registry') as registry, \
                patch('architecture_prebuilt.validate_preparation') as architecture, \
                patch('security_prebuilt.validate_bundle') as security:
            with provenance.candidate_transaction(self.candidate, phase=phase):
                registry.assert_called_once()
                architecture.assert_called_once()
                security.assert_called_once()
            self.assertEqual(security.call_count, 2)  # Existing final membership check remains.
        expected = [('begin', 'inventory.initial'), ('end', 'inventory.initial'),
                    ('begin', 'candidate.semantics')]
        for name in ('source_snapshot', 'engine', 'workflow', 'registry', 'architecture', 'security'):
            expected.extend([('begin', 'candidate.' + name), ('end', 'candidate.' + name)])
        expected.extend([('end', 'candidate.semantics'),
                         ('begin', 'inventory.final'), ('end', 'inventory.final')])
        self.assertEqual(events, expected)

    def test_failed_semantic_child_still_closes_parent_and_checks_final_inventory(self):
        from contextlib import contextmanager
        events = []
        @contextmanager
        def phase(name):
            events.append(('begin', name))
            try:
                yield
            except BaseException as error:
                events.append(('error', name, type(error).__name__))
                raise
            finally:
                events.append(('end', name))
        primary = RuntimeError('engine validation failed')
        original = provenance.read_record
        with patch.object(provenance, 'validate_engine', side_effect=primary), \
                patch.object(provenance, 'read_record', wraps=original) as reads:
            with self.assertRaises(RuntimeError) as caught:
                with provenance.candidate_transaction(self.candidate, phase=phase):
                    self.fail('failed semantics must not publish a candidate')
            self.assertIs(caught.exception, primary)
            self.assertEqual(reads.call_count, 2 * len(self.record['artifacts']))
        self.assertIn(('error', 'candidate.engine', 'RuntimeError'), events)
        self.assertIn(('error', 'candidate.semantics', 'RuntimeError'), events)
        self.assertEqual(events[-2:], [('begin', 'inventory.final'), ('end', 'inventory.final')])


class RenderContractProvenanceTests(unittest.TestCase):
    def test_missing_stale_and_boolean_versions_cannot_replace_compiled_contract(self):
        expected = dict(render_contract_version=3, local_effects_contract_version=1)
        provenance.validate_render_contracts(expected, expected)
        for field in expected:
            for value in (None, False, True, 0, '3', 2):
                manifest = dict(expected, **{field: value})
                with self.assertRaisesRegex(ValueError, 'rendering contract mismatch'):
                    provenance.validate_render_contracts(manifest, expected)

    def test_display_list_version_does_not_imply_native_fragment_ownership(self):
        with self.assertRaisesRegex(ValueError, 'local_effects'):
            provenance.validate_render_contracts({'render_contract_version': 3},
                {'render_contract_version': 3, 'local_effects_contract_version': 1})

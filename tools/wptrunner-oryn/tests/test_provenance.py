"""Candidate provenance regression sources; no live server required."""
import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch
from oryn_wptrunner import WPT_REVISION
from oryn_wptrunner import provenance


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.fixture = {'certificate_bypass': False, 'third_party_cookies': 'block', 'web_features': {'usermedia_element': 'standard'}, 'upstream_revision': WPT_REVISION,
                        'tls': {}, 'ports': {'http': [8000, 8001]}, 'hosts': ['web-platform.test']}
        for name in ('ca', 'certificate', 'key'):
            path = self.root / (name + '.pem')
            path.write_text('synthetic candidate fixture for schema validation')
            self.fixture['tls'][name] = {'path': path.name, 'sha256': provenance.digest(path)}
        self.fixture_path = self.root / 'fixture.json'
        self.fixture_path.write_text(json.dumps(self.fixture))
        self.candidate = {'schema_version': 1, 'artifacts': {}, 'plugin_sources': {}}
        for name in ('runtime', 'driver', 'source_snapshot', 'signature_evidence'):
            path = self.root / name
            path.write_text(name)
            self.candidate['artifacts'][name] = {'path': str(path), 'sha256': provenance.digest(path)}
        manifest = self.root / 'worker-manifest.json'
        manifest.write_text(json.dumps({'render_contract_version': 3, 'local_effects_contract_version': 1}))
        self.candidate['artifacts']['worker_manifest'] = {'path': str(manifest), 'sha256': provenance.digest(manifest)}
        repair_path = Path(provenance.__file__).with_name('fixture-repairs.json')
        self.candidate['artifacts']['fixture_repairs'] = {'path': str(repair_path), 'sha256': provenance.digest(repair_path)}
        self.candidate['artifacts']['fixture_contract'] = {'path': str(self.fixture_path), 'sha256': provenance.digest(self.fixture_path)}
        package = Path(provenance.__file__).parent
        self.candidate['plugin_sources'] = {path.relative_to(package).as_posix(): provenance.digest(path) for path in package.rglob('*.py')}
        self.candidate_path = self.root / 'candidate.json'
        self.contract_path = self.root / 'run.json'
        self.save_candidate()

    def save_candidate(self):
        self.candidate_path.write_text(json.dumps(self.candidate))
        self.contract = {'schema_version': 1, 'upstream_revision': WPT_REVISION,
                         'candidate': {'path': str(self.candidate_path), 'sha256': provenance.digest(self.candidate_path)}}
        self.contract_path.write_text(json.dumps(self.contract))

    def test_run_info_is_derived_from_verified_artifacts(self):
        extras = provenance.run_info_extras(None, oryn_run_contract=str(self.contract_path),
                                            binary=str(self.root / 'runtime'), webdriver_binary=str(self.root / 'driver'))
        self.assertEqual(extras['oryn_candidate_sha256'], provenance.digest(self.candidate_path))
        self.assertEqual(extras['oryn_rendering_contracts'], {'render_contract_version': 3, 'local_effects_contract_version': 1})
        self.assertEqual(extras['oryn_cookie_profile']['third_party_cookies'], 'block')
        self.assertEqual(extras['oryn_cookie_profile_sha256'], provenance.cookie_profile.digest(extras['oryn_cookie_profile']))
        self.assertEqual(extras['oryn_feature_profile']['web_features']['usermedia_element'], 'standard')
        self.assertEqual(extras['oryn_feature_profile_sha256'], provenance.feature_profile.digest(extras['oryn_feature_profile']))
        self.assertEqual(extras['oryn_executor'], 'upstream-wptrunner')
        self.assertEqual(extras['oryn_wpt_revision'], WPT_REVISION)
        self.assertEqual(extras['oryn_fixture_sha256'], provenance.digest(self.fixture_path))

    def test_cookie_profile_change_requires_new_fixture_and_candidate_identity(self):
        original = provenance.load_contract(self.contract_path)
        self.fixture['third_party_cookies'] = 'allow'
        self.fixture_path.write_text(json.dumps(self.fixture))
        with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
            provenance.load_contract(self.contract_path)
        self.candidate['artifacts']['fixture_contract']['sha256'] = provenance.digest(self.fixture_path)
        self.save_candidate()
        changed = provenance.load_contract(self.contract_path)
        self.assertEqual(changed['cookie_profile']['third_party_cookies'], 'allow')
        self.assertNotEqual(original['cookie_profile_sha256'], changed['cookie_profile_sha256'])
        self.assertNotEqual(original['candidate_sha256'], changed['candidate_sha256'])

    def test_legacy_feature_profile_requires_new_candidate_fixture_identity(self):
        original = provenance.load_contract(self.contract_path)
        self.fixture['web_features']['usermedia_element'] = 'legacy'
        self.fixture_path.write_text(json.dumps(self.fixture))
        with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
            provenance.load_contract(self.contract_path)
        self.candidate['artifacts']['fixture_contract']['sha256'] = provenance.digest(self.fixture_path)
        self.save_candidate()
        changed = provenance.load_contract(self.contract_path)
        self.assertNotEqual(original['feature_profile_sha256'], changed['feature_profile_sha256'])
        self.assertNotEqual(original['candidate_sha256'], changed['candidate_sha256'])

    def test_stale_signed_manifest_cannot_publish_current_run_info(self):
        path = self.root / 'worker-manifest.json'
        path.write_text(json.dumps({'render_contract_version': 2, 'local_effects_contract_version': 1}))
        self.candidate['artifacts']['worker_manifest']['sha256'] = provenance.digest(path)
        self.save_candidate()
        with self.assertRaisesRegex(ValueError, 'rendering contract'):
            provenance.run_info_extras(None, oryn_run_contract=str(self.contract_path))

    def test_missing_contract_does_not_make_diagnostic_identity(self):
        with patch.dict('os.environ', {}, clear=True), self.assertRaises(ValueError):
            provenance.load_contract()

    def test_changed_runtime_driver_candidate_plugin_or_ca_rejected(self):
        for name in ('runtime', 'driver', 'ca.pem'):
            path = self.root / name
            previous = path.read_bytes()
            path.write_text('changed')
            with self.subTest(name=name), self.assertRaises(ValueError):
                provenance.load_contract(self.contract_path)
            path.write_bytes(previous)
        self.candidate['plugin_sources']['product.py'] = '0' * 64
        self.save_candidate()
        with self.assertRaisesRegex(ValueError, 'plugin'):
            provenance.load_contract(self.contract_path)

    def test_binary_path_override_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'binary differs'):
            provenance.load_contract(self.contract_path, binary=str(self.root / 'driver'))

    def test_upstream_fixture_ports_and_hosts_cannot_drift(self):
        environment = provenance.FixtureEnvironment(self.contract_path)
        config = SimpleNamespace(ports=self.fixture['ports'], domains_set=set(self.fixture['hosts']),
                                 paths={'doc_root': str(self.root)})
        # Route bytes have separate fixture-repair tests; isolate the finalized
        # upstream ports/hosts contract while exercising its repair integration.
        with patch('oryn_wptrunner.fixture_repairs.validate', return_value=[]) as repairs:
            with environment({}, config):
                pass
        repairs.assert_called_once()
        self.assertEqual(repairs.call_args.args[2], str(self.root))
        config.ports = {'http': [8001, 8002]}
        with self.assertRaisesRegex(ValueError, 'ports differ'):
            with environment({}, config):
                pass
        config.ports = self.fixture['ports']
        config.domains_set = {'localhost'}
        with self.assertRaisesRegex(ValueError, 'host set'):
            with environment({}, config):
                pass

    def test_streamed_digest_matches_bytes_and_never_reads_whole_file(self):
        import hashlib
        path = self.root / 'large-artifact'
        contents = bytes(range(256)) * 8193
        path.write_bytes(contents)
        with patch.object(Path, 'read_bytes', side_effect=AssertionError('whole-file allocation')):
            self.assertEqual(provenance.digest(path), hashlib.sha256(contents).hexdigest())
        path.write_bytes(b'')
        self.assertEqual(provenance.digest(path), hashlib.sha256(b'').hexdigest())

    def test_final_inventory_still_rejects_mutation_during_semantic_validation(self):
        original = provenance.rendering_contracts
        def mutate(manifest):
            result = original(manifest)
            # Same-size drift after initial artifact validation must not pass.
            (self.root / 'runtime').write_bytes(b'RUNTIME')
            return result
        with patch.object(provenance, 'rendering_contracts', side_effect=mutate):
            with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
                provenance.load_contract(self.contract_path)


class SchemaTwoTransactionTests(unittest.TestCase):
    save_candidate = ProvenanceTests.save_candidate

    def setUp(self):
        import sys
        scripts = str(Path(provenance.__file__).resolve().parents[3] / 'scripts')
        if scripts not in sys.path:
            sys.path.insert(0, scripts)
        import completion_provenance as canonical
        self.canonical = canonical
        ProvenanceTests.setUp(self)
        self.candidate['source_tree_sha256'] = 'a' * 64
        for name in canonical.REQUIRED_ARTIFACTS - self.candidate['artifacts'].keys():
            path = self.root / ('canonical-' + name)
            path.write_text('canonical artifact ' + name)
            self.candidate['artifacts'][name] = {'path': str(path), 'sha256': provenance.digest(path)}
        workflow = self.root / 'test-harness/workflows/five-family-contract-v1.json'
        workflow.parent.mkdir(parents=True)
        workflow.write_bytes(Path(self.candidate['artifacts']['workflow_contract']['path']).read_bytes())
        self.save_candidate()
        self.contract['schema_version'] = 2
        self.contract_path.write_text(json.dumps(self.contract))
        for target, replacement in [('ROOT', self.root), ('validate_engine', None), ('validate_source_snapshot', None)]:
            mocker = patch.object(canonical, target, replacement) if target == 'ROOT' else patch.object(canonical, target)
            active = mocker.start()
            self.addCleanup(mocker.stop)
            if target != 'ROOT':
                setattr(self, target, active)
        self.seen_bound = None
        def selected(path, candidate=None):
            self.validate_engine.assert_called_once()
            self.validate_source_snapshot.assert_called_once()
            self.assertIsNotNone(candidate)
            self.assertEqual(candidate['record'], self.candidate)
            self.seen_bound = candidate
            return {'fixture_path': str(self.fixture_path), 'profile_sha256': provenance.digest(self.fixture_path),
                    'candidate': candidate}
        self.envelope = patch('core_profile_contract.resolve_envelope', side_effect=selected)
        self.envelope_mock = self.envelope.start()
        self.addCleanup(self.envelope.stop)

    def test_schema_two_uses_real_canonical_bound_and_exactly_two_broad_sweeps(self):
        original = self.canonical.read_record
        with patch.object(self.canonical, 'read_record', wraps=original) as reads:
            result = provenance.load_contract(self.contract_path)
        self.assertEqual(reads.call_count, 2 * len(self.candidate['artifacts']))
        self.assertIs(result['profile_envelope']['candidate'], self.seen_bound)
        self.assertEqual(result['candidate'], self.candidate)
        self.assertEqual(result['candidate_sha256'], provenance.digest(self.candidate_path))
        self.assertEqual(result['fixture_sha256'], provenance.digest(self.fixture_path))
        self.assertEqual(result['cookie_profile'], provenance.cookie_profile.from_fixture(self.fixture))
        self.assertEqual(result['feature_profile'], provenance.feature_profile.from_fixture(self.fixture))

    def test_plugin_final_work_is_inside_canonical_final_inventory(self):
        original = provenance._package_sources
        calls = 0
        def sources(package):
            nonlocal calls
            calls += 1
            result = original(package)
            if calls == 2:
                (self.root / 'runtime').write_text('mutation during final plugin membership check')
            return result
        with patch.object(provenance, '_package_sources', side_effect=sources):
            with self.assertRaisesRegex(ValueError, 'artifact missing or changed'):
                provenance.load_contract(self.contract_path)

    def test_schema_two_canonical_semantic_failure_prevents_envelope_work(self):
        self.validate_engine.side_effect = ValueError('canonical engine mismatch')
        with self.assertRaisesRegex(ValueError, 'canonical engine mismatch'):
            provenance.load_contract(self.contract_path)
        self.envelope_mock.assert_not_called()

    def test_schema_two_mutable_bound_cannot_relabel_artifact(self):
        original = self.envelope_mock.side_effect
        def substitute(path, candidate=None):
            result = original(path, candidate=candidate)
            candidate['record']['source_tree_sha256'] = 'b' * 64
            return result
        self.envelope_mock.side_effect = substitute
        with self.assertRaisesRegex(ValueError, 'binding changed during transaction'):
            provenance.load_contract(self.contract_path)

    def test_plugin_primary_failure_retains_final_drift_diagnostic(self):
        primary = RuntimeError('envelope consumer failed')
        def fail(*args, **kwargs):
            (self.root / 'runtime').write_text('changed during failed callback')
            raise primary
        self.envelope_mock.side_effect = fail
        with self.assertRaises(RuntimeError) as caught:
            provenance.load_contract(self.contract_path)
        self.assertIs(caught.exception, primary)
        self.assertTrue(any('candidate final integrity check failed' in note for note in primary.__notes__))

    def test_final_plugin_membership_addition_is_rejected(self):
        original = provenance._package_sources
        calls = 0
        def changed(package):
            nonlocal calls
            calls += 1
            result = original(package)
            if calls == 2:
                result['unexpected.py'] = ('0' * 64, self.root / 'unexpected.py')
            return result
        with patch.object(provenance, '_package_sources', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'plugin file set'):
                provenance.load_contract(self.contract_path)

    def test_tls_same_bytes_symlink_retarget_after_initial_read_is_rejected(self):
        ca = self.root / 'ca.pem'
        first, second = self.root / 'ca-first', self.root / 'ca-second'
        first.write_bytes(ca.read_bytes()); second.write_bytes(ca.read_bytes())
        ca.unlink(); ca.symlink_to(first)
        original = provenance.cookie_profile.from_fixture
        def changed(fixture):
            ca.unlink(); ca.symlink_to(second)
            return original(fixture)
        with patch.object(provenance.cookie_profile, 'from_fixture', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'TLS material or destination changed'):
                provenance.load_contract(self.contract_path)

    def test_envelope_and_candidate_same_bytes_symlink_retarget_are_rejected(self):
        for role in ('envelope', 'candidate'):
            with self.subTest(role=role):
                target = self.contract_path if role == 'envelope' else self.candidate_path
                twin = self.root / (role + '-twin.json'); twin.write_bytes(target.read_bytes())
                link = self.root / (role + '-link.json'); link.symlink_to(target)
                if role == 'candidate':
                    self.contract['candidate']['path'] = str(link)
                    self.contract_path.write_text(json.dumps(self.contract))
                original = self.envelope_mock.side_effect
                def changed(path, candidate=None):
                    result = original(path, candidate=candidate)
                    link.unlink(); link.symlink_to(twin)
                    return result
                self.envelope_mock.side_effect = changed
                self.validate_engine.reset_mock(); self.validate_source_snapshot.reset_mock()
                with self.assertRaisesRegex(ValueError, 'destination changed'):
                    provenance.load_contract(link if role == 'envelope' else self.contract_path)
                self.envelope_mock.side_effect = original



if __name__ == '__main__':
    unittest.main()

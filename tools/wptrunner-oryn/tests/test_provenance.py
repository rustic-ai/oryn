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


if __name__ == '__main__':
    unittest.main()

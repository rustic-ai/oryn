import copy
import unittest
from oryn_wptrunner import feature_profile


class FeatureProfileTests(unittest.TestCase):
    def record(self, mode):
        profile = feature_profile.from_fixture({'web_features': {'usermedia_element': mode}})
        record = {key: 'a' * 64 for key in ('upstream_revision', 'manifest_sha256', 'coverage_sha256',
            'policy_sha256', 'core_include_sha256', 'runtime_sha256', 'driver_sha256',
            'source_snapshot_sha256', 'worker_manifest_sha256', 'cookie_profile_sha256', 'feature_environment_sha256')}
        return {**record, 'core_total': 400, 'collection': 'core', 'custody_complete': True,
                'feature_profile': profile, 'feature_profile_sha256': feature_profile.digest(profile)}

    def test_explicit_modes_are_distinct_and_url_configuration_is_rejected(self):
        self.assertNotEqual(self.record('standard')['feature_profile_sha256'], self.record('legacy')['feature_profile_sha256'])
        for features in (None, {}, True, {'usermedia_element': True}, {'usermedia_element': 'auto'},
                         {'usermedia_element': {'/legacy/': 'legacy'}}, {'usermedia_element': 'legacy', 'url': '/legacy/'}):
            with self.assertRaises(ValueError):
                feature_profile.from_fixture({'web_features': features})

    def test_replay_rejects_changed_profile_even_with_old_digest(self):
        expected = self.record('standard')
        changed = copy.deepcopy(expected)
        changed['feature_profile']['web_features']['usermedia_element'] = 'legacy'
        with self.assertRaises(ValueError):
            feature_profile.verify_record(changed, expected['feature_profile'])
        for invalid in (None, [], True):
            with self.assertRaises(ValueError):
                feature_profile.verify_record(invalid, expected['feature_profile'])
        changed = copy.deepcopy(expected['feature_profile'])
        changed['schema_version'] = True
        with self.assertRaises(ValueError):
            feature_profile.digest(changed)

    def test_full_collection_comparison_never_creates_single_profile_conformance(self):
        left, right = self.record('standard'), self.record('legacy')
        result = feature_profile.comparison(left, right)
        self.assertEqual(result['core_total_per_profile'], 400)
        self.assertFalse(result['single_profile_conformance'])
        for key, value in (('core_total', 399), ('core_include_sha256', 'b' * 64),
                           ('feature_environment_sha256', 'b' * 64), ('custody_complete', False)):
            altered = {**right, key: value}
            with self.assertRaises(ValueError):
                feature_profile.comparison(left, altered)

    def test_feature_comparison_freezes_every_other_fixture_setting(self):
        fixture = {'web_features': {'usermedia_element': 'standard'}, 'third_party_cookies': 'block', 'hosts': ['web-platform.test']}
        legacy = {**fixture, 'web_features': {'usermedia_element': 'legacy'}}
        self.assertEqual(feature_profile.environment_digest(fixture), feature_profile.environment_digest(legacy))
        self.assertNotEqual(feature_profile.environment_digest(fixture), feature_profile.environment_digest({**legacy, 'third_party_cookies': 'allow'}))

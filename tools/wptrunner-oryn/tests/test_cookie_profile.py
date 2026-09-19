"""Profile custody and denominator regression sources; no runner launch."""
import copy
import unittest
from oryn_wptrunner.cookie_profile import from_fixture, digest, comparison


class CookieProfileTests(unittest.TestCase):
    def record(self, policy):
        profile = from_fixture({"third_party_cookies": policy})
        return {"collection": "core", "custody_complete": True, "cookie_profile": profile, "cookie_profile_sha256": digest(profile),
                "upstream_revision": "719d5e38fdd0903a18ed9007aba816c98cc491e0",
                "manifest_sha256": "manifest", "coverage_sha256": "coverage",
                "policy_sha256": "tiers", "core_include_sha256": "full-core",
                "core_total": 42, "runtime_sha256": "runtime", "driver_sha256": "driver",
                "source_snapshot_sha256": "source", "worker_manifest_sha256": "worker", "fixture_environment_sha256": "environment"}

    def test_configuration_requires_whole_run_native_policy(self):
        for invalid in ({}, {"third_party_cookies": True}, {"third_party_cookies": "auto"},
                        {"third_party_cookies": {"/cookies/": "allow"}}):
            with self.subTest(value=invalid), self.assertRaises(ValueError):
                from_fixture(invalid)
        profile = from_fixture({"third_party_cookies": "block"})
        with self.assertRaises(ValueError):
            digest(dict(profile, test_overrides={"test.html": "allow"}))

    def test_comparison_keeps_two_full_results_and_never_claims_single_profile_pass(self):
        left, right = self.record("allow"), self.record("block")
        original = copy.deepcopy((left, right))
        result = comparison(left, right)
        self.assertFalse(result["single_profile_conformance"])
        self.assertEqual(result["aggregation"], "separate_profiles_only")
        self.assertEqual(result["core_total_per_profile"], 42)
        self.assertEqual((left, right), original)

    def test_changed_denominator_runtime_or_policy_is_not_a_comparable_profile_run(self):
        left = self.record("allow")
        for key in ("manifest_sha256", "coverage_sha256", "core_include_sha256", "core_total",
                    "runtime_sha256", "driver_sha256", "source_snapshot_sha256", "policy_sha256", "worker_manifest_sha256", "fixture_environment_sha256"):
            right = self.record("block")
            right[key] = "changed"
            with self.subTest(key=key), self.assertRaises(ValueError):
                comparison(left, right)
        right = self.record("block")
        right["cookie_profile_sha256"] = left["cookie_profile_sha256"]
        with self.assertRaises(ValueError):
            comparison(left, right)


if __name__ == "__main__":
    unittest.main()

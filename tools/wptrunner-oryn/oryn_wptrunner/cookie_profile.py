"""One hash-bound native cookie profile for an entire unchanged WPT collection.

Profiles describe configuration, never expected outcomes or URL-specific rules.
A paired report retains both results; it must not take the better result per test.
"""
import hashlib
import json


def from_fixture(fixture):
    policy = fixture.get("third_party_cookies")
    if policy not in ("allow", "block"):
        raise ValueError("fixture requires explicit native third_party_cookies: allow or block")
    return {"schema_version": 1, "third_party_cookies": policy, "scope": "entire_run"}


def digest(profile):
    if profile != from_fixture(profile):
        raise ValueError("cookie profile has unknown or inconsistent fields")
    return hashlib.sha256(json.dumps(profile, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def environment_digest(fixture):
    environment = {key: value for key, value in fixture.items() if key != "third_party_cookies"}
    return hashlib.sha256(json.dumps(environment, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def comparison(left, right):
    """Compare independently admitted run identities without combining assertions."""
    for key in ("upstream_revision", "manifest_sha256", "coverage_sha256", "policy_sha256", "core_include_sha256", "core_total", "runtime_sha256", "driver_sha256", "source_snapshot_sha256", "worker_manifest_sha256", "fixture_environment_sha256"):
        if key not in left or left[key] != right.get(key):
            raise ValueError("profile comparison requires identical frozen " + key)
    for run in (left, right):
        if run.get("collection") != "core" or run.get("custody_complete") is not True:
            raise ValueError("comparison requires complete Core execution custody for each profile")
    profiles = []
    for run in (left, right):
        profile = run["cookie_profile"]
        if run.get("cookie_profile_sha256") != digest(profile):
            raise ValueError("profile digest differs from recorded native configuration")
        profiles.append(profile["third_party_cookies"])
    if set(profiles) != {"allow", "block"}:
        raise ValueError("comparison requires distinct allow and block runs")
    return {"schema_version": 1, "aggregation": "separate_profiles_only", "single_profile_conformance": False,
            "profiles": profiles, "core_total_per_profile": left["core_total"],
            "manifest_sha256": left["manifest_sha256"], "core_include_sha256": left["core_include_sha256"]}

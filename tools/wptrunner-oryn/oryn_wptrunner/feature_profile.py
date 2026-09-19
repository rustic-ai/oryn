"""Candidate-bound feature configuration for an entire unchanged collection."""
import hashlib
import json


def from_fixture(fixture):
    if not isinstance(fixture, dict):
        raise ValueError('fixture must be an object')
    features = fixture.get('web_features')
    if (not isinstance(features, dict) or set(features) != {'usermedia_element'}
            or features['usermedia_element'] not in ('standard', 'legacy')):
        raise ValueError('fixture requires explicit web_features.usermedia_element standard or legacy')
    return {'schema_version': 1, 'web_features': dict(features), 'scope': 'entire_run'}


def digest(profile):
    if (not isinstance(profile, dict) or type(profile.get('schema_version')) is not int
            or profile != from_fixture(profile)):
        raise ValueError('feature profile has unknown or inconsistent fields')
    return hashlib.sha256(json.dumps(profile, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def verify_record(record, expected):
    if not isinstance(record, dict):
        raise ValueError('feature profile evidence must be an object')
    profile = record.get('feature_profile')
    if profile != expected or record.get('feature_profile_sha256') != digest(expected):
        raise ValueError('native feature profile changed')


def environment_digest(fixture):
    environment = {key: value for key, value in fixture.items() if key != 'web_features'}
    return hashlib.sha256(json.dumps(environment, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def comparison(left, right):
    """Retain separate full-collection runs; never select a winner per URL."""
    for key in ('upstream_revision', 'manifest_sha256', 'coverage_sha256', 'policy_sha256',
                'core_include_sha256', 'core_total', 'runtime_sha256', 'driver_sha256',
                'source_snapshot_sha256', 'worker_manifest_sha256', 'cookie_profile_sha256',
                'feature_environment_sha256'):
        if key not in left or left[key] != right.get(key):
            raise ValueError('feature comparison requires identical frozen ' + key)
    modes = []
    for record in (left, right):
        if record.get('collection') != 'core' or record.get('custody_complete') is not True:
            raise ValueError('feature comparison requires complete Core custody')
        profile = record.get('feature_profile')
        if not isinstance(profile, dict):
            raise ValueError('missing native feature profile')
        verify_record(record, profile)
        modes.append(profile['web_features']['usermedia_element'])
    if set(modes) != {'standard', 'legacy'}:
        raise ValueError('feature comparison requires distinct standard and legacy runs')
    return {'schema_version': 1, 'aggregation': 'separate_profiles_only',
            'single_profile_conformance': False, 'profiles': modes,
            'core_total_per_profile': left['core_total'],
            'core_include_sha256': left['core_include_sha256']}

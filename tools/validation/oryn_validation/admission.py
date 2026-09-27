"""Current WPT declarations, with explicit assets and no historical traversal.

These validators port policy checks, not saved acceptance booleans. A registry
review approves its exact body; old registries never enter the runtime closure.
"""
import hashlib
import json
from pathlib import Path
import sys


SOURCE_ROOT = Path(__file__).resolve().parents[3]

# Reviewed ports now enforce every migrated gate from actual typed inputs and
# original execution evidence. An empty migration list is implementation status,
# never proof that a build/run passed. New unimplemented gates belong here until
# their source port and negative tests have independent review.
UNMIGRATED_GATES = ()


class MigrationGateError(ValueError):
    """Official launch is refused until all legacy policy gates are migrated."""


def require_migrated_gates(collection='core'):
    gates = [name for name in UNMIGRATED_GATES
             if collection == 'core' or not name.startswith('original-')]
    if gates:
        raise MigrationGateError('official WPT admission unavailable; semantic gates require migration: '
                                 + ', '.join(gates))


def legacy_algorithms():
    """Reuse source-owned pure validators during extraction, without old data."""
    for path in (SOURCE_ROOT / 'scripts', SOURCE_ROOT / 'tools/wptrunner-oryn'):
        if str(path) not in sys.path:
            sys.path.insert(0, str(path))


def object_digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def read_object(path):
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict):
        raise ValueError('WPT declaration must be an object')
    return value


def validate_registry(registry, assets, *, selected_profile):
    """Validate v2 current registry; asset names resolve only declared inputs.

    `history` is an opaque citation string, never a path or executable input.
    This intentionally cannot reinterpret a legacy v1 registry as v2.
    """
    legacy_algorithms()
    from core_profile_contract import (REVISION, MANIFEST_SHA256, POLICY_SHA256,
                                       NON_CORE, PLATFORM_OS, compare_axis)
    from wpt_fixture_contract import validate as validate_fixture
    expected = {'schema_version', 'kind', 'registry_id', 'upstream_revision',
                'manifest_sha256', 'policy_sha256', 'coverage_sha256', 'core_total',
                'primary_profile_id', 'profiles', 'required_profile_ids',
                'required_pairs', 'non_core_claims', 'scope_review', 'history'}
    if set(registry) != expected or registry['schema_version'] != 2 or registry['kind'] != 'core-profile-registry':
        raise ValueError('typed current Core profile registry schema differs')
    if (registry['upstream_revision'] != REVISION or registry['manifest_sha256'] != MANIFEST_SHA256
            or registry['policy_sha256'] != POLICY_SHA256 or registry['core_total'] != 16737):
        raise ValueError('Core profile registry changes pinned population')
    if not isinstance(registry['history'], str):
        raise ValueError('registry history must be an opaque citation string')
    review = registry['scope_review']
    if not isinstance(review, dict) or set(review) != {'author', 'reviewer', 'evidence'}:
        raise ValueError('current registry requires independent scope review')
    if any(not isinstance(review[name], str) or not review[name].strip() for name in review):
        raise ValueError('registry review fields must be nonempty strings')
    if review['author'].strip().casefold() == review['reviewer'].strip().casefold():
        raise ValueError('registry scope reviewer must differ from author')
    body = {key: value for key, value in registry.items() if key != 'scope_review'}
    decision = read_object(assets[review['evidence']])
    if decision != {'schema_version': 1, 'status': 'approved', 'author': review['author'],
                    'reviewer': review['reviewer'], 'registry_body_sha256': object_digest(body)}:
        raise ValueError('scope review does not approve the exact current registry')
    profiles, required = registry['profiles'], registry['required_profile_ids']
    if (not isinstance(profiles, dict) or not isinstance(required, list)
            or len(required) != len(set(required)) or set(required) != set(profiles)
            or len(profiles) != 3 or selected_profile not in profiles):
        raise ValueError('Core requires three uniquely declared profiles')
    fixtures = {}
    for name, profile in profiles.items():
        if (set(profile) != {'fixture', 'platform', 'processor', 'execution_policy'}
                or profile['platform'] not in PLATFORM_OS
                or profile['processor'] not in {'aarch64', 'x86_64'}
                or profile['execution_policy'] != {'scope': 'entire_run', 'renderer': 'native'}):
            raise ValueError('Core profile execution policy differs')
        path = Path(assets[profile['fixture']])
        fixture = read_object(path)
        validate_fixture(fixture, path.parent)
        # Nested TLS names are interpreted only in this documented schema and
        # must ALSO be independently declared as current immutable assets.
        for item in fixture['tls'].values():
            target = (path.parent / item['path']).resolve()
            if target not in {Path(value).resolve() for value in assets.values()}:
                raise ValueError('fixture TLS input absent from typed closure')
        fixtures[name] = fixture
    primary = registry['primary_profile_id']
    if primary not in profiles or fixtures[primary]['web_features'] != {'usermedia_element': 'standard'} or fixtures[primary]['third_party_cookies'] != 'block':
        raise ValueError('primary Core profile must use standard features and blocked cookies')
    pairs = registry['required_pairs']
    if not isinstance(pairs, dict) or set(pairs) != {'cookies', 'features'}:
        raise ValueError('both required profile axes must be declared')
    for axis, names in pairs.items():
        if not isinstance(names, list) or len(names) != 2 or len(set(names)) != 2 or not set(names) <= set(profiles):
            raise ValueError('profile pair missing or ambiguous')
        left, right = names
        if {k: v for k, v in profiles[left].items() if k != 'fixture'} != {k: v for k, v in profiles[right].items() if k != 'fixture'}:
            raise ValueError('profile comparison changes platform or execution policy')
        compare_axis(fixtures[left], fixtures[right], axis)
    if set(registry['non_core_claims']) != NON_CORE or any(value != primary for value in registry['non_core_claims'].values()):
        raise ValueError('non-Core claims must remain on production primary profile')
    return fixtures[selected_profile]


def validate_inventory(assets, *, expected_automated, expected_assessments):
    """Recompute the full pinned inventory and its exact automated selection."""
    legacy_algorithms()
    from wpt_full_qualification import validate_inventory as validate, bind_inventory, EXECUTORS
    coverage = [json.loads(line) for line in Path(assets['coverage']).read_text().splitlines()]
    core = validate(coverage, read_object(assets['inventory_plan']))
    bind_inventory(core, assets['core_manifest'], assets['policy'], assets['wpt_checkout'])
    automated = {row['url'] for row in core.values() if row['test_type'] in EXECUTORS}
    if len(automated) != expected_automated or len(core) - len(automated) != expected_assessments:
        raise ValueError('automated and separate assessment populations differ')
    if (expected_automated, expected_assessments) != (15985, 752):
        raise ValueError('Core required denominator differs')
    included = Path(assets.get('core_include', assets['include'])).read_text().splitlines()
    if len(included) != len(set(included)) or set(included) != automated:
        raise ValueError('include list must select each automated URL exactly once')
    return sorted(automated)

"""Synthetic temporary-only scope fixtures; never production review evidence."""
import copy
import hashlib
import json
from pathlib import Path
from unittest.mock import patch

import core_profile_contract as profiles


def write(path, value):
    path = Path(path); path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, sort_keys=True))
    return profiles.reference(path)


class ScopeFixture:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.fixture_root = self.root/'fixtures'; self.fixture_root.mkdir()
        template = json.loads((profiles.ROOT/'tools/wptrunner-oryn/fixture-contract.example.json').read_text())
        for name, material in template['tls'].items():
            path = self.fixture_root/material['path']; path.parent.mkdir(exist_ok=True)
            path.write_text('synthetic test TLS material: '+name)
            material['sha256'] = profiles.digest(path)
        for route in template['endpoint_routes']:
            if route['protocol'].endswith('-local'): route['connect_ip'] = '10.0.0.2'
            if route['protocol'].endswith('-public'): route['connect_ip'] = '8.8.8.8'
        self.fixtures = {}
        definitions = {}
        # Explicit test scope follows root's reviewed three-member decision;
        # the implementation never generates a Cartesian product from enums.
        for name, feature, cookies in [('primary', 'standard', 'block'), ('allow', 'standard', 'allow'), ('legacy', 'legacy', 'block')]:
            fixture = copy.deepcopy(template)
            fixture['web_features']['usermedia_element'] = feature; fixture['third_party_cookies'] = cookies
            self.fixtures[name] = fixture
            definitions[name] = {'fixture': write(self.fixture_root/(name+'.json'), fixture),
                'platform': 'macOS', 'processor': 'aarch64', 'execution_policy': {'scope': 'entire_run', 'renderer': 'native'}}
        sources = {}
        for name, relative in profiles.REQUIREMENT_SOURCES.items():
            path = self.root/relative; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('synthetic pinned requirement '+name)
            sources[name] = profiles.reference(path)
        self.registry = {'schema_version': 1, 'registry_id': 'test-reviewed-scope', 'revision': 1,
            'previous_registry': None, 'scope_review': {'author': 'test-author', 'reviewer': 'independent-test-reviewer',
                'evidence': {'path': str(self.root/'scope-review.json'), 'sha256': '0'*64}, 'requirement_sources': sources},
            'upstream_revision': profiles.REVISION, 'manifest_sha256': profiles.MANIFEST_SHA256,
            'policy_sha256': profiles.POLICY_SHA256, 'coverage_sha256': 'd'*64, 'core_total': 16737,
            'primary_profile_id': 'primary', 'profiles': definitions, 'required_profile_ids': list(definitions),
            'required_pairs': {'cookies': ['primary', 'allow'], 'features': ['primary', 'legacy']},
            'non_core_claims': {name: 'primary' for name in profiles.NON_CORE}}
        self.candidate = {'path': self.root/'candidate.json', 'sha256': '',
            'record': {'schema_version': 1, 'source_tree_sha256': 'c'*64, 'artifacts': {
                'fixture_contract': definitions['primary']['fixture'],
                'core_disposition_policy': write(self.root/'decisions.json', {'synthetic_test_only': True})}}, 'artifacts': {}}
        self.save()

    def save(self, *, review=True):
        if review:
            body = copy.deepcopy(self.registry); del body['scope_review']['evidence']
            body_hash = hashlib.sha256(json.dumps(body, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
            self.registry['scope_review']['evidence'] = write(self.root/'scope-review.json', {
                'schema_version': 1, 'status': 'approved', 'author': self.registry['scope_review']['author'],
                'reviewer': self.registry['scope_review']['reviewer'], 'registry_body_sha256': body_hash})
        self.candidate['record']['artifacts']['core_profile_registry'] = write(self.root/'registry.json', self.registry)
        write(self.candidate['path'], self.candidate['record'])
        self.candidate['sha256'] = profiles.digest(self.candidate['path'])
        self.candidate['artifacts'] = {name: Path(record['path']) for name, record in self.candidate['record']['artifacts'].items()}

    def change_fixture(self, name, key, value):
        self.fixtures[name][key] = value
        self.registry['profiles'][name]['fixture'] = write(self.fixture_root/(name+'.json'), self.fixtures[name])
        if name == 'primary': self.candidate['record']['artifacts']['fixture_contract'] = self.registry['profiles'][name]['fixture']
        self.save()

    def admitted(self):
        return patch.object(profiles, 'ROOT', self.root)

    def envelope(self, profile_id='primary', run='original', original=None):
        value = {'schema_version': 2, 'upstream_revision': profiles.REVISION,
            'candidate': profiles.reference(self.candidate['path']), 'profile_id': profile_id,
            'core_profile_registry_sha256': self.candidate['record']['artifacts']['core_profile_registry']['sha256'],
            'fixture_sha256': self.registry['profiles'][profile_id]['fixture']['sha256'],
            'run_id': run, 'attempt_id': run+'-attempt', 'original_run_id': original or run,
            'collection_ids': {kind: run+'-'+kind for kind in profiles.COLLECTIONS},
            'prepared': str(self.root/'prepared'/profile_id/run),
            'outputs': {kind: str(self.root/'outputs'/profile_id/run/kind) for kind in profiles.COLLECTIONS}}
        path = self.root/(profile_id+'-'+run+'-envelope.json'); write(path, value)
        with self.admitted(): bound = profiles.resolve_envelope(path, self.candidate)
        return path, bound

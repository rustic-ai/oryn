"""Candidate identities verified before upstream emits run_info or starts a driver."""
import importlib.util
import hashlib
import json
import os
import sys
from pathlib import Path
from . import WPT_REVISION
from . import cookie_profile, feature_profile


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def artifact(record):
    path = Path(record['path'])
    if not path.is_absolute() or not path.is_file() or digest(path) != record['sha256']:
        raise ValueError('candidate artifact missing or changed: ' + str(path))
    return path.resolve()


def rendering_contracts(manifest):
    source = Path(__file__).resolve().parents[3] / 'scripts/completion_provenance.py'
    spec = importlib.util.spec_from_file_location('wpt_completion_provenance', source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    expected = module.render_contracts()
    module.validate_render_contracts(manifest, expected)
    return expected


def load_contract(path=None, binary=None, webdriver_binary=None):
    selected = path or os.environ.get('ORYN_WPT_RUN_CONTRACT')
    if not selected:
        raise ValueError('official Oryn runs require --oryn-run-contract or ORYN_WPT_RUN_CONTRACT')
    contract_path = Path(selected).resolve()
    contract_sha256 = digest(contract_path)
    contract = json.loads(contract_path.read_text())
    if type(contract.get('schema_version')) is not int or contract['schema_version'] not in (1, 2) or contract.get('upstream_revision') != WPT_REVISION:
        raise ValueError('run contract version/revision mismatch')
    candidate_path = artifact(contract['candidate'])
    candidate = json.loads(candidate_path.read_text())
    if candidate.get('schema_version') != 1:
        raise ValueError('unsupported candidate record version')
    required = {'runtime', 'driver', 'source_snapshot', 'signature_evidence', 'fixture_contract', 'fixture_repairs', 'worker_manifest'}
    if not required <= set(candidate.get('artifacts', {})):
        raise ValueError('candidate record lacks required artifact bindings')
    artifacts = {name: artifact(value) for name, value in candidate['artifacts'].items()}
    for value, name in ((binary, 'runtime'), (webdriver_binary, 'driver')):
        if value is not None and Path(value).resolve() != artifacts[name]:
            raise ValueError('runner binary differs from frozen candidate: ' + name)
    render = rendering_contracts(json.loads(artifacts['worker_manifest'].read_text()))
    package = Path(__file__).resolve().parent
    expected_sources = {path.relative_to(package).as_posix(): digest(path) for path in package.rglob('*.py')}
    if candidate.get('plugin_sources') != expected_sources:
        raise ValueError('loaded plugin differs from candidate source bindings')
    selected = None
    if contract['schema_version'] == 2:
        scripts = str(Path(__file__).resolve().parents[3] / 'scripts')
        if scripts not in sys.path: sys.path.insert(0, scripts)
        from core_profile_contract import resolve_envelope
        selected = resolve_envelope(contract_path)
        fixture_path = Path(selected['fixture_path'])
    else:
        if 'core_profile_registry' in artifacts:
            raise ValueError('required-profile candidate cannot downgrade to a schema1 run contract')
        fixture_path = artifacts['fixture_contract']
    fixture = json.loads(fixture_path.read_text())
    if fixture.get('certificate_bypass') is not False or fixture.get('upstream_revision') != WPT_REVISION:
        raise ValueError('candidate fixture requires pinned revision and normal TLS validation')
    for record in fixture['tls'].values():
        path = (fixture_path.parent / record['path']).resolve()
        if not path.is_file() or digest(path) != record['sha256']:
            raise ValueError('fixture TLS material differs from candidate')
    features = feature_profile.from_fixture(fixture)
    profile = cookie_profile.from_fixture(fixture)
    repairs = artifacts.get('fixture_repairs')
    if repairs is not None:
        from .fixture_repairs import MANIFEST
        if repairs.read_bytes() != MANIFEST.read_bytes():
            raise ValueError('candidate fixture repairs differ from generated source')
    expected_fixture = selected['profile_sha256'] if selected is not None else candidate['artifacts']['fixture_contract']['sha256']
    if digest(fixture_path) != expected_fixture or digest(contract_path) != contract_sha256:
        raise ValueError('candidate fixture or run envelope changed during loading')
    artifact(contract['candidate'])
    for name, record in candidate['artifacts'].items():
        if artifact(record) != artifacts[name]:
            raise ValueError('candidate artifact destination changed during loading')
    result = {'feature_environment_sha256': feature_profile.environment_digest(fixture), 'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features), 'fixture_environment_sha256': cookie_profile.environment_digest(fixture), 'cookie_profile': profile, 'cookie_profile_sha256': cookie_profile.digest(profile), 'rendering_contracts': render, 'fixture_repairs': str(repairs) if repairs else None, 'fixture_repairs_sha256': digest(repairs) if repairs else None, 'path': str(contract_path), 'sha256': contract_sha256,
            'candidate_sha256': contract['candidate']['sha256'], 'candidate': candidate,
            'fixture_path': str(fixture_path),
            'fixture_sha256': expected_fixture,
            'fixture': fixture}
    if selected is not None:
        result['profile_envelope'] = selected
    return result


def run_info_extras(logger, **kwargs):
    contract = load_contract(kwargs.get('oryn_run_contract'), kwargs.get('binary'), kwargs.get('webdriver_binary'))
    result = {'oryn_feature_profile': contract['feature_profile'], 'oryn_feature_profile_sha256': contract['feature_profile_sha256'], 'oryn_cookie_profile': contract['cookie_profile'],
            'oryn_cookie_profile_sha256': contract['cookie_profile_sha256'],
            'oryn_rendering_contracts': contract['rendering_contracts'],
            'oryn_candidate_sha256': contract['candidate_sha256'],
            'oryn_wpt_revision': WPT_REVISION, 'oryn_executor': 'upstream-wptrunner',
            'oryn_run_contract_sha256': contract['sha256'],
            'oryn_fixture_sha256': contract['fixture_sha256'],
            'oryn_fixture_repairs_sha256': contract['fixture_repairs_sha256']}
    if contract.get('profile_envelope'):
        from core_profile_contract import identity
        collection = os.environ.get('ORYN_CORE_COLLECTION')
        if collection not in ('core', 'canaries', 'placement'):
            raise ValueError('profile envelope needs the admitted collection')
        result.update({'oryn_' + key: value for key, value in identity(contract['profile_envelope'], collection).items()})
    return result


class FixtureEnvironment:
    """Validate upstream's finalized fixture configuration before it starts."""
    def __init__(self, contract_path):
        self.contract_path = contract_path

    def __call__(self, options, config):
        self.config = config
        return self

    def __enter__(self):
        contract = load_contract(self.contract_path)
        fixture = contract['fixture']
        for protocol, ports in fixture['ports'].items():
            if list(self.config.ports.get(protocol, [])) != ports:
                raise ValueError('upstream fixture ports differ from candidate: ' + protocol)
        if set(fixture['hosts']) != set(self.config.domains_set):
            raise ValueError('fixture contract must include exactly the pinned upstream host set')
        self._repair_routes = []
        if contract.get('fixture_repairs'):
            from .fixture_repairs import validate
            from tools.serve import serve
            routes = validate(contract['fixture_repairs'], contract['fixture_repairs_sha256'], self.config.paths['doc_root'])
            if any(existing[1] == route[1] for existing in serve.rewrites for route in routes):
                raise ValueError('fixture repair collides with an upstream rewrite')
            serve.rewrites.extend(routes)
            self._repair_routes = routes
        return self

    def __exit__(self, *args):
        from tools.serve import serve
        for route in getattr(self, '_repair_routes', []):
            serve.rewrites.remove(route)
        self._repair_routes = []
        return False

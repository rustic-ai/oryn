"""Adapter projection of typed manifests, without fabricating legacy candidates."""
import os
from pathlib import Path

from .admission import SOURCE_ROOT, read_object
from .inputs import InputLease
from .wpt import _digest, load_inputs, official_admission
from .worker_environment import assert_worker_environment


def _bound(path):
    store, bundle = (os.environ.get(name) for name in
                     ('ORYN_VALIDATION_STORE_ROOT', 'ORYN_VALIDATION_BUNDLE_ROOT'))
    if not store or not bundle:
        raise ValueError('typed WPT requires explicit configured store and bundle roots')
    result = load_inputs(path, store_root=store, bundle_root=bundle)
    if result['assets']['runner_source'] != SOURCE_ROOT:
        raise ValueError('loaded WPT adapter differs from declared runner source')
    assert_worker_environment(result)
    return result


def run_snapshot(path):
    class ManifestSnapshot:
        def __init__(self, source):
            self.path = Path(source).absolute()
            self.before = self.observe()

        def observe(self):
            stat = self.path.lstat()
            return (self.path.resolve(), stat.st_dev, stat.st_ino, stat.st_mode,
                    stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)

        def check(self):
            if self.observe() != self.before:
                raise ValueError('typed run manifest metadata or destination changed')

    manifest = ManifestSnapshot(path)
    bound = _bound(path)
    lease = InputLease(bound['records'], store_root=bound['store_root'], bundle_root=bound['bundle_root'])
    lease.__enter__()
    manifest.check()
    # Existing adapter snapshots expose check(). There are no resources to
    # release: the adapter performs a fresh full typed load at finalization.
    return (lease, manifest)


def load_contract(path, binary=None, webdriver_binary=None):
    bound = _bound(path)
    with InputLease(bound['records'], store_root=bound['store_root'], bundle_root=bound['bundle_root']):
        selected = official_admission(bound)
        from oryn_wptrunner import cookie_profile, feature_profile
        from completion_provenance import render_contracts, validate_render_contracts
        for value, name in ((binary, 'runtime'), (webdriver_binary, 'driver')):
            if value is not None and Path(value).resolve() != bound['assets'][name]:
                raise ValueError('runner binary differs from typed sealed bundle: ' + name)
        worker = read_object(bound['assets']['worker_manifest'])
        render = render_contracts(SOURCE_ROOT)
        validate_render_contracts(worker, render)
        from oryn_wptrunner.fixture_repairs import MANIFEST
        if MANIFEST.read_bytes() != bound['assets']['fixture_repairs'].read_bytes():
            raise ValueError('fixture repairs differ from loaded adapter')
        fixture, fixture_path = selected['fixture'], selected['fixture_path']
        features, cookies = feature_profile.from_fixture(fixture), cookie_profile.from_fixture(fixture)
        result = dict(
            path=str(bound['path']), sha256=bound['sha256'], typed_run=bound['run'],
            worker_binding=assert_worker_environment(bound),
            candidate_sha256=bound['run']['build']['manifest_sha256'],
            # This projection preserves the adapter's artifact access ABI. It
            # is explicitly not a legacy candidate admission/qualification.
            candidate={'artifacts': {name: {'path': str(path), 'sha256': _digest(path)}
                                     for name, path in bound['assets'].items() if path.is_file()}},
            fixture_path=str(fixture_path), fixture_sha256=_digest(fixture_path), fixture=fixture,
            fixture_repairs=str(bound['assets']['fixture_repairs']),
            fixture_repairs_sha256=_digest(bound['assets']['fixture_repairs']),
            rendering_contracts=render, feature_profile=features,
            feature_profile_sha256=feature_profile.digest(features),
            feature_environment_sha256=feature_profile.environment_digest(fixture),
            cookie_profile=cookies, cookie_profile_sha256=cookie_profile.digest(cookies),
            fixture_environment_sha256=cookie_profile.environment_digest(fixture))
        if _digest(bound['path']) != bound['sha256']:
            raise ValueError('typed run manifest changed during loading')
        return result

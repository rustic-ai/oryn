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
        return _project(bound, selected, binary, webdriver_binary)


def _project(bound, selected, binary=None, webdriver_binary=None):
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
        parent_support_descriptor=selected.get('parent_support_descriptor'),
        path=str(bound['path']), sha256=bound['sha256'], typed_run={key: bound['run'][key] for key in
                ('build', 'run_id', 'profile', 'runner', 'original_run_id', 'attempt')},
        worker_binding=assert_worker_environment(bound),
        candidate_sha256=bound['run']['build']['manifest_sha256'],
        candidate_manifest_path=str(bound['assets']['build_manifest']),
        candidate_manifest_raw_sha256=bound['run']['assets']['build_manifest']['sha256'],
        admitted_runtime_sha256=bound['run']['assets']['runtime']['sha256'],
        # This projection preserves the adapter's artifact access ABI. It
        # is explicitly not a legacy candidate admission/qualification.
        candidate={'artifacts': {name: {'path': str(path), 'sha256': bound['run']['assets'][name]['sha256']}
                                 for name, path in bound['assets'].items()
                                 if bound['run']['assets'][name]['kind'] == 'file'}},
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


class TypedRunSession:
    """One process-local typed admission and final digest, never a saved authority.

    Discovery is untrusted until the single lease has verified every input.
    Semantic admission then runs once under that lease. The full manifest stays
    here; adapter lookups receive only the small runtime projection.
    """
    def __init__(self, path, binary=None, webdriver_binary=None):
        self.path = Path(path).absolute()
        self.binary, self.webdriver_binary = binary, webdriver_binary
        self.pid = os.getpid()
        self.collection = os.environ.get('ORYN_CORE_COLLECTION')
        self.lease = None
        self.bound = self.selected = self.contract = None
        self.closed = False
        self.manifest_before = self._manifest()

    def _process(self):
        if os.getpid() != self.pid:
            raise ValueError('typed admission cannot cross a process boundary')

    def _manifest(self):
        st = self.path.lstat()
        return (self.path.resolve(), st.st_dev, st.st_ino, st.st_mode,
                st.st_size, st.st_mtime_ns, st.st_ctime_ns)

    def _check_manifest(self):
        if self._manifest() != self.manifest_before:
            raise ValueError('typed run manifest metadata or destination changed')

    def admit(self):
        self._process()
        if self.lease is not None or self.closed:
            raise ValueError('typed admission cannot be reused')
        from .wpt import _resolve_inputs
        store, bundle = (os.environ.get(name) for name in
                         ('ORYN_VALIDATION_STORE_ROOT', 'ORYN_VALIDATION_BUNDLE_ROOT'))
        if not store or not bundle:
            raise ValueError('typed WPT requires explicit configured store and bundle roots')
        self.bound = _resolve_inputs(self.path, store_root=store, bundle_root=bundle)
        self._check_manifest()
        self.lease = InputLease(self.bound['records'], store_root=store, bundle_root=bundle)
        self.lease.__enter__()
        if self.bound['assets']['runner_source'] != SOURCE_ROOT:
            raise ValueError('loaded WPT adapter differs from declared runner source')
        self.check()
        self.selected = official_admission(self.bound, input_lease=self.lease)
        self.contract = _project(self.bound, self.selected, self.binary, self.webdriver_binary)
        self.check()
        return self.contract

    def check(self):
        self._process()
        if self.closed or self.lease is None:
            raise ValueError('typed admission is not active')
        self._check_manifest()
        self.lease.checkpoint()
        self._check_environment()

    def _check_environment(self):
        if os.environ.get('ORYN_CORE_COLLECTION') != self.collection:
            raise ValueError('typed run collection changed')
        if self.bound is None:
            return
        assert_worker_environment(self.bound)
        store = os.environ.get('ORYN_VALIDATION_STORE_ROOT')
        bundle = os.environ.get('ORYN_VALIDATION_BUNDLE_ROOT')
        if (not store or not bundle or Path(store).absolute() != self.bound['store_root']
                or Path(bundle).absolute() != self.bound['bundle_root']):
            raise ValueError('typed run roots changed')

    def finish(self):
        self._process()
        if self.closed:
            raise ValueError('typed admission is already closed')
        errors = []
        def attempt(check):
            try:
                check()
            except BaseException as error:
                errors.append(error)
        try:
            attempt(self._check_environment)
            if self.selected is not None:
                # Keep live observations before the final complete sweep. A
                # failure here must not skip that sweep or hide the primary error.
                from .wpt import python_environment, relay_environment
                from .builds import verify_worker_signature
                from .prerequisites import reference_environment
                attempt(lambda: python_environment(self.bound))
                attempt(lambda: relay_environment(self.bound))
                attempt(lambda: reference_environment(self.bound))
                def signature_check():
                    assets = self.bound['run']['assets']
                    signature = verify_worker_signature(SOURCE_ROOT, self.bound['bundle_root'],
                        worker_path=assets['worker']['path'],
                        manifest_path=assets['worker_manifest']['path'])
                    if signature != self.selected['production']['signature']:
                        raise ValueError('final native signature differs from admission')
                attempt(signature_check)
            attempt(self._check_environment)
            if self.lease is not None and self.lease.admission is not None:
                attempt(self.lease.checkpoint)
                attempt(self.lease.finalize)
            attempt(self._check_manifest)
            if self.bound is not None:
                def manifest_digest():
                    if _digest(self.path) != self.bound['sha256']:
                        raise ValueError('typed run manifest bytes changed')
                attempt(manifest_digest)
            attempt(self._check_environment)
        finally:
            self.closed = True
        if errors:
            for error in errors[1:]:
                errors[0].add_note('additional final verification failure: ' + str(error))
            raise errors[0]

    def execution_inputs(self):
        import copy
        self.check()
        if self.contract is None:
            raise ValueError('typed admission has not completed')
        return copy.deepcopy((self.bound, self.selected))

    def __reduce__(self):
        raise TypeError('live typed admission cannot be serialized')

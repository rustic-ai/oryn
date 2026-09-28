"""Candidate identities verified before upstream emits run_info or starts a driver."""
import importlib.util
import copy
import threading
from contextlib import contextmanager
import hashlib
import json
import os
import sys
from pathlib import Path
from . import WPT_REVISION
from . import cookie_profile, feature_profile
from .timing import span


def digest(path):
    # Stream on Python3.10+ without allocating an entire large candidate file.
    value = hashlib.sha256()
    with Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


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
    with span(None, 'contract.total') as measured:
        result = _load_contract(path, binary, webdriver_binary)
        measured.bind_contract(result)
        return result


@contextmanager
def _checked_exit(check):
    try:
        yield
    except BaseException as failure:
        try:
            check()
        except BaseException as integrity_failure:
            failure._oryn_run_integrity_failed = True
            integrity_failure._oryn_run_integrity_failed = True
            if isinstance(failure, SystemExit) and failure.code in (None, 0):
                integrity_failure.add_note('upstream requested successful SystemExit before final integrity failure')
                raise integrity_failure from failure
            failure.add_note('run contract final integrity check failed: ' + str(integrity_failure))
        raise
    else:
        try:
            check()
        except BaseException as failure:
            failure._oryn_run_integrity_failed = True
            raise


@contextmanager
def _candidate_scope(contract, candidate_path):
    if contract['schema_version'] == 2:
        scripts = str(Path(__file__).resolve().parents[3] / 'scripts')
        if scripts not in sys.path:
            sys.path.insert(0, scripts)
        from completion_provenance import candidate_transaction
        with candidate_transaction(contract['candidate']['path'],
                phase=lambda name: span(None, 'contract.' + name)) as bound:
            if bound['path'] != candidate_path or bound['sha256'] != contract['candidate']['sha256']:
                raise ValueError('run contract candidate differs from canonical binding')
            yield bound
        return
    # Legacy schema1 intentionally retains its existing required subset, without
    # claiming the full canonical schema2 semantics or accepting a downgrade.
    candidate = json.loads(candidate_path.read_text())
    if candidate.get('schema_version') != 1:
        raise ValueError('unsupported candidate record version')
    required = {'runtime', 'driver', 'source_snapshot', 'signature_evidence', 'fixture_contract', 'fixture_repairs', 'worker_manifest'}
    if not required <= set(candidate.get('artifacts', {})):
        raise ValueError('candidate record lacks required artifact bindings')
    with span(None, 'contract.inventory.initial'):
        artifacts = {name: artifact(value) for name, value in candidate['artifacts'].items()}
    def final_check():
        with span(None, 'contract.inventory.final'):
            if artifact(contract['candidate']) != candidate_path:
                raise ValueError('candidate manifest destination changed during loading')
            for name, record in candidate['artifacts'].items():
                if artifact(record) != artifacts[name]:
                    raise ValueError('candidate artifact destination changed during loading')
    with _checked_exit(final_check):
        yield dict(path=candidate_path, sha256=contract['candidate']['sha256'],
                   record=candidate, artifacts=artifacts)


def _package_sources(package):
    return {path.relative_to(package).as_posix(): (digest(path), path.resolve())
            for path in package.rglob('*.py')}


def _load_contract(path=None, binary=None, webdriver_binary=None):
    selected = path or os.environ.get('ORYN_WPT_RUN_CONTRACT')
    if not selected:
        raise ValueError('official Oryn runs require --oryn-run-contract or ORYN_WPT_RUN_CONTRACT')
    contract_input = Path(selected).absolute()
    contract_path = contract_input.resolve()
    contract_sha256 = digest(contract_path)
    contract = json.loads(contract_path.read_text())
    if contract.get('kind') == 'run':
        from oryn_validation.typed_provenance import load_contract as load_typed
        return load_typed(contract_input, binary, webdriver_binary)
    if type(contract.get('schema_version')) is not int or contract['schema_version'] not in (1, 2) or contract.get('upstream_revision') != WPT_REVISION:
        raise ValueError('run contract version/revision mismatch')
    candidate_path = artifact(contract['candidate'])
    with _candidate_scope(contract, candidate_path) as bound:
        candidate, artifacts = bound['record'], bound['artifacts']
        package = Path(__file__).resolve().parent
        sources = _package_sources(package)
        tls_records = []
        fixture_path = None
        expected_fixture = None
        def final_plugin_check():
            if contract_input.resolve() != contract_path or digest(contract_path) != contract_sha256:
                raise ValueError('run envelope bytes or destination changed during loading')
            if artifact(contract['candidate']) != candidate_path:
                raise ValueError('candidate manifest destination changed during loading')
            if _package_sources(package) != sources:
                raise ValueError('loaded plugin file set, bytes or destinations changed during loading')
            if fixture_path is not None and digest(fixture_path) != expected_fixture:
                raise ValueError('candidate fixture changed during loading')
            for original, destination, expected in tls_records:
                if original.resolve() != destination or not original.is_file() or digest(original) != expected:
                    raise ValueError('fixture TLS material or destination changed during loading')
        # This final plugin check finishes INSIDE the canonical transaction: its
        # final inventory is the last broad sweep over the whole operation.
        with _checked_exit(final_plugin_check):
            for value, name in ((binary, 'runtime'), (webdriver_binary, 'driver')):
                if value is not None and Path(value).resolve() != artifacts[name]:
                    raise ValueError('runner binary differs from frozen candidate: ' + name)
            render = rendering_contracts(json.loads(artifacts['worker_manifest'].read_text()))
            if candidate.get('plugin_sources') != {name: row[0] for name, row in sources.items()}:
                raise ValueError('loaded plugin differs from candidate source bindings')
            selected = None
            if contract['schema_version'] == 2:
                from core_profile_contract import resolve_envelope
                with span(None, 'contract.envelope'):
                    selected = resolve_envelope(contract_path, candidate=bound)
                fixture_path = Path(selected['fixture_path'])
                expected_fixture = selected['profile_sha256']
            else:
                if 'core_profile_registry' in artifacts:
                    raise ValueError('required-profile candidate cannot downgrade to a schema1 run contract')
                fixture_path = artifacts['fixture_contract']
                expected_fixture = candidate['artifacts']['fixture_contract']['sha256']
            fixture = json.loads(fixture_path.read_text())
            if fixture.get('certificate_bypass') is not False or fixture.get('upstream_revision') != WPT_REVISION:
                raise ValueError('candidate fixture requires pinned revision and normal TLS validation')
            for record in fixture['tls'].values():
                original = fixture_path.parent / record['path']
                destination = original.resolve()
                if not original.is_file() or digest(original) != record['sha256']:
                    raise ValueError('fixture TLS material differs from candidate')
                tls_records.append((original, destination, record['sha256']))
            features = feature_profile.from_fixture(fixture)
            profile = cookie_profile.from_fixture(fixture)
            repairs = artifacts.get('fixture_repairs')
            if repairs is not None:
                from .fixture_repairs import MANIFEST
                if repairs.read_bytes() != MANIFEST.read_bytes():
                    raise ValueError('candidate fixture repairs differ from generated source')
            result = {'feature_environment_sha256': feature_profile.environment_digest(fixture), 'feature_profile': features, 'feature_profile_sha256': feature_profile.digest(features), 'fixture_environment_sha256': cookie_profile.environment_digest(fixture), 'cookie_profile': profile, 'cookie_profile_sha256': cookie_profile.digest(profile), 'rendering_contracts': render, 'fixture_repairs': str(repairs) if repairs else None, 'fixture_repairs_sha256': digest(repairs) if repairs else None, 'path': str(contract_path), 'sha256': contract_sha256,
                    'candidate_sha256': contract['candidate']['sha256'], 'candidate': candidate,
                    'fixture_path': str(fixture_path), 'fixture_sha256': expected_fixture, 'fixture': fixture}
            if selected is not None:
                result['profile_envelope'] = selected
    return result


# Admission authority lives only in this process and inside run_admission().
_RUN_PID = os.getpid()
_RUN_LOCK = threading.RLock()
_RUN_ADMISSION = None


def _plugin_members(package):
    # Match _package_sources, excluding generated bytecode and unrelated logs.
    return tuple(sorted(str(path.absolute()) for path in package.rglob('*.py')))


def _run_snapshot(path, gate_path):
    if json.loads(Path(path).read_text()).get('kind') == 'run':
        from oryn_validation.typed_provenance import run_snapshot
        return run_snapshot(path)
    import suite_input_lease as lease
    package = Path(__file__).resolve().parent
    # Capture the lexical envelope and installed plugin BEFORE any discovery.
    local = lease.MetadataSnapshot([path, package / 'fixture-repairs.json'],
                             membership=lambda: _plugin_members(package))
    contract = json.loads(path.read_text())
    local.check()
    candidate = lease.candidate_snapshot(contract['candidate']['path'], gate_path,
                                         Path(lease.__file__).resolve().parents[1])
    local.check()
    return local, candidate


class _AdmissionTiming:
    """Two outer-span records until upstream installs its normal mozlog sink."""
    def __init__(self):
        self.pending = []

    def info(self, message):
        if len(self.pending) < 2:
            self.pending.append(message)

    def flush(self, *, final=False):
        try:
            from mozlog import get_default_logger
            logger = get_default_logger(component='oryn-timing')
            if logger is not None:
                while self.pending:
                    logger.info(self.pending[0])
                    self.pending.pop(0)
        except Exception:
            pass
        if final:
            # Early admission failure can precede any upstream logger. Retain
            # raw diagnostics in owned stderr, not a fabricated mozlog envelope.
            while self.pending:
                message = self.pending.pop(0)
                try:
                    sys.stderr.write(message + '\n')
                except Exception:
                    pass


class _RunAdmission:
    def __init__(self, path, gate_path, binary, webdriver_binary):
        selected = path or os.environ.get('ORYN_WPT_RUN_CONTRACT')
        if not selected:
            raise ValueError('run admission needs an explicit contract')
        self.path = Path(selected).absolute()
        self.gate_path = Path(gate_path).absolute() if gate_path is not None else None
        self.binary, self.webdriver_binary = binary, webdriver_binary
        self.collection = os.environ.get('ORYN_CORE_COLLECTION')
        self.pid = os.getpid()
        self.lock = threading.RLock()
        self.state = 'new'
        self.failure = None
        self.snapshots = ()
        self._contract = None
        self.typed_session = None
        self.timing = _AdmissionTiming()

    def _revoke(self, error):
        if self.failure is None:
            self.failure = str(error)

    def _process(self):
        # Never touch a potentially inherited locked mutex in a forked child.
        if os.getpid() != self.pid:
            raise ValueError('run admission cannot cross a process boundary')

    def _metadata(self):
        if self.failure is not None:
            raise ValueError('run admission revoked: ' + self.failure)
        if os.environ.get('ORYN_CORE_COLLECTION') != self.collection:
            raise ValueError('run admission collection changed')
        for snapshot in self.snapshots:
            snapshot.check()

    def admit(self):
        self._process()
        with self.lock:
            if self.state != 'new':
                raise ValueError('run admission cannot be reused')
            self.state = 'admitting'
            try:
                with span(self.timing, 'run.admission.full') as measured:
                    if json.loads(self.path.read_text()).get('kind') == 'run':
                        from oryn_validation.typed_provenance import TypedRunSession
                        self.typed_session = TypedRunSession(self.path, self.binary, self.webdriver_binary)
                        self.snapshots = (self.typed_session,)
                        result = self.typed_session.admit()
                    else:
                        self.snapshots = _run_snapshot(self.path, self.gate_path)
                        result = load_contract(self.path, self.binary, self.webdriver_binary)
                    self._metadata()
                    self._contract = copy.deepcopy(result)
                    measured.bind_contract(result)
                    self.state = 'active'
            except BaseException as error:
                self._revoke(error)
                raise

    def get(self, path=None, binary=None, webdriver_binary=None):
        self._process()
        with self.lock:
            try:
                if self.state != 'active':
                    raise ValueError('run admission is not active')
                selected = path or os.environ.get('ORYN_WPT_RUN_CONTRACT')
                if not selected or Path(selected).absolute() != self.path:
                    raise ValueError('run admission contract path changed')
                self.timing.flush()
                with span(None, 'run.restart.metadata'):
                    self._metadata()
                # Keep explicit runtime/driver content checks at every launch.
                with span(None, 'run.restart.binaries'):
                    for value, name in ((binary, 'runtime'), (webdriver_binary, 'driver')):
                        if value is not None:
                            record = self._contract['candidate']['artifacts'][name]
                            if Path(value).resolve() != artifact(record):
                                raise ValueError('runner binary differs from admitted candidate: ' + name)
                with span(None, 'run.restart.metadata'):
                    self._metadata()
                return copy.deepcopy(self._contract)
            except BaseException as error:
                self._revoke(error)
                raise

    def finish(self):
        self._process()
        with self.lock:
            if self.state == 'closed':
                raise ValueError('run admission already closed')
            self.state = 'finalizing'
            errors = []
            try:
                try:
                    self._metadata()
                except BaseException as error:
                    self._revoke(error)
                    errors.append(error)
                # Always verify final bytes, even after revocation. Typed runs
                # reuse their admitted semantics instead of reopening nested
                # input leases and repeating all production admission.
                try:
                    with span(None, 'run.final.full'):
                        if self.typed_session is not None:
                            self.typed_session.finish()
                        else:
                            current = load_contract(self.path, self.binary, self.webdriver_binary)
                            if self._contract is not None and current != self._contract:
                                raise ValueError('final full contract differs from admitted contract')
                except BaseException as error:
                    self._revoke(error)
                    errors.append(error)
                try:
                    if self.typed_session is None:
                        self._metadata()
                except BaseException as error:
                    self._revoke(error)
                    errors.append(error)
            finally:
                self.state = 'closed'
                self._contract = None
                self.timing.flush(final=True)
            if errors:
                failure = ValueError('run admission final verification failed: ' + str(errors[0]))
                for error in errors[1:]:
                    failure.add_note(type(error).__name__ + ': ' + str(error))
                raise failure


@contextmanager
def run_admission(path, gate_path, binary=None, webdriver_binary=None):
    """One explicit parent-process run, shared by upstream manager threads.

    The cache is neither serializable authority nor an environment flag. The
    strict default load_contract remains available and is never intercepted.
    """
    global _RUN_ADMISSION
    try:
        if os.getpid() != _RUN_PID:
            raise ValueError('run admission cannot cross a process boundary')
        inherited = _RUN_ADMISSION
        if inherited is not None:
            inherited._process()
        scope = _RunAdmission(path, gate_path, binary, webdriver_binary)
        with _RUN_LOCK:
            if _RUN_ADMISSION is not None:
                raise ValueError('overlapping run admission is forbidden')
            _RUN_ADMISSION = scope
    except BaseException as failure:
        failure._oryn_run_integrity_failed = True
        raise
    try:
        with _checked_exit(scope.finish):
            scope.admit()
            yield None
    finally:
        scope._process()
        with _RUN_LOCK:
            _RUN_ADMISSION = None


def load_run_contract(path=None, binary=None, webdriver_binary=None):
    """Opt-in runtime lookup; outside an explicit run retain strict validation."""
    scope = _RUN_ADMISSION
    if scope is None:
        return load_contract(path, binary, webdriver_binary)
    return scope.get(path, binary, webdriver_binary)


def admitted_execution_inputs(path):
    """Copy execution inputs from this process's live typed admission only."""
    scope = _RUN_ADMISSION
    if scope is None:
        raise ValueError('execution requires active typed admission')
    scope._process()
    with scope.lock:
        try:
            if scope.state != 'active' or scope.typed_session is None:
                raise ValueError('execution requires active typed admission')
            if Path(path).absolute() != scope.path:
                raise ValueError('run admission contract path changed')
            scope._metadata()
            return scope.typed_session.execution_inputs()
        except BaseException as error:
            scope._revoke(error)
            raise


def run_info_extras(logger, **kwargs):
    contract = load_run_contract(kwargs.get('oryn_run_contract'), kwargs.get('binary'), kwargs.get('webdriver_binary'))
    result = {'oryn_feature_profile': contract['feature_profile'], 'oryn_feature_profile_sha256': contract['feature_profile_sha256'], 'oryn_cookie_profile': contract['cookie_profile'],
            'oryn_cookie_profile_sha256': contract['cookie_profile_sha256'],
            'oryn_rendering_contracts': contract['rendering_contracts'],
            'oryn_candidate_sha256': contract['candidate_sha256'],
            'oryn_wpt_revision': WPT_REVISION, 'oryn_executor': 'upstream-wptrunner',
            'oryn_run_contract_sha256': contract['sha256'],
            'oryn_fixture_sha256': contract['fixture_sha256'],
            'oryn_fixture_repairs_sha256': contract['fixture_repairs_sha256']}
    if contract.get('typed_run'):
        run = contract['typed_run']
        result.update(oryn_build_id=run['build']['build_id'], oryn_run_id=run['run_id'],
                      oryn_profile_id=run['profile'], oryn_runner_commit=run['runner']['commit'],
                      oryn_original_run_id=run['original_run_id'], oryn_attempt=run['attempt'])
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
        self._contract_path = contract_path

    def __call__(self, options, config):
        self.config = config
        return self

    def __enter__(self):
        contract = load_run_contract(self._contract_path)
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
        self._request_evidence = None
        if os.environ.get('ORYN_WPT_CANARY_RESOURCE_AUDIT') == '1':
            try:
                from mozlog import get_default_logger
                from .canary_resource_audit import RequestEvidenceScope
                self._request_evidence = RequestEvidenceScope(
                    get_default_logger(component='wptserve'), self.config)
            except BaseException as failure:
                try:
                    for route in self._repair_routes:
                        serve.rewrites.remove(route)
                    self._repair_routes = []
                except BaseException as error:
                    failure.add_note('fixture entry rollback failed: ' + type(error).__name__)
                raise
        return self

    def __exit__(self, *args):
        from tools.serve import serve
        failure = args[1] if len(args) > 1 else None
        try:
            for route in getattr(self, '_repair_routes', []):
                serve.rewrites.remove(route)
            self._repair_routes = []
        except BaseException as error:
            if failure is None:
                failure = error
                raise
            failure.add_note('fixture route cleanup failed: ' + type(error).__name__)
        finally:
            evidence = getattr(self, '_request_evidence', None)
            if evidence is not None:
                try:
                    evidence.close()
                except BaseException as error:
                    if failure is None:
                        raise
                    failure.add_note('fixture audit cleanup failed: ' + type(error).__name__)
        return False

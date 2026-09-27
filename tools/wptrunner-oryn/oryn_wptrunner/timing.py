"""Monotonic wall spans; diagnostics never authorize execution or replace errors."""
import itertools
import json
import os
import threading
import time


_SEQUENCE = itertools.count(1)
_LOCAL = threading.local()
IDENTITY_FIELDS = (
    'candidate_sha256', 'registry_sha256', 'profile_id', 'profile_sha256',
    'envelope_sha256', 'run_id', 'collection_id', 'attempt_id',
    'original_run_id', 'platform',
)


def contract_identity(contract):
    """Copy an already validated result; never perform additional validation."""
    selected = contract.get('profile_envelope')
    collection = os.environ.get('ORYN_CORE_COLLECTION')
    if not selected or collection not in ('core', 'canaries', 'placement'):
        return None
    result = {key: selected.get(key) for key in IDENTITY_FIELDS if key != 'collection_id'}
    result['collection_id'] = selected.get('envelope', {}).get('collection_ids', {}).get(collection)
    return _identity(result)


def _identity(value):
    if (not isinstance(value, dict) or any(not isinstance(value.get(key), str)
            or not value[key] or len(value[key]) > 256 for key in IDENTITY_FIELDS)):
        return None
    return {key: value[key] for key in IDENTITY_FIELDS}


def _emit(logger, record):
    # Only ordinary diagnostic logging failures are suppressed. A sink cannot
    # change the operation's result; subsequent records expose lost coverage.
    try:
        if logger is None:
            from mozlog import get_default_logger
            logger = get_default_logger(component='oryn-timing')
        if logger is None:
            raise RuntimeError('timing logger unavailable')
        logger.info('oryn-timing ' + json.dumps(record, sort_keys=True, separators=(',', ':')))
    except Exception:
        _LOCAL.log_failures = getattr(_LOCAL, 'log_failures', 0) + 1


class span:
    """Low-frequency phase timer. No headers, bodies, paths or exception text.

    The optional test_id is the public upstream test URL. identity must come
    from an already validated contract, not a claimed caller environment.
    This helper is also available to protocol-owned session phase wrappers;
    do not emit one record per command for an unbounded full-suite trace.
    """
    def __init__(self, logger, phase, *, identity=None, test_id=None, custody_id=None, service=None, webdriver_session_id=None):
        self.webdriver_session_id = webdriver_session_id
        self.service = service
        self.logger = logger
        self.phase = phase
        self.identity = _identity(identity)
        self.test_id = test_id if isinstance(test_id, str) and len(test_id) <= 4096 else None
        self.custody_id = (custody_id if isinstance(custody_id, str) and len(custody_id) == 32
                           and all(c in '0123456789abcdef' for c in custody_id) else None)

    def bind_contract(self, contract):
        self.identity = contract_identity(contract)

    def __enter__(self):
        self.cpu_started = time.thread_time_ns()
        self.started = time.perf_counter_ns()
        self.pid = os.getpid()
        self.identifier = str(self.pid) + ':' + str(next(_SEQUENCE))
        self.parent = getattr(_LOCAL, 'active', None)
        if self.parent is not None:
            if self.webdriver_session_id is None:
                self.webdriver_session_id = self.parent.webdriver_session_id
            if self.service is None:
                self.service = self.parent.service
            if self.test_id is None:
                self.test_id = self.parent.test_id
        _LOCAL.active = self
        self._record('begin')
        return self

    def _record(self, event, elapsed=None, exception_type=None):
        record = {'schema_version': 1, 'component': 'oryn-timing', 'event': event,
                  'phase': self.phase, 'pid': self.pid, 'thread_id': threading.get_ident(),
                  'span_id': self.identifier,
                  'parent_span_id': self.parent.identifier if self.parent else None,
                  'start_monotonic_ns': self.started,
                  'elapsed_ns': elapsed, 'outcome': None if event == 'begin' else
                      'exception' if exception_type else 'returned',
                  'exception_type': exception_type,
                  'identity': self.identity, 'test_id': self.test_id,
                  'webdriver_session_id': (self.webdriver_session_id if isinstance(self.webdriver_session_id, str)
                      and 0 < len(self.webdriver_session_id) <= 128 else None),
                  'service': dict(self.service) if isinstance(self.service, dict) else None,
                  'cpu_scope': 'calling-python-thread',
                  'thread_cpu_elapsed_ns': (None if event == 'begin' else
                                            time.thread_time_ns() - self.cpu_started),
                  'custody_id': self.custody_id,
                  'prior_log_failures': getattr(_LOCAL, 'log_failures', 0)}
        _emit(self.logger, record)

    def __exit__(self, error_type, error, traceback):
        elapsed = time.perf_counter_ns() - self.started
        _LOCAL.active = self.parent
        self._record('end', elapsed, error_type.__name__ if error_type else None)
        return False

"""Six retained native Dedicated workers in the serial placement original.

This does not claim six upstream managers. Kernel birth/path observations must
prove six distinct, simultaneously live, owned workers of the signed bundle.
"""
import ctypes
import json
from pathlib import Path
import sys
import time
from urllib.parse import urlsplit

from .admission import legacy_algorithms
from .production import _hash, _require
from .processes import supervisor

BINDINGS = ('nonce', 'candidate_sha256', 'run_contract_sha256', 'test')


def _path(pid):
    if sys.platform == 'darwin':
        library = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
        library.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
        library.proc_pidpath.restype = ctypes.c_int
        value = ctypes.create_string_buffer(4096)
        if library.proc_pidpath(pid, value, len(value)) <= 0:
            raise OSError(ctypes.get_errno(), 'native worker executable path unavailable')
        return value.value.decode()
    if sys.platform.startswith('linux'):
        return str(Path('/proc', str(pid), 'exe').resolve(strict=True))
    raise ValueError('live-six kernel executable path unsupported')


def _identity(value):
    _require(isinstance(value, (tuple, list)) and len(value) == 2
             and isinstance(value[0], (tuple, list)) and len(value[0]) in (1, 2)
             and type(value[1]) is int and value[1] > 0, 'live-six kernel identity missing')
    birth = value[0]
    valid = ((len(birth) == 2 and type(birth[0]) is int and birth[0] > 0
              and type(birth[1]) is int and 0 <= birth[1] < 1000000)
             or (len(birth) == 1 and ((type(birth[0]) is int and birth[0] > 0)
                 or (isinstance(birth[0], str) and birth[0].isdecimal() and int(birth[0]) > 0))))
    _require(valid, 'live-six kernel birth fields malformed')
    return [list(value[0]), value[1]]


def _workers(snapshot, context, worker_sha256):
    legacy_algorithms()
    from wpt_shared_placement import TEST, indexed, placement, digest
    _require(context.get('test') == TEST and snapshot.get('schema') == 1
             and snapshot.get('nonce') == context['nonce'], 'live-six snapshot binding differs')
    pages = indexed(snapshot.get('pages', []))
    selected = [(pid, row) for pid, row in pages.items()
                if urlsplit(row.get('url', '')).path + '?' + urlsplit(row.get('url', '')).query == TEST]
    _require(len(selected) == 1, 'live-six exact placement page missing')
    page_id, page = selected[0]
    parent = snapshot.get('parent_pid')
    _require(type(parent) is int and parent > 0, 'live-six runtime identity missing')
    page_pid = placement(page, worker_sha256, parent)
    page_url = urlsplit(page['url'])
    agents = indexed(snapshot.get('agents', []))
    workers = {}
    for identifier, row in agents.items():
        url = urlsplit(row.get('script_url', ''))
        if (row.get('kind') == 'Dedicated' and row.get('live') is True
                and (url.scheme, url.netloc) == (page_url.scheme, page_url.netloc)
                and url.path == '/__oryn_shared_memory__/shared-memory/worker.js' and url.query == 'placement=1'
                and any(owner.get('kind') == 'window' and owner.get('id') == page_id and owner.get('frame') is None
                        for owner in row.get('owners', []))):
            pid = placement(row, worker_sha256, parent)
            _require(digest(row['placement'].get('bootstrap_sha256')), 'live-six worker bootstrap attestation missing')
            _require(pid != page_pid and pid not in workers.values(), 'live-six workers share a process')
            workers[identifier] = pid
    _require(len(workers) == 6, 'live-six requires exactly six live Dedicated worker identities')
    return workers, parent


def _scan(pids, root_pid, *, identity_reader=None, path_reader=None, clock=None):
    identity_reader = identity_reader or supervisor().process_identity
    path_reader = path_reader or _path
    clock = clock or time.monotonic_ns
    result = {'started_ns': clock(), 'nodes': {}}
    for worker in pids:
        pid, chain = worker, set()
        while True:
            _require(pid not in chain and len(chain) < 64, 'live-six ownership chain loops or exceeds bound')
            chain.add(pid)
            before = _identity(identity_reader(pid))
            path = path_reader(pid)
            after = _identity(identity_reader(pid))
            _require(before == after, 'live-six process changed during kernel observation')
            row = {'identity': before, 'path': path}
            key = str(pid)
            _require(key not in result['nodes'] or result['nodes'][key] == row,
                     'live-six process changed within common scan')
            result['nodes'][key] = row
            if pid == root_pid:
                break
            pid = before[1]
    result['ended_ns'] = clock()
    return result


def _validate_scans(scans, workers, parent, preflight, worker_path, runtime_path, diagnostic_interval):
    _require(isinstance(scans, list) and len(scans) == 2, 'live-six requires before/after kernel scans')
    before, after = scans
    _require(isinstance(diagnostic_interval, list) and len(diagnostic_interval) == 2,
             'live-six confirming diagnostic interval missing')
    times = [before.get('started_ns'), before.get('ended_ns'), *diagnostic_interval,
             after.get('started_ns'), after.get('ended_ns')]
    _require(all(type(x) is int and x > 0 for x in times) and times == sorted(times)
             and before['ended_ns'] <= after['started_ns'], 'live-six simultaneous observation interval missing')
    _require(before.get('nodes') == after.get('nodes'), 'live-six kernel birth/path/parent changed')
    nodes = before['nodes']; expected_nodes = set()
    root = preflight.get('child_pid')
    _require(type(root) is int and root > 0 and str(root) in nodes,
             'live-six original owned process missing')
    _require(nodes[str(root)]['identity'] == _identity(preflight.get('child_identity')),
             'live-six original owned process birth differs')
    _require(nodes.get(str(parent), {}).get('path') == runtime_path, 'live-six runtime path differs from selected bundle')
    for worker in workers.values():
        _require(nodes.get(str(worker), {}).get('path') == worker_path,
                 'live-six worker executable path differs from signed bundle')
        pid, chain = worker, set()
        while True:
            _require(pid not in chain and len(chain) < 64 and str(pid) in nodes, 'live-six ownership chain incomplete')
            chain.add(pid); expected_nodes.add(str(pid))
            row = nodes[str(pid)]
            _require(set(row) == {'identity', 'path'} and isinstance(row['path'], str)
                     and Path(row['path']).is_absolute(), 'live-six process observation malformed')
            identity = _identity(row['identity'])
            if pid == root:
                break
            pid = identity[1]
        _require(parent in chain, 'live-six worker is outside selected runtime ownership')
    _require(set(nodes) == expected_nodes, 'live-six process population contains unrelated observations')


def capture(snapshot, context, artifacts, output, *, diagnostic_reader):
    """Called by the typed placement adapter while the retained workers live."""
    output = Path(output)
    preflight = json.loads((output / 'process/preflight.json').read_text())
    worker = artifacts['worker']; runtime = artifacts['runtime']
    for artifact in (worker, runtime):
        _require(_hash(artifact['path']) == artifact['sha256'], 'live-six selected binary bytes changed')
    workers, parent = _workers(snapshot, context, worker['sha256'])
    before = _scan(workers.values(), preflight['child_pid'])
    diagnostic_started = time.monotonic_ns()
    confirmed = diagnostic_reader()
    diagnostic_interval = [diagnostic_started, time.monotonic_ns()]
    current_workers, current_parent = _workers(confirmed, context, worker['sha256'])
    _require(current_workers == workers and current_parent == parent,
             'live-six confirming diagnostic changed worker population')
    after = _scan(workers.values(), preflight['child_pid'])
    snapshot = confirmed
    scans = [before, after]
    _validate_scans(scans, workers, parent, preflight, worker['path'], runtime['path'], diagnostic_interval)
    result = {'schema_version': 1, 'kind': 'live-six-native-dedicated-workers',
            **{key: context[key] for key in BINDINGS}, 'capture_id': snapshot['capture_id'],
            'workers': {str(key): value for key, value in workers.items()},
            'worker_sha256': worker['sha256'], 'runtime_sha256': runtime['sha256'], 'scans': scans,
            'diagnostic_interval': diagnostic_interval}
    return result, confirmed


def verify(record, diagnostics, context, preflight, *, worker_path, worker_sha256, runtime_path, runtime_sha256):
    _require(record.get('schema_version') == 1 and record.get('kind') == 'live-six-native-dedicated-workers'
             and not record.get('error'), 'live-six capture missing or failed')
    _require(all(record.get(key) == context[key] for key in BINDINGS)
             and record.get('worker_sha256') == worker_sha256 and record.get('runtime_sha256') == runtime_sha256,
             'live-six original or binary identity differs')
    snapshot = diagnostics['snapshot']
    _require(record.get('capture_id') == snapshot.get('capture_id'), 'live-six diagnostic capture differs')
    workers, parent = _workers(snapshot, context, worker_sha256)
    _require(record.get('workers') == {str(key): value for key, value in workers.items()},
             'live-six diagnostic worker population differs')
    _validate_scans(record.get('scans'), workers, parent, preflight, worker_path, runtime_path, record.get('diagnostic_interval'))
    return {'accepted': True, 'native_dedicated_workers': 6, 'capture_id': snapshot['capture_id']}

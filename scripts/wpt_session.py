"""Bounded JSONL client for an explicit persistent native WPT parent."""
from collections import deque
import json
import subprocess
import threading
import uuid


MAX_DIAGNOSTIC_EVENTS = 256
MAX_DIAGNOSTIC_BYTES = 1024 * 1024


def validate_diagnostic_evidence(message):
    # Older historical frames may omit this additive evidence. Omission is
    # preserved as unavailable by the report consumer, never as a complete audit.
    if 'diagnostic_events' not in message and 'diagnostic_audit' not in message:
        return
    events, audit = message.get('diagnostic_events'), message.get('diagnostic_audit')
    if not isinstance(events, list) or not isinstance(audit, dict):
        raise ValueError('invalid diagnostic evidence')
    fields = ('recorded_events', 'encoded_bytes', 'dropped_events', 'dropped_bytes', 'event_limit', 'byte_limit')
    if (type(audit.get('schema_version')) is not int or audit['schema_version'] != 1
            or audit.get('scope') != 'pre_cleanup_live_pages'
            or any(type(audit.get(key)) is not int or audit[key] < 0 for key in fields)
            or any(type(audit.get(key)) is not bool for key in ('complete', 'overflow', 'source_overflow'))
            or audit['event_limit'] != MAX_DIAGNOSTIC_EVENTS or audit['byte_limit'] != MAX_DIAGNOSTIC_BYTES
            or audit['recorded_events'] != len(events) or len(events) > MAX_DIAGNOSTIC_EVENTS):
        raise ValueError('invalid diagnostic audit')
    error = audit.get('collection_error')
    if 'collection_error' not in audit or error is not None and not isinstance(error, str):
        raise ValueError('invalid diagnostic collection error')
    if audit.get('coverage_snapshot') != 'before_archive':
        raise ValueError('invalid diagnostic page coverage snapshot')
    def page_id(value):
        return isinstance(value, str) and value.isascii() and value.isdigit() and int(value) > 0
    coverage = {}
    for key in ('expected_live_pages', 'captured_pages', 'missing_pages'):
        values = audit.get(key)
        if (not isinstance(values, list) or not all(page_id(value) for value in values)
                or values != sorted(set(values))):
            raise ValueError('invalid diagnostic page population')
        coverage[key] = set(values)
    if coverage['missing_pages'] != coverage['expected_live_pages'] - coverage['captured_pages']:
        raise ValueError('diagnostic missing-page population contradicts capture')
    identities, size, source_overflow = set(), 0, False
    for row in events:
        if (not isinstance(row, dict) or set(row) != {'page_id', 'sequence', 'event'}
                or not page_id(row['page_id']) or row['page_id'] not in coverage['captured_pages']
                or type(row['sequence']) is not int or row['sequence'] < 0
                or not isinstance(row['event'], dict)):
            raise ValueError('invalid diagnostic event identity')
        identity = (row['page_id'], row['sequence'])
        if identity in identities:
            raise ValueError('duplicate diagnostic event identity')
        identities.add(identity)
        event = row['event']
        if event.get('kind') == 'script_diagnostic':
            if not isinstance(event.get('capability'), str) or not isinstance(event.get('detail'), str):
                raise ValueError('invalid script diagnostic')
            source_overflow |= event['capability'] == 'diagnostics.overflow'
        elif event.get('kind') == 'console':
            if event.get('level') != 'error' or not isinstance(event.get('message'), str):
                raise ValueError('invalid console diagnostic')
        else:
            raise ValueError('invalid diagnostic event kind')
        size += len(json.dumps(row, ensure_ascii=False, separators=(',', ':')).encode('utf-8'))
    if (size != audit['encoded_bytes'] or size > MAX_DIAGNOSTIC_BYTES
            or audit['overflow'] is not (audit['dropped_events'] != 0)
            or source_overflow and not audit['source_overflow']
            or audit['complete'] is not (not audit['overflow'] and not audit['source_overflow'] and error is None and not coverage['missing_pages'])):
        raise ValueError('diagnostic audit contradicts original events')


def validate_result_frame(message):
    if type(message.get('page_id')) is not int or message['page_id'] < 1:
        raise ValueError('invalid result page identity')
    for key in ('queue_ms', 'run_ms', 'teardown_ms'):
        if type(message.get(key)) is not int or message[key] < 0:
            raise ValueError('invalid result phase timing')
    for key in ('error', 'replacement_error', 'resource_error'):
        if key not in message or message[key] is not None and not isinstance(message[key], str):
            raise ValueError('invalid result error field')
    if not isinstance(message.get('cleanup_errors'), list) or not all(isinstance(x, str) for x in message['cleanup_errors']):
        raise ValueError('invalid teardown evidence')
    if not isinstance(message.get('resources'), list) or type(message.get('resource_evidence_complete')) is not bool:
        raise ValueError('invalid resource evidence')
    validate_diagnostic_evidence(message)


class WptSession:
    def __init__(self, binary, prewarmed=True):
        command = [str(binary), 'native', '--wpt-session']
        if prewarmed:
            command += ['--worker-mode', 'isolated-prewarmed', '--worker-pool-size', '8']
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, bufsize=1)
        self.lock = threading.Lock()
        self.pending = {}
        self.errors = deque(maxlen=64)
        self.ready = threading.Event()
        self.shutdown = threading.Event()
        self.failure = None
        self.prepare_ms = None
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.stderr_reader = threading.Thread(target=self._stderr, daemon=True)
        self.reader.start()
        self.stderr_reader.start()
        if not self.ready.wait(60) or self.failure:
            self.abort('parent preparation failed')
            self.close()
            raise RuntimeError(self.failure)

    def _stderr(self):
        for line in self.process.stderr:
            self.errors.append(line[:2000])

    def _read(self):
        try:
            while True:
                line = self.process.stdout.readline(25 * 1024 * 1024 + 1)
                if not line:
                    break
                if len(line) > 25 * 1024 * 1024:
                    raise ValueError('parent frame exceeds limit')
                message = json.loads(line)
                if not isinstance(message, dict):
                    raise ValueError('parent frame is not an object')
                if message.get('event') == 'ready':
                    if self.ready.is_set() or message.get('schema_version') != 1:
                        raise ValueError('duplicate ready or unsupported protocol')
                    self.prepare_ms = message.get('prepare_ms')
                    self.ready.set()
                elif message.get('event') == 'shutdown':
                    if not self.ready.is_set() or self.shutdown.is_set():
                        raise ValueError('invalid shutdown ordering')
                    with self.lock:
                        if any(not slot['event'].is_set() for slot in self.pending.values()):
                            raise ValueError('shutdown with unfinished executions')
                    self.shutdown.set()
                elif message.get('event') == 'result':
                    validate_result_frame(message)
                    if not self.ready.is_set() or self.shutdown.is_set():
                        raise ValueError('invalid result ordering')
                    with self.lock:
                        slot = self.pending.get(message.get('id'))
                        if slot is None or slot['event'].is_set():
                            raise ValueError('unknown or duplicate execution response')
                        slot['value'] = message
                        slot['event'].set()
                else:
                    raise ValueError('unknown parent message')
            if not self.shutdown.is_set():
                raise ValueError('unexpected parent EOF')
        except (ValueError, OSError) as error:
            self._fail(str(error))

    def _fail(self, error):
        with self.lock:
            self.failure = self.failure or error
            self.ready.set()
            for slot in self.pending.values():
                slot['event'].set()

    def run(self, identifier, url, timeout_seconds, uploads=()):
        request_id = identifier + ':' + uuid.uuid4().hex
        slot = {'event': threading.Event()}
        with self.lock:
            if self.failure:
                raise RuntimeError(self.failure)
            self.pending[request_id] = slot
            try:
                self.process.stdin.write(json.dumps({'command': 'run', 'id': request_id,
                    'url': url, 'uploads': list(uploads),
                    'timeout_ms': min(120000, int(timeout_seconds * 1000))}) + '\n')
                self.process.stdin.flush()
            except OSError as error:
                self.pending.pop(request_id, None)
                raise RuntimeError('parent request write failed') from error
        if not slot['event'].wait(timeout_seconds + 60):
            self.abort('parent response deadline exceeded')
        with self.lock:
            self.pending.pop(request_id, None)
            if self.failure:
                raise RuntimeError(self.failure)
        if 'value' not in slot:
            raise RuntimeError('missing parent result')
        return slot['value']

    def abort(self, reason):
        self._fail(reason)
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=15)

    def progress(self):
        """A 30-second monitor snapshot, not an IPC-loop subprocess."""
        result = {'parent_pid': self.process.pid, 'prepare_ms': self.prepare_ms,
                  'pending': len(self.pending), 'error': self.failure}
        try:
            output = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss='], text=True, timeout=5)
            rows = [tuple(map(int, line.split())) for line in output.splitlines()]
            parent = next((rss * 1024 for pid, _, rss in rows if pid == self.process.pid), None)
            children = [{'pid': pid, 'rss_bytes': rss * 1024} for pid, ppid, rss in rows if ppid == self.process.pid]
            result.update(parent_rss_bytes=parent, workers=children,
                          aggregate_worker_rss_bytes=sum(row['rss_bytes'] for row in children))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            result['rss_error'] = str(error)
        return result

    def close(self):
        if self.process.poll() is None and not self.failure:
            try:
                self.process.stdin.write(json.dumps({'command': 'shutdown', 'id': uuid.uuid4().hex}) + '\n')
                self.process.stdin.flush()
                self.process.stdin.close()
            except OSError as error:
                self._fail('parent shutdown write failed: ' + str(error))
        try:
            code = self.process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            self.abort('parent shutdown timeout')
            code = self.process.returncode
        self.reader.join(5)
        self.stderr_reader.join(5)
        for pipe in (self.process.stdin, self.process.stdout, self.process.stderr):
            if pipe is not None and not pipe.closed:
                pipe.close()
        return {'returncode': code, 'shutdown_acknowledged': self.shutdown.is_set(),
                'error': self.failure, 'stderr': ''.join(self.errors)}

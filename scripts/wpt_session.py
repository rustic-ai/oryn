"""Bounded JSONL client for an explicit persistent native WPT parent."""
from collections import deque
import json
import subprocess
import threading
import uuid


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

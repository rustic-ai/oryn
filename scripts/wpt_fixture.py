"""Execution-owned direct upstream fixtures; no browser-visible response proxy."""
import json
import select
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path
from wpt_tls import fixture_certificate

ROOT = Path(__file__).resolve().parent


class DirectFixture:
    def __init__(self, checkout):
        self.checkout = Path(checkout)
        self.process = None
        self.log = None
        self.reservations = []
        self.identity = uuid.uuid4().hex
        self.generations = []
        self.failure = None
        self.shutdown_complete = False
        self.timings = {'startup_ms': 0, 'teardown_ms': 0}
        self.tls = None
        try:
            for _ in range(7):
                sock = socket.socket()
                self.reservations.append(sock)
                sock.bind(('127.0.0.1', 0))
        except OSError:
            for sock in self.reservations:
                sock.close()
            raise
        self.ports = [sock.getsockname()[1] for sock in self.reservations]

    def start(self, backend):
        if self.process is not None:
            raise RuntimeError('fixture already running')
        if self.failure:
            raise RuntimeError('fixture retired after integrity failure')
        if backend not in ('oryn', 'chromium'):
            raise ValueError('invalid fixture backend')
        for sock in self.reservations:
            sock.close()
        self.reservations = []
        python = self.checkout / '_venv3/bin/python'
        self.tls = fixture_certificate(self.checkout)
        command = [str(python) if python.is_file() else sys.executable,
                   str(ROOT / 'wptserve-helper.py'), '--checkout', str(self.checkout), '--direct',
                   '--vendor-adapter', str(ROOT / f'wpt-testdriver-{backend}.js'),
                   '--tls-certificate', self.tls['certificate'], '--tls-key', self.tls['key']]
        for flag, port in zip(('--public-port', '--public-https-port', '--public-port-2',
                               '--public-https-port-2', '--h2-port', '--ws-port', '--wss-port'), self.ports):
            command.extend((flag, str(port)))
        self.log = tempfile.TemporaryFile(mode='w+')
        started = time.monotonic()
        self.shutdown_complete = False
        try:
            self.process = subprocess.Popen(command, stdout=subprocess.PIPE,
                                            stderr=self.log, text=True)
            if not select.select([self.process.stdout], [], [], 20)[0]:
                raise RuntimeError('direct fixture startup timeout')
            metadata = json.loads(self.process.stdout.readline())
            if metadata['port'] != self.ports[0] or metadata['https_port'] != self.ports[1]:
                raise RuntimeError('direct fixture endpoint mismatch')
            self.generations.append({'backend': backend, 'id': uuid.uuid4().hex})
            return metadata
        except Exception as error:
            self.failure = type(error).__name__
            self.close()
            raise
        finally:
            self.timings['startup_ms'] += round((time.monotonic() - started) * 1000)

    def close(self):
        started = time.monotonic()
        forced = False
        try:
            if self.process is not None:
                if self.process.poll() is None:
                    self.process.terminate()
                try:
                    code = self.process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    code = self.process.wait(timeout=5)
                    forced = True
                self.process.stdout.close()
                if forced or code != 0:
                    self.failure = 'fixture_teardown_failed'
                    raise RuntimeError('fixture teardown failed')
            self.shutdown_complete = self.failure is None
        finally:
            self.process = None
            if self.log:
                self.log.close()
                self.log = None
            for sock in self.reservations:
                sock.close()
            self.reservations = []
            self.timings['teardown_ms'] += round((time.monotonic() - started) * 1000)

    def evidence(self):
        return {'schema_version': 1, 'id': self.identity, 'server': 'direct_wptserve',
                'generations': self.generations, 'ports': self.ports, 'timings': self.timings,
                'tls_validation': 'native_verified_reference_bypassed',
                'tls': {key:value for key,value in (self.tls or {}).items() if key.endswith('_sha256')},
                'shutdown_complete': self.shutdown_complete,
                'error': self.failure}

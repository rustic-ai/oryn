"""Owned TLS/focus/native-fault prerequisites, separate from the Core denominator.

Only main invokes this module after the complete generation barrier. TLS fixtures
use frozen certificate/key inputs; reference trust must actually be configured.
"""
from contextlib import contextmanager
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import ssl
from generation_gate import check_execution_generation
import subprocess
import threading
import time
from urllib.parse import urlsplit
from completion_provenance import digest, read_record

TLS_KINDS = ('trusted', 'untrusted', 'wrong_hostname')
MARKER = 'Oryn native prerequisite control'


def validate_contract(contract, fixture):
    if contract.get('schema_version') != 1 or set(contract.get('tls', {})) != set(TLS_KINDS):
        raise ValueError('complete TLS control contract required')
    endpoints = {route['port']: route for route in fixture['endpoint_routes']}
    seen = set()
    for kind in TLS_KINDS:
        entry = contract['tls'][kind]; url = urlsplit(entry['url'])
        if (url.scheme != 'https' or url.hostname not in fixture['hosts'] or not url.port
                or url.username or url.password or url.query or url.fragment or url.path != '/native-prerequisite'):
            raise ValueError('TLS control URL outside frozen fixture authority')
        route = endpoints.get(url.port)
        if route is None or route['protocol'] not in ('https', 'h2') or route['connect_ip'] != '127.0.0.1' or url.port in seen:
            raise ValueError('TLS controls require three distinct frozen loopback HTTPS/h2 endpoints')
        seen.add(url.port)
        read_record(entry['certificate']); read_record(entry['key'])
    return contract


@contextmanager
def focus_lease():
    import fcntl
    import tempfile
    path = Path(tempfile.gettempdir()) / f'oryn-wpt-reference-focus-{os.getuid()}.lock'
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'rb') as stream:
        deadline = time.monotonic() + 30
        while True:
            try: fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB); break
            except BlockingIOError:
                if time.monotonic() >= deadline: raise TimeoutError('reference host focus lease unavailable')
                time.sleep(.05)
        try: yield
        finally: fcntl.flock(stream, fcntl.LOCK_UN)


class Fixture:
    def __init__(self, contract): self.contract = contract; self.servers = []; self.threads = []
    def __enter__(self):
        check_execution_generation()
        body = ('<!doctype html><title>' + MARKER + '</title><body>' + MARKER + '</body>').encode()
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path != '/native-prerequisite': self.send_error(404); return
                self.send_response(200); self.send_header('Content-Type', 'text/html; charset=utf-8')
                self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
            def log_message(self, *args): pass
        class TlsServer(ThreadingHTTPServer):
            def get_request(self):
                connection, address = super().get_request()
                connection.settimeout(10)
                try: return self.tls_context.wrap_socket(connection, server_side=True), address
                except BaseException:
                    connection.close(); raise
        try:
            for entry in self.contract['tls'].values():
                url = urlsplit(entry['url'])
                check_execution_generation()
                server = TlsServer(('127.0.0.1', url.port), Handler)
                self.servers.append(server)
                context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
                context.load_cert_chain(read_record(entry['certificate']), read_record(entry['key']))
                server.tls_context = context
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start(); self.threads.append(thread)
            return self
        except BaseException:
            self.__exit__(None, None, None); raise
    def __exit__(self, *args):
        for server in self.servers[:len(self.threads)]: server.shutdown()
        for server in self.servers: server.server_close()
        for thread in self.threads: thread.join(timeout=5)
        if any(thread.is_alive() for thread in self.threads): raise RuntimeError('TLS fixture cleanup incomplete')


def tls_probe(entry, ca):
    """Independent real certificate verification; no unverified TLS context."""
    url = urlsplit(entry['url']); context = ssl.create_default_context(cafile=str(ca))
    try:
        with socket.create_connection(('127.0.0.1', url.port), timeout=10) as raw:
            with context.wrap_socket(raw, server_hostname=url.hostname) as secured:
                import hashlib
                return {'verified': True, 'leaf_sha256': hashlib.sha256(secured.getpeercert(binary_form=True)).hexdigest()}
    except ssl.SSLCertVerificationError as error:
        return {'verified': False, 'verify_code': error.verify_code, 'reason': error.verify_message}


class Wire:
    def __init__(self, port, token=None): self.port = port; self.token = token; self.session = None; self.log = []
    def command(self, method, path, body=None):
        headers = {'Content-Type': 'application/json'}
        if self.token: headers['Authorization'] = 'Bearer ' + self.token
        encoded = json.dumps(body) if body is not None else None
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=45)
        try:
            connection.request(method, path, encoded, headers)
            response = connection.getresponse(); raw = response.read(4 * 1024 * 1024 + 1)
            if len(raw) > 4 * 1024 * 1024: raise ValueError('control response exceeded bound')
            payload = json.loads(raw)
            if not isinstance(payload, dict) or 'value' not in payload: raise ValueError('malformed WebDriver control response')
            row = {'method': method, 'path': path, 'body': body, 'status': response.status, 'response': payload}
            self.log.append(row)
            return row
        finally: connection.close()
    def value(self, method, path, body=None):
        row = self.command(method, path, body)
        if row['status'] != 200: raise ValueError('unexpected WebDriver control error: ' + str(row['response']))
        return row['response']['value']
    def scoped(self, path): return '/session/' + self.session + '/' + path
    def script(self, script): return self.value('POST', self.scoped('execute/sync'), {'script': script, 'args': []})


class Driver:
    def __init__(self, binary, args, directory, token=None, allowed_exit=(0,)):
        self.binary = binary; self.args = args; self.directory = Path(directory); self.token = token; self.allowed_exit = allowed_exit
        self.process = None; self.outputs = []; self.cleanup = None
    def __enter__(self):
        check_execution_generation()
        with socket.socket() as reserve:
            reserve.bind(('127.0.0.1', 0)); self.port = reserve.getsockname()[1]
        self.directory.mkdir()
        self.outputs = [(self.directory / name).open('xb') for name in ('stdout.log', 'stderr.log')]
        try:
            argv = [str(self.binary), *[part.replace('{port}', str(self.port)) for part in self.args]]
            # Inherit the component supervisor's process group. A hard timeout
            # of the collector must also retire its browser/driver descendants.
            check_execution_generation()
            self.process = subprocess.Popen(argv, stdout=self.outputs[0], stderr=self.outputs[1])
            self.wire = Wire(self.port, self.token)
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                if self.process.poll() is not None: raise RuntimeError('control driver exited before readiness')
                try:
                    if self.wire.value('GET', '/status').get('ready') is True: return self
                except (OSError, ValueError): pass
                time.sleep(.05)
            raise TimeoutError('control driver readiness timeout')
        except BaseException:
            self.__exit__(None, None, None); raise
    def __exit__(self, *args):
        errors = []
        if self.process is not None:
            if self.wire.session:
                try: self.wire.value('DELETE', '/session/' + self.wire.session)
                except Exception as error: errors.append(type(error).__name__ + ': session teardown failed')
                self.wire.session = None
            if self.process.poll() is None: self.process.send_signal(signal.SIGTERM)
            try: self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                errors.append('driver shutdown timeout'); self.process.kill(); self.process.wait(timeout=15)
            # The outer original-run receipt separately checks/retires the
            # inherited process group after this collector exits.
            if self.process.returncode not in self.allowed_exit: errors.append('driver exited abnormally')
            self.cleanup = {'returncode': self.process.returncode, 'errors': errors, 'complete': not errors}
        for stream in self.outputs: stream.close()


def browser_controls(wire, contract, capabilities, native):
    created = wire.value('POST', '/session', {'capabilities': {'alwaysMatch': capabilities}})
    wire.session = created['sessionId']
    if created.get('capabilities', {}).get('acceptInsecureCerts') is not False:
        raise ValueError('browser did not attest normal certificate validation')
    results = {}
    for kind in TLS_KINDS:
        row = wire.command('POST', wire.scoped('url'), {'url': contract['tls'][kind]['url']})
        results['tls_' + kind] = row
        if kind == 'trusted':
            if row['status'] != 200 or wire.script('return document.title') != MARKER:
                raise ValueError('trusted TLS control did not render')
    # Restore trusted document before exercising actual focus transitions.
    wire.value('POST', wire.scoped('url'), {'url': contract['tls']['trusted']['url']})
    wire.script('window.focus(); return null')
    first = wire.value('GET', wire.scoped('window'))
    results['initial_focus'] = wire.script('return document.hasFocus()')
    wire.script('window.__controlFocus = []; window.__controlTimer = setInterval(() => window.__controlFocus.push(document.hasFocus()), 10); return null')
    second = wire.value('POST', wire.scoped('window/new'), {'type': 'window'})['handle']
    wire.value('POST', wire.scoped('window'), {'handle': second})
    wire.script('window.focus(); return null')
    wire.value('POST', wire.scoped('execute/async'), {'script': 'const done=arguments[arguments.length-1];setTimeout(()=>done(null),200)', 'args': []})
    # The original document samples focus while the other window is foreground.
    # Switching back may focus it, so a post-switch hasFocus call is not evidence.
    wire.value('POST', wire.scoped('window'), {'handle': first})
    results['focus_samples'] = wire.script('clearInterval(window.__controlTimer); return window.__controlFocus')
    results['denied_focus'] = not any(value is False for value in results['focus_samples'])
    wire.script('window.focus(); return null')
    results['restored_focus'] = wire.script('return document.hasFocus()')
    if native:
        results['unsupported'] = wire.command('POST', wire.scoped('oryn/qualification/unsupported'), {})
        wire.value('POST', wire.scoped('timeouts'), {'script': 100})
        results['missing_completion'] = wire.command('POST', wire.scoped('execute/async'), {'script': 'return null', 'args': []})
        results['duplicate_completion'] = wire.command('POST', wire.scoped('execute/async'), {'script': 'arguments[arguments.length-1](1); arguments[arguments.length-1](2)', 'args': []})
        results['worker_crash'] = wire.command('POST', wire.scoped('oryn/qualification/worker-crash'), {})
        results['after_crash'] = wire.command('GET', wire.scoped('title'))
    return results


def validate_results(record):
    errors = []
    if record.get('focus_isolation') != 'host_serialized': errors.append('reference host focus lease missing')
    probes = record.get('tls_probes', {})
    if probes.get('trusted', {}).get('verified') is not True: errors.append('trusted certificate verification missing')
    if probes.get('untrusted', {}).get('verified') is not False or probes.get('untrusted', {}).get('verify_code') not in (18, 19, 20, 21):
        errors.append('untrusted issuer control did not reach certificate rejection')
    if probes.get('wrong_hostname', {}).get('verified') is not False or probes.get('wrong_hostname', {}).get('verify_code') != 62:
        errors.append('wrong hostname control did not reach name rejection')
    for name in ('native', 'reference'):
        browser = record.get(name, {}); controls = browser.get('controls', {})
        if (browser.get('cleanup') or {}).get('complete') is not True or browser.get('errors'):
            errors.append(name + ': execution or cleanup failed')
        if controls.get('tls_trusted', {}).get('status') != 200:
            errors.append(name + ': trusted navigation failed')
        for kind in ('untrusted', 'wrong_hostname'):
            row = controls.get('tls_' + kind, {}); error = row.get('response', {}).get('value', {})
            message = str(error.get('message', '')).lower()
            if (type(row.get('status')) is not int or row['status'] < 400 or
                    not (error.get('error') == 'insecure certificate' or any(value in message for value in ('certificate', 'err_cert', 'unknownissuer', 'notvalidforname')))):
                errors.append(name + ': ' + kind + ' did not observe browser certificate rejection')
        if (controls.get('initial_focus') is not True or controls.get('restored_focus') is not True
                or controls.get('denied_focus') is not False or not isinstance(controls.get('focus_samples'), list)
                or not any(value is False for value in controls.get('focus_samples', []))):
            errors.append(name + ': actual foreground/background focus transition missing')
    native = record.get('native', {}).get('controls', {})
    for name, expected in (('unsupported', 'unknown command'), ('missing_completion', 'script timeout'), ('after_crash', 'no such window')):
        if native.get(name, {}).get('response', {}).get('value', {}).get('error') != expected:
            errors.append('native ' + name + ' was not rejected')
    duplicate = native.get('duplicate_completion', {})
    if duplicate.get('status') != 200 or duplicate.get('response', {}).get('value') != 1:
        errors.append('native duplicate completion did not preserve the first terminal value')
    crash = native.get('worker_crash', {})
    value = crash.get('response', {}).get('value', {})
    if crash.get('status') != 200 or value.get('reaped') is not True or value.get('signal') != signal.SIGKILL or type(value.get('worker_pid')) is not int or value['worker_pid'] <= 0:
        errors.append('native owned-worker crash/reap evidence missing')
    faults = record.get('protocol_faults', {})
    required = {'test_malformed_command_response_reaches_real_upstream_decoder',
                'test_failed_native_teardown_is_logged_and_propagated',
                'test_custody_cannot_be_redirected_to_a_symlink',
                'test_checked_session_requires_native_receipt_for_already_retired_session'}
    if (faults.get('successful') is not True or type(faults.get('tests_run')) is not int or faults.get('tests_run') != len(required)
            or {name.rsplit('.', 1)[-1] for name in faults.get('tests', [])} != required
            or faults.get('failures') or faults.get('errors') or faults.get('skipped')):
        errors.append('production protocol fault controls incomplete')
    return errors

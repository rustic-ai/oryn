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
    def __init__(self, contract, *, check_inputs=None):
        self.contract = contract; self.servers = []; self.threads = []
        self.check_inputs = check_execution_generation if check_inputs is None else check_inputs
    def __enter__(self):
        self.check_inputs()
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
                self.check_inputs()
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
    def command(self, method, path, body=None, *, deadline=None):
        headers = {'Content-Type': 'application/json'}
        if self.token: headers['Authorization'] = 'Bearer ' + self.token
        encoded = json.dumps(body) if body is not None else None
        def remaining():
            value = 45 if deadline is None else min(45, deadline - time.monotonic())
            if value <= 0: raise TimeoutError('control request deadline exhausted')
            return value
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=remaining())
        response = None
        try:
            connection.request(method, path, encoded, headers)
            transport = connection.sock
            if transport is not None: transport.settimeout(remaining())
            response = connection.getresponse()
            chunks = []; size = 0
            while size <= 4 * 1024 * 1024:
                if transport is not None: transport.settimeout(remaining())
                else: remaining()
                chunk = response.read1(min(65536, 4 * 1024 * 1024 + 1 - size))
                remaining()
                if not chunk: break
                chunks.append(chunk); size += len(chunk)
                if response.isclosed(): break
            raw = b''.join(chunks)
            if len(raw) > 4 * 1024 * 1024: raise ValueError('control response exceeded bound')
            payload = json.loads(raw)
            if not isinstance(payload, dict) or 'value' not in payload: raise ValueError('malformed WebDriver control response')
            row = {'method': method, 'path': path, 'body': body, 'status': response.status, 'response': payload}
            self.log.append(row)
            return row
        finally:
            try:
                if response is not None: response.close()
            finally:
                connection.close()
    def value(self, method, path, body=None, *, deadline=None):
        row = self.command(method, path, body, deadline=deadline)
        if row['status'] != 200: raise ValueError('unexpected WebDriver control error: ' + str(row['response']))
        return row['response']['value']
    def scoped(self, path): return '/session/' + self.session + '/' + path
    def script(self, script, *, deadline=None): return self.value('POST', self.scoped('execute/sync'), {'script': script, 'args': []}, deadline=deadline)


class Driver:
    def __init__(self, binary, args, directory, token=None, allowed_exit=(0,), *, check_inputs=None):
        self.binary = binary; self.args = args; self.directory = Path(directory); self.token = token; self.allowed_exit = allowed_exit
        self.process = None; self.outputs = []; self.cleanup = None
        self.check_inputs = check_execution_generation if check_inputs is None else check_inputs
    def __enter__(self):
        self.check_inputs()
        with socket.socket() as reserve:
            reserve.bind(('127.0.0.1', 0)); self.port = reserve.getsockname()[1]
        self.directory.mkdir()
        self.outputs = [(self.directory / name).open('xb') for name in ('stdout.log', 'stderr.log')]
        try:
            argv = [str(self.binary), *[part.replace('{port}', str(self.port)) for part in self.args]]
            # Inherit the component supervisor's process group. A hard timeout
            # of the collector must also retire its browser/driver descendants.
            self.check_inputs()
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


# The fixed local fixture serves MARKER on every accepted TLS connection.
# Observe the negative document immediately, before restoring trusted navigation.
CERTIFICATE_DOCUMENT_SCRIPT = """return {
 title: document.title.slice(0, 256),
 code: (document.querySelector('#error-code')?.textContent || '').trim().slice(0, 128),
 fixture_marker_present: (document.body?.innerText || '').includes('Oryn native prerequisite control')
} """


def reference_capabilities(binary):
    # Focus is an actual foreground/background prerequisite; headless tabs do not
    # demonstrate that transition. The caller holds the existing host focus lease.
    return {'browserName': 'chrome', 'acceptInsecureCerts': False,
            'goog:chromeOptions': {'binary': str(binary), 'args': []}}


def mapping(value):
    return value if isinstance(value, dict) else {}


def response_value(row):
    return mapping(mapping(row).get('response')).get('value')


def reference_certificate_interstitial(controls, kind):
    row = mapping(controls.get('tls_' + kind))
    observation = mapping(controls.get('tls_' + kind + '_document'))
    session = controls.get('session_id')
    url = mapping(controls.get('tls_urls')).get(kind)
    capabilities = mapping(controls.get('capabilities'))
    document = mapping(response_value(observation))
    codes = {'untrusted': 'net::ERR_CERT_AUTHORITY_INVALID',
             'wrong_hostname': 'net::ERR_CERT_COMMON_NAME_INVALID'}
    return (isinstance(session, str) and bool(session) and '/' not in session
            and isinstance(url, str) and url.startswith('https://')
            and capabilities.get('browserName') == 'chrome'
            and capabilities.get('acceptInsecureCerts') is False
            and row.get('status') == 200 and response_value(row) is None
            and mapping(row.get('response')).get('value', 'missing') is None
            and row.get('method') == 'POST' and row.get('path') == '/session/' + session + '/url'
            and row.get('body') == {'url': url}
            and observation.get('method') == 'POST'
            and observation.get('path') == '/session/' + session + '/execute/sync'
            and observation.get('body') == {'script': CERTIFICATE_DOCUMENT_SCRIPT, 'args': []}
            and observation.get('status') == 200
            and document.get('title') == 'Privacy error' and document.get('code') == codes[kind]
            and document.get('fixture_marker_present') is False)


CONTROL_STAGES = ('session', 'tls_trusted', 'tls_untrusted', 'tls_wrong_hostname', 'focus',
                  'unsupported', 'missing_completion', 'duplicate_completion', 'worker_crash', 'after_crash')


def new_controls(native):
    return {'control_execution': {name: {'status': 'not_run'}
            for name in (CONTROL_STAGES if native else CONTROL_STAGES[:5])}}


@contextmanager
def control_stage(results, name):
    state = results['control_execution'][name]
    state['status'] = 'running'
    try:
        yield
    except BaseException as error:
        state.update(status='failed', error=type(error).__name__ + ': ' + str(error))
        raise
    else:
        state['status'] = 'completed'


def activate_reference_window(wire, results, phase):
    """Activate the exact current Chromium target; never emulate document focus.

    Reuses Target.activateTarget/Page.bringToFront from wpt-chromium-canary.mjs.
    ChromeDriver's scoped CDP route provides the native target/window telemetry.
    """
    deadline = time.monotonic() + 5
    observation = {'phase': phase, 'samples': [], 'activation_budget_seconds': 5}
    results.setdefault('focus_activation', []).append(observation)
    def cdp(command, params=None):
        return wire.value('POST', wire.scoped('goog/cdp/execute'),
                          {'cmd': command, 'params': params or {}}, deadline=deadline)
    handle = wire.value('GET', wire.scoped('window'), deadline=deadline)
    target = cdp('Target.getTargetInfo')['targetInfo']
    observation.update(handle=handle, target=target)
    if (target.get('type') != 'page' or not isinstance(target.get('targetId'), str)
            or not target['targetId']
            or handle not in (target['targetId'], 'CDwindow-' + target['targetId'])):
        raise ValueError('reference native target does not match selected window')
    target_id = target['targetId']
    observation['window_before'] = cdp('Browser.getWindowForTarget', {'targetId': target_id})
    observation['focused_before'] = wire.script('return document.hasFocus()', deadline=deadline)
    cdp('Target.activateTarget', {'targetId': target_id})
    cdp('Page.bringToFront')
    observation_deadline = min(deadline, time.monotonic() + 2)
    for _ in range(101):
        focused = wire.script('return document.hasFocus()', deadline=observation_deadline)
        observation['samples'].append(focused)
        if type(focused) is not bool:
            raise ValueError('reference focus observation is not boolean')
        if focused:
            break
        if time.monotonic() >= observation_deadline:
            raise TimeoutError('actual reference foreground focus unavailable after native activation')
        time.sleep(.02)
    else:
        raise TimeoutError('actual reference foreground focus sample bound exhausted')
    observation['target_after'] = cdp('Target.getTargetInfo')['targetInfo']
    observation['window_after'] = cdp('Browser.getWindowForTarget', {'targetId': target_id})
    if (wire.value('GET', wire.scoped('window'), deadline=deadline) != handle
            or observation['target_after'].get('targetId') != target_id):
        raise ValueError('reference focus target changed during activation')


REFERENCE_FOCUS_START = """return (() => {
 const key = '__orynReferenceFocusControl';
 if (window[key]) throw new Error('reference focus observer already installed');
 const state = {rows: [], overflow: 0, listeners: [], timer: null};
 const sample = phase => {
   const row = {phase, at_ms: performance.now(), focused: document.hasFocus()};
   if (state.rows.length < 2048) state.rows.push(row); else state.overflow++;
 };
 for (const phase of ['blur', 'focus']) {
   const listener = () => sample(phase);
   window.addEventListener(phase, listener);
   state.listeners.push([phase, listener]);
 }
 state.stop = () => {
   clearInterval(state.timer);
   for (const [phase, listener] of state.listeners) window.removeEventListener(phase, listener);
   delete window[key];
   return {rows: state.rows, overflow: state.overflow, document_url: document.URL};
 };
 window[key] = state;
 sample('installed');
 state.timer = setInterval(() => sample('interval'), 10);
 return null;
})()"""
REFERENCE_FOCUS_STOP = "return window.__orynReferenceFocusControl ? window.__orynReferenceFocusControl.stop() : null"


def reference_focus_samples(observation):
    import math
    if (not isinstance(observation, dict) or set(observation) != {'rows', 'overflow', 'document_url'}
            or type(observation['overflow']) is not int or observation['overflow'] != 0
            or not isinstance(observation['document_url'], str)
            or not isinstance(observation['rows'], list) or not 1 <= len(observation['rows']) <= 2048):
        raise ValueError('reference focus observation missing, malformed or overflowed')
    previous = -1
    samples = []
    for row in observation['rows']:
        if (not isinstance(row, dict) or set(row) != {'phase', 'at_ms', 'focused'}
                or row['phase'] not in ('installed', 'interval', 'blur', 'focus')
                or type(row['at_ms']) not in (int, float) or not math.isfinite(row['at_ms'])
                or row['at_ms'] < previous or type(row['focused']) is not bool):
            raise ValueError('reference focus sample identity/type/order invalid')
        previous = row['at_ms']; samples.append(row['focused'])
    if observation['rows'][0]['phase'] != 'installed':
        raise ValueError('reference focus initial observer sample missing')
    return samples


def assert_reference_tab_topology(results):
    activations = results.get('focus_activation', [])
    initial = [row for row in activations if row.get('phase') == 'initial']
    other = [row for row in activations if row.get('phase') == 'other']
    if len(initial) != 1 or len(other) != 1:
        raise ValueError('reference tab activation identity observations missing')
    initial, other = initial[0], other[0]
    first_id = mapping(initial.get('target_after')).get('targetId')
    second_id = mapping(other.get('target_after')).get('targetId')
    first_window = mapping(initial.get('window_after')).get('windowId')
    second_window = mapping(other.get('window_after')).get('windowId')
    handles = results.get('focus_windows', {})
    if (not isinstance(first_id, str) or not first_id or not isinstance(second_id, str)
            or not second_id or first_id == second_id
            or initial.get('handle') != handles.get('first')
            or other.get('handle') != handles.get('second')
            or type(first_window) is not int or first_window <= 0
            or type(second_window) is not int or second_window != first_window):
        raise ValueError('reference tabs must be distinct targets in one actual native window')
    results['focus_topology'] = {'scope': 'foreground/background top-level documents in one native window',
        'native_window_id': first_window, 'first_target_id': first_id, 'second_target_id': second_id}


def reference_focus_transition(wire, results, first):
    """Capture real tab-document focus without depending on background timers.

    The existing 200ms observation and activation deadlines are unchanged.
    The timer remains a fallback; event names never determine focus truth.
    """
    stopped = False
    try:
        wire.script(REFERENCE_FOCUS_START)
        second = wire.value('POST', wire.scoped('window/new'), {'type': 'tab'})['handle']
        results['focus_windows'] = {'first': first, 'second': second}
        if not isinstance(second, str) or second == first:
            raise ValueError('reference background control requires a distinct window')
        wire.value('POST', wire.scoped('window'), {'handle': second})
        wire.script('window.focus(); return null')
        activate_reference_window(wire, results, 'other')
        assert_reference_tab_topology(results)
        wire.value('POST', wire.scoped('execute/async'), {'script': 'const done=arguments[arguments.length-1];setTimeout(()=>done(null),200)', 'args': []})
        wire.value('POST', wire.scoped('window'), {'handle': first})
        observation = wire.script(REFERENCE_FOCUS_STOP)
        stopped = True
        results['reference_focus_observation'] = observation
        if not isinstance(observation, dict) or observation.get('document_url') != results['tls_urls']['trusted']:
            raise ValueError('reference focus observer document changed')
        results['focus_samples'] = reference_focus_samples(observation)
        results['denied_focus'] = not any(value is False for value in results['focus_samples'])
        wire.script('window.focus(); return null')
        activate_reference_window(wire, results, 'restored')
        results['restored_focus'] = wire.script('return document.hasFocus()')
    finally:
        if not stopped:
            try:
                deadline = time.monotonic() + 5
                wire.value('POST', wire.scoped('window'), {'handle': first}, deadline=deadline)
                observation = wire.script(REFERENCE_FOCUS_STOP, deadline=deadline)
                results['reference_focus_observation'] = observation
                results['reference_focus_cleanup'] = {'complete': True}
            except BaseException as error:
                # Preserve initiating error; Driver teardown retains final custody.
                results['reference_focus_cleanup'] = {'complete': False, 'error': type(error).__name__ + ': ' + str(error)}


def browser_controls(wire, contract, capabilities, native, *, results=None):
    # Caller owns this object before any request, so an exception cannot erase
    # prior observations or turn unattempted controls into apparent rejection failures.
    if results is None: results = {}
    results.update(new_controls(native))
    with control_stage(results, 'session'):
        created = wire.value('POST', '/session', {'capabilities': {'alwaysMatch': capabilities}})
        wire.session = created['sessionId']
        results.update(session_id=wire.session,
            capabilities={key: created['capabilities'].get(key) for key in ('browserName', 'acceptInsecureCerts')},
            tls_urls={kind: contract['tls'][kind]['url'] for kind in TLS_KINDS})
        if created.get('capabilities', {}).get('acceptInsecureCerts') is not False:
            raise ValueError('browser did not attest normal certificate validation')
    for kind in TLS_KINDS:
        with control_stage(results, 'tls_' + kind):
            row = wire.command('POST', wire.scoped('url'), {'url': contract['tls'][kind]['url']})
            results['tls_' + kind] = row
            if not native and kind != 'trusted' and row['status'] == 200:
                results['tls_' + kind + '_document'] = wire.command(
                    'POST', wire.scoped('execute/sync'), {'script': CERTIFICATE_DOCUMENT_SCRIPT, 'args': []})
            if kind == 'trusted':
                if row['status'] == 200:
                    results['trusted_title'] = wire.script('return document.title')
                if row['status'] != 200 or results.get('trusted_title') != MARKER:
                    raise ValueError('trusted TLS control did not render')
    with control_stage(results, 'focus'):
        wire.value('POST', wire.scoped('url'), {'url': contract['tls']['trusted']['url']})
        wire.script('window.focus(); return null')
        first = wire.value('GET', wire.scoped('window'))
        if not native: activate_reference_window(wire, results, 'initial')
        results['initial_focus'] = wire.script('return document.hasFocus()')
        if not native:
            reference_focus_transition(wire, results, first)
        else:
            wire.script('window.__controlFocus = []; window.__controlTimer = setInterval(() => window.__controlFocus.push(document.hasFocus()), 10); return null')
            second = wire.value('POST', wire.scoped('window/new'), {'type': 'window'})['handle']
            wire.value('POST', wire.scoped('window'), {'handle': second})
            wire.script('window.focus(); return null')
            if not native: activate_reference_window(wire, results, 'other')
            wire.value('POST', wire.scoped('execute/async'), {'script': 'const done=arguments[arguments.length-1];setTimeout(()=>done(null),200)', 'args': []})
            # Samples are collected in the first document while the other is foreground.
            wire.value('POST', wire.scoped('window'), {'handle': first})
            results['focus_samples'] = wire.script('clearInterval(window.__controlTimer); return window.__controlFocus')
            results['denied_focus'] = not any(value is False for value in results['focus_samples'])
            wire.script('window.focus(); return null')
            if not native: activate_reference_window(wire, results, 'restored')
            results['restored_focus'] = wire.script('return document.hasFocus()')
    if native:
        with control_stage(results, 'unsupported'):
            results['unsupported'] = wire.command('POST', wire.scoped('oryn/qualification/unsupported'), {})
        with control_stage(results, 'missing_completion'):
            wire.value('POST', wire.scoped('timeouts'), {'script': 100})
            results['missing_completion'] = wire.command('POST', wire.scoped('execute/async'), {'script': 'return null', 'args': []})
        with control_stage(results, 'duplicate_completion'):
            results['duplicate_completion'] = wire.command('POST', wire.scoped('execute/async'), {'script': 'arguments[arguments.length-1](1); arguments[arguments.length-1](2)', 'args': []})
        with control_stage(results, 'worker_crash'):
            results['worker_crash'] = wire.command('POST', wire.scoped('oryn/qualification/worker-crash'), {})
        with control_stage(results, 'after_crash'):
            results['after_crash'] = wire.command('GET', wire.scoped('title'))
    return results


def validate_results(record):
    errors = []
    if record.get('focus_isolation') != 'host_serialized': errors.append('reference host focus lease missing')
    def completed(browser, controls, stage):
        if 'control_execution' not in controls:
            return True  # Historical schema retains its existing strict checks.
        status = mapping(mapping(controls.get('control_execution')).get(stage)).get('status')
        if status == 'completed': return True
        if status == 'not_run': errors.append(browser + ': ' + stage + ' not run after earlier control failure')
        elif status == 'failed': errors.append(browser + ': ' + stage + ' control execution failed')
        else: errors.append(browser + ': ' + stage + ' control execution incomplete')
        return False
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
        completed(name, controls, 'session')
        if completed(name, controls, 'tls_trusted'):
            if (controls.get('tls_trusted', {}).get('status') != 200
                    or ('control_execution' in controls and controls.get('trusted_title') != MARKER)):
                errors.append(name + ': trusted navigation failed')
        for kind in ('untrusted', 'wrong_hostname'):
            if not completed(name, controls, 'tls_' + kind): continue
            row = mapping(controls.get('tls_' + kind)); error = mapping(response_value(row))
            if (type(row.get('status')) is not int or row['status'] < 400 or
                    error.get('error') != 'insecure certificate'):
                if name != 'reference' or not reference_certificate_interstitial(controls, kind):
                    errors.append(name + ': ' + kind + ' did not observe browser certificate rejection')
        if completed(name, controls, 'focus') and (controls.get('initial_focus') is not True or controls.get('restored_focus') is not True
                or controls.get('denied_focus') is not False or not isinstance(controls.get('focus_samples'), list)
                or not any(value is False for value in controls.get('focus_samples', []))):
            errors.append(name + ': actual foreground/background focus transition missing')
    native = record.get('native', {}).get('controls', {})
    for name, expected in (('unsupported', 'unknown command'), ('missing_completion', 'script timeout'), ('after_crash', 'no such window')):
        if completed('native', native, name) and mapping(response_value(native.get(name))).get('error') != expected:
            errors.append('native ' + name + ' was not rejected')
    duplicate = mapping(native.get('duplicate_completion'))
    if completed('native', native, 'duplicate_completion') and (duplicate.get('status') != 200 or response_value(duplicate) != 1):
        errors.append('native duplicate completion did not preserve the first terminal value')
    crash = mapping(native.get('worker_crash'))
    value = mapping(response_value(crash))
    if completed('native', native, 'worker_crash') and (crash.get('status') != 200 or value.get('reaped') is not True or value.get('signal') != signal.SIGKILL or type(value.get('worker_pid')) is not int or value['worker_pid'] <= 0):
        errors.append('native owned-worker crash/reap evidence missing')
    faults = record.get('protocol_faults', {})
    required = {'test_malformed_command_response_reaches_real_upstream_decoder',
                'test_failed_native_teardown_is_logged_and_propagated',
                'test_custody_cannot_be_redirected_to_a_symlink',
                'test_checked_session_requires_native_receipt_for_already_retired_session',
                'test_checked_session_preserves_delete_error_when_transport_close_fails',
                'test_checked_session_failure_reaches_exact_protocol_terminal_receipt'}
    if (faults.get('successful') is not True or type(faults.get('tests_run')) is not int or faults.get('tests_run') != len(required)
            or {name.rsplit('.', 1)[-1] for name in faults.get('tests', [])} != required
            or faults.get('failures') or faults.get('errors') or faults.get('skipped')):
        errors.append('production protocol fault controls incomplete')
    return errors

"""Direct-serving fidelity and state isolation; live tests use pinned WPT."""
import http.client
import hashlib
import json
import re
import os
from pathlib import Path
import sys
import subprocess
import time
import ssl
import socket
import unittest
from urllib.parse import urlsplit
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_fixture import DirectFixture


def select_http_diagnostics(raw):
    events = []
    allowed = {'accepted','request_line_wait','request_line_complete','headers_complete',
               'handler_enter','handler_error','response_ready','dispatch_return','dispatch_error','records_truncated'}
    for line in raw.splitlines():
        prefix = 'ORYN_WPT_HTTP_DIAGNOSTIC '
        if not line.startswith(prefix) or len(line.encode()) > 8192:
            continue
        try:
            value = json.loads(line[len(prefix):])
            if value.get('schema_version') != 1 or value.get('kind') != 'wpt_http_boundary' or value.get('event') not in allowed:
                continue
            selected = {'event':value['event']}
            for key in ('at_ms','server_port','peer_port','request_sequence','status','limit'):
                entry = value.get(key)
                if isinstance(entry,int) and not isinstance(entry,bool) and 0 <= entry <= 2**53-1:
                    selected[key] = entry
            for key in ('valid','close_connection','opaque_target'):
                if isinstance(value.get(key),bool): selected[key] = value[key]
            if value.get('method') in ('GET','POST','PUT','DELETE','HEAD','OPTIONS','PATCH','CONNECT','TRACE'):
                selected['method'] = value['method']
            if isinstance(value.get('path'),str) and value['path'].startswith('/'):
                path = urlsplit(value['path'])
                selected.update(path=path.path[:2048],path_truncated=len(path.path)>2048 or value.get('path_truncated') is True,
                                query_present=bool(path.query) or value.get('query_present') is True,
                                fragment_present=bool(path.fragment) or value.get('fragment_present') is True)
            if value.get('error_type') in ('OSError','TimeoutError','ValueError','TypeError','ConnectionError','BrokenPipeError','ConnectionResetError','Exception'):
                selected['error_type'] = value['error_type']
            events.append(selected)
        except (ValueError,TypeError,AttributeError):
            continue
    return events


def _server_snapshot(fixture):
    """Read bounded fixture state without moving the live server's log offset."""
    process = fixture.process
    state = dict(ports=list(fixture.ports[:7]), pid=process.pid if process else None,
                 returncode=process.poll() if process else None,
                 generations=[dict(id=str(item['id'])[:256], backend=str(item['backend'])[:32]) for item in fixture.generations[-16:]],
                 timings={key:fixture.timings.get(key) for key in ('startup_ms','teardown_ms')}, shutdown_complete=fixture.shutdown_complete,
                         failure=str(fixture.failure)[:256] if fixture.failure is not None else None, request_log=[], request_log_available=False,
                         http_boundaries=[],http_boundary_log_available=False)
    try:
        if fixture.log is not None:
            descriptor = fixture.log.fileno()
            size = os.fstat(descriptor).st_size
            raw = os.pread(descriptor, min(size, 65536), max(0, size - 65536))
            state.update(log_bytes=size, log_tail_truncated=size > len(raw))
            requests = []
            decoded = raw.decode('utf-8', errors='replace')
            boundaries = select_http_diagnostics(decoded)
            state.update(http_boundaries=boundaries[-256:],http_boundary_log_available=bool(boundaries),
                         http_boundary_records_omitted=max(0,len(boundaries)-256))
            for line in decoded.splitlines():
                match = re.search(r'''(["'])(GET|POST|PUT|DELETE|HEAD|OPTIONS|PATCH) ([^ ]+) HTTP/[0-9.]+\1 ([0-9]{3})''', line)
                if match:
                    location = urlsplit(match[3])
                    requests.append(dict(method=match[2], path=location.path[:2048],
                                         path_truncated=len(location.path)>2048,
                                         query_present=bool(location.query), status=int(match[4])))
            state.update(request_log=requests[-64:], request_log_omitted=max(0,len(requests)-64),
                         request_log_available=bool(requests))
    except (OSError, ValueError) as error:
        state['log_read_error'] = type(error).__name__
    return state


def server_snapshot(fixture):
    try:
        return _server_snapshot(fixture)
    except Exception as error:
        return {'snapshot_error':type(error).__name__}


def safe_diagnostic_location(value):
    if not isinstance(value,dict): return {'invalid_url':True}
    origin=value.get('origin')
    if isinstance(origin,str):
        parsed=urlsplit(origin)
        if parsed.scheme in ('http','https'):
            path=value.get('path','')
            if not isinstance(path,str) or not path.startswith('/'): return {'invalid_url':True}
            clean=urlsplit(path)
            return dict(origin=(parsed.scheme+'://'+parsed.netloc.rsplit('@',1)[-1])[:512],
                        path=clean.path[:2048], path_truncated=len(clean.path)>2048 or value.get('path_truncated') is True,
                        query_present=bool(clean.query) or value.get('query_present') is True,
                        fragment_present=bool(clean.fragment) or value.get('fragment_present') is True)
    if value.get('scheme')=='about:' and value.get('page') in ('blank','srcdoc'):
        return {'scheme':'about:','page':value['page']}
    scheme=value.get('scheme')
    if isinstance(scheme,str) and re.fullmatch(r'[a-z][a-z0-9+.-]{0,63}:',scheme):
        return {'scheme':scheme,'opaque_body_redacted':True}
    return {'invalid_url':True}


def select_canary_diagnostics(stderr):
    """Retain only bounded records emitted by the diagnostic helper, never raw stacks."""
    records = []
    allowed = {'schema_version','kind','phase','failure_phase','requested_location','elapsed_ms','error',
               'last_timeout','pending_cdp','pending_cdp_omitted','timeline','timeline_dropped',
               'document_tracking_overflow','request_tracking_overflow','stage','serialization_budget_bytes',
               'serialization_omitted_events','serialization_omitted_pending','serialization_omitted_browser_records'}
    event_fields = {'at_ms','kind','name','phase','event','pid','code','signal','error_code','id','method',
                    'session_id','started_ms','elapsed_ms','failed','timeout_ms','child_session_id','type',
                    'target_id','waiting_for_debugger','location','context_id','unique_id','frame_id','is_default',
                    'parent_id','loader_id','request_id','status','protocol','error_text','canceled','blocked_reason'}
    for line in stderr.splitlines():
        prefix = 'ORYN_CHROMIUM_DIAGNOSTIC '
        if not line.startswith(prefix) or len(line.encode()) > 65600: continue
        try:
            value = json.loads(line[len(prefix):])
            if value.get('schema_version') != 1 or value.get('kind') != 'chromium_reference_failure': continue
            def fields(item, names):
                if not isinstance(item,dict): return {}
                selected={}
                for key in names:
                    if key not in item: continue
                    entry=item.get(key)
                    if isinstance(entry,str): selected[key]=entry[:256]
                    elif entry is None or isinstance(entry,(bool,int)): selected[key]=entry
                    elif isinstance(entry,float) and entry==entry and abs(entry)!=float('inf'): selected[key]=entry
                return selected
            selected = fields(value,allowed)
            selected['requested_location']=safe_diagnostic_location(value.get('requested_location'))
            error=value.get('error',{})
            selected['error']={'name':error.get('name') if isinstance(error,dict) and error.get('name') in ('Error','TypeError','RangeError','SyntaxError','ReferenceError','URIError','EvalError','AbortError','TimeoutError') else 'Error'}
            selected['last_timeout']=fields(value.get('last_timeout'),event_fields) if value.get('last_timeout') is not None else None
            selected['pending_cdp']=[fields(item,event_fields) for item in value.get('pending_cdp',[])[:256] if isinstance(item,dict)]
            selected['timeline']=[]
            for item in value.get('timeline',[])[:256]:
                if not isinstance(item,dict):continue
                event=fields(item,event_fields)
                if 'location' in item:event['location']=safe_diagnostic_location(item['location'])
                selected['timeline'].append(event)
            native=value.get('browser_stderr')
            if isinstance(native,dict):
                selected['browser_stderr']={}
                for key in ('bytes_seen','bytes_inspected','bytes_omitted','lines_seen','unclassified_lines','oversized_or_partial_lines','records_omitted'):
                    count=native.get(key)
                    if isinstance(count,int) and not isinstance(count,bool) and count>=0:
                        selected['browser_stderr'][key]=count
                selected['browser_stderr']['stream_ended']=native.get('stream_ended') is True
                selected['browser_stderr']['records']=[]
                for item in native.get('records',[])[:32]:
                    if not isinstance(item,dict): continue
                    clean={}
                    for key,choices in (('severity',('WARNING','ERROR','FATAL')),('component',('network_service','child_process','sandbox','other')),
                                        ('category',('network_service_crashed','network_service_restarting','launch_failed','permission_denied','sandbox_failure','unclassified'))):
                        if item.get(key) in choices: clean[key]=item[key]
                    for key in ('at_ms','pid','os_error'):
                        if isinstance(item.get(key),int) and not isinstance(item[key],bool): clean[key]=item[key]
                    if isinstance(item.get('net_error'),str) and re.fullmatch(r'net::ERR_[A-Z0-9_]{1,80}',item['net_error']):
                        clean['net_error']=item['net_error']
                    selected['browser_stderr']['records'].append(clean)
            records.append(selected)
            if len(records) == 2: break
        except (ValueError, TypeError, AttributeError):
            continue
    return records


def run_chromium_reference(fixture, path, url, sample=None):
    started = time.monotonic()
    before = server_snapshot(fixture)
    def _retain(result=None, error=None):
        stderr = result.stderr if result is not None else getattr(error, 'stderr', '') or ''
        if isinstance(stderr, bytes): stderr = stderr.decode('utf-8', errors='replace')
        record = dict(schema_version=1, kind='chromium_fixture_failure', fixture_path=path,
                      sample=sample, elapsed_ms=round((time.monotonic()-started)*1000),
                      returncode=result.returncode if result is not None else None,
                      exception=type(error).__name__ if error is not None else None,
                      server_before=before, server_after=server_snapshot(fixture),
                      canary_diagnostics=select_canary_diagnostics(stderr), stderr_bytes=len(stderr.encode()),
                      stderr_sha256=hashlib.sha256(stderr.encode()).hexdigest())
        # The server often has no request logger enabled. Say so explicitly;
        # the browser's bounded Network/Frame timeline supplies that observation.
        sys.stderr.write('ORYN_CHROMIUM_FIXTURE_DIAGNOSTIC ' + json.dumps(record) + '\n')
    def retain(result=None,error=None):
        try:
            _retain(result,error)
        except Exception:
            try: sys.stderr.write('ORYN_CHROMIUM_FIXTURE_DIAGNOSTIC_UNAVAILABLE\n')
            except Exception: pass
    try:
        result = subprocess.run([os.environ['ORYN_WPT_NODE'],
            str(Path(__file__).resolve().parents[1] / 'wpt-chromium-canary.mjs'),
            os.environ['ORYN_WPT_CHROMIUM'], url, '10000'],
            capture_output=True, text=True, timeout=35)
    except BaseException as error:
        retain(error=error)
        raise
    if result.returncode != 0:
        retain(result=result)
    return result


class FixtureFailureTests(unittest.TestCase):
    def test_partial_reservations_are_closed(self):
        with patch('wpt_fixture.socket.socket') as socket:
            socket.return_value.bind.side_effect = PermissionError('denied')
            with self.assertRaises(PermissionError):
                DirectFixture('/unavailable')
            socket.return_value.close.assert_called_once()


@unittest.skipUnless(os.environ.get('ORYN_WPT_CHECKOUT'), 'requires pinned WPT')
class DirectFixtureTests(unittest.TestCase):
    def setUp(self):
        self.fixture = DirectFixture(os.environ['ORYN_WPT_CHECKOUT'])
        self.fixture.start('oryn')

    def tearDown(self):
        self.fixture.close()

    def test_close_delimited_https_sends_tls_close_notify(self):
        context = ssl.create_default_context(cafile=self.fixture.tls['ca'])
        path = '/infrastructure/testdriver/actions/support/actions.html?parent'
        for port in (self.fixture.ports[1], self.fixture.ports[3]):
            with socket.create_connection(('127.0.0.1', port), timeout=5) as tcp:
                with context.wrap_socket(tcp, server_hostname='localhost',
                                         suppress_ragged_eofs=False) as stream:
                    stream.sendall((f'GET {path} HTTP/1.1\r\nHost: localhost:{port}\r\n'
                                    'Connection: close\r\n\r\n').encode())
                    chunks = []
                    while data := stream.recv(4096):
                        chunks.append(data)
                    headers, body = b''.join(chunks).split(b'\r\n\r\n', 1)
                    self.assertIn(b'200', headers.split(b'\r\n')[0])
                    self.assertEqual(body, (self.fixture.checkout / path.split('?')[0][1:]).read_bytes())

    def request(self, path, method='GET', body=None, port=None):
        conn = http.client.HTTPConnection('127.0.0.1', port or self.fixture.ports[0], timeout=5)
        try:
            conn.request(method, path, body=body)
            response = conn.getresponse()
            return response.status, response.read()
        finally:
            conn.close()

    def test_adapter_and_upstream_wrapper(self):
        status, body = self.request('/resources/testharnessreport.js')
        self.assertEqual(status, 200)
        self.assertIn(b'setup({output: false})', body)
        status, body = self.request('/url/historical.any.worker.html?variant=one')
        self.assertEqual(status, 200)
        self.assertIn(b'new Worker("/url/historical.any.worker.js?variant=one")', body)

    def test_headers_and_first_chunk_arrive_before_eof(self):
        conn = http.client.HTTPConnection('127.0.0.1', self.fixture.ports[0], timeout=5)
        started = time.monotonic()
        try:
            conn.request('GET', '/xhr/resources/trickle.py?ms=200&count=10&specifylength=1')
            response = conn.getresponse()
            self.assertLess(time.monotonic() - started, 1.5)
            chunk = b'TEST_TRICKLE\n'
            self.assertEqual(response.read(len(chunk)), chunk)
            self.assertLess(time.monotonic() - started, 1.5)
            # Cancel while upstream is still writing; fixture teardown must bound it.
        finally:
            conn.close()

    def test_stash_reset_and_distinct_browser_adapter(self):
        path = '/common/dispatcher/dispatcher.py?uuid=85323643-1b53-46de-b182-f4314669689c'
        self.assertEqual(self.request(path, 'POST', b'old-generation'), (200, b'done'))
        ports = self.fixture.ports[:]
        self.fixture.close()
        self.fixture.start('chromium')
        self.assertEqual(self.fixture.ports, ports)
        # Dispatcher responds "not ready" for an empty fresh stash.
        self.assertEqual(self.request(path), (200, b'not ready'))
        status, body = self.request('/resources/testdriver-vendor.js')
        self.assertEqual(status, 200)
        self.assertIn(b'__oryn_driver_request', body)
        self.assertNotIn(b'__oryn_testdriver_click(element)', body)
        self.assertNotEqual(*[g['id'] for g in self.fixture.generations])

    def test_https_and_wss_validate_run_ca_and_hostname(self):
        context = ssl.create_default_context(cafile=self.fixture.tls['ca'])
        self.assertEqual(context.verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(context.check_hostname)
        conn = http.client.HTTPSConnection('localhost', self.fixture.ports[1], context=context, timeout=5)
        try:
            conn.request('GET','/resources/testharnessreport.js')
            response = conn.getresponse()
            self.assertEqual(response.status,200)
            self.assertIn(b'setup({output: false})',response.read())
        finally:
            conn.close()
        with socket.create_connection(('127.0.0.1',self.fixture.ports[6]),timeout=5) as tcp:
            with context.wrap_socket(tcp,server_hostname='localhost') as stream:
                stream.sendall((f'GET /echo HTTP/1.1\r\nHost: localhost:{self.fixture.ports[6]}\r\n'
                    'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n'
                    'Sec-WebSocket-Version: 13\r\n\r\n').encode())
                self.assertIn(b'101',stream.recv(4096).split(b'\r\n')[0])
        with socket.create_connection(('127.0.0.1',self.fixture.ports[1]),timeout=5) as tcp:
            with self.assertRaises(ssl.SSLCertVerificationError):
                context.wrap_socket(tcp,server_hostname='outside.example')

    def test_direct_h2_extended_connect_echo_and_close(self):
        import json
        result = subprocess.run([str(self.fixture.checkout / '_venv3/bin/python'),
            str(Path(__file__).resolve().parents[1] / 'wpt-h2-probe.py'),
            '--port', str(self.fixture.ports[4]), '--ca', self.fixture.tls['ca']],
            capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout),
            {'alpn':'h2','extended_connect':True,'echo':True,'close':True})

    @unittest.skipUnless(os.environ.get('ORYN_WPT_BINARY'), 'requires signed binary')
    def test_cold_and_prewarmed_websocket_audits_are_final_before_result(self):
        from wpt_session import WptSession
        with patch.dict(os.environ, {'ORYN_WPT_CA_CERT':self.fixture.tls['ca'],
                                      'ORYN_WPT_ALLOW_INVALID_CERTS':'0'}):
            for prewarmed in (False, True):
                session = WptSession(Path(os.environ['ORYN_WPT_BINARY']), prewarmed=prewarmed)
                try:
                    for variant in ('default','wss'):
                        result = session.run(variant,
                            f'http://localhost:{self.fixture.ports[0]}/websockets/Send-0byte-data.any.html?{variant}',30)
                        self.assertIsNone(result['error'],result)
                        self.assertFalse(result['cleanup_errors'],result)
                        self.assertEqual(result['completion']['harness'],0,result)
                        # Engine assertion outcomes are intentionally not constrained.
                        audit = result['resource_audit']
                        self.assertTrue(result['resource_evidence_complete'],result)
                        self.assertTrue(audit['websockets'],result)
                        for record in audit['websockets']:
                            self.assertEqual(record['handshake_status'],101,record)
                            self.assertIsNotNone(record['terminal'],record)
                finally:
                    shutdown = session.close()
                self.assertEqual(shutdown['returncode'],0,shutdown)
                self.assertTrue(shutdown['shutdown_acknowledged'],shutdown)

    @unittest.skipUnless(os.environ.get('ORYN_WPT_CHROMIUM') and os.environ.get('ORYN_WPT_NODE'),
                         'requires Chromium and Node')
    def test_chromium_completion_drains_reentrant_send_keys(self):
        import json
        self.fixture.close()
        self.fixture.start('chromium')
        path = 'html/interaction/focus/sequential-focus-navigation-and-the-tabindex-attribute/focus-tabindex-order.html'
        for sample in range(3):
            result = run_chromium_reference(self.fixture, path,
                f'http://localhost:{self.fixture.ports[0]}/{path}', sample=sample)
            self.assertEqual(result.returncode, 0, (sample, result.stderr))
            report = json.loads(result.stdout)
            self.assertEqual(report['completion']['harness'], 0, report)
            self.assertEqual([t['status'] for t in report['completion']['tests']], [0])
            for events in (report['automation_events'], report['completion']['testdriver']):
                self.assertEqual(len(events), 18, (sample, events))
                for operation_id in range(1, 10):
                    self.assertEqual([e['outcome'] for e in events if e['id'] == operation_id],
                                     ['started', 'completed'], (sample, events))

    @unittest.skipUnless(os.environ.get('ORYN_WPT_CHROMIUM') and os.environ.get('ORYN_WPT_NODE'),
                         'requires Chromium and Node')
    def test_chromium_click_and_send_keys_reference(self):
        import json
        for path in ('click.html', 'send_keys.html', 'actions/elementPosition.html', 'actions/actionsWithKeyPressed.html',
                     'click_iframe.html', 'click_iframe_crossorigin.sub.html'):
            self.fixture.close()
            self.fixture.start('chromium')
            result = run_chromium_reference(self.fixture, 'infrastructure/testdriver/' + path,
                f'http://localhost:{self.fixture.ports[0]}/infrastructure/testdriver/{path}')
            self.assertEqual(result.returncode, 0, result.stderr)
            completion = json.loads(result.stdout)['completion']
            self.assertEqual(completion['harness'], 0, completion)
            self.assertTrue(completion['tests'], completion)
            self.assertTrue(all(t['status'] == 0 for t in completion['tests']), completion)
            events = json.loads(result.stdout)['automation_events']
            self.assertTrue(events, path)
            self.assertTrue(all(e['outcome'] in ('started','completed') for e in events), events)
            self.assertFalse(json.loads(result.stdout)['automation_bridge_errors'])


if __name__ == '__main__':
    unittest.main()

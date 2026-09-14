"""Direct-serving fidelity and state isolation; live tests use pinned WPT."""
import http.client
import os
from pathlib import Path
import sys
import subprocess
import time
import ssl
import socket
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_fixture import DirectFixture


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
            result = subprocess.run([os.environ['ORYN_WPT_NODE'],
                str(Path(__file__).resolve().parents[1] / 'wpt-chromium-canary.mjs'),
                os.environ['ORYN_WPT_CHROMIUM'],
                f'http://localhost:{self.fixture.ports[0]}/{path}', '10000'],
                capture_output=True, text=True, timeout=35)
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
            result = subprocess.run([os.environ['ORYN_WPT_NODE'],
                str(Path(__file__).resolve().parents[1] / 'wpt-chromium-canary.mjs'),
                os.environ['ORYN_WPT_CHROMIUM'],
                f'http://localhost:{self.fixture.ports[0]}/infrastructure/testdriver/{path}', '10000'],
                capture_output=True, text=True, timeout=35)
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

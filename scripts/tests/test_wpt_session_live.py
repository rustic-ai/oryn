"""Signed transport/isolation canaries; opt in with ORYN_WPT_BINARY and checkout."""
from concurrent.futures import ThreadPoolExecutor
import http.server
import os
import json
import subprocess
from pathlib import Path
import sys
import threading
import signal
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from wpt_session import WptSession


@unittest.skipUnless(os.environ.get('ORYN_WPT_BINARY') and os.environ.get('ORYN_WPT_CHECKOUT'),
                     'requires signed binary and pinned WPT')
class SignedSessionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        checkout = Path(os.environ['ORYN_WPT_CHECKOUT'])
        cls.pending_started = threading.Event()
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_GET(self):
                path = self.path.split('?')[0]
                if path == '/pending.html':
                    cls.pending_started.set()
                mime = 'text/html'
                status = 200
                extra_headers = {}
                if path == '/resources/testharness.js':
                    body = (checkout / 'resources/testharness.js').read_bytes()
                    mime = 'text/javascript'
                elif path == '/trace-child.html':
                    body = b"<script>__oryn_record_automation_event(1,'click','started');__oryn_record_automation_event(1,'click','completed');parent.postMessage('traced','*')</script>"
                elif path == '/resources/testharnessreport.js':
                    body = (ROOT / 'scripts/wpt-report.js').read_bytes()
                    mime = 'text/javascript'
                elif path == '/helper.js':
                    body = b'globalThis.helperLoaded = true;'
                    mime = 'text/javascript'
                elif path == '/unreported-focus.html':
                    body = b"<script>globalThis.__oryn_wpt_completion={complete:true,harness:0,completion_count:1,tests:[{name:'unverified',status:0}]}</script>"
                elif path == '/missing':
                    body, status = b'expected missing fixture', 404
                elif path == '/redirect.html':
                    body, status = b'', 302
                    extra_headers['location'] = '/ready.html'
                elif path == '/module.js':
                    body, mime = b"globalThis.moduleLoaded=true;", 'text/javascript'
                else:
                    script = {
                        '/write.html': "test(()=>{globalThis.previousLease=123;localStorage.setItem('lease','old');document.cookie='lease=old;path=/';document.body.appendChild(document.createElement('aside'));},'write state');",
                        '/read.html': "test(()=>{assert_equals(globalThis.previousLease,undefined);assert_equals(localStorage.getItem('lease'),null);assert_equals(document.cookie,'');assert_equals(document.querySelector('aside'),null);},'fresh state');",
                        '/dynamic.html': "promise_test(async()=>{await new Promise((resolve,reject)=>{let s=document.createElement('script');s.src='/helper.js';s.onload=resolve;s.onerror=reject;document.head.appendChild(s)});assert_true(helperLoaded);let r=await fetch('/missing');assert_equals(r.status,404);},'dynamic and intentional error');",
                        '/pending.html': "async_test('never completes');",
                        '/popup.html': "promise_test(async()=>{let child=window.open('/ready.html');assert_not_equals(child,null);await new Promise(resolve=>setTimeout(resolve,500));child.close();},'isolated auxiliary page');",
                        '/navigate.html': "location.href='/ready.html';",
                        '/self-close.html': "async_test('closed without completion');setTimeout(()=>window.close(),10);",
                        '/csp.html': "test(()=>assert_equals(globalThis.blockedScript,undefined),'CSP fixture');",
                        '/module.html': "promise_test(async()=>{await import('/module.js');assert_true(moduleLoaded);},'module fixture');",
                        '/focus-entry.html': "test(()=>{assert_true(document.hasFocus());let input=document.createElement('input');document.body.appendChild(input);let events=0;input.addEventListener('focus',()=>events++);input.focus();assert_equals(events,1);assert_equals(document.activeElement,input);input.blur();assert_not_equals(document.activeElement,input);},'parser-time native focus');",
                        '/frame.html': "promise_test(()=>new Promise(resolve=>{let frame=document.createElement('iframe');frame.src='/ready.html';frame.onload=()=>{assert_not_equals(frame.contentDocument,null);resolve()};document.body.appendChild(frame)}),'nested frame fixture');",
                        '/trace.html': f"promise_test(()=>new Promise(resolve=>{{__oryn_record_automation_event(1,'click','started');__oryn_record_automation_event(1,'click','completed');let frame=document.createElement('iframe');addEventListener('message',event=>{{if(event.data==='traced'){{frame.remove();resolve()}}}});frame.src='http://127.0.0.1:{self.server.server_port}/trace-child.html';document.body.appendChild(frame)}}),'cross-origin native trace');",
                    }.get(path, "test(()=>assert_true(true),'ready');")
                    prefix = ''
                    if path == '/csp.html':
                        extra_headers['content-security-policy'] = "script-src 'self' 'nonce-canary'"
                        prefix = '<script>globalThis.blockedScript=true;</script>'
                    body = ('<!doctype html><script src=/resources/testharness.js></script>'
                            '<script src=/resources/testharnessreport.js></script><body>' + prefix + '<script nonce=canary>'
                            + script + '</script>').encode()
                self.send_response(status)
                self.send_header('content-type', mime)
                self.send_header('content-length', str(len(body)))
                for key, value in extra_headers.items():
                    self.send_header(key, value)
                self.end_headers()
                self.wfile.write(body)
        cls.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = f'http://localhost:{cls.server.server_port}'

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def test_isolation_concurrency_resources_and_timeout_replacement(self):
        session = WptSession(Path(os.environ['ORYN_WPT_BINARY']))
        try:
            def run(name, timeout=15):
                return session.run(name, self.base + '/' + name + '.html', timeout)
            first, second = run('write'), run('read')
            for result in (first, second):
                self.assertIsNone(result['error'])
                self.assertFalse(result['cleanup_errors'])
                self.assertEqual(result['completion']['harness'], 0)
                self.assertTrue(all(t['status'] == 0 for t in result['completion']['tests']), result)
                self.assertTrue(result['resource_evidence_complete'], result)
                self.assertTrue(all(r['worker_pid'] for r in result['resources']))
            self.assertNotEqual(first['resources'][0]['worker_pid'], second['resources'][0]['worker_pid'])
            dynamic = run('dynamic')
            self.assertIsNone(dynamic['error'])
            self.assertTrue(all(t['status'] == 0 for t in dynamic['completion']['tests']), dynamic)
            self.assertTrue(any(r['url'].endswith('/helper.js') and r['response_sha256'] for r in dynamic['resources']))
            self.assertTrue(any(r['url'].endswith('/missing') and r['status'] == 404 for r in dynamic['resources']))
            with ThreadPoolExecutor(max_workers=8) as executor:
                results = list(executor.map(run, ['ready'] * 20))
            pids = set()
            for result in results:
                self.assertIsNone(result['error'])
                self.assertFalse(result['cleanup_errors'])
                pids.add(result['resources'][0]['worker_pid'])
            self.assertEqual(len(pids), 20, 'isolated workers must never be recycled')
            failed = run('pending', 1)
            self.assertEqual(failed['error'], 'outer_execution_timeout')
            recovered = run('ready')
            self.assertIsNone(recovered['error'])
            self.assertFalse(recovered['cleanup_errors'])
        finally:
            shutdown = session.close()
        self.assertEqual(shutdown['returncode'], 0, shutdown)
        self.assertTrue(shutdown['shutdown_acknowledged'], shutdown)

    @unittest.skipUnless(os.environ.get('ORYN_WPT_NODE') and os.environ.get('ORYN_WPT_CHROMIUM'),
                         'requires explicit Node and Chromium for differential')
    def test_chromium_fixture_completion_parity(self):
        session = WptSession(Path(os.environ['ORYN_WPT_BINARY']))
        try:
            for name in ('ready', 'dynamic', 'redirect', 'csp', 'module', 'frame'):
                url = self.base + '/' + name + '.html'
                reference = subprocess.run([os.environ['ORYN_WPT_NODE'], str(ROOT / 'scripts/wpt-chromium-canary.mjs'),
                    os.environ['ORYN_WPT_CHROMIUM'], url, '15000'],
                    capture_output=True, text=True, timeout=360)
                self.assertEqual(reference.returncode, 0, f'{name}: {reference.stderr}')
                chromium = json.loads(reference.stdout)['completion']
                native = session.run(name, url, 15)
                self.assertIsNone(native['error'], native)
                self.assertEqual(native['completion']['harness'], chromium['harness'], name)
                self.assertEqual([t['name'] for t in native['completion']['tests']],
                                 [t['name'] for t in chromium['tests']], name)
                # Browser assertion differences remain browser outcomes, not a
                # reason to modify fixtures or omit observed assertions.
                self.assertTrue(native['resource_evidence_complete'], native)
        finally:
            shutdown = session.close()
        self.assertEqual(shutdown['returncode'], 0, shutdown)

    @unittest.skipUnless(os.environ.get('ORYN_WPT_NODE') and os.environ.get('ORYN_WPT_CHROMIUM'),
                         'requires explicit Node and Chromium for focus')
    def test_chromium_parser_time_focus_is_verified_before_tests(self):
        def run(_):
            result = subprocess.run([os.environ['ORYN_WPT_NODE'],
                str(ROOT / 'scripts/wpt-chromium-canary.mjs'),
                os.environ['ORYN_WPT_CHROMIUM'],
                self.base + '/focus-entry.html', '5000'], capture_output=True, text=True, timeout=345)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(result.stdout)
            self.assertIs(data['initial_focus_verified'], True)
            self.assertIs(data['entry_focus_verified'], True)
            self.assertEqual(data['entry_focus_phase'], 'report_ready')
            self.assertEqual(data['focus_isolation'], 'host_serialized')
            self.assertEqual(data['completion']['tests'][0]['status'], 0, data)
        with ThreadPoolExecutor(max_workers=2) as executor:
            list(executor.map(run, range(4)))

    @unittest.skipUnless(os.environ.get('ORYN_WPT_NODE') and os.environ.get('ORYN_WPT_CHROMIUM'),
                         'requires explicit Node and Chromium for focus')
    def test_chromium_rejects_completion_without_test_entry_focus(self):
        result = subprocess.run([os.environ['ORYN_WPT_NODE'],
            str(ROOT / 'scripts/wpt-chromium-canary.mjs'),
            os.environ['ORYN_WPT_CHROMIUM'],
            self.base + '/unreported-focus.html', '5000'], capture_output=True, text=True, timeout=345)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Chromium document-entry focus unverified (missing observation)', result.stderr)
        self.assertEqual(result.stdout, '')

    def test_auxiliary_pages_do_not_exhaust_primary_pool(self):
        session = WptSession(Path(os.environ['ORYN_WPT_BINARY']))
        try:
            with ThreadPoolExecutor(max_workers=8) as executor:
                results = list(executor.map(
                    lambda index: session.run(str(index), self.base + '/popup.html', 20), range(16)))
            for result in results:
                self.assertIsNone(result['error'], result)
                self.assertFalse(result['cleanup_errors'], result)
                self.assertIsNone(result['replacement_error'], result)
                self.assertTrue(all(t['status'] == 0 for t in result['completion']['tests']), result)
                self.assertGreaterEqual(len({r['worker_pid'] for r in result['resources']}), 2, result)
            recovered = session.run('after-popups', self.base + '/ready.html', 15)
            self.assertIsNone(recovered['error'], recovered)
        finally:
            shutdown = session.close()
        self.assertEqual(shutdown['returncode'], 0, shutdown)

    def test_cross_origin_native_trace_survives_frame_removal_and_isolates_pages(self):
        for prewarmed in (False, True):
            session = WptSession(Path(os.environ['ORYN_WPT_BINARY']), prewarmed=prewarmed)
            try:
                for index in range(2):
                    result = session.run(f'trace-{index}',self.base+'/trace.html',15)
                    self.assertIsNone(result['error'],result)
                    trace=result['completion']['automation_trace']
                    self.assertTrue(trace['complete'],result)
                    self.assertFalse(trace['overflow'],result)
                    self.assertEqual(trace['scope'],'page_isolate')
                    self.assertEqual(len(trace['events']),4,result)
                    self.assertEqual(trace['events'],result['completion']['testdriver'])
                    self.assertEqual(len({e['context_id'] for e in trace['events']}),2)
                    self.assertEqual({e['id'] for e in trace['events']},{1})
            finally:
                shutdown=session.close()
            self.assertEqual(shutdown['returncode'],0,shutdown)

    def test_retirement_after_navigation_close_and_timeout_churn(self):
        session = WptSession(Path(os.environ['ORYN_WPT_BINARY']))
        try:
            for fixture in ('navigate', 'self-close', 'pending'):
                with ThreadPoolExecutor(max_workers=8) as executor:
                    results = list(executor.map(lambda index: session.run(
                        f'{fixture}-{index}', self.base + '/' + fixture + '.html', 1), range(16)))
                for result in results:
                    self.assertNotIn('pool capacity', result.get('error') or '', result)
                    self.assertNotIn('worker_retirement_timeout', result['cleanup_errors'], result)
                    self.assertIsNone(result['replacement_error'], result)
                ready = session.run('after-' + fixture, self.base + '/ready.html', 15)
                self.assertIsNone(ready['error'], ready)
                self.assertFalse(ready['cleanup_errors'], ready)
        finally:
            shutdown = session.close()
        self.assertEqual(shutdown['returncode'], 0, shutdown)

    def test_crashed_assigned_worker_is_replaced_without_retrying_test(self):
        session = WptSession(Path(os.environ['ORYN_WPT_BINARY']))
        try:
            # Preparation publishes workers in order; checkout is LIFO.
            workers = session.progress()['workers']
            self.assertEqual(len(workers), 8)
            victim = max(worker['pid'] for worker in workers)
            self.pending_started.clear()
            with ThreadPoolExecutor(max_workers=1) as executor:
                pending = executor.submit(session.run, 'crash', self.base + '/pending.html', 15)
                self.assertTrue(self.pending_started.wait(5))
                # Only a confirmed child of this test-owned parent is targeted.
                self.assertIn(victim, {worker['pid'] for worker in session.progress()['workers']})
                os.kill(victim, signal.SIGKILL)
                failed = pending.result(timeout=30)
            self.assertIsNotNone(failed['error'])
            self.assertNotEqual(failed['error'], 'outer_execution_timeout')
            recovered = session.run('after-crash', self.base + '/ready.html', 15)
            self.assertIsNone(recovered['error'], recovered)
            self.assertNotEqual(recovered['resources'][0]['worker_pid'], victim)
        finally:
            shutdown = session.close()
        self.assertEqual(shutdown['returncode'], 0, shutdown)

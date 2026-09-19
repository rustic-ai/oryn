"""Cross-origin PKCE application boundaries and durable record authority."""
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import unittest
from urllib.parse import parse_qs, urlencode, urlsplit

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('saas_v2', ROOT / 'test-harness/workflows/saas_fixture_v2.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class SaaSOAuthTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / 'saas.sqlite'
        self.now = 1000
        self.fixture = module.Fixture(self.database, clock=lambda: self.now)
        self.fixture.origin = 'http://127.0.0.1:3104'
        self.server = module.transport.create_server(self.fixture.provider, self.fixture.mfa_clients)
        self.fixture.issuer = f'http://127.0.0.1:{self.server.server_port}'
        self.thread = threading.Thread(target=self.server.serve_forever)
        self.thread.start()
        self.case = 'a' * 32
        self.fixture.command(dict(command='create', case_id=self.case))

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.fixture.db.close()
        self.fixture.provider.close()
        self.directory.cleanup()

    def flow(self):
        status, _, headers = self.fixture.begin(self.case)
        self.assertEqual(status, 303)
        headers = dict(headers)
        cookie = module.SimpleCookie(headers['Set-Cookie'])['saas_pending'].value
        handle = self.fixture.provider.begin(headers['Location'])
        redirect, _, status = self.fixture.provider.authorize(handle, 'agent@example.test', 'fixture-only-password')
        self.assertEqual(status, 302)
        query = parse_qs(urlsplit(redirect['Location']).query)
        return query, cookie

    def login(self):
        query, cookie = self.flow()
        status, _, headers = self.fixture.callback(self.case, query, cookie)
        self.assertEqual(status, 303)
        cookies = module.SimpleCookie()
        for key, value in headers:
            if key == 'Set-Cookie':
                cookies.load(value)
        return cookies['saas_session'].value

    def test_real_http_exchange_authorizes_persisted_record_and_logout(self):
        cookie = self.login()
        case = self.fixture.db.execute('SELECT * FROM cases WHERE case_id=?', (self.case,)).fetchone()
        body = urlencode(dict(csrf=case['csrf'], title='Approved', version='0')).encode()
        self.assertEqual(self.fixture.post(self.case, 'create', body, cookie)[0], 303)
        self.assertEqual(self.fixture.post(self.case, 'logout', urlencode(dict(csrf=case['csrf'])).encode(), cookie)[0], 303)
        self.assertEqual(self.fixture.get(self.case, 'records', {}, cookie)[0], 401)
        data = module.snapshot(self.database, self.case)
        self.assertEqual(data['records'], [dict(case_id=self.case, title='Approved', version=1)])
        self.assertEqual(data['cases'], [dict(case_id=self.case, authenticated=0, logins=1)])
        self.assertEqual(data['oauth']['codes'], [dict(consumed=1)])
        self.assertEqual(data['oauth']['tokens'], [dict(revoked=0)])
        self.assertNotIn(cookie, json.dumps(data))

    def test_foreign_cookie_state_and_replayed_callback_never_authenticate(self):
        query, cookie = self.flow()
        self.assertEqual(self.fixture.callback(self.case, query, 'foreign')[0], 400)
        changed = dict(query, state=['foreign'])
        self.assertEqual(self.fixture.callback(self.case, changed, cookie)[0], 400)
        self.assertEqual(module.snapshot(self.database, self.case)['oauth']['tokens'], [])
        self.assertEqual(self.fixture.callback(self.case, query, cookie)[0], 303)
        self.assertEqual(self.fixture.callback(self.case, query, cookie)[0], 400)
        self.assertEqual(module.snapshot(self.database, self.case)['cases'][0]['logins'], 1)

    def test_expired_and_superseded_attempts_cannot_create_a_session(self):
        query, cookie = self.flow()
        self.fixture.begin(self.case)
        self.assertEqual(self.fixture.callback(self.case, query, cookie)[0], 400)
        query, cookie = self.flow()
        self.now += 120
        self.assertEqual(self.fixture.callback(self.case, query, cookie)[0], 400)
        self.assertEqual(module.snapshot(self.database, self.case)['cases'][0]['authenticated'], 0)

    def test_new_login_during_exchange_retires_old_callback(self):
        query, cookie = self.flow()
        original = self.fixture.json_request
        entered, release = threading.Event(), threading.Event()
        errors, results = [], []

        def delayed(request):
            if request.full_url.endswith('/token'):
                entered.set()
                if not release.wait(5):
                    raise RuntimeError('test release missing')
            return original(request)

        self.fixture.json_request = delayed

        def callback():
            try:
                results.append(self.fixture.callback(self.case, query, cookie))
            except Exception as error:
                errors.append(error)

        worker = threading.Thread(target=callback)
        worker.start()
        try:
            self.assertTrue(entered.wait(5))
            self.assertEqual(self.fixture.begin(self.case)[0], 303)
        finally:
            release.set()
            worker.join(10)
        self.assertFalse(worker.is_alive())
        self.assertEqual(errors, [])
        self.assertEqual(results[0][0], 409)
        self.assertEqual(module.snapshot(self.database, self.case)['cases'][0]['authenticated'], 0)

    def test_foreign_identity_and_direct_password_route_are_rejected(self):
        query, cookie = self.flow()
        original = self.fixture.json_request

        def foreign(request):
            result = original(request)
            if '/identity?' in request.full_url:
                return {'case_id': 'b' * 32}
            return result

        self.fixture.json_request = foreign
        self.assertEqual(self.fixture.callback(self.case, query, cookie)[0], 401)
        self.assertEqual(self.fixture.post(self.case, 'login', b'username=agent@example.test&password=fixture-only-password', '')[0], 405)
        self.assertEqual(module.snapshot(self.database, self.case)['cases'][0]['authenticated'], 0)

    def test_record_forms_are_hydrated_without_exposing_authority_in_script(self):
        cookie = self.login()
        status, body, _ = self.fixture.get(self.case, 'records', {}, cookie)
        self.assertEqual(status, 200)
        self.assertIn('<main id="saas-app" data-state="loading">', body)
        self.assertIn('type="module" src="./saas-client.mjs"', body)
        self.assertIn('name=csrf', body)
        status, script, headers = self.fixture.get(self.case, 'saas-client.mjs', {}, cookie)
        self.assertEqual(status, 200)
        self.assertIn(('Content-Type', 'text/javascript; charset=utf-8'), headers)
        self.assertNotIn(cookie, script)
        self.assertIn('new FormData(form, event.submitter)', script)

    def test_concurrent_writer_rejects_stale_save_without_losing_its_update(self):
        self.fixture.concurrent_edits.add(self.case)
        cookie = self.login()
        case = self.fixture.db.execute('SELECT * FROM cases WHERE case_id=?', (self.case,)).fetchone()
        def save(version, title, action='update'):
            return self.fixture.post(self.case, action, urlencode(dict(csrf=case['csrf'], title=title, version=str(version))).encode(), cookie)
        self.assertEqual(save(0, 'Draft', 'create')[0], 303)
        status, original, _ = self.fixture.get(self.case, 'records', {}, cookie)
        self.assertEqual(status, 200)
        self.assertIn('id=version>1', original)
        self.assertEqual(save(1, 'Rejected stale edit')[0], 409)
        data = module.snapshot(self.database, self.case)
        self.assertEqual(data['records'], [dict(case_id=self.case, title='Concurrent', version=2)])
        self.assertEqual(data['concurrent_edits'], [dict(title='Concurrent', version=2)])
        status, reloaded, _ = self.fixture.get(self.case, 'records', {}, cookie)
        self.assertEqual(status, 200)
        self.assertIn('id=version>2', reloaded)
        self.assertEqual(save(2, 'Approved')[0], 303)
        self.assertEqual(save(2, 'Approved')[0], 409)
        data = module.snapshot(self.database, self.case)
        self.assertEqual(data['records'], [dict(case_id=self.case, title='Approved', version=3)])
        self.assertEqual([row['outcome'] for row in data['requests']], [303, 303, 409, 303, 409])

    def test_expired_session_relogin_rotates_cookie_and_csrf_before_recovery(self):
        self.fixture.expire_after_create.add(self.case)
        old_cookie = self.login()
        old = self.fixture.db.execute('SELECT * FROM cases WHERE case_id=?', (self.case,)).fetchone()
        create = urlencode(dict(csrf=old['csrf'], title='Draft', version='0')).encode()
        self.assertEqual(self.fixture.post(self.case, 'create', create, old_cookie)[0], 303)
        self.assertEqual(self.fixture.get(self.case, 'records', {}, old_cookie)[0], 200)
        stale = urlencode(dict(csrf=old['csrf'], title='Rejected', version='1')).encode()
        self.assertEqual(self.fixture.post(self.case, 'update', stale, old_cookie)[0], 401)
        new_cookie = self.login()
        new = self.fixture.db.execute('SELECT * FROM cases WHERE case_id=?', (self.case,)).fetchone()
        self.assertNotEqual(old_cookie, new_cookie)
        self.assertNotEqual(old['csrf'], new['csrf'])
        self.assertEqual(self.fixture.post(self.case, 'update', stale, old_cookie)[0], 401)
        self.assertEqual(self.fixture.post(self.case, 'update', stale, new_cookie)[0], 403)
        fresh = urlencode(dict(csrf=new['csrf'], title='Approved', version='1')).encode()
        self.assertEqual(self.fixture.post(self.case, 'update', fresh, new_cookie)[0], 303)
        data = module.snapshot(self.database, self.case)
        self.assertEqual(data['records'], [dict(case_id=self.case, title='Approved', version=2)])
        self.assertEqual(data['expirations'], [dict(case_id=self.case, reason='after_create')])
        self.assertEqual(data['oauth']['codes'], [dict(consumed=1), dict(consumed=1)])
        serialized = json.dumps(data)
        for secret in (old_cookie, new_cookie, old['csrf'], new['csrf']):
            self.assertNotIn(secret, serialized)

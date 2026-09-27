"""Real OAuthlib grant controls; generated for the final test phase."""
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from http.client import HTTPConnection
from http.cookies import SimpleCookie
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import unittest
from urllib.parse import urlencode, urlsplit, parse_qs

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('oauth_provider', ROOT / 'test-harness/workflows/oauth_provider_v2.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
http_spec = importlib.util.spec_from_file_location('oauth_http', ROOT / 'test-harness/workflows/oauth_http_v2.py')
transport = importlib.util.module_from_spec(http_spec)
http_spec.loader.exec_module(transport)


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.now = 1000
        self.provider = module.Provider(Path(self.directory.name) / 'idp.sqlite', clock=lambda: self.now)
        self.case = 'a' * 32
        self.redirect = f'http://127.0.0.1:3104/saas/{self.case}/callback'
        self.provider.register(self.case, self.redirect)
        self.verifier = 'v' * 64
        self.challenge = base64.urlsafe_b64encode(hashlib.sha256(self.verifier.encode()).digest()).decode().rstrip('=')

    def tearDown(self):
        self.provider.close()
        self.directory.cleanup()

    def uri(self, **overrides):
        return 'http://127.0.0.1:3105/authorize?' + urlencode(dict(client_id=self.case,
            redirect_uri=self.redirect, response_type='code', scope='records', state='s' * 43,
            code_challenge=self.challenge, code_challenge_method='S256') | overrides)

    def issue(self, mfa=False):
        handle = self.provider.begin(self.uri(), require_mfa=mfa)
        headers, _, status = self.provider.authorize(handle, 'agent@example.test',
            'fixture-only-password', '246810' if mfa else '')
        self.assertEqual(status, 302)
        query = parse_qs(urlsplit(headers['Location']).query)
        self.assertEqual(query['state'], ['s' * 43])
        return query['code'][0]

    def exchange(self, code, **overrides):
        fields = dict(client_id=self.case, redirect_uri=self.redirect, code=code,
                      grant_type='authorization_code', code_verifier=self.verifier) | overrides
        _, body, status = self.provider.exchange('http://127.0.0.1:3105/token', urlencode(fields))
        return status, json.loads(body)

    def test_code_exchange_is_bound_and_one_use(self):
        code = self.issue()
        status, token = self.exchange(code)
        self.assertEqual(status, 200)
        self.assertNotIn('refresh_token', token)
        self.assertEqual(self.provider.identity(token['access_token'], self.case), self.case)
        self.assertIsNone(self.provider.identity(token['access_token'], 'b' * 32))
        self.assertEqual(self.exchange(code)[0], 400)
        self.assertIsNone(self.provider.identity(token['access_token'], self.case))
        snapshot = self.provider.snapshot(self.case)
        self.assertEqual(snapshot['codes'], [dict(consumed=1)])
        self.assertEqual(snapshot['tokens'], [dict(revoked=1)])
        text = json.dumps(snapshot)
        for secret in (code, self.verifier, self.challenge, token['access_token'], 'fixture-only-password'):
            self.assertNotIn(secret, text)

    def test_wrong_verifier_redirect_and_client_cannot_exchange(self):
        code = self.issue()
        self.assertEqual(self.exchange(code, code_verifier='w' * 64)[0], 400)
        self.assertEqual(self.exchange(code, redirect_uri=self.redirect + '/foreign')[0], 400)
        self.assertIn(self.exchange(code, client_id='b' * 32)[0], (400, 401))
        self.assertEqual(self.provider.snapshot(self.case)['tokens'], [])
        self.assertEqual(self.exchange(code)[0], 200)

    def test_expired_codes_and_tokens_fail(self):
        code = self.issue()
        self.now += 60
        self.assertEqual(self.exchange(code)[0], 400)
        code = self.issue()
        status, token = self.exchange(code)
        self.assertEqual(status, 200)
        self.now += 120
        self.assertIsNone(self.provider.identity(token['access_token'], self.case))

    def test_mfa_is_an_explicit_test_credential_boundary(self):
        handle = self.provider.begin(self.uri(), require_mfa=True)
        with self.assertRaises(ValueError):
            self.provider.authorize(handle, 'agent@example.test', 'fixture-only-password')
        self.assertEqual(self.provider.snapshot(self.case)['codes'], [])
        self.assertEqual(self.provider.authorize(handle, 'agent@example.test', 'fixture-only-password', '246810')[2], 302)
        with self.assertRaises(ValueError):
            self.provider.authorize(handle, 'agent@example.test', 'fixture-only-password', '246810')
        self.assertIn(dict(action='credentials_rejected', outcome=401), self.provider.snapshot(self.case)['events'])

    def test_concurrent_replay_never_issues_two_tokens(self):
        code = self.issue()
        with ThreadPoolExecutor(max_workers=2) as executor:
            results = list(executor.map(lambda _: self.exchange(code), range(2)))
        self.assertEqual(sorted(status for status, _ in results), [200, 400])
        self.assertEqual(len(self.provider.snapshot(self.case)['tokens']), 1)

    def test_invalid_authorization_does_not_redirect_or_issue(self):
        for override in (dict(redirect_uri='https://foreign.example/callback'),
                         dict(code_challenge_method='plain'), dict(state=''),
                         dict(response_type='token'), dict(scope='admin'), dict(client_id='b' * 32)):
            with self.subTest(override=override), self.assertRaises(ValueError):
                self.provider.begin(self.uri(**override))
        self.assertEqual(self.provider.snapshot(self.case)['codes'], [])

    def test_duplicate_parameters_and_missing_verifier_fail(self):
        with self.assertRaises(ValueError):
            self.provider.begin(self.uri() + '&client_id=' + self.case)
        code = self.issue()
        self.assertEqual(self.exchange(code, code_verifier='')[0], 400)
        self.assertEqual(self.provider.snapshot(self.case)['tokens'], [])


    @contextmanager
    def http_provider(self):
        server = transport.create_server(self.provider, set())
        thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05})
        thread.start()
        try:
            yield server.server_port
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            self.assertFalse(thread.is_alive(), 'owned provider thread did not settle')

    def http_request(self, port, method, path, body=None, headers=None):
        connection = HTTPConnection('127.0.0.1', port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            return response.status, response.read().decode(), dict(response.getheaders())
        finally:
            connection.close()

    def test_authorization_document_csp_allows_only_its_registered_callback_origin(self):
        other_case = 'b' * 32
        self.provider.register(other_case, f'http://127.0.0.1:3204/saas/{other_case}/callback')
        with self.http_provider() as port:
            path = '/authorize?' + urlsplit(self.uri()).query
            status, body, headers = self.http_request(
                port, 'GET', path, headers={'Host': 'untrusted.example:9000'})
            self.assertEqual(status, 200)
            self.assertIn('Authorize test application', body)
            self.assertEqual(headers['Content-Security-Policy'],
                "default-src 'none'; form-action 'self' http://127.0.0.1:3104; frame-ancestors 'none'")
            self.assertNotIn('3204', headers['Content-Security-Policy'])
            self.assertNotIn('untrusted.example', headers['Content-Security-Policy'])
            cookie = SimpleCookie(headers['Set-Cookie'])['idp_pending'].value
            status, _body, redirect = self.http_request(port, 'POST', '/authorize',
                urlencode(dict(csrf=cookie, username='agent@example.test',
                               password='fixture-only-password')),
                {'Content-Type': 'application/x-www-form-urlencoded',
                 'Cookie': f'idp_pending={cookie}'})
            self.assertEqual(status, 302)
            destination = urlsplit(redirect['Location'])
            self.assertEqual(destination.scheme + '://' + destination.netloc + destination.path,
                             self.redirect)
            self.assertEqual(parse_qs(destination.query)['state'], ['s' * 43])
            self.assertTrue(parse_qs(destination.query)['code'][0])
        self.assertEqual(self.provider.snapshot(self.case)['codes'], [dict(consumed=0)])
        self.assertEqual(self.provider.snapshot(self.case)['tokens'], [])

    def test_invalid_http_authorization_never_expands_csp_or_redirects(self):
        with self.http_provider() as port:
            for override in (
                    dict(redirect_uri='https://foreign.example/callback'),
                    dict(redirect_uri=self.redirect + '/unregistered'),
                    dict(redirect_uri=self.redirect.replace(':3104/', ':3204/')),
                    dict(client_id='b' * 32)):
                with self.subTest(override=override):
                    status, _body, headers = self.http_request(
                        port, 'GET', '/authorize?' + urlsplit(self.uri(**override)).query)
                    self.assertEqual(status, 400)
                    self.assertNotIn('Location', headers)
                    self.assertNotIn('Set-Cookie', headers)
                    self.assertEqual(headers['Content-Security-Policy'],
                        "default-src 'none'; form-action 'self'; frame-ancestors 'none'")
        self.assertEqual(self.provider.snapshot(self.case)['codes'], [])
        self.assertEqual(self.provider.snapshot(self.case)['tokens'], [])

    def test_callback_origin_requires_issued_live_pending_authorization(self):
        with self.assertRaises(ValueError):
            self.provider.authorization_callback_origin('unissued')
        handle = self.provider.begin(self.uri())
        self.assertEqual(self.provider.authorization_callback_origin(handle),
                         'http://127.0.0.1:3104')
        self.provider.authorize(handle, 'agent@example.test', 'fixture-only-password')
        with self.assertRaises(ValueError):
            self.provider.authorization_callback_origin(handle)
        expired = self.provider.begin(self.uri())
        self.now += 120
        with self.assertRaises(ValueError):
            self.provider.authorization_callback_origin(expired)

"""Public same-origin landing preserves SaaS session and OAuth authority."""
import importlib.util
from http.cookies import SimpleCookie
from pathlib import Path
import tempfile
import unittest
from urllib.parse import parse_qs, urlencode, urlsplit
from unittest.mock import patch

HERE = Path(__file__).resolve().parents[1] / 'test-harness' / 'workflows'


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MODULES = {version: load('saas_fixture' + (f'_v{version}' if version != 1 else ''))
           for version in (1, 2, 4)}


class SaaSResumeNavigationTests(unittest.TestCase):
    def fixture(self, version):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        module = MODULES[version]
        fixture = module.Fixture(Path(directory.name) / 'app.sqlite')
        self.addCleanup(fixture.db.close)
        if version != 1:
            self.addCleanup(fixture.provider.db.close)
            fixture.origin = 'http://127.0.0.1:3104'
            fixture.issuer = 'http://127.0.0.1:3105'
        case = 'a' * 32
        fixture.command(dict(command='create', case_id=case))
        return fixture, case

    def login(self, fixture, case, version):
        if version == 1:
            result = fixture.post(case, 'login', urlencode(dict(
                username='agent@example.test', password='fixture-only-password')).encode(), '')
        else:
            status, _, headers = fixture.begin(case)
            self.assertEqual(status, 303)
            fields = dict(headers)
            pending = SimpleCookie(fields['Set-Cookie'])['saas_pending'].value
            state = parse_qs(urlsplit(fields['Location']).query)['state']
            # Only the identity provider transport is replaced. The fixture's
            # original state/cookie/epoch checks and session minting execute.
            with patch.object(fixture, 'json_request', side_effect=[
                dict(token_type='Bearer', access_token='provider-token'), dict(case_id=case)
            ]):
                result = fixture.callback(case, dict(state=state, code=['issued-code']), pending)
        self.assertEqual(result[0], 303)
        cookie = next(SimpleCookie(value)['saas_session'] for key, value in result[2]
                      if key == 'Set-Cookie' and 'saas_session' in SimpleCookie(value))
        self.assertEqual(cookie['samesite'], 'Strict')
        self.assertTrue(cookie['httponly'])
        self.assertEqual(cookie['path'], f'/saas/{case}/')
        return cookie.value

    def test_public_landing_is_read_only_and_contains_no_session_authority(self):
        for version in MODULES:
            with self.subTest(version=version):
                fixture, case = self.fixture(version)
                before = list(fixture.db.iterdump())
                secrets = fixture.db.execute('SELECT session,csrf FROM cases').fetchone()
                for cookie in ('', 'foreign'):
                    status, body, headers = fixture.get(case, 'resume', {}, cookie)
                    self.assertEqual(status, 200)
                    self.assertIn('Resume navigation', body)
                    self.assertEqual(headers, [])
                    self.assertNotIn('<script', body)
                    self.assertNotIn('<form', body)
                    self.assertTrue(all(secret not in body for secret in secrets))
                self.assertEqual(list(fixture.db.iterdump()), before)

    def test_landing_never_authenticates_unknown_logged_out_or_expired_session(self):
        for version in MODULES:
            with self.subTest(version=version):
                fixture, case = self.fixture(version)
                self.assertEqual(fixture.get('b' * 32, 'resume', {}, '')[0], 404)
                self.assertEqual(fixture.get(case, 'resume', {}, '')[0], 200)
                self.assertEqual(fixture.get(case, 'records', {}, '')[0], 401)
                cookie = self.login(fixture, case, version)
                fixture.command(dict(command='expire', case_id=case))
                self.assertEqual(fixture.get(case, 'resume', {}, cookie)[0], 200)
                self.assertEqual(fixture.get(case, 'records', {}, cookie)[0], 401)

    def test_strict_session_survives_landing_without_rotation_or_new_login(self):
        for version in MODULES:
            with self.subTest(version=version):
                fixture, case = self.fixture(version)
                cookie = self.login(fixture, case, version)
                before = list(fixture.db.iterdump())
                self.assertEqual(fixture.get(case, 'resume', {}, '')[0], 200)
                self.assertEqual(list(fixture.db.iterdump()), before)
                self.assertEqual(fixture.get(case, 'records', {}, cookie)[0], 200)
                self.assertEqual(fixture.get(case, 'records', {}, 'foreign')[0], 401)

    def test_landing_does_not_bypass_mutation_cookie_or_csrf_guards(self):
        for version in MODULES:
            with self.subTest(version=version):
                fixture, case = self.fixture(version)
                cookie = self.login(fixture, case, version)
                csrf = fixture.db.execute('SELECT csrf FROM cases').fetchone()[0]
                fixture.get(case, 'resume', {}, '')
                body = urlencode(dict(csrf=csrf, title='Owned', version='0')).encode()
                self.assertEqual(fixture.post(case, 'create', body, '')[0], 401)
                self.assertEqual(fixture.post(case, 'create', body, 'foreign')[0], 401)
                forged = urlencode(dict(csrf='foreign', title='Owned', version='0')).encode()
                self.assertEqual(fixture.post(case, 'create', forged, cookie)[0], 403)
                self.assertEqual(fixture.db.execute('SELECT count(*) FROM records').fetchone()[0], 0)
                self.assertEqual(fixture.post(case, 'create', body, cookie)[0], 303)
                self.assertEqual(fixture.db.execute('SELECT title,version FROM records').fetchone()[:], ('Owned', 1))

    def test_landing_preserves_pending_oauth_state_and_rejects_foreign_callback(self):
        for version in (2, 4):
            with self.subTest(version=version):
                fixture, case = self.fixture(version)
                _, _, headers = fixture.begin(case)
                fields = dict(headers)
                state = parse_qs(urlsplit(fields['Location']).query)['state']
                before = list(fixture.db.iterdump())
                fixture.get(case, 'resume', {}, '')
                self.assertEqual(list(fixture.db.iterdump()), before)
                with patch.object(fixture, 'json_request', side_effect=AssertionError('must not exchange')):
                    self.assertEqual(fixture.callback(case, dict(state=state, code=['code']), 'foreign')[0], 400)
                self.assertEqual(fixture.db.execute('SELECT authenticated FROM cases').fetchone()[0], 0)
                self.assertEqual(fixture.db.execute('SELECT consumed FROM login_flows').fetchone()[0], 0)


if __name__ == '__main__':
    unittest.main()

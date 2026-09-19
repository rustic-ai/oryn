"""Real code issuance/token HTTP rejection and redacted durable history sources."""
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import unittest
from http.cookies import SimpleCookie
from urllib.parse import parse_qs,urlsplit,urlencode
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('oauth_adversarial_test',ROOT/'test-harness/workflows/saas_fixture_v4.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class OAuthRejectionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.database=Path(self.temp.name)/'saas.sqlite3';self.fixture=module.Fixture(self.database)
        self.fixture.origin='http://127.0.0.1:41004'
        self.server=module.v2.transport.create_server(self.fixture.provider,self.fixture.mfa_clients)
        self.fixture.issuer=f'http://127.0.0.1:{self.server.server_port}'
        self.thread=threading.Thread(target=self.server.serve_forever);self.thread.start()
        self.case='a'*32;self.fixture.command(dict(command='create',case_id=self.case,scenario='normal'))
    def tearDown(self):
        self.server.shutdown();self.server.server_close();self.thread.join();self.fixture.db.close();self.fixture.provider.close();self.temp.cleanup()
    def flow(self,mode):
        _,_,headers=self.fixture.get(self.case,'sso',dict(probe=[mode]),'');headers=dict(headers)
        cookie=SimpleCookie(headers['Set-Cookie'])['saas_pending'].value
        handle=self.fixture.provider.begin(headers['Location'])
        redirect,_,status=self.fixture.provider.authorize(handle,'agent@example.test','fixture-only-password')
        self.assertEqual(status,302)
        return parse_qs(urlsplit(redirect['Location']).query),cookie
    def assert_rejected(self,mode,error="invalid_grant"):
        query,cookie=self.flow(mode);status,body,_=self.fixture.callback(self.case,query,cookie)
        self.assertEqual(status,400);self.assertIn(error,body)
        data=module.snapshot(self.database,self.case)
        self.assertEqual(data['cases'][0]['authenticated'],0);self.assertEqual(data['cases'][0]['logins'],0)
        self.assertNotIn(query['code'][0],json.dumps(data));self.assertEqual(data['requests'],[])
        return data
    def test_wrong_verifier_is_rejected_by_real_token_endpoint(self):
        data=self.assert_rejected('verifier');self.assertEqual(data['oauth']['tokens'],[])
    def test_wrong_redirect_is_rejected_by_real_token_endpoint(self):
        # Pinned oauthlib 3.3.1 raises MismatchingRedirectURIError
        # (invalid_request); rejection and zero issued tokens remain mandatory.
        data=self.assert_rejected('redirect',error='invalid_request');self.assertEqual(data['oauth']['tokens'],[])
    def test_expired_actual_grant_cannot_issue_token(self):
        data=self.assert_rejected('expiry');self.assertEqual(data['oauth']['codes'],[dict(consumed=0)])
    def test_replay_revokes_first_token_without_creating_application_session(self):
        data=self.assert_rejected('replay');self.assertEqual(data['oauth']['tokens'],[dict(revoked=1)])
        self.assertEqual([row['outcome'] for row in data['oauth']['events'] if row['action']=='token_exchange'],[200,400])
    def test_state_attack_requires_actual_second_callback_and_rejects_it(self):
        query,cookie=self.flow('state');status,body,_=self.fixture.callback(self.case,query,cookie)
        self.assertEqual(status,200);self.assertIn('Continue with altered state',body)
        status,body,_=self.fixture.callback(self.case,dict(query,state=['invalid-state']),cookie)
        self.assertEqual(status,400);self.assertIn('state_rejected',body)
        self.assertEqual(module.snapshot(self.database,self.case)['oauth']['tokens'],[])
    def test_foreign_cookie_cannot_enter_adversarial_exchange(self):
        query,_=self.flow('verifier');self.assertEqual(self.fixture.callback(self.case,query,'foreign')[0],400)
        self.assertEqual(module.snapshot(self.database,self.case)['oauth']['tokens'],[])

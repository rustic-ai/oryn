"""Controlled OAuthlib authorization-code/S256 provider for the SaaS corpus.

Callers own HTTP transport, trusted client registration, and the application-side
state/cookie binding. No static authorization codes or password-grant shortcut.
Only public synthetic accounts are accepted; MFA is an explicit test action.
"""
import hashlib
from importlib.metadata import version
import json
import re
import secrets
import sqlite3
import threading
import time
from types import SimpleNamespace
from urllib.parse import parse_qs, urlsplit

from oauthlib.oauth2 import RequestValidator, WebApplicationServer, OAuth2Error


def fingerprint(value):
    return hashlib.sha256(value.encode()).hexdigest()


class Validator(RequestValidator):
    def __init__(self, provider):
        self.provider = provider

    def client(self, client_id):
        return self.provider.db.execute("SELECT * FROM oauth_clients WHERE client_id=?", (client_id,)).fetchone()

    def validate_client_id(self, client_id, request, *args, **kwargs):
        if not self.client(client_id):
            return False
        request.client = SimpleNamespace(client_id=client_id)
        return True

    def client_authentication_required(self, request, *args, **kwargs):
        return False  # Public client; S256 is mandatory on every grant.

    def authenticate_client_id(self, client_id, request, *args, **kwargs):
        return self.validate_client_id(client_id, request)

    def validate_redirect_uri(self, client_id, redirect_uri, request, *args, **kwargs):
        row = self.client(client_id)
        return bool(row and row['redirect_uri'] == redirect_uri)

    def get_default_redirect_uri(self, client_id, request, *args, **kwargs):
        row = self.client(client_id)
        return row['redirect_uri'] if row else None

    def validate_response_type(self, client_id, response_type, client, request, *args, **kwargs):
        return response_type == 'code'

    def validate_grant_type(self, client_id, grant_type, client, request, *args, **kwargs):
        return grant_type == 'authorization_code'

    def get_default_scopes(self, client_id, request, *args, **kwargs):
        return ['records']

    def validate_scopes(self, client_id, scopes, client, request, *args, **kwargs):
        return scopes == ['records']

    def is_pkce_required(self, client_id, request):
        return True

    def save_authorization_code(self, client_id, code, request, *args, **kwargs):
        self.provider.db.execute('INSERT INTO oauth_codes VALUES(?,?,?,?,?,?,?,?,0)',
            (fingerprint(code['code']), client_id, request.user, request.redirect_uri,
             request.code_challenge, request.code_challenge_method, json.dumps(request.scopes),
             self.provider.clock() + 60))
        self.provider.audit(request.user, 'code_issued', 302)

    def code(self, value):
        return self.provider.db.execute('SELECT * FROM oauth_codes WHERE digest=?', (fingerprint(value),)).fetchone()

    def validate_code(self, client_id, code, client, request, *args, **kwargs):
        row = self.code(code)
        if not row or row['client_id'] != client_id:
            return False
        if row['consumed']:
            # A replay retires tokens issued by this grant, even if a caller kept
            # the original successful token response.
            self.provider.db.execute('UPDATE oauth_tokens SET revoked=1 WHERE code_digest=?', (row['digest'],))
            return False
        if row['expires'] <= self.provider.clock():
            return False
        request.user = row['case_id']
        request.scopes = json.loads(row['scopes'])
        request.code_challenge = row['challenge']
        request.code_challenge_method = row['method']
        return True

    def get_code_challenge(self, code, request):
        row = self.code(code)
        return row['challenge'] if row else None

    def get_code_challenge_method(self, code, request):
        row = self.code(code)
        return row['method'] if row else None

    def confirm_redirect_uri(self, client_id, code, redirect_uri, client, request, *args, **kwargs):
        row = self.code(code)
        return bool(row and row['client_id'] == client_id and row['redirect_uri'] == redirect_uri)

    def invalidate_authorization_code(self, client_id, code, request, *args, **kwargs):
        self.provider.db.execute('UPDATE oauth_codes SET consumed=1 WHERE digest=? AND client_id=?',
                                 (fingerprint(code), client_id))

    def save_bearer_token(self, token, request, *args, **kwargs):
        # This corpus intentionally has no refresh-token grant. Reauthentication
        # goes through authorization/MFA instead of a hidden password shortcut.
        token.pop('refresh_token', None)
        self.provider.db.execute('INSERT INTO oauth_tokens VALUES(?,?,?,?,?,0)',
            (fingerprint(token['access_token']), request.user, request.client_id,
             fingerprint(request.code), self.provider.clock() + token['expires_in']))


class Provider:
    def __init__(self, database, clock=time.time):
        if version('oauthlib') != '3.3.1':
            raise RuntimeError('The versioned SaaS corpus requires oauthlib==3.3.1')
        self.clock = clock
        self.lock = threading.RLock()
        self.db = sqlite3.connect(database, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript('''
            CREATE TABLE oauth_clients(client_id TEXT PRIMARY KEY, case_id TEXT UNIQUE, redirect_uri TEXT);
            CREATE TABLE oauth_pending(digest TEXT PRIMARY KEY, case_id TEXT, uri TEXT, expires REAL, mfa INTEGER);
            CREATE TABLE oauth_codes(digest TEXT PRIMARY KEY, client_id TEXT, case_id TEXT, redirect_uri TEXT,
              challenge TEXT, method TEXT, scopes TEXT, expires REAL, consumed INTEGER);
            CREATE TABLE oauth_tokens(digest TEXT PRIMARY KEY, case_id TEXT, client_id TEXT, code_digest TEXT,
              expires REAL, revoked INTEGER);
            CREATE TABLE oauth_events(case_id TEXT, action TEXT, outcome INTEGER);
        ''')
        self.validator = Validator(self)
        self.server = WebApplicationServer(self.validator, token_expires_in=120)

    def register(self, case_id, redirect_uri):
        parsed = urlsplit(redirect_uri)
        # The harness registers exact callbacks on its own two loopback servers.
        if (not re.fullmatch('[0-9a-f]{32}', case_id) or parsed.scheme != 'http'
                or parsed.hostname != '127.0.0.1' or not parsed.port or parsed.username
                or parsed.password or parsed.query or parsed.fragment
                or parsed.path != f'/saas/{case_id}/callback'):
            raise ValueError('invalid controlled callback')
        with self.lock, self.db:
            if self.db.execute('SELECT count(*) FROM oauth_clients').fetchone()[0] >= 128:
                raise ValueError('client capacity')
            self.db.execute('INSERT INTO oauth_clients VALUES(?,?,?)', (case_id, case_id, redirect_uri))
        return case_id

    def audit(self, case_id, action, outcome):
        self.db.execute('INSERT INTO oauth_events VALUES(?,?,?)', (case_id, action, outcome))

    def begin(self, uri, require_mfa=False):
        """Validate before showing credentials; never redirect an invalid client."""
        fields = parse_qs(urlsplit(uri).query, keep_blank_values=True, max_num_fields=16)
        client_id = fields.get('client_id', [''])[0]
        with self.lock, self.db:
            self.db.execute('DELETE FROM oauth_pending WHERE expires<=?', (self.clock(),))
            if (any(len(v) != 1 for v in fields.values())
                    or fields.get('code_challenge_method') != ['S256']
                    or not re.fullmatch(r'[A-Za-z0-9_-]{43}', fields.get('code_challenge', [''])[0])
                    or not re.fullmatch(r'[A-Za-z0-9_-]{32,128}', fields.get('state', [''])[0])):
                self.audit(client_id, 'authorization_rejected', 400)
                self.db.commit()
                raise ValueError('invalid authorization parameters')
            try:
                self.server.validate_authorization_request(uri)
            except OAuth2Error:
                self.audit(client_id, 'authorization_rejected', 400)
                self.db.commit()
                raise ValueError('authorization rejected') from None
            if self.db.execute('SELECT count(*) FROM oauth_pending').fetchone()[0] >= 256:
                raise ValueError('authorization capacity')
            handle = secrets.token_urlsafe(32)
            self.db.execute('INSERT INTO oauth_pending VALUES(?,?,?,?,?)',
                            (fingerprint(handle), client_id, uri, self.clock() + 120, int(require_mfa)))
            self.audit(client_id, 'authorization_started', 200)
            return handle

    def authorization_callback_origin(self, handle):
        """The credential document permits only its validated registered client.

        Derive CSP from server-owned registration after begin() validation, never
        from the request Host or an untrusted redirect_uri parameter.
        """
        with self.lock:
            row = self.db.execute(
                'SELECT c.redirect_uri FROM oauth_pending p JOIN oauth_clients c '
                'ON c.client_id=p.case_id WHERE p.digest=? AND p.expires>?',
                (fingerprint(handle), self.clock())).fetchone()
            if not row:
                raise ValueError('authorization expired or consumed')
            parsed = urlsplit(row['redirect_uri'])
            # register() admits only exact http://127.0.0.1:<port> callbacks.
            # Canonical fields exclude path, query, credentials and header text.
            return f'{parsed.scheme}://{parsed.hostname}:{parsed.port}'

    def authorize(self, handle, username, password, mfa_code=''):
        """Called only by the IdP credential form, bound to its HttpOnly cookie."""
        with self.lock, self.db:
            row = self.db.execute('SELECT * FROM oauth_pending WHERE digest=?', (fingerprint(handle),)).fetchone()
            if not row or row['expires'] <= self.clock():
                raise ValueError('authorization expired or consumed')
            valid = (secrets.compare_digest(username, 'agent@example.test')
                     and secrets.compare_digest(password, 'fixture-only-password'))
            if row['mfa']:
                valid = valid and secrets.compare_digest(mfa_code, '246810')
            if not valid:
                self.audit(row['case_id'], 'credentials_rejected', 401)
                self.db.commit()
                raise ValueError('credentials rejected')
            result = self.server.create_authorization_response(row['uri'], scopes=['records'],
                                                               credentials={'user': row['case_id']})
            self.db.execute('DELETE FROM oauth_pending WHERE digest=?', (row['digest'],))
            self.audit(row['case_id'], 'mfa_authorized' if row['mfa'] else 'authorized', result[2])
            return result

    def exchange(self, uri, body):
        fields = parse_qs(body, keep_blank_values=True, max_num_fields=16)
        client = fields.get('client_id', [''])[0]
        with self.lock, self.db:
            if (any(len(v) != 1 for v in fields.values())
                    or not re.fullmatch(r'[A-Za-z0-9._~-]{43,128}', fields.get('code_verifier', [''])[0])):
                result = ({'Content-Type': 'application/json', 'Cache-Control': 'no-store'},
                          json.dumps({'error': 'invalid_request'}), 400)
            else:
                result = self.server.create_token_response(uri, http_method='POST', body=body,
                    headers={'Content-Type': 'application/x-www-form-urlencoded'})
            self.audit(client, 'token_exchange', result[2])
            return result

    def identity(self, token, client_id):
        with self.lock:
            row = self.db.execute('SELECT * FROM oauth_tokens WHERE digest=? AND client_id=?',
                                  (fingerprint(token), client_id)).fetchone()
            return row['case_id'] if row and not row['revoked'] and row['expires'] > self.clock() else None

    def snapshot(self, case_id):
        with self.lock:
            return dict(events=[dict(row) for row in self.db.execute(
                'SELECT action,outcome FROM oauth_events WHERE case_id=? ORDER BY rowid', (case_id,))],
                codes=[dict(row) for row in self.db.execute(
                    'SELECT consumed FROM oauth_codes WHERE case_id=? ORDER BY rowid', (case_id,))],
                tokens=[dict(row) for row in self.db.execute(
                    'SELECT revoked FROM oauth_tokens WHERE case_id=? ORDER BY rowid', (case_id,))])

    def close(self):
        self.db.close()

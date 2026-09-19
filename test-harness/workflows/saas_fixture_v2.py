"""Versioned cross-origin OAuth login over the existing persisted SaaS backend."""
import argparse
import base64
from contextlib import closing
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import secrets
import sqlite3
import sys
import threading
import time
from http.cookies import CookieError, SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


legacy = load('saas_backend_v1', HERE / 'saas_fixture.py')
oauth = load('saas_oauth_provider_v2', HERE / 'oauth_provider_v2.py')
transport = load('saas_oauth_http_v2', HERE / 'oauth_http_v2.py')


def snapshot(database, case_id):
    result = legacy.snapshot(database, case_id)
    with closing(sqlite3.connect(f'file:{database}?mode=ro', uri=True)) as db:
        db.row_factory = sqlite3.Row
        result['concurrent_edits'] = [dict(row) for row in db.execute(
            'SELECT title,version FROM concurrent_edits WHERE case_id=? ORDER BY rowid', (case_id,))]
        result['login_events'] = [dict(row) for row in db.execute(
            'SELECT action,outcome FROM login_events WHERE case_id=? ORDER BY rowid', (case_id,))]
    with closing(sqlite3.connect(f'file:{database}.oauth?mode=ro', uri=True)) as db:
        db.row_factory = sqlite3.Row
        result['oauth'] = {key: [dict(row) for row in db.execute(query, (case_id,))] for key, query in (
            ('events', 'SELECT action,outcome FROM oauth_events WHERE case_id=? ORDER BY rowid'),
            ('codes', 'SELECT consumed FROM oauth_codes WHERE case_id=? ORDER BY rowid'),
            ('tokens', 'SELECT revoked FROM oauth_tokens WHERE case_id=? ORDER BY rowid'))}
    return result


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class Fixture(legacy.Fixture):
    def __init__(self, database, clock=time.time):
        super().__init__(database)
        self.clock = clock
        self.provider = oauth.Provider(str(database) + '.oauth', clock=clock)
        self.mfa_clients = set()
        self.concurrent_edits = set()
        self.origin = None
        self.issuer = None
        self.db.executescript('''
            CREATE TABLE concurrent_edits(case_id TEXT, title TEXT, version INTEGER);
            CREATE TABLE login_epochs(case_id TEXT PRIMARY KEY, epoch INTEGER);
            CREATE TABLE login_flows(state_digest TEXT PRIMARY KEY, cookie_digest TEXT, case_id TEXT,
              verifier TEXT, expires REAL, epoch INTEGER, consumed INTEGER);
            CREATE TABLE login_events(case_id TEXT, action TEXT, outcome INTEGER);
        ''')

    def command(self, message):
        if message['command'] == 'snapshot':
            with self.lock:
                return snapshot(self.database, message['case_id'])
        mapped = dict(message)
        if mapped.get('scenario') in ('mfa', 'concurrent_edit'):
            mapped['scenario'] = 'normal'
        with self.lock:
            result = super().command(mapped)
            if message['command'] == 'create':
                case = message['case_id']
                self.provider.register(case, f'{self.origin}/saas/{case}/callback')
                if message.get('scenario') == 'concurrent_edit':
                    self.concurrent_edits.add(case)
                if message.get('scenario') == 'mfa':
                    self.mfa_clients.add(case)
                with self.db:
                    self.db.execute('INSERT INTO login_epochs VALUES(?,0)', (case,))
            return result

    def event(self, case, action, status):
        self.db.execute('INSERT INTO login_events VALUES(?,?,?)', (case, action, status))

    def begin(self, case):
        with self.lock, self.db:
            if not self.db.execute('SELECT 1 FROM cases WHERE case_id=?', (case,)).fetchone():
                return 404, 'Unknown case', []
            self.db.execute('DELETE FROM login_flows WHERE expires<=?', (self.clock(),))
            if self.db.execute('SELECT count(*) FROM login_flows').fetchone()[0] >= 256:
                return 429, 'Too many login attempts', []
            self.db.execute('UPDATE login_epochs SET epoch=epoch+1 WHERE case_id=?', (case,))
            epoch = self.db.execute('SELECT epoch FROM login_epochs WHERE case_id=?', (case,)).fetchone()[0]
            state, cookie, verifier = (secrets.token_urlsafe(32) for _ in range(3))
            challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).decode().rstrip('=')
            self.db.execute('INSERT INTO login_flows VALUES(?,?,?,?,?,?,0)',
                (oauth.fingerprint(state), oauth.fingerprint(cookie), case, verifier, self.clock() + 120, epoch))
            query = urlencode(dict(response_type='code', client_id=case, scope='records', state=state,
                redirect_uri=f'{self.origin}/saas/{case}/callback', code_challenge=challenge, code_challenge_method='S256'))
            self.event(case, 'started', 303)
            return 303, '', [('Location', self.issuer + '/authorize?' + query),
                ('Set-Cookie', f'saas_pending={cookie}; HttpOnly; SameSite=Lax; Path=/saas/{case}/callback')]

    @staticmethod
    def json_request(request):
        opener = build_opener(ProxyHandler({}), NoRedirect())
        with opener.open(request, timeout=10) as response:
            body = response.read(16385)
            if response.status != 200 or len(body) > 16384 or response.headers.get_content_type() != 'application/json':
                raise ValueError('invalid identity response')
            result = json.loads(body)
            if not isinstance(result, dict):
                raise ValueError('invalid identity response')
            return result

    def callback(self, case, query, cookie):
        state = query.get('state', [''])[0]
        with self.lock, self.db:
            row = self.db.execute('SELECT * FROM login_flows WHERE state_digest=? AND case_id=?',
                                  (oauth.fingerprint(state), case)).fetchone()
            epoch = self.db.execute('SELECT epoch FROM login_epochs WHERE case_id=?', (case,)).fetchone()
            if (set(query) != {'state', 'code'} or any(len(v) != 1 for v in query.values())
                    or not query['code'][0] or not row or row['consumed'] or row['expires'] <= self.clock()
                    or not secrets.compare_digest(row['cookie_digest'], oauth.fingerprint(cookie))
                    or not epoch or epoch[0] != row['epoch']):
                self.event(case, 'callback_rejected', 400)
                return 400, self.page('Login rejected', 'Start a new sign-in attempt.'), []
            self.db.execute('UPDATE login_flows SET consumed=1,verifier=? WHERE state_digest=?', ('', row['state_digest']))
        # No application DB lock spans network I/O. A new login or logout can
        # retire this attempt while the identity server is responding.
        try:
            body = urlencode(dict(grant_type='authorization_code', client_id=case, code=query['code'][0],
                redirect_uri=f'{self.origin}/saas/{case}/callback', code_verifier=row['verifier'])).encode()
            token = self.json_request(Request(self.issuer + '/token', data=body,
                headers={'Content-Type': 'application/x-www-form-urlencoded'}))
            if (not isinstance(token.get('token_type'), str) or token['token_type'].lower() != 'bearer'
                    or not isinstance(token.get('access_token'), str) or not token['access_token']):
                raise ValueError('invalid token response')
            identity = self.json_request(Request(self.issuer + '/identity?' + urlencode(dict(client_id=case)),
                headers={'Authorization': 'Bearer ' + token['access_token']}))
            if identity.get('case_id') != case:
                raise ValueError('foreign identity')
        except (HTTPError, URLError, OSError, ValueError):
            with self.lock, self.db:
                self.event(case, 'exchange_rejected', 401)
            return 401, self.page('Login rejected', 'Start a new sign-in attempt.'), []
        with self.lock, self.db:
            current = self.db.execute('SELECT epoch FROM login_epochs WHERE case_id=?', (case,)).fetchone()
            if not current or current[0] != row['epoch'] or row['expires'] <= self.clock():
                self.event(case, 'superseded', 409)
                return 409, 'Login superseded', []
            session, csrf = secrets.token_hex(24), secrets.token_hex(24)
            self.db.execute('UPDATE cases SET session=?,csrf=?,authenticated=1,logins=logins+1 WHERE case_id=?', (session, csrf, case))
            self.event(case, 'authenticated', 303)
            self.db.execute('INSERT INTO requests VALUES(?,?,?,?,?)', (case, 'oauth_login', 303, 0, hashlib.sha256(b'').hexdigest()))
            return 303, '', [('Location', f'/saas/{case}/records'),
                ('Set-Cookie', f'saas_session={session}; HttpOnly; SameSite=Strict; Path=/saas/{case}/'),
                ('Set-Cookie', f'saas_pending=; HttpOnly; SameSite=Lax; Path=/saas/{case}/callback; Max-Age=0')]

    @staticmethod
    def hydrate(response):
        status, body, headers = response
        if status != 200:
            return response
        offset = body.find('<h1>')
        if offset < 0:
            return response
        body = (body[:offset] + '<main id="saas-app" data-state="loading">' + body[offset:]
                + '</main><script type="module" src="./saas-client.mjs"></script>')
        return status, body, headers

    def get(self, case, path, query, cookie):
        if path == 'saas-client.mjs':
            with self.lock:
                if not self.db.execute('SELECT 1 FROM cases WHERE case_id=?', (case,)).fetchone():
                    return 404, 'Unknown case', []
            return 200, (HERE / 'v2/saas-client.mjs').read_text(), [('Content-Type', 'text/javascript; charset=utf-8')]
        if path == 'sso':
            return self.begin(case)
        if path == 'login':
            response = super().get(case, path, query, cookie)
            if response[0] != 200:
                return response
            return self.hydrate((200, self.page('Sign in', f'<a href="/saas/{case}/sso">Sign in with test identity</a>'), []))
        with self.lock:
            response = super().get(case, path, query, cookie)
            if path == 'records' and response[0] == 200 and case in self.concurrent_edits:
                # Deliver the original edit form, while another database writer
                # commits a change before the user's next save. This scenario is
                # selected only on the trusted fixture control channel.
                with closing(sqlite3.connect(self.database)) as writer, writer:
                    changed = writer.execute("UPDATE records SET title='Concurrent',version=version+1 WHERE case_id=? AND version=1", (case,)).rowcount
                    if changed:
                        writer.execute("INSERT INTO concurrent_edits VALUES(?,'Concurrent',2)", (case,))
                        self.concurrent_edits.remove(case)
            return self.hydrate(response)

    def post(self, case, action, body, cookie):
        if action == 'login':
            return 405, 'Use the identity provider', []
        with self.lock:
            response = super().post(case, action, body, cookie)
            if action == 'logout' and response[0] == 303:
                with self.db:
                    self.db.execute('UPDATE login_epochs SET epoch=epoch+1 WHERE case_id=?', (case,))
            return response


def serve(database, fixture_type=Fixture, extra_routes=()):
    fixture = fixture_type(database)
    idp = transport.create_server(fixture.provider, fixture.mfa_clients)
    fixture.issuer = f'http://127.0.0.1:{idp.server_port}'

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def reply(self, response):
            status, body, headers = response
            data = body.encode('utf-8')
            self.send_response(status)
            if not any(key.lower() == 'content-type' for key, _ in headers):
                self.send_header('Content-Type', 'text/html; charset=utf-8')
            for key, value in [('Content-Length', str(len(data))),
                               ('Cache-Control', 'no-store'), ('Referrer-Policy', 'no-referrer'), *headers]:
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(data)

        def route(self):
            if len(self.path) > 16384:
                raise ValueError('request too large')
            parsed = urlsplit(self.path)
            match = re.fullmatch(r'/saas/([0-9a-f]{32})/(' + '|'.join(re.escape(route) for route in ('login','sso','callback','records','create','update','logout','saas-client.mjs',*extra_routes)) + r')', parsed.path)
            cookies = SimpleCookie()
            cookies.load(self.headers.get('Cookie', ''))
            name = 'saas_pending' if match and match[2] == 'callback' else 'saas_session'
            cookie = cookies.get(name)
            return match, parse_qs(parsed.query, keep_blank_values=True, max_num_fields=16), cookie.value if cookie else ''

        def do_GET(self):
            try:
                match, query, cookie = self.route()
                if not match:
                    return self.reply((404, 'Not found', []))
                response = fixture.callback(match[1], query, cookie) if match[2] == 'callback' else fixture.get(*match.groups(), query, cookie)
                self.reply(response)
            except (ValueError, CookieError):
                self.reply((400, 'Invalid request', []))

        def do_POST(self):
            try:
                match, _, cookie = self.route()
                lengths = self.headers.get_all('Content-Length', [])
                if (not match or len(lengths) != 1 or self.headers.get('Transfer-Encoding')
                        or self.headers.get('Content-Type', '').split(';')[0] != 'application/x-www-form-urlencoded'):
                    raise ValueError('invalid request')
                length = int(lengths[0])
                if not 0 <= length <= 16384:
                    raise ValueError('request too large')
                body = self.rfile.read(length)
                if len(body) != length:
                    raise ValueError('truncated request')
                self.reply(fixture.post(match[1], match[2], body, cookie))
            except (ValueError, CookieError):
                self.reply((400, 'Invalid request', []))

    app = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    app.daemon_threads = False
    fixture.origin = f'http://127.0.0.1:{app.server_port}'
    servers = [app, idp]
    threads = [threading.Thread(target=server.serve_forever, daemon=True) for server in servers]
    for thread in threads:
        thread.start()
    print(json.dumps({'event': 'ready', 'port': app.server_port, 'issuer_port': idp.server_port}), flush=True)
    sealed = False

    def stop():
        # Stop application admission first; its in-flight callbacks can still
        # finish token/identity calls before stopping the identity server.
        for server, thread in zip(servers, threads):
            server.shutdown()
            server.server_close()
            thread.join()

    try:
        while line := sys.stdin.readline(8193):
            try:
                if len(line) > 8192:
                    raise ValueError('control frame limit')
                message = json.loads(line)
                if not isinstance(message, dict):
                    raise ValueError('invalid command')
                if message.get('command') == 'shutdown':
                    break
                if message.get('command') == 'seal':
                    if not sealed:
                        stop()
                        sealed = True
                    result = {'sealed': True}
                else:
                    if sealed and message.get('command') != 'snapshot':
                        raise ValueError('fixture sealed')
                    result = fixture.command(message)
                print(json.dumps({'ok': True, 'result': result}), flush=True)
            except (ValueError, KeyError, TypeError, sqlite3.Error):
                print(json.dumps({'ok': False, 'error': 'Invalid fixture command'}), flush=True)
    finally:
        if not sealed:
            stop()
        fixture.db.close()
        fixture.provider.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', required=True)
    serve(parser.parse_args().database)

"""Loopback HTTP transport for the controlled OAuthlib provider.

The application uses another port, so authorization is a cross-origin browser
navigation and token exchange is an actual HTTP request. Request logging is
suppressed because authorization URLs and credential bodies contain secrets.
"""
import html
from http.cookies import CookieError, SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import secrets
from urllib.parse import parse_qs, urlsplit


def create_server(provider, mfa_clients):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def reply(self, status, body, headers=()):
            data = body.encode('utf-8')
            self.send_response(status)
            for name, value in [('Content-Length', str(len(data))), ('Cache-Control', 'no-store'),
                                ('Referrer-Policy', 'no-referrer'),
                                ('Content-Security-Policy', "default-src 'none'; form-action 'self'; frame-ancestors 'none'"), *headers]:
                self.send_header(name, value)
            self.end_headers()
            self.wfile.write(data)

        def uri(self):
            # Do not trust Host for issuer or redirect identity.
            return f'http://127.0.0.1:{self.server.server_port}{self.path}'

        def do_GET(self):
            try:
                if len(self.path) > 16384:
                    return self.reply(414, 'Request too large')
                parsed = urlsplit(self.path)
                if parsed.path == '/authorize':
                    fields = parse_qs(parsed.query, keep_blank_values=True, max_num_fields=16)
                    client = fields.get('client_id', [''])[0]
                    require_mfa = client in mfa_clients
                    handle = provider.begin(self.uri(), require_mfa=require_mfa)
                    mfa = ('<p>Authorized synthetic MFA test. Enter test code 246810 to continue.</p>'
                           '<label>Test MFA code<input name="mfa_code" required></label>') if require_mfa else ''
                    body = ('<!doctype html><meta charset=utf-8><title>Controlled identity provider</title>'
                            '<h1>Sign in with test identity</h1><p>Use only the public synthetic fixture account.</p>'
                            '<form method="post" action="/authorize">'
                            f'<input type="hidden" name="csrf" value="{html.escape(handle, quote=True)}">'
                            '<label>Username<input name="username" required></label>'
                            '<label>Password<input type="password" name="password" required></label>'
                            f'{mfa}<button>Authorize test application</button></form>')
                    return self.reply(200, body, [('Content-Type', 'text/html; charset=utf-8'),
                        ('Set-Cookie', f'idp_pending={handle}; HttpOnly; SameSite=Lax; Path=/authorize')])
                if parsed.path == '/identity':
                    fields = parse_qs(parsed.query, max_num_fields=2)
                    authorization = self.headers.get('Authorization', '')
                    client = fields.get('client_id', [])
                    case_id = provider.identity(authorization[7:], client[0]) if authorization.startswith('Bearer ') and len(client) == 1 else None
                    return self.reply(200 if case_id else 401, json.dumps({'case_id': case_id}),
                                      [('Content-Type', 'application/json')])
                return self.reply(404, 'Not found')
            except (ValueError, CookieError):
                return self.reply(400, 'Authorization rejected')

        def do_POST(self):
            try:
                lengths = self.headers.get_all('Content-Length', [])
                if (len(self.path) > 16384 or len(lengths) != 1 or self.headers.get('Transfer-Encoding')
                        or self.headers.get('Content-Type', '').split(';')[0] != 'application/x-www-form-urlencoded'):
                    raise ValueError('invalid request')
                length = int(lengths[0])
                if not 0 <= length <= 16384:
                    raise ValueError('request too large')
                data = self.rfile.read(length)
                if len(data) != length:
                    raise ValueError('truncated body')
                body = data.decode('utf-8')
                parsed = urlsplit(self.path)
                if parsed.path == '/token':
                    headers, body, status = provider.exchange(self.uri(), body)
                    return self.reply(status, body or '', list(headers.items()))
                if parsed.path != '/authorize':
                    return self.reply(404, 'Not found')
                fields = parse_qs(body, keep_blank_values=True, max_num_fields=8)
                if (any(len(values) != 1 for values in fields.values())
                        or set(fields) not in ({'csrf', 'username', 'password'}, {'csrf', 'username', 'password', 'mfa_code'})):
                    raise ValueError('invalid credential form')
                cookies = SimpleCookie()
                cookies.load(self.headers.get('Cookie', ''))
                handle = cookies.get('idp_pending')
                if not handle or not secrets.compare_digest(handle.value, fields['csrf'][0]):
                    return self.reply(403, 'Invalid authorization form')
                headers, body, status = provider.authorize(handle.value, fields['username'][0],
                    fields['password'][0], fields.get('mfa_code', [''])[0])
                return self.reply(status, body or '', [*headers.items(),
                    ('Set-Cookie', 'idp_pending=; HttpOnly; SameSite=Lax; Path=/authorize; Max-Age=0')])
            except (ValueError, UnicodeError, CookieError):
                return self.reply(400, 'Authorization rejected')

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.daemon_threads = False
    return server

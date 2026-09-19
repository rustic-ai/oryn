"""Hydrated shop with a distinct-origin, controlled payment authorization frame.

The existing SQLite cart/quote/order/inventory backend remains authoritative.
Payment authorizes only a fixture quote; it never contacts a real payment service.
"""
import argparse
from contextlib import closing
import html
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import re
import secrets
import sqlite3
import threading
from urllib.parse import parse_qs

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('shopping_backend_v1', HERE/'shopping_fixture.py')
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


def snapshot(database, case_id):
    result = legacy.snapshot(database, case_id)
    with closing(sqlite3.connect(f'file:{database}?mode=ro', uri=True)) as db:
        db.row_factory = sqlite3.Row
        result['payments'] = [dict(row) for row in db.execute(
            'SELECT case_id,quote,receipt,authorized FROM payments WHERE case_id=? ORDER BY rowid', (case_id,))]
        result['payment_events'] = [dict(row) for row in db.execute(
            'SELECT case_id,quote,action,outcome FROM payment_events WHERE case_id=? ORDER BY rowid', (case_id,))]
    return result


class Fixture(legacy.Fixture):
    def __init__(self, database):
        super().__init__(database)
        self.db.executescript('''CREATE TABLE payments(case_id TEXT, quote TEXT PRIMARY KEY,
          token TEXT UNIQUE, receipt TEXT UNIQUE, authorized INTEGER DEFAULT 0);
          CREATE TABLE payment_events(case_id TEXT,quote TEXT,action TEXT,outcome INTEGER);''')
        self.payment_origin = None
        self.shop_origin = None

    def command(self, message):
        result = super().command(message)
        if message['command'] == 'snapshot':
            return snapshot(self.database, message['case_id'])
        return result

    def page(self, case_id, title, body):
        base = f'/shop/{case_id}'
        return ('<!doctype html><meta charset=utf-8><title>Controlled hydrated shopping</title>'
                '<style>body{font:18px sans-serif}label,input,select,button,a{display:block;margin:8px}</style>'
                f'<main id="shop-app" data-base="{base}" data-state="hydrating">'
                f'<h1>{html.escape(title)}</h1><p>Test inventory. No real purchases.</p>{body}'
                f'<a href="{base}/catalog">Browse catalog</a><a href="{base}/cart">View cart</a></main>'
                f'<script type="module" src="{base}/shopping-client.mjs"></script>')

    def get(self, case_id, path, query, cookie):
        with self.lock:
            if path == 'shopping-client.mjs':
                row = self.db.execute('SELECT session FROM cases WHERE case_id=?', (case_id,)).fetchone()
                if row is None or row['session'] != cookie:
                    return 401, 'Session required', []
                return 200, (HERE/'v2/shopping-client.mjs').read_text(), [('Content-Type', 'text/javascript; charset=utf-8')]
            status, body, headers = super().get(case_id, path, query, cookie)
            if status == 200 and path == 'review':
                quote = self.db.execute('SELECT quote FROM quotes WHERE case_id=? ORDER BY rowid DESC LIMIT 1', (case_id,)).fetchone()['quote']
                token = secrets.token_hex(32)
                with self.db:
                    self.db.execute('INSERT INTO payments(case_id,quote,token) VALUES(?,?,?)', (case_id, quote, token))
                frame = (f'<iframe id="payment-frame" title="Controlled payment" src="{self.payment_origin}/payment/{token}"></iframe>'
                         f'<p id="payment-status" data-origin="{self.payment_origin}" data-quote="{quote}" data-state="waiting">Authorize controlled payment</p>')
                body = body.replace('</main>', frame+'</main>')
            return status, body, headers

    def payment(self, token, authorize=False, origin=None):
        with self.lock, self.db:
            row = self.db.execute('SELECT * FROM payments WHERE token=?', (token,)).fetchone()
            if row is None:
                return 404, 'Unknown payment'
            if authorize:
                if origin != self.payment_origin:
                    self.db.execute('INSERT INTO payment_events VALUES(?,?,?,?)', (row['case_id'],row['quote'],'origin_denied',403))
                    return 403, 'Payment origin required'
                if row['authorized']:
                    self.db.execute('INSERT INTO payment_events VALUES(?,?,?,?)', (row['case_id'],row['quote'],'replay_denied',409))
                    return 409, 'Payment already authorized'
                receipt = secrets.token_hex(16)
                self.db.execute('UPDATE payments SET authorized=1,receipt=? WHERE token=?', (receipt,token))
                self.db.execute('INSERT INTO payment_events VALUES(?,?,?,?)', (row['case_id'],row['quote'],'authorized',200))
                message = json.dumps(dict(type='controlled-payment', quote=row['quote'], receipt=receipt))
                return 200, ('<!doctype html><p id="payment-authorized">Controlled payment authorized</p>'
                             f'<script>parent.postMessage({message},{json.dumps(self.shop_origin)});</script>')
            self.db.execute('INSERT INTO payment_events VALUES(?,?,?,?)', (row['case_id'],row['quote'],'frame_view',200))
            return 200, ('<!doctype html><h1>Controlled payment</h1><p>No real charge.</p>'
                         f'<form method="post" action="/payment/{token}"><button>Authorize test payment</button></form>')

    def post(self, case_id, action, body, cookie):
        with self.lock:
            if action == 'checkout':
                try:
                    fields = parse_qs(body.decode(), keep_blank_values=True)
                    quote = fields.get('quote', [''])[0]
                except (ValueError, UnicodeError):
                    quote = ''
                row = self.db.execute('SELECT authorized FROM payments WHERE case_id=? AND quote=?', (case_id,quote)).fetchone()
                if row is None or not row['authorized']:
                    with self.db:
                        self.db.execute('INSERT INTO payment_events VALUES(?,?,?,?)', (case_id,quote,'checkout_denied',409))
                    return 409, self.conflict(case_id, 'Controlled payment required'), []
            return super().post(case_id, action, body, cookie)


def configure_server(fixture, shop_server):
    fixture.shop_origin = f'http://127.0.0.1:{shop_server.server_port}'
    class PaymentHandler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass
        def setup(self):
            super().setup()
            self.connection.settimeout(10)
        def handle_payment(self, authorize):
            if len(self.headers.get_all('Origin', [])) > 1:
                self.send_error(400)
                return
            match = re.fullmatch(r'/payment/([0-9a-f]{64})', self.path)
            # Only the tokenized frame endpoint exists; no cookie or oracle APIs.
            response = fixture.payment(match[1], authorize, self.headers.get('Origin')) if match else (404, 'Not found')
            status, body = response
            data = body.encode()
            self.send_response(status)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('Referrer-Policy', 'no-referrer')
            self.end_headers()
            self.wfile.write(data)
        def do_GET(self):
            self.handle_payment(False)
        def do_POST(self):
            if (self.headers.get('Transfer-Encoding') or len(self.headers.get_all('Content-Length', [])) > 1
                    or self.headers.get('Content-Length', '0') != '0'):
                self.send_error(400)
                return
            self.handle_payment(True)
    server = ThreadingHTTPServer(('127.0.0.1',0), PaymentHandler)
    server.daemon_threads = False
    fixture.payment_origin = f'http://127.0.0.1:{server.server_port}'
    thread = threading.Thread(target=server.serve_forever)
    thread.start()
    def close():
        server.shutdown()
        server.server_close()
        thread.join(timeout=10)
        if thread.is_alive():
            raise ValueError('payment fixture did not retire')
    return close


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', required=True)
    legacy.serve(parser.parse_args().database, fixture_type=Fixture,
                 extra_paths=('shopping-client.mjs',), configure_server=configure_server)

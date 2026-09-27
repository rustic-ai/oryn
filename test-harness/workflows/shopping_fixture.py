"""Controlled shopping only: SQLite persistence, loopback HTTP, stdin-only oracle.

No payment service or real inventory is contacted. Price/stock transitions happen
after the first quote is rendered, modeling a stale checkout deterministically.
"""

import argparse
from contextlib import closing
import hashlib
import html
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
import secrets
import sqlite3
import sys
import threading
from urllib.parse import parse_qs, urlsplit


def snapshot(database, case_id):
    """Read persisted rows, also usable after the HTTP process exits."""
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        result = {}
        for table in ("cases", "carts", "quotes", "orders", "requests", "views"):
            rows = db.execute(f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (case_id,))
            result[table] = [dict(row) for row in rows]
        # Session tokens are not needed in evidence.
        for case in result["cases"]:
            case.pop("session", None)
        return result


class Fixture:
    def __init__(self, database):
        self.database = str(database)
        self.lock = threading.RLock()
        self.db = sqlite3.connect(database, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            PRAGMA journal_mode=DELETE;
            CREATE TABLE cases(case_id TEXT PRIMARY KEY, scenario TEXT, session TEXT,
              unit_price INTEGER, blue_stock INTEGER, green_stock INTEGER, quote_count INTEGER);
            CREATE TABLE carts(case_id TEXT PRIMARY KEY, variant TEXT, quantity INTEGER);
            CREATE TABLE quotes(quote TEXT PRIMARY KEY, case_id TEXT, variant TEXT,
              quantity INTEGER, unit_price INTEGER, total INTEGER);
            CREATE TABLE orders(case_id TEXT PRIMARY KEY, order_id TEXT UNIQUE, quote TEXT UNIQUE,
              variant TEXT, quantity INTEGER, unit_price INTEGER, total INTEGER, name TEXT, email TEXT);
            CREATE TABLE requests(case_id TEXT, action TEXT, outcome INTEGER, body_sha256 TEXT,
              fields TEXT, cookie_valid INTEGER);
            CREATE TABLE views(case_id TEXT, page TEXT, cookie_valid INTEGER);
        """)

    def command(self, message):
        case_id = message.get("case_id", "")
        if not re.fullmatch(r"[0-9a-f]{32}", case_id):
            raise ValueError("invalid case id")
        with self.lock, self.db:
            if message["command"] == "snapshot":
                return snapshot(self.database, case_id)
            if message["command"] != "create":
                raise ValueError("unknown oracle command")
            scenario = message.get("scenario", "normal")
            if scenario not in ("normal", "price", "stock"):
                raise ValueError("invalid scenario")
            if self.db.execute("SELECT count(*) FROM cases").fetchone()[0] >= 128:
                raise ValueError("case capacity")
            self.db.execute(
                "INSERT INTO cases VALUES(?,?,?,?,?,?,0)",
                (case_id, scenario, secrets.token_hex(24), 2500, 10, 10),
            )
            return {"created": case_id}

    def page(self, case_id, title, body):
        base = f"/shop/{case_id}"
        return (
            "<!doctype html><meta charset=utf-8><title>Controlled shopping</title>"
            "<style>body{font:18px sans-serif;max-width:760px;margin:24px}"
            "label,input,select,button,a{margin:8px;display:block}</style>"
            f"<h1>{html.escape(title)}</h1><p>Test inventory. No real purchases.</p>{body}"
            f'<a href="{base}/catalog">Browse catalog</a><a href="{base}/cart">View cart</a>'
        )

    def conflict(self, case_id, reason):
        base = f"/shop/{case_id}"
        return self.page(
            case_id,
            "Checkout needs review",
            f'<p id="conflict">{reason}</p>'
            f'<a href="{base}/review">Refresh review</a>'
            f'<a href="{base}/cart">Edit cart</a>',
        )

    def get(self, case_id, path, query, cookie):
        with self.lock, self.db:
            row = self.db.execute("SELECT * FROM cases WHERE case_id=?", (case_id,)).fetchone()
            if row is None:
                return 404, "Unknown case", []
            base = f"/shop/{case_id}"
            if path == "start":
                return (
                    303,
                    "Start",
                    [
                        ("Location", base + "/catalog"),
                        (
                            "Set-Cookie",
                            f"shop_session={row['session']}; Path={base}/; HttpOnly; SameSite=Lax",
                        ),
                    ],
                )
            valid = cookie == row["session"]
            self.db.execute("INSERT INTO views VALUES(?,?,?)", (case_id, path, int(valid)))
            if not valid:
                return 401, "Session required", []
            if path == "catalog":
                category = query.get("category", ["all"])[0]
                search = query.get("search", [""])[0].lower()
                products = [
                    ("Field kit", "field", "outdoor", 2500),
                    ("Desk set", "desk", "office", 4000),
                ]
                cards = "".join(
                    f"<article><h2>{name}</h2><p>Unit price {price} cents</p>"
                    f'<a href="{base}/product/{slug}">Inspect {name.lower()}</a></article>'
                    for name, slug, group, price in products
                    if category in ("all", group) and search in name.lower()
                )
                body = (
                    f'<form method=get action="{base}/catalog"><label for=search>Search catalog</label>'
                    "<input id=search name=search><label for=category>Category</label>"
                    "<select id=category name=category><option value=all>All</option>"
                    "<option value=outdoor>Outdoor</option><option value=office>Office</option></select>"
                    "<button>Filter catalog</button></form>" + cards
                )
                return 200, self.page(case_id, "Catalog", body), []
            if path == "product/desk":
                return (
                    200,
                    self.page(case_id, "Desk set", "<p id=desk-spec>Office set, 4000 cents</p>"),
                    [],
                )
            if path == "product/field":
                body = (
                    f"<p id=field-spec>Outdoor kit, {row['unit_price']} cents per unit</p>"
                    f'<form method=post action="{base}/cart"><label for=variant>Variant</label>'
                    "<select id=variant name=variant><option value=blue>Blue</option><option value=green>Green</option></select>"
                    "<label for=quantity>Quantity</label><input id=quantity name=quantity type=number min=1 max=5 value=1 required>"
                    "<button>Add to cart</button></form>"
                )
                return 200, self.page(case_id, "Field kit", body), []
            cart = self.db.execute("SELECT * FROM carts WHERE case_id=?", (case_id,)).fetchone()
            if path == "cart":
                if cart is None:
                    return 200, self.page(case_id, "Cart", "<p id=empty-cart>Cart is empty</p>"), []
                options = "".join(
                    f"<option value={v}{' selected' if cart['variant'] == v else ''}>{v.title()}</option>"
                    for v in ("blue", "green")
                )
                body = (
                    f"<p id=cart-state>Field kit {cart['variant']}, quantity {cart['quantity']}</p>"
                    f'<form method=post action="{base}/cart"><label for=variant>Cart variant</label>'
                    f"<select id=variant name=variant>{options}</select><label for=quantity>Cart quantity</label>"
                    f"<input id=quantity name=quantity type=number min=1 max=5 value={cart['quantity']} required>"
                    "<button>Update cart</button></form>"
                    f'<a href="{base}/cart">Reload cart</a><a href="{base}/review">Review checkout</a>'
                )
                return 200, self.page(case_id, "Cart", body), []
            if path == "review" and cart is not None:
                quote = secrets.token_hex(16)
                total = row["unit_price"] * cart["quantity"]
                self.db.execute(
                    "INSERT INTO quotes VALUES(?,?,?,?,?,?)",
                    (quote, case_id, cart["variant"], cart["quantity"], row["unit_price"], total),
                )
                if row["quote_count"] == 0:
                    if row["scenario"] == "price":
                        self.db.execute(
                            "UPDATE cases SET unit_price=3000 WHERE case_id=?", (case_id,)
                        )
                    elif row["scenario"] == "stock":
                        self.db.execute("UPDATE cases SET blue_stock=0 WHERE case_id=?", (case_id,))
                self.db.execute(
                    "UPDATE cases SET quote_count=quote_count+1 WHERE case_id=?", (case_id,)
                )
                body = (
                    f"<p id=quote-total>Total {total} cents, {cart['quantity']} {cart['variant']} field kits</p>"
                    f'<form method=post action="{base}/checkout"><input type=hidden name=quote value={quote}>'
                    "<label for=name>Recipient</label><input id=name name=name required>"
                    "<label for=email>Contact email</label><input id=email name=email type=email required>"
                    "<button>Place test order</button></form>"
                )
                return 200, self.page(case_id, "Review checkout", body), []
            if path == "order":
                order = self.db.execute(
                    "SELECT * FROM orders WHERE case_id=?", (case_id,)
                ).fetchone()
                if order is not None:
                    body = (
                        f"<p id=order-total>Order total {order['total']} cents</p>"
                        f'<a href="{base}/order">Reload order</a>'
                        f'<form method=post action="{base}/checkout">'
                        f"<input type=hidden name=quote value={order['quote']}>"
                        f'<input type=hidden name=name value="{html.escape(order["name"], quote=True)}">'
                        f'<input type=hidden name=email value="{html.escape(order["email"], quote=True)}">'
                        "<button>Repeat order request</button></form>"
                    )
                    return 200, self.page(case_id, "Test order confirmed", body), []
            return 404, "Not found", []

    def post(self, case_id, action, body, cookie):
        with self.lock, self.db:
            case = self.db.execute("SELECT * FROM cases WHERE case_id=?", (case_id,)).fetchone()
            if case is None:
                return 404, "Unknown case", []
            valid_cookie = cookie == case["session"]
            fields = {}
            outcome, location, reason = 400, None, "Invalid request"
            try:
                fields = parse_qs(
                    body.decode("utf-8"),
                    keep_blank_values=True,
                    strict_parsing=True,
                    max_num_fields=8,
                )
                if not valid_cookie:
                    outcome, reason = 401, "Session required"
                elif action == "cart" and set(fields) == {"variant", "quantity"}:
                    variant, quantity = fields["variant"], fields["quantity"]
                    if (
                        variant in (["blue"], ["green"])
                        and len(quantity) == 1
                        and quantity[0] in ("1", "2", "3", "4", "5")
                    ):
                        self.db.execute(
                            "INSERT OR REPLACE INTO carts VALUES(?,?,?)",
                            (case_id, variant[0], int(quantity[0])),
                        )
                        outcome, location = 303, "cart"
                elif (
                    action == "checkout"
                    and set(fields) == {"quote", "name", "email"}
                    and all(len(v) == 1 for v in fields.values())
                ):
                    quote = self.db.execute(
                        "SELECT * FROM quotes WHERE quote=? AND case_id=?",
                        (fields["quote"][0], case_id),
                    ).fetchone()
                    order = self.db.execute(
                        "SELECT * FROM orders WHERE case_id=?", (case_id,)
                    ).fetchone()
                    cart = self.db.execute(
                        "SELECT * FROM carts WHERE case_id=?", (case_id,)
                    ).fetchone()
                    if not fields["name"][0] or not re.fullmatch(
                        r"[^\s@]+@[^\s@]+\.[^\s@]+", fields["email"][0]
                    ):
                        outcome, reason = 422, "Recipient details invalid"
                    elif order is not None:
                        outcome, location = (
                            (303, "order") if order["quote"] == fields["quote"][0] else (409, None)
                        )
                    elif quote is None or cart is None:
                        outcome, reason = 409, "Quote missing"
                    elif (quote["variant"], quote["quantity"]) != (
                        cart["variant"],
                        cart["quantity"],
                    ):
                        outcome, reason = 409, "Cart changed"
                    elif quote["unit_price"] != case["unit_price"]:
                        outcome, reason = 409, "Price changed"
                    elif case[quote["variant"] + "_stock"] < quote["quantity"]:
                        outcome, reason = 409, "Variant unavailable"
                    else:
                        self.db.execute(
                            "INSERT INTO orders VALUES(?,?,?,?,?,?,?,?,?)",
                            (
                                case_id,
                                secrets.token_hex(16),
                                quote["quote"],
                                quote["variant"],
                                quote["quantity"],
                                quote["unit_price"],
                                quote["total"],
                                fields["name"][0],
                                fields["email"][0],
                            ),
                        )
                        self.db.execute(
                            f"UPDATE cases SET {quote['variant']}_stock={quote['variant']}_stock-? WHERE case_id=?",
                            (quote["quantity"], case_id),
                        )
                        outcome, location = 303, "order"
            except (UnicodeError, ValueError):
                pass
            self.db.execute(
                "INSERT INTO requests VALUES(?,?,?,?,?,?)",
                (
                    case_id,
                    action,
                    outcome,
                    hashlib.sha256(body).hexdigest(),
                    json.dumps(fields, sort_keys=True),
                    int(valid_cookie),
                ),
            )
            if location:
                return outcome, "Redirect", [("Location", f"/shop/{case_id}/{location}")]
            return outcome, self.conflict(case_id, reason), []


def serve(database, fixture_type=Fixture, extra_paths=(), configure_server=None):
    fixture = fixture_type(database)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def reply(self, response):
            status, body, headers = response
            body = body.encode("utf-8")
            self.send_response(status)
            for key, value in [
                *([] if any(key.lower() == "content-type" for key, _ in headers)
                  else [("Content-Type", "text/html; charset=utf-8")]),
                ("Content-Length", str(len(body))),
                ("Cache-Control", "no-store"),
                *headers,
            ]:
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(body)

        def route(self):
            parsed = urlsplit(self.path)
            match = re.fullmatch(
                r"/shop/([0-9a-f]{32})/(" + "|".join(re.escape(route) for route in
                    ("start", "catalog", "product/field", "product/desk", "cart", "review", "checkout", "order", *extra_paths)) + r")",
                parsed.path,
            )
            cookies = SimpleCookie()
            cookies.load(self.headers.get("Cookie", ""))
            token = cookies.get("shop_session")
            return match, parse_qs(parsed.query), token.value if token else ""

        def do_GET(self):
            match, query, cookie = self.route()
            self.reply(
                fixture.get(*match.groups(), query, cookie) if match else (404, "Not found", [])
            )

        def do_POST(self):
            match, _, cookie = self.route()
            try:
                length = int(self.headers.get("Content-Length", "-1"))
                if (
                    not match
                    or not 0 <= length <= 16384
                    or self.headers.get("Transfer-Encoding")
                    or self.headers.get("Content-Type", "").split(";")[0]
                    != "application/x-www-form-urlencoded"
                ):
                    raise ValueError("invalid request")
                body = self.rfile.read(length)
                if len(body) != length:
                    raise ValueError("truncated request")
                self.reply(fixture.post(*match.groups(), body, cookie))
            except (ValueError, OSError):
                self.reply((400, "Invalid request", []))

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    close_extra = configure_server(fixture, server) if configure_server else None
    threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.05}, daemon=True).start()
    print(json.dumps({"event": "ready", "port": server.server_port}), flush=True)
    sealed = False
    try:
        while line := sys.stdin.readline(8193):
            try:
                if len(line) > 8192:
                    raise ValueError("control frame limit")
                message = json.loads(line)
                if message.get("command") == "shutdown":
                    break
                if message.get("command") == "seal":
                    if not sealed:
                        server.shutdown()
                        server.server_close()
                        if close_extra is not None:
                            close_extra()
                            close_extra = None
                        sealed = True
                    result = {"sealed": True}
                else:
                    if sealed and message.get("command") != "snapshot":
                        raise ValueError("fixture sealed")
                    result = fixture.command(message)
                print(json.dumps({"ok": True, "result": result}), flush=True)
            except (ValueError, KeyError, TypeError, sqlite3.Error) as error:
                print(json.dumps({"ok": False, "error": str(error)}), flush=True)
    finally:
        server.shutdown()
        server.server_close()
        if close_extra is not None:
            close_extra()
        fixture.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    serve(parser.parse_args().database)

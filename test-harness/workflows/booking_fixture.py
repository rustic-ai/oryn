"""Controlled reservations only; no external calendar or real booking is contacted."""

import argparse
from contextlib import closing
from datetime import datetime
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
from zoneinfo import ZoneInfo

ZONES = ("UTC", "America/New_York", "America/Vancouver")
SLOTS = {"early": "2026-11-01T05:30:00+00:00", "late": "2026-11-01T06:30:00+00:00"}


def local_time(slot, zone):
    if zone not in ZONES:
        raise ValueError("unsupported time zone")
    return datetime.fromisoformat(SLOTS[slot]).astimezone(ZoneInfo(zone)).isoformat()


def snapshot(database, case_id):
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        result = {
            table: [
                dict(row)
                for row in db.execute(
                    f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (case_id,)
                )
            ]
            for table in (
                "cases",
                "slots",
                "offers",
                "reservations",
                "requests",
                "views",
            )
        }
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
            CREATE TABLE cases(case_id TEXT PRIMARY KEY, scenario TEXT, session TEXT, reviews INTEGER);
            CREATE TABLE slots(case_id TEXT, slot TEXT, available INTEGER, version INTEGER,
                PRIMARY KEY(case_id,slot));
            CREATE TABLE offers(case_id TEXT, offer TEXT PRIMARY KEY, slot TEXT, version INTEGER,
                zone TEXT, utc TEXT, local TEXT);
            CREATE TABLE reservations(case_id TEXT, reservation TEXT PRIMARY KEY, slot TEXT,
                offer TEXT, request_key TEXT, fields TEXT, utc TEXT, local TEXT, zone TEXT,
                UNIQUE(case_id,slot), UNIQUE(case_id,request_key));
            CREATE TABLE requests(case_id TEXT, status INTEGER, fields TEXT, cookie_valid INTEGER);
            CREATE TABLE views(case_id TEXT, path TEXT, cookie_valid INTEGER);
        """)

    def command(self, message):
        case_id = message.get("case_id", "")
        if not re.fullmatch(r"[0-9a-f]{32}", case_id):
            raise ValueError("invalid case id")
        with self.lock, self.db:
            if message["command"] == "snapshot":
                return snapshot(self.database, case_id)
            if message["command"] != "create" or message.get(
                "scenario", "normal"
            ) not in ("normal", "stale"):
                raise ValueError("invalid control request")
            if self.db.execute("SELECT count(*) FROM cases").fetchone()[0] >= 128:
                raise ValueError("case capacity")
            self.db.execute(
                "INSERT INTO cases VALUES(?,?,?,0)",
                (case_id, message.get("scenario", "normal"), secrets.token_hex(24)),
            )
            self.db.executemany(
                "INSERT INTO slots VALUES(?,?,1,1)", [(case_id, slot) for slot in SLOTS]
            )
            return {"created": case_id}

    def page(self, case_id, title, body):
        return (
            "<!doctype html><meta charset=utf-8><title>Controlled booking</title>"
            "<style>body{font:18px sans-serif;margin:24px}label,input,select,button,a{display:block;margin:8px}</style>"
            f"<h1>{html.escape(title)}</h1><p>Test appointments only. No real reservations.</p>{body}"
            f'<a href="/booking/{case_id}/availability">Search availability</a>'
        )

    def authorized(self, case_id, cookie):
        row = self.db.execute(
            "SELECT * FROM cases WHERE case_id=?", (case_id,)
        ).fetchone()
        return row if row and secrets.compare_digest(row["session"], cookie) else None

    def get(self, case_id, path, query, cookie):
        with self.lock, self.db:
            base = f"/booking/{case_id}"
            if path == "start":
                row = self.db.execute(
                    "SELECT session FROM cases WHERE case_id=?", (case_id,)
                ).fetchone()
                return (
                    (
                        303,
                        "",
                        [
                            ("Location", base + "/availability"),
                            (
                                "Set-Cookie",
                                f"booking_session={row['session']}; Path={base}/; HttpOnly; SameSite=Lax",
                            ),
                        ],
                    )
                    if row
                    else (404, "Unknown case", [])
                )
            case = self.authorized(case_id, cookie)
            self.db.execute(
                "INSERT INTO views VALUES(?,?,?)",
                (case_id, path, int(case is not None)),
            )
            if not case:
                return 403, "Session required", []
            if path == "availability":
                options = "".join(
                    f'<option value="{slot}" {"" if row["available"] else "disabled"}>{slot}: {SLOTS[slot]}</option>'
                    for row in self.db.execute(
                        "SELECT * FROM slots WHERE case_id=? ORDER BY slot", (case_id,)
                    )
                    for slot in [row["slot"]]
                )
                zones = "".join(
                    f'<option value="{zone}">{zone}</option>' for zone in ZONES
                )
                return (
                    200,
                    self.page(
                        case_id,
                        "Available appointments",
                        f'<form action="{base}/review"><label>Time zone<select name=zone>{zones}</select></label>'
                        f"<label>Appointment<select name=slot>{options}</select></label>"
                        "<button>Review appointment</button></form>",
                    ),
                    [],
                )
            if path == "review":
                slot, zone = query.get("slot", [""])[0], query.get("zone", [""])[0]
                row = self.db.execute(
                    "SELECT * FROM slots WHERE case_id=? AND slot=?", (case_id, slot)
                ).fetchone()
                if not row or not row["available"] or zone not in ZONES:
                    return (
                        409,
                        self.page(
                            case_id,
                            "Unavailable",
                            "<p id=conflict>Choose an available appointment</p>",
                        ),
                        [],
                    )
                offer, key = secrets.token_hex(16), secrets.token_hex(16)
                local = local_time(slot, zone)
                self.db.execute(
                    "INSERT INTO offers VALUES(?,?,?,?,?,?,?)",
                    (case_id, offer, slot, row["version"], zone, SLOTS[slot], local),
                )
                self.db.execute(
                    "UPDATE cases SET reviews=reviews+1 WHERE case_id=?", (case_id,)
                )
                if case["scenario"] == "stale" and case["reviews"] == 0:
                    self.db.execute(
                        "UPDATE slots SET available=0,version=version+1 WHERE case_id=? AND slot=?",
                        (case_id, slot),
                    )
                return (
                    200,
                    self.page(
                        case_id,
                        "Review appointment",
                        f"<p id=appointment-time>{local}</p><p id=appointment-zone>{zone}</p>"
                        f'<form method=post action="{base}/reserve"><input type=hidden name=offer value="{offer}">'
                        f'<input type=hidden name=request_key value="{key}">'
                        "<label>Attendee<input name=name required></label><label>Email<input name=email type=email required></label>"
                        "<button>Reserve test appointment</button></form>",
                    ),
                    [],
                )
            if path == "confirmation":
                row = self.db.execute(
                    "SELECT * FROM reservations WHERE case_id=? ORDER BY rowid DESC LIMIT 1",
                    (case_id,),
                ).fetchone()
                if not row:
                    return 404, "No reservation", []
                fields = json.loads(row["fields"])
                hidden = "".join(
                    f'<input type=hidden name="{html.escape(k)}" value="{html.escape(v[0], quote=True)}">'
                    for k, v in fields.items()
                )
                return (
                    200,
                    self.page(
                        case_id,
                        "Reservation confirmed",
                        f"<p id=reservation>{row['reservation']}</p><p id=appointment-time>{row['local']}</p>"
                        f'<a href="{base}/confirmation">Reload reservation</a>'
                        f'<form method=post action="{base}/reserve">{hidden}<button>Repeat reservation request</button></form>',
                    ),
                    [],
                )
            return 404, "Not found", []

    def post(self, case_id, path, body, cookie):
        fields = parse_qs(body.decode("utf-8"), keep_blank_values=True)
        encoded = json.dumps(fields, sort_keys=True)
        with self.lock, self.db:
            valid = self.authorized(case_id, cookie) is not None
            status = 403
            if valid:
                status = 400
                if (
                    path == "reserve"
                    and set(fields) == {"offer", "request_key", "name", "email"}
                    and all(
                        len(v) == 1 and 0 < len(v[0]) <= 200 for v in fields.values()
                    )
                    and re.fullmatch(r"[0-9a-f]{32}", fields["request_key"][0])
                    and re.fullmatch(r"[^\s@]+@[^\s@]+\.[^\s@]+", fields["email"][0])
                ):
                    key = fields["request_key"][0]
                    previous = self.db.execute(
                        "SELECT * FROM reservations WHERE case_id=? AND request_key=?",
                        (case_id, key),
                    ).fetchone()
                    offer = self.db.execute(
                        "SELECT * FROM offers WHERE case_id=? AND offer=?",
                        (case_id, fields["offer"][0]),
                    ).fetchone()
                    status = 409
                    if previous:
                        status = 303 if previous["fields"] == encoded else 409
                    elif offer:
                        updated = self.db.execute(
                            "UPDATE slots SET available=0,version=version+1 WHERE case_id=? AND slot=? AND available=1 AND version=?",
                            (case_id, offer["slot"], offer["version"]),
                        ).rowcount
                        if updated:
                            self.db.execute(
                                "INSERT INTO reservations VALUES(?,?,?,?,?,?,?,?,?)",
                                (
                                    case_id,
                                    secrets.token_hex(16),
                                    offer["slot"],
                                    offer["offer"],
                                    key,
                                    encoded,
                                    offer["utc"],
                                    offer["local"],
                                    offer["zone"],
                                ),
                            )
                            status = 303
            self.db.execute(
                "INSERT INTO requests VALUES(?,?,?,?)",
                (case_id, status, encoded, int(valid)),
            )
            if status == 303:
                return status, "", [("Location", f"/booking/{case_id}/confirmation")]
            return (
                status,
                self.page(
                    case_id,
                    "Reservation not accepted",
                    "<p id=conflict>Review availability and retry</p>",
                ),
                [],
            )


def serve(database):
    fixture = Fixture(database)

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
                ("Content-Type", "text/html; charset=utf-8"),
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
                r"/booking/([0-9a-f]{32})/(start|availability|review|reserve|confirmation)",
                parsed.path,
            )
            cookies = SimpleCookie()
            cookies.load(self.headers.get("Cookie", ""))
            token = cookies.get("booking_session")
            return match, parse_qs(parsed.query), token.value if token else ""

        def do_GET(self):
            match, query, cookie = self.route()
            self.reply(
                fixture.get(*match.groups(), query, cookie)
                if match
                else (404, "Not found", [])
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
    threading.Thread(target=server.serve_forever, daemon=True).start()
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
        fixture.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    serve(parser.parse_args().database)

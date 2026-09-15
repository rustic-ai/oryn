"""Controlled authenticated records; no external identity or business service.

Credentials are synthetic fixture data. Session/CSRF values never enter oracle
snapshots. Every mutation is authorized and recorded in SQLite independently of
browser observations. HTTP transport and OIL qualification are separate gates.
"""

from contextlib import closing
import argparse
import hashlib
import html
from http.cookies import CookieError, SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
import secrets
import sqlite3
import threading
import sys
from urllib.parse import parse_qs, urlsplit


def snapshot(database, case_id):
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        result = {}
        for table in ("cases", "records", "requests", "views", "expirations"):
            result[table] = [dict(row) for row in db.execute(
                f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (case_id,)
            )]
        for case in result["cases"]:
            for name in ("session", "csrf"):
                case.pop(name, None)
        return result


class Fixture:
    def __init__(self, database):
        self.database = str(database)
        self.lock = threading.RLock()
        self.expire_after_create = set()
        self.db = sqlite3.connect(database, check_same_thread=False)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            CREATE TABLE cases(case_id TEXT PRIMARY KEY, session TEXT, csrf TEXT,
              authenticated INTEGER, logins INTEGER);
            CREATE TABLE records(case_id TEXT PRIMARY KEY, title TEXT, version INTEGER);
            CREATE TABLE requests(case_id TEXT, action TEXT, outcome INTEGER,
              authorized INTEGER, body_sha256 TEXT);
            CREATE TABLE views(case_id TEXT, page TEXT, authorized INTEGER);
            CREATE TABLE expirations(case_id TEXT, reason TEXT);
        """)

    def command(self, message):
        case_id = message.get("case_id", "")
        if not re.fullmatch(r"[0-9a-f]{32}", case_id):
            raise ValueError("invalid case id")
        with self.lock, self.db:
            command = message["command"]
            if command == "snapshot":
                return snapshot(self.database, case_id)
            if command == "create":
                scenario = message.get("scenario", "normal")
                if scenario not in ("normal", "expire_after_create"):
                    raise ValueError("invalid scenario")
                if self.db.execute("SELECT count(*) FROM cases").fetchone()[0] >= 128:
                    raise ValueError("case capacity")
                self.db.execute("INSERT INTO cases VALUES(?,?,?,0,0)",
                                (case_id, secrets.token_hex(24), secrets.token_hex(24)))
                if scenario == "expire_after_create":
                    self.expire_after_create.add(case_id)
                return {"created": case_id}
            if command == "expire":
                if self.db.execute("UPDATE cases SET authenticated=0 WHERE case_id=?", (case_id,)).rowcount != 1:
                    raise ValueError("unknown case")
                self.db.execute("INSERT INTO expirations VALUES(?,?)", (case_id, "oracle"))
                return {"expired": True}
            raise ValueError("unknown oracle command")

    @staticmethod
    def authorized(case, cookie):
        return bool(case["authenticated"] and secrets.compare_digest(case["session"], cookie))

    @staticmethod
    def page(title, body):
        return ("<!doctype html><meta charset=utf-8><title>Controlled SaaS</title>"
                "<style>body{font:18px sans-serif;margin:24px}label,input,button,a{display:block;margin:8px}</style>"
                f"<h1>{html.escape(title)}</h1>{body}")

    def get(self, case_id, path, query, cookie):
        with self.lock, self.db:
            case = self.db.execute("SELECT * FROM cases WHERE case_id=?", (case_id,)).fetchone()
            if case is None:
                return 404, "Unknown case", []
            base = f"/saas/{case_id}"
            valid = self.authorized(case, cookie)
            self.db.execute("INSERT INTO views VALUES(?,?,?)", (case_id, path, int(valid)))
            if path == "login":
                return 200, self.page("Sign in", f'<form method=post action="{base}/login">'
                    '<label for=username>Username</label><input id=username name=username required>'
                    '<label for=password>Password</label><input id=password name=password type=password required>'
                    '<button>Sign in</button></form>'), []
            if not valid:
                return 401, self.page("Session required", f'<a href="{base}/login">Sign in again</a>'), []
            if path != "records":
                return 404, "Not found", []
            record = self.db.execute("SELECT * FROM records WHERE case_id=?", (case_id,)).fetchone()
            # Deterministic expiry after delivering the first persisted record's
            # edit form. No wall-clock race and no HTTP-accessible oracle hook.
            if record is not None and case_id in self.expire_after_create:
                self.expire_after_create.remove(case_id)
                self.db.execute("UPDATE cases SET authenticated=0 WHERE case_id=?", (case_id,))
                self.db.execute("INSERT INTO expirations VALUES(?,?)", (case_id, "after_create"))
            action = "update" if record else "create"
            title = html.escape(record["title"], quote=True) if record else ""
            version = record["version"] if record else 0
            body = (f'<p id=record>{title or "No record"}</p><p id=version>{version}</p>'
                    f'<form method=post action="{base}/{action}">'
                    f'<input type=hidden name=csrf value="{case["csrf"]}">'
                    f'<input type=hidden name=version value="{version}">'
                    f'<label for=title>Record title</label><input id=title name=title value="{title}" required maxlength=200>'
                    f'<button>{"Save changes" if record else "Create record"}</button></form>'
                    f'<a href="{base}/records">Reload record</a>'
                    f'<form method=post action="{base}/logout"><input type=hidden name=csrf value="{case["csrf"]}">'
                    '<button>Sign out</button></form>')
            return 200, self.page("Records", body), []

    def post(self, case_id, action, body, cookie):
        if len(body) > 16384:
            return 413, "Request too large", []
        try:
            fields = parse_qs(body.decode("utf-8"), keep_blank_values=True, max_num_fields=16)
        except (UnicodeError, ValueError):
            return 400, "Invalid form", []
        with self.lock, self.db:
            case = self.db.execute("SELECT * FROM cases WHERE case_id=?", (case_id,)).fetchone()
            if case is None:
                return 404, "Unknown case", []
            base = f"/saas/{case_id}"
            valid = self.authorized(case, cookie)
            status, message, headers = 400, "Invalid form", []
            unique = all(len(value) == 1 for value in fields.values())
            if action == "login" and unique and set(fields) == {"username", "password"}:
                if fields["username"] == ["agent@example.test"] and fields["password"] == ["fixture-only-password"]:
                    session, csrf = secrets.token_hex(24), secrets.token_hex(24)
                    self.db.execute("UPDATE cases SET session=?,csrf=?,authenticated=1,logins=logins+1 WHERE case_id=?",
                                    (session, csrf, case_id))
                    status, message = 303, "Signed in"
                    headers = [("Location", base + "/records"),
                               ("Set-Cookie", f"saas_session={session}; Path={base}/; HttpOnly; SameSite=Strict")]
                else:
                    status, message = 401, "Invalid credentials"
            elif action != "login" and not valid:
                status, message = 401, "Session expired"
            elif action != "login" and (not unique or fields.get("csrf") != [case["csrf"]]):
                status, message = 403, "Invalid CSRF token"
            elif action == "logout" and set(fields) == {"csrf"}:
                self.db.execute("UPDATE cases SET authenticated=0 WHERE case_id=?", (case_id,))
                status, message, headers = 303, "Signed out", [("Location", base + "/login")]
            elif action in ("create", "update") and set(fields) == {"csrf", "title", "version"}:
                title, version = fields["title"][0].strip(), fields["version"][0]
                record = self.db.execute("SELECT * FROM records WHERE case_id=?", (case_id,)).fetchone()
                if not title or len(title) > 200:
                    status, message = 422, "Invalid title"
                elif (action == "create" and (record is not None or version != "0")) or (
                    action == "update" and (record is None or version != str(record["version"]))
                ):
                    status, message = 409, "Record changed; reload before saving"
                else:
                    if action == "create":
                        self.db.execute("INSERT INTO records VALUES(?,?,1)", (case_id, title))
                    else:
                        self.db.execute("UPDATE records SET title=?,version=version+1 WHERE case_id=?", (title, case_id))
                    status, message, headers = 303, "Saved", [("Location", base + "/records")]
            self.db.execute("INSERT INTO requests VALUES(?,?,?,?,?)",
                            (case_id, action, status, int(valid), hashlib.sha256(body).hexdigest()))
            if not headers:
                message = self.page(message, f'<a href="{base}/login">Sign in again</a><a href="{base}/records">Reload record</a>')
            return status, message, headers


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
            encoded = body.encode("utf-8")
            self.send_response(status)
            for name, value in [("Content-Type", "text/html; charset=utf-8"),
                                ("Content-Length", str(len(encoded))),
                                ("Cache-Control", "no-store"), *headers]:
                self.send_header(name, value)
            self.end_headers()
            self.wfile.write(encoded)

        def route(self):
            parsed = urlsplit(self.path)
            match = re.fullmatch(r"/saas/([0-9a-f]{32})/(login|records|create|update|logout)", parsed.path)
            cookies = SimpleCookie()
            try:
                cookies.load(self.headers.get("Cookie", ""))
            except CookieError:
                return None, {}, ""
            token = cookies.get("saas_session")
            return match, parse_qs(parsed.query, max_num_fields=16), token.value if token else ""

        def do_GET(self):
            try:
                match, query, cookie = self.route()
                self.reply(fixture.get(*match.groups(), query, cookie) if match else (404, "Not found", []))
            except ValueError:
                self.reply((400, "Invalid request", []))

        def do_POST(self):
            try:
                match, _, cookie = self.route()
                lengths = self.headers.get_all("Content-Length", [])
                if len(lengths) != 1:
                    raise ValueError("ambiguous length")
                length = int(lengths[0])
                if (not match or not 0 <= length <= 16384
                    or self.headers.get("Transfer-Encoding")
                    or self.headers.get("Content-Type", "").split(";")[0] != "application/x-www-form-urlencoded"):
                    raise ValueError("invalid request")
                body = self.rfile.read(length)
                if len(body) != length:
                    raise ValueError("truncated request")
                self.reply(fixture.post(*match.groups(), body, cookie))
            except (ValueError, OSError):
                self.reply((400, "Invalid request", []))

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    serving = threading.Thread(target=server.serve_forever, daemon=True)
    serving.start()
    print(json.dumps({"event": "ready", "port": server.server_port}), flush=True)
    sealed = False
    try:
        while line := sys.stdin.readline(8193):
            try:
                if len(line) > 8192:
                    raise ValueError("control frame limit")
                message = json.loads(line)
                if not isinstance(message, dict):
                    raise ValueError("invalid control message")
                if message.get("command") == "shutdown":
                    break
                if message.get("command") == "seal":
                    if not sealed:
                        server.shutdown()
                        server.server_close()
                        serving.join()
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
        serving.join()
        fixture.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    serve(parser.parse_args().database)

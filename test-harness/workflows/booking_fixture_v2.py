"""Client-timezone booking frontend over the existing controlled SQLite backend."""
import argparse
from datetime import datetime, timezone
import importlib.util
import json
import html
from pathlib import Path
import re
import sqlite3
import sys
import threading
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit
from zoneinfo import ZoneInfo

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("booking_backend_v1", HERE / "booking_fixture.py")
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


SLOTS = legacy.SLOTS
local_time = legacy.local_time
OPTIONS = ("video", "phone")
AUDIT_TABLES = ("client_selections", "offer_options", "cancellations")


def snapshot(database, case_id):
    result = legacy.snapshot(database, case_id)
    from contextlib import closing
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        for table in AUDIT_TABLES:
            result[table] = [dict(row) for row in db.execute(
                f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (case_id,))]
    return result


class Fixture(legacy.Fixture):
    def __init__(self, database):
        super().__init__(database)
        self.db.execute("CREATE TABLE client_selections(case_id TEXT, local TEXT, zone TEXT, utc TEXT, valid INTEGER)")
        self.db.execute("CREATE TABLE offer_options(case_id TEXT, offer TEXT PRIMARY KEY, appointment_option TEXT)")
        self.db.execute("CREATE TABLE cancellations(case_id TEXT, offer TEXT, PRIMARY KEY(case_id,offer))")
        self.db.commit()

    def command(self, message):
        result = super().command(message)
        if message["command"] == "snapshot":
            with self.lock:
                for table in AUDIT_TABLES:
                    result[table] = [dict(row) for row in self.db.execute(
                        f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (message["case_id"],))]
        return result

    def get(self, case_id, path, query, cookie):
        with self.lock:
            if path != "start" and not self.authorized(case_id, cookie):
                return 403, "Session required", []
            if path in ("booking-client.mjs", "timezone-resolution.mjs"):
                return 200, (HERE / "v2" / path).read_text(), [("Content-Type", "text/javascript; charset=utf-8")]
            if path == "availability-data":
                slots = [dict(slot=row["slot"], available=bool(row["available"]), version=row["version"],
                              utc=datetime.fromisoformat(legacy.SLOTS[row["slot"]]).isoformat(timespec="milliseconds").replace("+00:00", "Z"))
                         for row in self.db.execute("SELECT * FROM slots WHERE case_id=? ORDER BY slot", (case_id,))]
                return 200, json.dumps({"slots": slots}), [("Content-Type", "application/json")]
            if path == "review":
                local = query.get("client_local", [""])[0]
                zone = query.get("zone", [""])[0]
                instant = query.get("client_utc", [""])[0]
                slot = query.get("slot", [""])[0]
                option = query.get("appointment_option", ["video"])[0]
                valid = False
                try:
                    parsed = datetime.fromisoformat(instant.replace("Z", "+00:00"))
                    valid = (option in OPTIONS and all(len(values) == 1 for values in query.values())
                             and zone in legacy.ZONES and slot in legacy.SLOTS
                             and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}", local) is not None
                             and parsed.tzinfo is not None and parsed.utcoffset() == timezone.utc.utcoffset(parsed)
                             and parsed == datetime.fromisoformat(legacy.SLOTS[slot])
                             and parsed.astimezone(ZoneInfo(zone)).strftime("%Y-%m-%dT%H:%M") == local)
                except (ValueError, KeyError):
                    pass
                with self.db:
                    self.db.execute("INSERT INTO client_selections VALUES(?,?,?,?,?)",
                                    (case_id, local, zone, instant, int(valid)))
                if not valid:
                    return 400, self.page(case_id, "Invalid local appointment", "<p id=invalid-time>Choose a valid local appointment and occurrence.</p>"), []
            response = super().get(case_id, path, query, cookie)
            if path == "review" and response[0] == 200:
                offer = self.db.execute("SELECT offer FROM offers WHERE case_id=? ORDER BY rowid DESC LIMIT 1", (case_id,)).fetchone()["offer"]
                with self.db:
                    self.db.execute("INSERT INTO offer_options VALUES(?,?,?)", (case_id, offer, option))
                controls = (f'<p id="appointment-option">{html.escape(option)}</p>'
                            f'<form method="post" action="/booking/{case_id}/cancel">'
                            f'<input type="hidden" name="offer" value="{offer}">'
                            '<button>Cancel review</button></form>')
                return response[0], response[1] + controls, response[2]
            if path == "confirmation" and response[0] == 200:
                row = self.db.execute("SELECT o.appointment_option FROM reservations r JOIN offer_options o ON r.offer=o.offer WHERE r.case_id=? ORDER BY r.rowid DESC LIMIT 1", (case_id,)).fetchone()
                return response[0], response[1] + f'<p id="appointment-option">{html.escape(row[0])}</p>', response[2]
            if path != "availability" or response[0] != 200:
                return response
            body = '''<form id="appointment-search">
<label>Local appointment<input name="local" type="datetime-local" value="2026-11-01T01:30" required></label>
<label>Time zone<select name="zone"><option>America/New_York</option><option>America/Vancouver</option><option>UTC</option></select></label>
<label>Occurrence<select id="occurrence" required></select></label>
<label>Appointment option<select name="appointment_option"><option value="video">Video</option><option value="phone">Phone</option></select></label>
<p id="selection-status" role="status" data-state="loading">Loading availability</p>
<button type="button" id="refresh-availability">Refresh availability</button>
<button id="review-appointment" disabled>Review appointment</button>
</form><script type="module" src="./booking-client.mjs"></script>'''
            return 200, self.page(case_id, "Choose a local appointment", body), []

    def post(self, case_id, path, body, cookie):
        fields = parse_qs(body.decode("utf-8"), keep_blank_values=True)
        with self.lock, self.db:
            if not self.authorized(case_id, cookie):
                return super().post(case_id, path, body, cookie)
            offer = fields.get("offer", [""])[0]
            if path == "cancel":
                owned = self.db.execute("SELECT 1 FROM offers WHERE case_id=? AND offer=?", (case_id, offer)).fetchone()
                reserved = self.db.execute("SELECT 1 FROM reservations WHERE case_id=? AND offer=?", (case_id, offer)).fetchone()
                if set(fields) != {"offer"} or len(fields["offer"]) != 1 or not owned or reserved:
                    return 409, "Review cannot be cancelled", []
                self.db.execute("INSERT OR IGNORE INTO cancellations VALUES(?,?)", (case_id, offer))
                return 303, "", [("Location", f"/booking/{case_id}/availability")]
            if path == "reserve" and self.db.execute("SELECT 1 FROM cancellations WHERE case_id=? AND offer=?", (case_id, offer)).fetchone():
                self.db.execute("INSERT INTO requests VALUES(?,?,?,1)", (case_id, 409, json.dumps(fields, sort_keys=True)))
                return 409, self.page(case_id, "Cancelled review", "<p id=conflict>Review availability and retry</p>"), []
            return super().post(case_id, path, body, cookie)


def serve(database, fixture_type=Fixture):
    fixture = fixture_type(database)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def reply(self, response):
            status, body, headers = response
            body = body.encode()
            self.send_response(status)
            if not any(key.lower() == "content-type" for key, _ in headers):
                self.send_header("Content-Type", "text/html; charset=utf-8")
            for key, value in [("Content-Length", str(len(body))), ("Cache-Control", "no-store"), *headers]:
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(body)

        def route(self):
            parsed = urlsplit(self.path)
            match = re.fullmatch(r"/booking/([0-9a-f]{32})/(start|availability|availability-data|review|reserve|cancel|confirmation|workflow-outcome|booking-client\.mjs|timezone-resolution\.mjs)", parsed.path)
            cookies = SimpleCookie()
            cookies.load(self.headers.get("Cookie", ""))
            token = cookies.get("booking_session")
            return match, parse_qs(parsed.query), token.value if token else ""

        def do_GET(self):
            if len(self.path) > 16384:
                return self.reply((414, "Request too long", []))
            match, query, cookie = self.route()
            self.reply(fixture.get(*match.groups(), query, cookie) if match else (404, "Not found", []))

        def do_POST(self):
            match, _, cookie = self.route()
            try:
                length = int(self.headers.get("Content-Length", "-1"))
                if (not match or not 0 <= length <= 16384 or self.headers.get("Transfer-Encoding")
                        or self.headers.get("Content-Type", "").split(";")[0] != "application/x-www-form-urlencoded"):
                    raise ValueError("invalid request")
                body = self.rfile.read(length)
                if len(body) != length:
                    raise ValueError("truncated body")
                self.reply(fixture.post(*match.groups(), body, cookie))
            except (ValueError, OSError):
                self.reply((400, "Invalid request", []))

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.05}, daemon=True)
    thread.start()
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
                        thread.join()
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
        thread.join()
        fixture.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    serve(parser.parse_args().database)

"""Controlled research sources; SQLite request evidence is not exposed over HTTP."""

import argparse
from contextlib import closing
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import re
import sqlite3
import sys
import threading
from urllib.parse import parse_qs


def snapshot(database, case_id):
    with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as db:
        db.row_factory = sqlite3.Row
        return {
            table: [
                dict(row)
                for row in db.execute(
                    f"SELECT * FROM {table} WHERE case_id=? ORDER BY rowid", (case_id,)
                )
            ]
            for table in ("cases", "visits")
        }


class Fixture:
    def __init__(self, database):
        self.database = str(database)
        self.lock = threading.RLock()
        self.db = sqlite3.connect(database, check_same_thread=False)
        self.db.execute("PRAGMA journal_mode=DELETE")
        self.db.executescript("""
            CREATE TABLE cases(case_id TEXT PRIMARY KEY);
            CREATE TABLE visits(case_id TEXT, path TEXT, query TEXT, status INTEGER, body_sha256 TEXT);
        """)

    def command(self, message):
        case_id = message["case_id"]
        if not re.fullmatch("[0-9a-f]{32}", case_id):
            raise ValueError("invalid case identity")
        with self.lock, self.db:
            if message["command"] == "create":
                if self.db.execute("SELECT COUNT(*) FROM cases").fetchone()[0] >= 128:
                    raise ValueError("case limit")
                self.db.execute("INSERT INTO cases VALUES (?)", (case_id,))
                return {}
            if message["command"] == "snapshot":
                return snapshot(self.database, case_id)
            raise ValueError("unsupported control command")

    @staticmethod
    def page(body):
        return (
            "<!doctype html><meta charset=utf-8><title>Controlled research</title><main>"
            + body
            + "</main>"
        )

    @staticmethod
    def results(base, query, page):
        if query == "absent":
            return "<p id=empty>No matching sources</p>"
        if query == "redirect":
            return f'<a href="{base}/old-alpha">Archived Alpha report</a>'
        if query == "missing":
            return f'<a href="{base}/gone">Unavailable report</a><a href="{base}/article/alpha">Alpha report</a>'
        if page == "2":
            return f'<a href="{base}/article/beta">Beta report</a>'
        return f'<a href="{base}/article/alpha">Alpha report</a><a href="{base}/search?q=energy&amp;page=2">Next results</a>'

    def get(self, case_id, path, query):
        with self.lock, self.db:
            if not self.db.execute(
                "SELECT 1 FROM cases WHERE case_id=?", (case_id,)
            ).fetchone():
                return 404, "Unknown case", []
            base = f"/research/{case_id}"
            status, headers = 200, []
            if path == "start":
                body = f'<form action="{base}/search"><label>Search sources<input name=q required></label><button>Search reports</button></form>'
            elif path in ("search", "api"):
                term = query.get("q", [""])[0]
                page = query.get("page", ["1"])[0]
                if term not in (
                    "energy",
                    "redirect",
                    "missing",
                    "absent",
                    "dynamic",
                    "refresh",
                ) or page not in ("1", "2"):
                    status, body = 400, "Invalid search"
                elif path == "search" and term in ("dynamic", "refresh"):
                    initial = (
                        "" if term == "dynamic" else self.results(base, "energy", "1")
                    )
                    api = (
                        base
                        + "/api?q=energy&page="
                        + ("1" if term == "dynamic" else "2")
                    )
                    script = (
                        "fetch("
                        + json.dumps(api)
                        + ").then(r=>r.text()).then(text=>{document.querySelector('#results').innerHTML=text;document.querySelector('#ready').hidden=false;})"
                    )
                    body = f"<div id=results>{initial}</div><p id=ready hidden>Results loaded</p>"
                    body += (
                        "<script>" + script + "</script>"
                        if term == "dynamic"
                        else '<button id=refresh>Refresh results</button><script>document.querySelector("#refresh").onclick=()=>{'
                        + script
                        + "};</script>"
                    )
                else:
                    body = self.results(base, term, page)
                    if path == "search":
                        body = "<div id=results>" + body + "</div>"
            elif path == "old-alpha":
                status, body, headers = 303, "", [("Location", base + "/article/alpha")]
            elif path == "gone":
                status = 404
                body = f'<p id=missing>Source unavailable</p><a href="{base}/search?q=missing">Return to results</a>'
            elif path in ("article/alpha", "article/beta"):
                name, value = (
                    ("Alpha", "11") if path.endswith("alpha") else ("Beta", "22")
                )
                body = (
                    f"<h1>{name} report</h1><p id=facts>{name} generated {value} MWh in 2025.</p>"
                    f"<table id=measurements><tr><th>Year</th><th>MWh</th></tr><tr><td>2025</td><td>{value}</td></tr></table>"
                    f'<nav id=sources><a href="{base}/{path}">Canonical source</a></nav>'
                    f'<a href="{base}/search?q=energy">Back to results</a>'
                )
            else:
                status, body = 404, "Not found"
            if path != "api":
                body = self.page(body)
            self.db.execute(
                "INSERT INTO visits VALUES (?,?,?,?,?)",
                (
                    case_id,
                    path,
                    json.dumps(query, sort_keys=True),
                    status,
                    hashlib.sha256(body.encode()).hexdigest(),
                ),
            )
            return status, body, headers


def serve(database):
    fixture = Fixture(database)

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def do_GET(self):
            path, _, query = self.path.partition("?")
            match = re.fullmatch(
                r"/research/([0-9a-f]{32})/(start|search|api|old-alpha|gone|article/alpha|article/beta)",
                path,
            )
            status, body, headers = (
                fixture.get(*match.groups(), parse_qs(query))
                if match
                else (404, "Not found", [])
            )
            payload = body.encode()
            self.send_response(status)
            for key, value in [
                ("Content-Type", "text/html; charset=utf-8"),
                ("Content-Length", str(len(payload))),
                ("Cache-Control", "no-store"),
                *headers,
            ]:
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    thread = threading.Thread(target=server.serve_forever)
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
                        thread.join(timeout=10)
                        if thread.is_alive():
                            raise ValueError("server did not stop")
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
        thread.join(timeout=10)
        fixture.db.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True)
    serve(parser.parse_args().database)

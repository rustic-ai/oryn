"""Loopback-only form receiver; its independent oracle is available only on stdin/stdout."""

from email import policy
from email.parser import BytesParser
import hashlib
import html
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import re
import sys
import threading
from urllib.parse import parse_qsl, urlsplit

ROOT = Path(__file__).resolve().parent
MAX_BODY = 128 * 1024


def decode_submission(content_type, body):
    fields, files = [], []
    if content_type.split(";")[0].strip().lower() == "application/x-www-form-urlencoded":
        fields = [
            list(pair)
            for pair in parse_qsl(
                body.decode("utf-8"), keep_blank_values=True, strict_parsing=True, max_num_fields=32
            )
        ]
    elif content_type.lower().startswith("multipart/form-data;"):
        message = BytesParser(policy=policy.default).parsebytes(
            b"Content-Type: "
            + content_type.encode("ascii")
            + b"\r\nMIME-Version: 1.0\r\n\r\n"
            + body
        )
        if not message.is_multipart() or message.defects:
            raise ValueError("invalid multipart body")
        parts = list(message.iter_parts())
        if len(parts) > 32:
            raise ValueError("too many multipart fields")
        for part in parts:
            name = part.get_param("name", header="content-disposition")
            if (
                part.defects
                or part.is_multipart()
                or part.get_content_disposition() != "form-data"
                or not name
            ):
                raise ValueError("invalid multipart part")
            value = part.get_payload(decode=True)
            if value is None:
                raise ValueError("missing part data")
            filename = part.get_filename()
            if filename is None:
                fields.append([name, value.decode("utf-8")])
            else:
                files.append(
                    {
                        "name": name,
                        "filename": filename,
                        "bytes": len(value),
                        "sha256": hashlib.sha256(value).hexdigest(),
                    }
                )
    else:
        raise ValueError("unsupported submission encoding")
    return fields, files


class Fixture:
    def __init__(self):
        self.lock = threading.Lock()
        self.cases = {}

    def command(self, message):
        case_id = message.get("case_id", "")
        if not re.fullmatch(r"[0-9a-f]{32}", case_id):
            raise ValueError("invalid case identity")
        with self.lock:
            if message["command"] == "create":
                if case_id in self.cases or len(self.cases) >= 128:
                    raise ValueError("case capacity or duplicate identity")
                self.cases[case_id] = {
                    "upload": bool(message.get("upload")),
                    "requests": [],
                    "receipts": [],
                    "receipt_views": 0,
                }
                return {"created": case_id}
            if message["command"] == "snapshot" and case_id in self.cases:
                return json.loads(json.dumps(self.cases[case_id]))
            raise ValueError("unknown command or case")

    def receipt_view(self, case_id):
        """Called while the request handler holds the fixture mutation lock."""
        self.cases[case_id]["receipt_views"] += 1

    def receive(self, case_id, content_type, body):
        with self.lock:
            case = self.cases.get(case_id)
            if case is None:
                return 404
            if len(case["requests"]) >= 16:
                return 429
            request = {
                "content_type": content_type,
                "bytes": len(body),
                "sha256": hashlib.sha256(body).hexdigest(),
            }
            case["requests"].append(request)
            try:
                fields, files = decode_submission(content_type, body)
                request.update(fields=fields, files=files)
                values = {}
                for name, value in fields:
                    values.setdefault(name, []).append(value)
                valid = (
                    values.get("case_id") == [case_id]
                    and len(values.get("name", [])) == 1
                    and bool(values["name"][0])
                    and len(values["name"][0].encode("utf-16-le")) // 2 <= 10
                    and len(values.get("email", [])) == 1
                    and re.fullmatch(r"[^\s@]+@[^\s@]+\.[^\s@]+", values["email"][0])
                    and len(values.get("message", [])) == 1
                    and 5 <= len(values["message"][0].encode("utf-16-le")) // 2 <= 64
                    and values.get("role") in (["designer"], ["developer"])
                    and values.get("quantity") in (["1"], ["3"], ["5"], ["7"], ["9"])
                    and len(values.get("date", [])) == 1
                    and re.fullmatch(r"2026-10-(0[1-9]|[12][0-9]|3[01])", values["date"][0])
                    and len(values.get("time", [])) == 1
                    and re.fullmatch(r"(2[1-3]|0[0-5]):[0-5][0-9]|06:00", values["time"][0])
                    and values.get("intent") == ["apply"]
                    and len(values.get("reference", [])) == 1
                    and re.fullmatch(r"[A-Z]{3}[0-9]{3}", values["reference"][0])
                    and ("disabled_field" not in values)
                    and (
                        len(files) == 1 and files[0]["name"] == "document"
                        if case["upload"]
                        else not files
                    )
                )
                if not valid:
                    request["status"] = 422
                    return 422
                if case["receipts"]:
                    request["status"] = 409
                    return 409
                request["status"] = 303
                case["receipts"].append(
                    {
                        "case_id": case_id,
                        "fields": fields,
                        "files": files,
                        "request_sha256": request["sha256"],
                    }
                )
                return 303
            except (ValueError, UnicodeError) as error:
                request.update(status=400, error=str(error))
                return 400


def serve(fixture_type=Fixture, render_page=None, extra_get=None):
    fixture = fixture_type()

    class Handler(BaseHTTPRequestHandler):
        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def log_message(self, *_args):
            pass  # No request bodies or fixture data in shared logs.

        def reply(self, status, body, headers=()):
            body = body.encode("utf-8")
            self.send_response(status)
            if not any(key.lower() == "content-type" for key, _ in headers):
                self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            for key, value in headers:
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if extra_get is not None and extra_get(self, fixture):
                return
            match = re.fullmatch(r"/(form|receipt)/([0-9a-f]{32})", urlsplit(self.path).path)
            if not match:
                self.reply(404, "Not found")
                return
            kind, case_id = match.groups()
            with fixture.lock:
                case = fixture.cases.get(case_id)
                if case is None or (kind == "receipt" and not case["receipts"]):
                    self.reply(404, "No receipt")
                    return
                if kind == "receipt":
                    fixture.receipt_view(case_id)
                    self.reply(
                        200,
                        '<h1 id="receipt">Application received</h1>'
                        f"<p>Receipt {html.escape(case_id)}</p>"
                        f'<a href="/receipt/{case_id}">Reload receipt</a>' + (fixture.repeat_form(case_id) if hasattr(fixture,"repeat_form") else ""),
                    )
                    return
                upload = case["upload"]
            page = (ROOT / "forms.html").read_text().replace("__CASE_ID__", case_id)
            page = page.replace(
                "__ENCODING__",
                "multipart/form-data" if upload else "application/x-www-form-urlencoded",
            )
            page = page.replace(
                "__UPLOAD_CONTROL__",
                '<label for="document">Application document</label>'
                '<input id="document" type="file" name="document" required>'
                if upload
                else "",
            )
            self.reply(200, render_page(page, case_id) if render_page else page)

        def do_POST(self):
            match = re.fullmatch(r"/submit/([0-9a-f]{32})", urlsplit(self.path).path)
            if not match:
                self.reply(404, "Not found")
                return
            self.connection.settimeout(10)
            try:
                length = int(self.headers.get("Content-Length", "-1"))
                if not 0 <= length <= MAX_BODY or self.headers.get("Transfer-Encoding"):
                    self.reply(413, "Invalid body size")
                    return
                body = self.rfile.read(length)
                if len(body) != length:
                    raise ValueError("truncated body")
                status = fixture.receive(match[1], self.headers.get("Content-Type", ""), body)
                headers = [("Location", "/receipt/" + match[1])] if status == 303 else []
                self.reply(status, "Submitted" if status == 303 else "Rejected" + (fixture.repeat_form(match[1]) if status == 409 and hasattr(fixture,"repeat_form") else ""), headers)
            except (ValueError, OSError):
                self.reply(400, "Invalid request")

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = False
    threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.05}, daemon=True).start()
    print(json.dumps({"event": "ready", "port": server.server_port}), flush=True)
    sealed = False
    try:
        while True:
            line = sys.stdin.readline(8193)
            if not line:
                break
            if len(line) > 8192:
                raise ValueError("control frame too large")
            try:
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
                        raise ValueError("fixture is sealed")
                    result = fixture.command(message)
                print(json.dumps({"ok": True, "result": result}), flush=True)
            except (ValueError, KeyError, TypeError) as error:
                print(json.dumps({"ok": False, "error": str(error)}), flush=True)
    finally:
        server.shutdown()
        server.server_close()


if __name__ == "__main__":
    serve()

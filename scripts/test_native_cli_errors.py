"""Signed CLI regression; opt in with ORYN_TEST_NATIVE_BIN and ORYN_PAGE_WORKER."""

import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


@unittest.skipUnless(os.environ.get("ORYN_TEST_NATIVE_BIN"), "requires explicit built native CLI")
class NativeCliErrorTests(unittest.TestCase):
    def run_cli(self, *flags, source="invalid-command-for-regression\nobserve\nexit\n"):
        return subprocess.run(
            [str(Path(os.environ["ORYN_TEST_NATIVE_BIN"]).resolve()), "native", *flags],
            input=source,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_piped_error_is_nonzero_and_stops_before_next_command(self):
        result = self.run_cli()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid-command-for-regression", result.stderr)
        self.assertNotIn('"kind":"observation"', result.stdout)

    def test_sdk_repl_can_recover_after_an_error(self):
        result = self.run_cli("--sdk-repl")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("invalid-command-for-regression", result.stderr)
        self.assertIn('"kind":"observation"', result.stdout)

    def test_successful_piped_commands_remain_successful(self):
        result = self.run_cli(source="observe\nexit\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"kind":"observation"', result.stdout)
        self.assertEqual(result.stderr, "")

    def test_html_input_reaches_signed_worker_with_parent_policy_grant(self):
        with tempfile.TemporaryDirectory() as directory:
            html = Path(directory) / "parent-policy.html"
            html.write_text("<!doctype html><p id='value'>parent policy body</p>", encoding="utf-8")
            result = self.run_cli(
                "--html", str(html), "--evaluate",
                "document.getElementById('value').textContent", source="exit\n",
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "parent policy body")

    def test_http_redirect_commits_final_policy_and_runs_document_script(self):
        requests = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                requests.append(self.path)
                if self.path == "/start":
                    self.send_response(302)
                    self.send_header("Location", "/document")
                    self.send_header("Content-Security-Policy", "script-src 'none'")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                body = (
                    b"<!doctype html><title>before script</title>"
                    b"<script>document.title='HTTP policy committed'</script>"
                )
                self.send_response(200)
                self.send_header("Content-Type", "text/html; charset=utf-8")
                self.send_header("Set-Cookie", "response_policy=private; HttpOnly")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                pass

        with ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
            thread = threading.Thread(target=lambda: server.serve_forever(poll_interval=0.05))
            thread.start()
            try:
                result = self.run_cli(
                    "--allow-loopback", "--url",
                    f"http://127.0.0.1:{server.server_port}/start",
                    "--evaluate", "document.title", source="exit\n",
                )
            finally:
                server.shutdown()
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive(), "HTTP fixture failed to stop")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "HTTP policy committed")
        self.assertEqual(requests[:2], ["/start", "/document"])


if __name__ == "__main__":
    unittest.main()

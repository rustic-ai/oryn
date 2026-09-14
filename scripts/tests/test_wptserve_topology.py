"""Live upstream-server regression; set ORYN_WPT_CHECKOUT to the pinned checkout."""
import importlib.util
import http.client
import json
import os
from pathlib import Path
import select
import socket
import ssl
import subprocess
import tempfile
import unittest
import threading

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("runner", ROOT / "scripts/run-upstream-wpt.py")
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


@unittest.skipUnless(os.environ.get("ORYN_WPT_CHECKOUT"), "requires pinned WPT checkout")
class LiveTopologyTests(unittest.TestCase):
    def test_public_server_close_joins_active_request_handlers(self):
        started = threading.Event()
        release = threading.Event()
        finished = threading.Event()
        class Handler(RUNNER.http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                started.set()
                release.wait(5)
                finished.set()
            def log_message(self, *args):
                pass
        server = RUNNER.ConformanceHTTPServer(("127.0.0.1", 0), Handler)
        serving = threading.Thread(target=server.serve_forever)
        serving.start()
        client = socket.create_connection(server.server_address, timeout=5)
        closer = None
        try:
            client.sendall(b"GET / HTTP/1.0\r\n\r\n")
            self.assertTrue(started.wait(5))
            server.shutdown()
            serving.join(5)
            closer = threading.Thread(target=server.server_close)
            closer.start()
            closer.join(0.05)
            self.assertTrue(closer.is_alive(), "close must wait for active handler")
            release.set()
            closer.join(5)
            self.assertFalse(closer.is_alive())
            self.assertTrue(finished.is_set())
        finally:
            release.set()
            client.close()
            server.shutdown()
            server.server_close()
            serving.join(5)
            if closer:
                closer.join(5)

    def test_shared_templates_tls_and_websocket_listeners(self):
        checkout = Path(os.environ["ORYN_WPT_CHECKOUT"])
        with tempfile.TemporaryFile(mode="w+") as errors:
            process = subprocess.Popen([str(checkout / "_venv3/bin/python"),
                str(ROOT / "scripts/wptserve-helper.py"), "--checkout", str(checkout),
                "--public-port", "8100", "--public-port-2", "8101",
                "--public-https-port", "8443", "--public-https-port-2", "8444"],
                stdout=subprocess.PIPE, stderr=errors, text=True)
            try:
                self.assertTrue(select.select([process.stdout], [], [], 20)[0])
                metadata = json.loads(process.stdout.readline())
                probe = subprocess.run([str(checkout / "_venv3/bin/python"),
                    str(ROOT / "scripts/wpt-h2-probe.py"), "--port", str(metadata["h2_port"])],
                    capture_output=True, text=True, timeout=20)
                self.assertEqual(probe.returncode, 0, probe.stderr)
                self.assertTrue(json.loads(probe.stdout)["extended_connect"])
                wrong_transport = subprocess.run([str(checkout / "_venv3/bin/python"),
                    str(ROOT / "scripts/wpt-h2-probe.py"), "--port", str(metadata["https_port"])],
                    capture_output=True, text=True, timeout=20)
                self.assertNotEqual(wrong_transport.returncode, 0)
                self.assertIn("HTTP/2 ALPN required", wrong_transport.stderr)
                checks = RUNNER.verify_wpt_helpers(metadata["port"], metadata["https_port"])
                self.assertEqual(len(checks), 16)
                connection = http.client.HTTPConnection("127.0.0.1", metadata["port"], timeout=5)
                try:
                    for path, expected in [
                        ("/url/historical.any.worker.html?variant=one", b'new Worker("/url/historical.any.worker.js?variant=one")'),
                        ("/url/historical.any.worker.js", b'importScripts("/url/historical.any.js")'),
                        ("/url/historical.any.html", b'GLOBAL'),
                    ]:
                        connection.request("GET", path, headers={"Host": "www1.localhost:8100"})
                        response = connection.getresponse()
                        body = response.read()
                        self.assertEqual(response.status, 200, body[:200])
                        self.assertIn(expected, body)
                    connection.request("POST", "/common/dispatcher/dispatcher.py?uuid=85323643-1b53-46de-b182-f4314669689c",
                                       body=b"across-origins", headers={"Origin": "https://alt.localhost:8443"})
                    response = connection.getresponse()
                    body = response.read()
                    self.assertEqual(response.status, 200, body.decode(errors="replace"))
                    self.assertEqual(body, b"done")
                    self.assertEqual(response.getheader("Access-Control-Allow-Origin"), "https://alt.localhost:8443")
                    connection.request("GET", "/common/dispatcher/dispatcher.py?uuid=85323643-1b53-46de-b182-f4314669689c",
                                       headers={"Host": "alt.localhost:8101"})
                    response = connection.getresponse()
                    self.assertEqual(response.read(), b"across-origins")
                finally:
                    connection.close()
                for scheme, ports in metadata["websocket_ports"].items():
                    with self.subTest(scheme=scheme):
                        connection = socket.create_connection(("127.0.0.1", ports[0]), timeout=5)
                        if scheme == "wss":
                            connection = ssl._create_unverified_context().wrap_socket(connection, server_hostname="localhost")
                        with connection:
                            connection.sendall((f"GET /echo HTTP/1.1\r\nHost: localhost:{ports[0]}\r\n"
                                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                                "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                                "Sec-WebSocket-Version: 13\r\n\r\n").encode())
                            self.assertIn(b"101", connection.recv(4096).split(b"\r\n")[0])
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                process.stdout.close()
            errors.seek(0)
            self.assertNotIn("Error in sourcing handler", errors.read())


if __name__ == "__main__":
    unittest.main()

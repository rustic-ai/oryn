#!/usr/bin/env python3
"""Run pinned wptserve privately behind Oryn's conformance harness."""

from __future__ import annotations

import argparse
import json
import logging
import signal
import socket
import ssl
import sys
import threading
import uuid
from contextlib import ExitStack
from pathlib import Path


def free_loopback_port() -> int:
    probe = socket.socket()
    probe.bind(("127.0.0.1", 0))
    port = int(probe.getsockname()[1])
    probe.close()
    return port


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--public-port", type=int, required=True)
    parser.add_argument("--public-https-port", type=int, required=True)
    parser.add_argument("--public-port-2", type=int, required=True)
    parser.add_argument("--public-https-port-2", type=int, required=True)
    parser.add_argument("--direct", action="store_true")
    parser.add_argument("--vendor-adapter", type=Path)
    parser.add_argument("--h2-port", type=int)
    parser.add_argument("--ws-port", type=int, default=0)
    parser.add_argument("--wss-port", type=int, default=0)
    parser.add_argument("--tls-certificate", type=Path)
    parser.add_argument("--tls-key", type=Path)
    args = parser.parse_args()
    if args.direct and args.vendor_adapter is None:
        parser.error('--direct requires --vendor-adapter')
    checkout = args.checkout.resolve()
    certificate = args.tls_certificate or checkout / 'tools/certs/web-platform.test.pem'
    key = args.tls_key or checkout / 'tools/certs/web-platform.test.key'
    sys.path.insert(0, str(checkout / "tools"))
    sys.path.insert(0, str(checkout / "tools/wptserve"))
    sys.path.insert(0, str(checkout / "tools/third_party/pywebsocket3"))

    try:
        import localpaths  # Upstream's pinned Python dependency paths.
        from serve.serve import RoutesBuilder, rewrites, _subdomains, _not_subdomains
        import wptserve
        from wptserve.config import ConfigBuilder
        from wptserve.stash import StashServer
        from pywebsocket3 import standalone as websocket
    except ModuleNotFoundError as error:
        raise SystemExit(
            "pinned wptserve dependencies are missing; run "
            f"{checkout / '_venv3/bin/pip'} install -e {checkout / 'tools/wptserve'}"
        ) from error

    logger = logging.getLogger("oryn-wptserve")
    class GracefulTlsWebTestServer(wptserve.server.WebTestServer):
        def shutdown_request(self, request):
            # Python's TCPServer closes SSLSocket without close_notify. For
            # upstream close-delimited responses (no Content-Length), strict
            # TLS clients correctly treat that as truncation. Preserve the
            # upstream HTTP bytes and terminate the fixture's TLS transport.
            if isinstance(request, ssl.SSLSocket):
                try:
                    request.settimeout(1)
                    request = request.unwrap()
                except (OSError, ValueError):
                    pass  # A peer may already have aborted the connection.
            super().shutdown_request(request)
    internal_port = args.public_port if args.direct else free_loopback_port()
    h2_port = args.h2_port or free_loopback_port()
    stopped = threading.Event()

    websocket_servers = []
    websocket_threads = []
    websocket_ports = {}
    def close_websocket(instance, thread):
        instance.shutdown()
        instance.server_close()
        thread.join(timeout=5)

    with StashServer(None, authkey=uuid.uuid4().hex), ExitStack() as cleanup:
        for scheme in ("ws", "wss"):
            command = ["-H", "127.0.0.1", "-p", str(args.ws_port if scheme == 'ws' else args.wss_port), "-d", str(checkout),
                       "-w", str(checkout / "websockets/handlers")]
            if scheme == "wss":
                command += ["--tls", "--private-key", str(key), "--certificate", str(certificate)]
            options, _ = websocket._parse_args_and_config(command)
            options.cgi_directories = []
            options.is_executable_method = None
            instance = websocket.WebSocketServer(options)
            websocket_servers.append(instance)
            websocket_ports[scheme] = [instance._sockets[0][0].getsockname()[1]]
            thread = threading.Thread(target=instance.serve_forever, daemon=True)
            thread.start()
            cleanup.callback(close_websocket, instance, thread)
            websocket_threads.append(thread)
        with ConfigBuilder(
            logger,
            browser_host="localhost",
            alternate_hosts={"alt": "alt.localhost"},
            subdomains=_subdomains,
            not_subdomains=_not_subdomains,
            ports={"http": [args.public_port, args.public_port_2],
                   "https": [args.public_https_port, args.public_https_port_2],
                   "h2": [h2_port], **websocket_ports},
            check_subdomains=False,
            doc_root=str(checkout),
            bind_address="127.0.0.1",
            ssl={"type": "pregenerated", "pregenerated": {
                "host_key_path": str(key),
                "host_cert_path": str(certificate),
            }},
        ) as config:
            routes = RoutesBuilder().get_routes()
            if args.direct:
                def adapter(path):
                    payload = path.read_bytes()
                    def serve(request, response):
                        return [(b"Content-Type", b"application/javascript"),
                                (b"Cache-Control", b"no-store")], payload
                    return wptserve.handlers.FunctionHandler(serve)
                routes = [("GET", "/resources/testharnessreport.js",
                           adapter(Path(__file__).with_name('wpt-report.js'))),
                          ("GET", "/resources/testdriver-vendor.js",
                           adapter(args.vendor_adapter))] + routes
            server = wptserve.WebTestHttpd(
                host="127.0.0.1",
                port=internal_port,
                use_ssl=False,
                certificate=None,
                doc_root=str(checkout),
                routes=routes,
                rewrites=rewrites if args.direct else [
                    (
                        "GET",
                        "/resources/WebIDLParser.js",
                        "/resources/webidl2/lib/webidl2.js",
                    )
                ],
                config=config,
            )

            def stop(_signum: int, _frame: object) -> None:
                stopped.set()

            signal.signal(signal.SIGTERM, stop)
            signal.signal(signal.SIGINT, stop)
            server.start()
            cleanup.callback(server.stop)
            tls_server = wptserve.WebTestHttpd(
                server_cls=GracefulTlsWebTestServer,
                host="127.0.0.1", port=args.public_https_port if args.direct else free_loopback_port(), use_ssl=True,
                certificate=str(certificate),
                key_file=str(key),
                doc_root=str(checkout), config=config,
                routes=routes, rewrites=rewrites,
            )
            tls_server.start()
            cleanup.callback(tls_server.stop)
            if args.direct:
                for port, secure in ((args.public_port_2, False), (args.public_https_port_2, True)):
                    secondary = wptserve.WebTestHttpd(
                        server_cls=GracefulTlsWebTestServer if secure else None,
                        host='127.0.0.1', port=port, use_ssl=secure,
                        certificate=str(certificate) if secure else None,
                        key_file=str(key) if secure else None,
                        doc_root=str(checkout), config=config, routes=routes, rewrites=rewrites)
                    secondary.start()
                    cleanup.callback(secondary.stop)
            h2_server = wptserve.WebTestHttpd(
                host="127.0.0.1", port=h2_port, use_ssl=True, http2=True,
                handler_cls=wptserve.server.Http2WebTestRequestHandler,
                certificate=str(certificate),
                key_file=str(key),
                doc_root=str(checkout), ws_doc_root=str(checkout / "websockets/handlers"),
                config=config, routes=routes, rewrites=rewrites,
            )
            h2_server.start()
            cleanup.callback(h2_server.stop)
            print(json.dumps({"port": internal_port, "https_port": tls_server.port,
                              "h2_port": h2_port,
                              "websocket_ports": websocket_ports}), flush=True)
            stopped.wait()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

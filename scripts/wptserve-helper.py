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
import time
import uuid
from contextlib import ExitStack
from pathlib import Path
from urllib.parse import urlsplit


def http_diagnostic_writer(stream, limit=256):
    """Bounded metadata only; diagnostic failures never replace HTTP outcomes."""
    if not isinstance(limit,int) or isinstance(limit,bool) or not 1 <= limit <= 256:
        raise ValueError('invalid HTTP diagnostic limit')
    lock = threading.Lock()
    started = time.monotonic()
    count = 0

    def emit(event, **fields):
        nonlocal count
        try:
            if event not in ('accepted','request_line_wait','request_line_complete','headers_complete',
                             'handler_enter','handler_error','response_ready','dispatch_return','dispatch_error'):
                return
            with lock:
                if count > limit:
                    return
                record = dict(schema_version=1, kind='wpt_http_boundary', event=event,
                              at_ms=round((time.monotonic()-started)*1000))
                if count == limit:
                    record['event'] = 'records_truncated'
                    record['limit'] = limit
                else:
                    for key in ('server_port','peer_port','request_sequence','status'):
                        value = fields.get(key)
                        if isinstance(value,int) and not isinstance(value,bool) and 0 <= value <= 2**53-1:
                            record[key] = value
                    for key in ('valid','close_connection'):
                        if key in fields:
                            record[key] = fields[key] is True
                    method = fields.get('method')
                    if method in ('GET','POST','PUT','DELETE','HEAD','OPTIONS','PATCH','CONNECT','TRACE'):
                        record['method'] = method
                    if 'path' in fields:
                        parsed = urlsplit(str(fields['path']))
                        if (not parsed.scheme or parsed.scheme in ('http','https')) and parsed.path.startswith('/'):
                            # An astral code point needs two JSON \u escapes (12 bytes).
                            # 512 points leave room for every fixed field under 8 KiB.
                            record.update(path=parsed.path[:512], path_truncated=len(parsed.path)>512,
                                          query_present=bool(parsed.query), fragment_present=bool(parsed.fragment))
                        else:
                            record['opaque_target'] = True
                    error = fields.get('error_type')
                    if error is not None:
                        record['error_type'] = error if error in ('OSError','TimeoutError','ValueError','TypeError','ConnectionError','BrokenPipeError','ConnectionResetError') else 'Exception'
                count += 1
                stream.write('ORYN_WPT_HTTP_DIAGNOSTIC '+json.dumps(record,separators=(',',':'))+'\n')
                stream.flush()
        except Exception:
            pass
    return emit


def http_diagnostic_classes(server_base, handler_base, emit):
    """Observe the existing HTTP dispatch calls without changing their bytes."""
    def fields(handler):
        return dict(server_port=handler.server.server_port, peer_port=handler.client_address[1],
                    request_sequence=getattr(handler,'_diagnostic_sequence',0))

    def report(event, **values):
        try:
            emit(event,**values)
        except Exception:
            pass

    class DiagnosticServer(server_base):
        def get_request(self):
            request, address = super().get_request()
            report('accepted',server_port=self.server_port,peer_port=address[1])
            return request, address

    class DiagnosticHandler(handler_base):
        def get_request_line(self):
            self._diagnostic_sequence = getattr(self,'_diagnostic_sequence',0)+1
            report('request_line_wait',**fields(self))
            valid = super().get_request_line()
            report('request_line_complete',valid=valid,close_connection=self.close_connection,**fields(self))
            return valid

        def parse_request(self):
            valid = super().parse_request()
            report('headers_complete',valid=valid,method=getattr(self,'command',None),
                 path=getattr(self,'path',''),**fields(self))
            return valid

        def finish_handling(self, request, response, handler):
            def observed_handler(current_request,current_response):
                report('handler_enter',method=request.method,path=request.request_path,**fields(self))
                try:
                    result = handler(current_request,current_response)
                except BaseException as error:
                    report('handler_error',error_type=type(error).__name__,**fields(self))
                    raise
                report('response_ready',status=current_response.status[0],**fields(self))
                return result
            # Upstream uses these routing attributes before invoking the handler.
            for name in ('base_path','url_base'):
                if handler is not None and hasattr(handler,name):
                    setattr(observed_handler,name,getattr(handler,name))
            try:
                result = super().finish_handling(request,response,observed_handler if handler is not None else None)
            except BaseException as error:
                report('dispatch_error',error_type=type(error).__name__,**fields(self))
                raise
            report('dispatch_return',status=response.status[0],close_connection=self.close_connection,**fields(self))
            return result
    return DiagnosticServer, DiagnosticHandler


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
    parser.add_argument("--http-diagnostics", action="store_true")
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
    http_classes = {}
    if args.http_diagnostics:
        server_class, handler_class = http_diagnostic_classes(
            wptserve.server.WebTestServer, wptserve.server.Http1WebTestRequestHandler,
            http_diagnostic_writer(sys.stderr))
        http_classes = dict(server_cls=server_class,handler_cls=handler_class)
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
                **http_classes,
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

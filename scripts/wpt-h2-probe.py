#!/usr/bin/env python3
"""Independently qualify the pinned server's RFC 8441 WebSocket transport."""
import argparse
import json
import socket
import ssl
import time

from h2.config import H2Configuration
from h2.connection import H2Connection
from h2.events import DataReceived, ResponseReceived
from h2.settings import SettingCodes


def qualify(port, ca=None):
    context = ssl.create_default_context(cafile=ca) if ca else ssl._create_unverified_context()
    context.set_alpn_protocols(["h2"])
    with socket.create_connection(("127.0.0.1", port), timeout=5) as raw:
        with context.wrap_socket(raw, server_hostname="localhost") as sock:
            assert sock.selected_alpn_protocol() == "h2", "HTTP/2 ALPN required"
            conn = H2Connection(config=H2Configuration(client_side=True, header_encoding="utf-8"))
            conn.initiate_connection()
            sock.sendall(conn.data_to_send())
            deadline = time.monotonic() + 10
            def receive():
                assert time.monotonic() < deadline, "HTTP/2 probe deadline"
                data = sock.recv(65536)
                assert data, "HTTP/2 connection closed early"
                events = conn.receive_data(data)
                for event in events:
                    if isinstance(event, DataReceived):
                        conn.acknowledge_received_data(event.flow_controlled_length, event.stream_id)
                pending = conn.data_to_send()
                if pending:
                    sock.sendall(pending)
                return events
            while conn.remote_settings[SettingCodes.ENABLE_CONNECT_PROTOCOL] != 1:
                receive()
            conn.send_headers(1, [(":method", "CONNECT"), (":protocol", "websocket"),
                (":scheme", "https"), (":authority", f"localhost:{port}"),
                (":path", "/echo"), ("sec-websocket-version", "13")])
            sock.sendall(conn.data_to_send())
            status = None
            while status is None:
                for event in receive():
                    if isinstance(event, ResponseReceived):
                        status = dict(event.headers)[":status"]
            assert status == "200", f"extended CONNECT failed: {status}"
            def send_frame(opcode, payload):
                mask = b"wpt!"
                masked = bytes(byte ^ mask[i % 4] for i, byte in enumerate(payload))
                conn.send_data(1, bytes([0x80 | opcode, 0x80 | len(payload)]) + mask + masked)
                sock.sendall(conn.data_to_send())
            send_frame(1, b"hello")
            payload = b""
            while b"\x81\x05hello" not in payload:
                for event in receive():
                    if isinstance(event, DataReceived):
                        payload += event.data
            send_frame(8, b"\x03\xe8")
            payload = b""
            while b"\x88\x02\x03\xe8" not in payload:
                for event in receive():
                    if isinstance(event, DataReceived):
                        payload += event.data
            conn.close_connection()
            sock.sendall(conn.data_to_send())
    return {"alpn": "h2", "extended_connect": True, "echo": True, "close": True}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ca")
    args = parser.parse_args()
    print(json.dumps(qualify(args.port, args.ca)))

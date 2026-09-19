"""Wire/error regressions with in-memory peers; no fixture service execution."""
import base64
import hashlib
from pathlib import Path
import struct
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_topology_probe as probe


class Stream:
    def __init__(self, data=b''): self.data = data; self.sent = []
    def recv(self, size):
        chunk, self.data = self.data[:size], self.data[size:]
        return chunk
    def sendall(self, data): self.sent.append(data)


class TopologyProbeTests(unittest.TestCase):
    def test_truncated_and_oversize_server_frames_fail(self):
        for data in (b'\x81\x05hi', b'\x81\xff', b'\x01\x01x',
                     b'\x81\x7f' + struct.pack('!Q', probe.LIMIT + 1)):
            with self.subTest(data=data), self.assertRaises(ValueError): probe.frame(Stream(data))

    def test_client_frames_are_masked_and_round_trip_payload(self):
        stream = Stream()
        with patch.object(probe.os, 'urandom', return_value=b'abcd'):
            probe.send_frame(stream, 1, b'echo')
        value = stream.sent[0]
        self.assertEqual(value[:2], b'\x81\x84')
        self.assertEqual(bytes(value[6 + n] ^ value[2 + n % 4] for n in range(4)), b'echo')

    def test_echo_and_close_require_exact_upstream_responses(self):
        key_bytes = b'a' * 16
        key = base64.b64encode(key_bytes).decode()
        accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        payload = ('oryn-topology-' + key_bytes.hex()).encode()
        header = ('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n').encode()
        close = b'\x88\x02' + struct.pack('!H', 1000)
        with patch.object(probe.os, 'urandom', side_effect=lambda size: b'a' * size):
            record = probe.websocket(Stream(header + bytes([0x81, len(payload)]) + payload + close), 8888)
            self.assertEqual(record['request_sha256'], record['response_sha256'])
            for bad in (header.replace(accept.encode(), b'bad'), header + b'\x81\x01x',
                        header + bytes([0x81, len(payload)]) + payload + b'\x88\x00'):
                with self.assertRaises(ValueError): probe.websocket(Stream(bad), 8888)

    def test_metadata_only_or_wrong_helper_cannot_pass_observation_validation(self):
        row = {'protocol': 'https', 'helper_path': probe.HELPER,
               'response_sha256': 'a' * 64, 'certificate_sha256': 'b' * 64}
        probe.validate_observations({'endpoints': [row]}, 'a' * 64)
        for changes in ({'response_sha256': 'c' * 64}, {'certificate_sha256': None}, {'helper_path': '/elsewhere'}):
            with self.assertRaises(ValueError): probe.validate_observations({'endpoints': [dict(row, **changes)]}, 'a' * 64)
        with self.assertRaises(ValueError):
            probe.validate_observations({'endpoints': [{'protocol': 'ws', 'helper_response_verified': True}]}, 'a' * 64)

    def test_http_probe_records_observed_peer_and_rejects_wrong_body(self):
        import io
        class Peer(Stream):
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def settimeout(self, timeout): pass
            def getpeername(self): return ('192.168.1.7', 8000)
            def makefile(self, *args): return io.BytesIO(self.data)
        route = {'protocol': 'http-local', 'port': 8000, 'connect_ip': '192.168.1.7'}
        raw = Peer(b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nPNG')
        with patch.object(probe.socket, 'create_connection', return_value=raw) as connect:
            row = probe.probe(route, 'unused-ca', b'PNG')
            self.assertEqual(row['peer_ip'], '192.168.1.7')
            self.assertTrue(row['helper_response_verified'])
            connect.assert_called_once_with(('192.168.1.7', 8000), timeout=10)
        for wire in (b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad',
                     b'HTTP/1.1 404 Missing\r\nContent-Length: 0\r\n\r\n'):
            with patch.object(probe.socket, 'create_connection', return_value=Peer(wire)), self.assertRaises(ValueError):
                probe.probe(route, 'unused-ca', b'PNG')

    def test_tls_probe_uses_real_name_and_requires_h2_alpn(self):
        from unittest.mock import MagicMock
        raw = MagicMock(); raw.__enter__.return_value = raw
        raw.getpeername.return_value = ('127.0.0.1', 8443)
        secure = MagicMock(); secure.getpeercert.return_value = b'certificate'
        secure.selected_alpn_protocol.return_value = 'http/1.1'
        context = MagicMock(); context.wrap_socket.return_value = secure
        with patch.object(probe.socket, 'create_connection', return_value=raw), patch.object(probe.ssl, 'create_default_context', return_value=context) as factory:
            with self.assertRaisesRegex(ValueError, 'negotiate HTTP/2'):
                probe.probe({'protocol': 'h2', 'port': 8443, 'connect_ip': '127.0.0.1'}, '/test-ca.pem', b'PNG')
            factory.assert_called_once_with(cafile='/test-ca.pem')
            context.wrap_socket.assert_called_once_with(raw, server_hostname='web-platform.test')
            context.set_alpn_protocols.assert_called_once_with(['h2'])
            secure.close.assert_called_once()

    def test_http2_completion_flow_and_reset_are_not_interchangeable(self):
        from h2.config import H2Configuration
        from h2.connection import H2Connection
        from h2.events import RequestReceived
        class H2Peer:
            def __init__(self, reset=False):
                self.server = H2Connection(config=H2Configuration(client_side=False))
                self.server.initiate_connection(); self.reset = reset
            def sendall(self, data):
                for event in self.server.receive_data(data):
                    if isinstance(event, RequestReceived):
                        if self.reset: self.server.reset_stream(event.stream_id)
                        else:
                            self.server.send_headers(event.stream_id, [(':status', '200')])
                            self.server.send_data(event.stream_id, b'PNG', end_stream=True)
            def recv(self, size): return self.server.data_to_send(size)
        self.assertEqual(probe.http2(H2Peer(), 8443), b'PNG')
        with self.assertRaisesRegex(ValueError, 'reset'):
            probe.http2(H2Peer(reset=True), 8443)
        with self.assertRaisesRegex(ValueError, 'before helper completion'):
            probe.http2(Stream(), 8443)

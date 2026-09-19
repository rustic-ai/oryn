"""Bounded real wire probes; configuration alone never counts as observation."""
import base64
import hashlib
import http.client
import json
import os
import socket
import ssl
import struct
from pathlib import Path

HOST = 'web-platform.test'
HELPER = '/common/square.png'
ECHO = '/echo'
LIMIT = 1024 * 1024


def exact(stream, count):
    result = b''
    while len(result) < count:
        part = stream.recv(count - len(result))
        if not part: raise ValueError('truncated fixture response')
        result += part
    return result


def frame(stream):
    first, second = exact(stream, 2)
    if first & 0x70 or not first & 0x80 or second & 0x80:
        raise ValueError('fragmented, extended or masked server frame')
    size = second & 127
    if size == 126: size = struct.unpack('!H', exact(stream, 2))[0]
    elif size == 127: size = struct.unpack('!Q', exact(stream, 8))[0]
    if size > LIMIT: raise ValueError('fixture websocket frame exceeds limit')
    return first & 15, exact(stream, size)


def send_frame(stream, opcode, payload):
    if len(payload) >= 126: raise ValueError('control probe payload too large')
    mask = os.urandom(4)
    stream.sendall(bytes((0x80 | opcode, 0x80 | len(payload))) + mask +
                   bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload)))


def websocket(stream, port):
    key = base64.b64encode(os.urandom(16)).decode()
    stream.sendall((f'GET {ECHO} HTTP/1.1\r\nHost: {HOST}:{port}\r\nUpgrade: websocket\r\n'
                   f'Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n').encode())
    header = b''
    while not header.endswith(b'\r\n\r\n'):
        if len(header) >= 16384: raise ValueError('oversize websocket headers')
        header += exact(stream, 1)
    lines = header.decode('ascii').split('\r\n')
    if lines[0].split(' ', 2)[1] != '101': raise ValueError('websocket handshake rejected')
    headers = {}
    for line in lines[1:]:
        if line:
            name, value = line.split(':', 1)
            if name.lower() in headers: raise ValueError('duplicate websocket header')
            headers[name.lower()] = value.strip()
    accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
    if (headers.get('sec-websocket-accept') != accept or headers.get('upgrade', '').lower() != 'websocket'
            or 'upgrade' not in headers.get('connection', '').lower().split(', ')):
        raise ValueError('invalid websocket handshake')
    payload = ('oryn-topology-' + os.urandom(16).hex()).encode()
    send_frame(stream, 1, payload)
    opcode, observed = frame(stream)
    if opcode != 1 or observed != payload: raise ValueError('fixture websocket echo mismatch')
    send_frame(stream, 8, struct.pack('!H', 1000))
    opcode, close = frame(stream)
    if opcode != 8 or close != struct.pack('!H', 1000): raise ValueError('fixture websocket close mismatch')
    return {'request_sha256': hashlib.sha256(payload).hexdigest(),
            'response_sha256': hashlib.sha256(observed).hexdigest(), 'close_code': 1000}


def http2(stream, port):
    from h2.connection import H2Connection
    from h2.events import ResponseReceived, DataReceived, StreamEnded, StreamReset, ConnectionTerminated
    connection = H2Connection(); connection.initiate_connection()
    connection.send_headers(1, [(':method', 'GET'), (':scheme', 'https'),
                               (':authority', f'{HOST}:{port}'), (':path', HELPER)], end_stream=True)
    stream.sendall(connection.data_to_send())
    body = bytearray(); status = None
    for _ in range(4096):
        data = stream.recv(65536)
        if not data: raise ValueError('HTTP/2 ended before helper completion')
        ended = False
        for event in connection.receive_data(data):
            if isinstance(event, (StreamReset, ConnectionTerminated)): raise ValueError('HTTP/2 fixture reset')
            if isinstance(event, ResponseReceived):
                status = dict(event.headers).get(b':status')
            if isinstance(event, DataReceived):
                body.extend(event.data)
                if len(body) > LIMIT: raise ValueError('oversize HTTP/2 fixture helper')
                connection.acknowledge_received_data(event.flow_controlled_length, event.stream_id)
            if isinstance(event, StreamEnded) and event.stream_id == 1: ended = True
        pending = connection.data_to_send()
        if pending: stream.sendall(pending)
        if ended:
            if status != b'200': raise ValueError('HTTP/2 fixture helper status')
            return bytes(body)
    raise ValueError('HTTP/2 fixture event limit')


def probe(route, ca, expected):
    protocol = route['protocol']; port = route['port']
    row = dict(protocol=protocol, port=port, helper_response_verified=False)
    with socket.create_connection((route['connect_ip'], port), timeout=10) as raw:
        raw.settimeout(10)
        row['peer_ip'] = raw.getpeername()[0]
        stream = raw
        if protocol.startswith(('https', 'h2', 'wss')):
            context = ssl.create_default_context(cafile=str(ca))
            context.set_alpn_protocols(['h2'] if protocol == 'h2' else ['http/1.1'])
            stream = context.wrap_socket(raw, server_hostname=HOST)
            row.update(certificate_verified=True, certificate_sha256=hashlib.sha256(stream.getpeercert(binary_form=True)).hexdigest())
        try:
            if protocol in ('ws', 'wss'):
                row.update(websocket(stream, port)); row['helper_path'] = ECHO
            else:
                if protocol == 'h2':
                    if stream.selected_alpn_protocol() != 'h2': raise ValueError('fixture did not negotiate HTTP/2')
                    body = http2(stream, port)
                else:
                    stream.sendall(f'GET {HELPER} HTTP/1.1\r\nHost: {HOST}:{port}\r\nConnection: close\r\n\r\n'.encode())
                    response = http.client.HTTPResponse(stream); response.begin()
                    if response.status != 200: raise ValueError('fixture helper status is not 200')
                    body = response.read(LIMIT + 1)
                if body != expected: raise ValueError('fixture helper bytes differ from pinned source')
                row.update(helper_path=HELPER, response_sha256=hashlib.sha256(body).hexdigest())
            row['helper_response_verified'] = True
        finally:
            if stream is not raw: stream.close()
    return row


def validate_observations(record, helper_sha256):
    """Independently check transport detail before fixture contract admission."""
    for row in record.get('endpoints', []):
        if row['protocol'] in ('ws', 'wss'):
            request = row.get('request_sha256')
            if (not isinstance(request, str) or len(request) != 64
                    or any(char not in '0123456789abcdef' for char in request)
                    or row.get('response_sha256') != request or row.get('close_code') != 1000
                    or row.get('helper_path') != ECHO):
                raise ValueError('websocket echo/close evidence incomplete')
        elif row.get('response_sha256') != helper_sha256 or row.get('helper_path') != HELPER:
            raise ValueError('HTTP helper evidence differs from pinned source')
        if row['protocol'].startswith(('https', 'h2', 'wss')):
            certificate = row.get('certificate_sha256')
            if (not isinstance(certificate, str) or len(certificate) != 64
                    or any(char not in '0123456789abcdef' for char in certificate)):
                raise ValueError('missing observed TLS leaf identity')

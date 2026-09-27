"""Pinned transport/teardown negative controls; no browser process is required."""
import io
import json
import os
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from oryn_wptrunner import custody
from oryn_wptrunner.protocol import AuthenticatedTransport, OrynProtocol, CheckedSession
from webdriver.transport import HTTPWireProtocol


def checked_session(command, close=lambda: None):
    # Retain original negative controls while making isolated cleanup explicit.
    wire = SimpleNamespace(host='127.0.0.1', port=4444, close=lambda: None)
    transport = SimpleNamespace(retire=lambda: None, settlement_timeout=5,
                                run_isolated=lambda operation: operation(wire), close=close)
    return SimpleNamespace(session_id='owned', send_command=command, transport=transport,
                           _lifecycle=threading.RLock())


def end_checked(session):
    cleanup = SimpleNamespace(send_command=session.send_command, transport=None)
    with patch('oryn_wptrunner.protocol.Session', return_value=cleanup):
        CheckedSession.end(session)


class ProtocolFaultTests(unittest.TestCase):
    def test_malformed_command_response_reaches_real_upstream_decoder(self):
        response = io.BytesIO(b'{not-json')
        response.status = 200
        response.getheaders = lambda: [('Content-Type', 'application/json')]
        transport = AuthenticatedTransport('127.0.0.1', 4444, 'a' * 64)
        with patch.object(HTTPWireProtocol, '_request', return_value=response):
            with self.assertRaisesRegex(ValueError, 'decode response body'):
                transport.send('GET', '/status', None)

    def test_failed_native_teardown_is_logged_and_propagated(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'custody.jsonl'
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(descriptor)
            def fail(): raise RuntimeError('intentional teardown failure')
            with patch.dict(os.environ, {'ORYN_WPT_PROTOCOL_CUSTODY': str(path),
                                         'ORYN_WPT_CANDIDATE_SHA256': 'a' * 64}):
                identity = custody.begin()
                protocol = SimpleNamespace(webdriver=SimpleNamespace(end=fail), _custody_session=identity)
                with self.assertRaisesRegex(RuntimeError, 'teardown failure'):
                    OrynProtocol.teardown(protocol)
            records = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual([row['phase'] for row in records], ['connect', 'teardown'])
            self.assertFalse(records[-1]['success'])
            self.assertEqual(records[-1]['error_type'], 'RuntimeError')
            self.assertNotIn('intentional teardown failure', path.read_text())
            self.assertIsNone(protocol.webdriver)

    def test_custody_cannot_be_redirected_to_a_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'target'; target.write_text('unchanged')
            link = Path(directory) / 'link'; link.symlink_to(target)
            with patch.dict(os.environ, {'ORYN_WPT_PROTOCOL_CUSTODY': str(link),
                                         'ORYN_WPT_CANDIDATE_SHA256': 'a' * 64}):
                with self.assertRaises(OSError): custody.begin()
            self.assertEqual(target.read_text(), 'unchanged')


    def test_checked_session_requires_native_receipt_for_already_retired_session(self):
        from webdriver.error import InvalidSessionIdException
        for status in ('complete', 'pending', 'failed'):
            def command(method, path):
                if method == 'DELETE': raise InvalidSessionIdException('already closed')
                self.assertEqual(path, 'session/owned/oryn/diagnostics/retirement')
                return {'schema_version': 1, 'session_id': 'owned', 'status': status}
            session = checked_session(command)
            if status == 'complete': end_checked(session)
            else:
                with self.assertRaisesRegex(ValueError, 'not completed'): end_checked(session)
            self.assertIsNone(session.session_id)
        def disconnected(*args): raise OSError('connection lost')
        session = checked_session(disconnected)
        with self.assertRaises(OSError): end_checked(session)

    def test_checked_session_preserves_delete_error_when_transport_close_fails(self):
        primary = OSError('native DELETE failed')
        calls = []
        def command(method, path):
            calls.append((method, path))
            raise primary
        def close():
            raise RuntimeError('transport cleanup failed')
        session = checked_session(command, close=close)
        with self.assertRaises(OSError) as caught:
            end_checked(session)
        self.assertIs(caught.exception, primary)
        self.assertEqual(calls, [('DELETE', 'session/owned')])
        self.assertIsNone(session.session_id)
        self.assertTrue(any('transport cleanup failed' in note for note in primary.__notes__))

    def test_checked_session_failure_reaches_exact_protocol_terminal_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'custody.jsonl'
            descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            os.close(descriptor)
            primary = OSError('native DELETE failed')
            calls = []
            def command(method, endpoint):
                calls.append((method, endpoint)); raise primary
            session = checked_session(command)
            session.end = lambda:end_checked(session)
            with patch.dict(os.environ, {'ORYN_WPT_PROTOCOL_CUSTODY':str(path), 'ORYN_WPT_CANDIDATE_SHA256':'a'*64}):
                identity = custody.begin()
                protocol = SimpleNamespace(webdriver=session, _custody_session=identity)
                with self.assertRaises(OSError) as caught:
                    OrynProtocol.teardown(protocol)
            self.assertIs(caught.exception, primary)
            records = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual([row['phase'] for row in records], ['connect','teardown'])
            self.assertFalse(records[-1]['success'])
            self.assertEqual(records[-1]['error_type'], 'OSError')
            self.assertEqual(calls, [('DELETE','session/owned')])
            self.assertIsNone(protocol.webdriver)

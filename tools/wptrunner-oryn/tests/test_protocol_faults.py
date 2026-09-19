"""Pinned transport/teardown negative controls; no browser process is required."""
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from oryn_wptrunner import custody
from oryn_wptrunner.protocol import AuthenticatedTransport, OrynProtocol, CheckedSession
from webdriver.transport import HTTPWireProtocol


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
            session = SimpleNamespace(session_id='owned', send_command=command,
                                      transport=SimpleNamespace(close=lambda: None))
            if status == 'complete': CheckedSession.end(session)
            else:
                with self.assertRaisesRegex(ValueError, 'not completed'): CheckedSession.end(session)
            self.assertIsNone(session.session_id)
        def disconnected(*args): raise OSError('connection lost')
        session = SimpleNamespace(session_id='owned', send_command=disconnected,
                                  transport=SimpleNamespace(close=lambda: None))
        with self.assertRaises(OSError): CheckedSession.end(session)

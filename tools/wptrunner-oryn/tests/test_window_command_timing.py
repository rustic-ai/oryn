import unittest
from unittest.mock import patch

from oryn_wptrunner.protocol import CheckedSession


class WindowCommandTimingTests(unittest.TestCase):
    def test_command_instrumentation_preserves_request_result_and_exception(self):
        session = CheckedSession('127.0.0.1', 1234)
        session.session_id = 'example'
        for method, uri in [('POST', 'window/new'), ('POST', 'window'),
                ('POST', 'execute/async'), ('POST', 'execute/sync'),
                ('POST', 'element/abc/click'), ('DELETE', 'actions'), ('DELETE', 'window')]:
            with self.subTest(uri=uri), patch.object(session, 'send_command', return_value='result') as send, \
                    patch('oryn_wptrunner.protocol.phase') as phase:
                body = {'key': 'value'}
                self.assertEqual(session.send_session_command(method, uri, body, 7), 'result')
                send.assert_called_once_with(method, 'session/example/' + uri, body, 7)
                phase.return_value.__exit__.assert_called_once_with(None, None, None)
                send.side_effect = ValueError('native failure')
                with self.assertRaisesRegex(ValueError, 'native failure'):
                    session.send_session_command(method, uri, body, 7)
                self.assertEqual(phase.return_value.__exit__.call_args.args[0], ValueError)
        session._ending = True

    def test_retired_session_does_not_send_profiled_commands(self):
        session = CheckedSession('127.0.0.1', 1234); session._ending = True
        with patch.object(session, 'send_command') as send, self.assertRaisesRegex(Exception, 'ending'):
            session.send_session_command('POST', 'window/new')
        send.assert_not_called()

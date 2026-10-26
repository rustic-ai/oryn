import unittest
from types import SimpleNamespace
from unittest.mock import patch
from oryn_wptrunner.completion_diagnostics import capture


class CompletionDiagnosticsTests(unittest.TestCase):
    def test_encoding_population_counts_are_opt_in_and_validated(self):
        for span_count, test_count, expected in (
            (13459, 636.0, '"encoding_test_count": 636'),
            (True, -1, None),
            (1000001, 'private payload', None),
            (1.5, float('nan'), None),
        ):
            logs = []
            commands = []
            def send(method, uri, body=None, timeout=None):
                commands.append(uri)
                if uri == 'window' and method == 'GET':
                    return 'original'
                if uri == 'window/handles':
                    return ['child']
                if uri == 'execute/sync':
                    return {'encoding_span_count': span_count,
                            'encoding_test_count': test_count}
            protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                       logger=SimpleNamespace(info=logs.append))
            with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
                capture(protocol, '/encoding/legacy-mb-japanese/euc-jp/eucjp-decode-cseucpkdfmtjapanese.html?1001-2000')
            self.assertIn('execute/sync', commands)
            if expected is not None:
                self.assertIn(expected, logs[0])
                self.assertIn('"encoding_span_count": 13459', logs[0])
            else:
                self.assertNotIn('encoding_span_count', logs[0])
                self.assertNotIn('encoding_test_count', logs[0])
            self.assertNotIn('private payload', logs[0])

    def test_disabled_does_not_touch_driver(self):
        with patch.dict('os.environ', {}, clear=True):
            capture(SimpleNamespace(), '/FileAPI/BlobURL/cross-partition.https.html')

    def test_enabled_other_fixture_does_not_touch_driver(self):
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(SimpleNamespace(), '/other.html')
            capture(SimpleNamespace(), '/other.html?/FileAPI/BlobURL/cross-partition.https.html')
            capture(SimpleNamespace(), 'http://[invalid')

    def test_observation_is_bounded_and_restores_original(self):
        commands = []
        logs = []
        def send(method, uri, body=None, timeout=None):
            commands.append((method, uri, body, timeout))
            if (method, uri) == ('GET', 'window'):
                return 'original'
            if (method, uri) == ('GET', 'window/handles'):
                return ['secret-handle'] * 10
            if uri == 'execute/sync':
                return {'frame_count': 0.0, 'ready_state': 'private payload', 'extra': 'private'}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(protocol, '/FileAPI/BlobURL/cross-partition.https.html')
        self.assertEqual(sum(uri == 'execute/sync' for _, uri, _, _ in commands), 4)
        self.assertEqual(commands[-1][2], {'handle': 'original'})
        self.assertTrue(all(timeout == 2 for _, _, _, timeout in commands))
        self.assertNotIn('secret-handle', logs[0])
        self.assertNotIn('private', logs[0])
        self.assertIn('"frame_count": 0', logs[0])

    def test_collection_failure_never_emits_exception_payload(self):
        def fail(*args, **kwargs):
            raise RuntimeError('private error payload')
        logs = []
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=fail),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(protocol, '/FileAPI/BlobURL/cross-partition.https.html')
        self.assertIn('RuntimeError', logs[0])
        self.assertNotIn('private error payload', logs[0])

    def test_failed_observation_still_attempts_restore_and_preserves_error(self):
        commands = []
        logs = []
        def send(method, uri, body=None, timeout=None):
            commands.append((method, uri, body))
            if (method, uri) == ('GET', 'window'):
                return 'original'
            if (method, uri) == ('GET', 'window/handles'):
                return ['child']
            if body == {'handle': 'child'}:
                return None
            raise RuntimeError('private payload')
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(protocol, '/FileAPI/BlobURL/cross-partition.https.html')
        self.assertEqual(commands[-1], ('POST', 'window', {'handle': 'original'}))
        self.assertIn('collection_error_type', logs[0])
        self.assertIn('restore_error_type', logs[0])
        self.assertNotIn('private payload', logs[0])

import unittest
from types import SimpleNamespace
from unittest.mock import patch
from oryn_wptrunner.completion_diagnostics import capture, capture_result


class CompletionDiagnosticsTests(unittest.TestCase):
    def test_returned_result_counts_are_payload_free_and_do_not_mutate_input(self):
        logs = []
        result = ['private-url', 2.0, 'private-message', 'private-stack', [
            ['private-name', code, 'private-message', 'private-stack']
            for code in (0.0, 0, 2.0, 3, 3.0)]]
        protocol = SimpleNamespace(logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture_result(protocol, '/FileAPI/BlobURL/cross-partition.https.html', result)
        self.assertIn('"subtest_codes": [2, 0, 1, 2, 0]', logs[0])
        self.assertIn('"harness_code": 2', logs[0])
        self.assertNotIn('private', logs[0])
        self.assertEqual(result[2], 'private-message')
        self.assertEqual(len(result[4]), 5)

    def test_result_diagnostic_is_opt_in_selected_and_rejects_malformed_codes(self):
        logs = []
        protocol = SimpleNamespace(logger=SimpleNamespace(info=logs.append))
        good = ['', 0, None, None, []]
        with patch.dict('os.environ', {}, clear=True):
            capture_result(protocol, '/FileAPI/BlobURL/cross-partition.https.html', good)
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture_result(protocol, '/other', good)
            for value in (None, ['', True, None, None, []],
                          ['', 0, None, None, [['', True, None, None]]],
                          ['', 0.5, None, None, []],
                          ['', float('nan'), None, None, []],
                          ['', 0, None, None, [['', 1.5, None, None]]],
                          ['', 0, None, None, [['', float('inf'), None, None]]],
                          ['', 0, None, None, [[]]],
                          ['', 0, None, None, [None] * 10001]):
                capture_result(protocol, '/FileAPI/BlobURL/cross-partition.https.html', value)
        self.assertEqual(logs, [])

    def test_result_logger_failure_does_not_replace_result(self):
        def fail(message):
            raise RuntimeError('private')
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture_result(SimpleNamespace(logger=SimpleNamespace(info=fail)),
                           '/FileAPI/BlobURL/cross-partition.https.html',
                           ['', 0, None, None, []])

    def test_child_realm_observation_restores_top_level_scope(self):
        commands = []
        logs = []
        selected_child = False
        def send(method, uri, body=None, timeout=None):
            nonlocal selected_child
            commands.append((uri, body))
            if uri == 'window' and method == 'GET':
                return 'original'
            if uri == 'window/handles':
                return ['popup']
            if uri == 'frame':
                selected_child = body['id'] == 0
            if uri == 'execute/sync':
                return {'frame_count': 0 if selected_child else 1,
                        'script_count': 5, 'executor_type': 'function',
                        'worker_path_present': True, 'token_function': False,
                        'ready_state': 'complete', 'uuid': 'private payload'}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(protocol, '/FileAPI/BlobURL/cross-partition.https.html')
        self.assertFalse(selected_child)
        self.assertIn(('frame', {'id': None}), commands)
        self.assertIn('"frame_index": 0', logs[0])
        self.assertIn('"worker_path_present": true', logs[0])
        self.assertNotIn('private payload', logs[0])

    def test_child_dispatcher_metadata_is_bounded_and_payload_free(self):
        logs = []
        def send(method, uri, body=None, timeout=None):
            if uri == 'window' and method == 'GET':
                return 'original'
            if uri == 'window/handles':
                return ['child']
            if uri == 'execute/sync':
                return {'child_dispatcher': {'accessible': True,
                    'script_count': 5.0, 'request_count': True,
                    'executor_type': 'function', 'receive_type': 'private payload',
                    'ready_state': 'complete', 'uuid': 'private payload'}}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
            capture(protocol, '/FileAPI/BlobURL/cross-partition.https.html')
        self.assertIn('"script_count": 5', logs[0])
        self.assertIn('"executor_type": "function"', logs[0])
        self.assertNotIn('request_count', logs[0])
        self.assertNotIn('private payload', logs[0])

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
                            'encoding_live_span_count': span_count,
                            'encoding_child_ready_state': 'loading' if expected else 'private payload',
                            'encoding_test_count': test_count}
            protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                       logger=SimpleNamespace(info=logs.append))
            with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_DIAGNOSTICS': '1'}):
                capture(protocol, '/encoding/legacy-mb-japanese/euc-jp/eucjp-decode-cseucpkdfmtjapanese.html?1001-2000')
            self.assertIn('execute/sync', commands)
            if expected is not None:
                self.assertIn(expected, logs[0])
                self.assertIn('"encoding_span_count": 13459', logs[0])
                self.assertIn('"encoding_live_span_count": 13459', logs[0])
                self.assertIn('"encoding_child_ready_state": "loading"', logs[0])
            else:
                self.assertNotIn('encoding_span_count', logs[0])
                self.assertNotIn('encoding_live_span_count', logs[0])
                self.assertNotIn('encoding_child_ready_state', logs[0])
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
                return {'frame_count': 0.0, 'ready_state': 'private payload', 'extra': 'private',
                        'body_present': True, 'iframe_task_parses': False,
                        'import_source_native': 'private payload'}
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
        self.assertIn('"body_present": true', logs[0])
        self.assertIn('"iframe_task_parses": false', logs[0])
        self.assertNotIn('import_source_native', logs[0])

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

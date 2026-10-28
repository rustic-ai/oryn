import unittest
import uuid
from types import SimpleNamespace
from unittest.mock import patch
from oryn_wptrunner.completion_replay import replay

URL = '/FileAPI/BlobURL/cross-partition.https.html'


class ReplayTests(unittest.TestCase):
    def test_load_probe_never_injects_fresh_task_and_restores_frame(self):
        logs, calls = [], []
        selected = None
        def send(method, uri, body=None, timeout=None):
            nonlocal selected
            calls.append((uri, body))
            if uri == 'window' and method == 'GET': return 'original'
            if uri == 'window/handles': return ['root', 'popup', 'other']
            if uri == 'window': selected = body['handle']
            if uri == 'execute/sync':
                if 'create_dedicated_worker_js' in body['script']:
                    return {'kind': 'worker', 'source': 'private task'}
                if 'script_count' in body['script']: return {'script_count': 5}
                return selected if selected != 'other' else 'popup'
            if uri == 'execute/async':
                self.assertNotIn('eval(', body['script'])
                if 'limiter_available' in body['script']: return {'limiter_available': True}
                if 'dispatchEvent' in body['script']:
                    return {'handler_present': True, 'script_count': 5, 'payload': 'private'}
                return {'frame_count': 1}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': 'load'}, clear=True):
            replay(protocol, URL)
        self.assertEqual(calls[-1], ('window', {'handle': 'original'}))
        self.assertIn(('frame', {'id': None}), calls)
        self.assertIn('"synthetic_load_probe": true', logs[0])
        self.assertIn('"other_child_after_load": true', logs[0])
        self.assertNotIn('private', logs[0])

    def test_successful_worker_replay_checks_reply_without_logging_queue_id(self):
        logs = []
        selected = None
        child = False
        def send(method, uri, body=None, timeout=None):
            nonlocal selected, child
            if uri == 'window' and method == 'GET': return 'original'
            if uri == 'window/handles': return ['root', 'popup']
            if uri == 'window': selected = body['handle']
            if uri == 'frame': child = body['id'] == 0
            if uri == 'execute/sync':
                if 'create_dedicated_worker_js' in body['script']:
                    return {'kind': 'worker', 'source': 'private source'}
                return selected
            if uri == 'execute/async':
                if 'reply_received' in body['script']:
                    self.assertEqual(selected, 'root')
                    self.assertFalse(child)
                    uuid.UUID(body['args'][0])
                    return {'reply_received': True, 'payload': 'private value'}
                if 'limiter_available' in body['script']: return {'limiter_available': True}
                if not body['args']: return {'frame_count': 1}
                return {'settled': True, 'failed': False}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': '1'}, clear=True):
            replay(protocol, URL)
        self.assertIn('"root_reply_received": true', logs[0])
        self.assertNotIn('private', logs[0])

    def test_worker_creation_replay_selects_child_and_restores_scope(self):
        calls, logs = [], []
        selected = None
        child = False
        def send(method, uri, body=None, timeout=None):
            nonlocal selected, child
            calls.append((uri, body))
            if uri == 'window' and method == 'GET': return 'original'
            if uri == 'window/handles': return ['root', 'popup']
            if uri == 'window': selected = body['handle']
            if uri == 'frame': child = body['id'] == 0
            if uri == 'execute/sync':
                if 'create_dedicated_worker_js' in body['script']:
                    return {'kind': 'worker', 'source': 'private source'}
                return selected
            if uri == 'execute/async':
                if 'limiter_available' in body['script']:
                    self.assertEqual(selected, 'root')
                    self.assertNotIn('fetch(', body['script'])
                    return {'limiter_available': False}
                if not body['args']: return {'frame_count': 1, 'script_count': 5}
                self.assertTrue(child)
                return {'settled': True, 'failed': True,
                        'exception_type': 'InvalidStateError', 'message': 'private payload'}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send),
                                   logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': '1'}, clear=True):
            replay(protocol, URL)
        self.assertFalse(child)
        self.assertIn(('frame', {'id': None}), calls)
        self.assertIn('"worker_creation": true', logs[0])
        self.assertIn('"root_limiter_available": false', logs[0])
        self.assertIn('InvalidStateError', logs[0])
        self.assertNotIn('private', logs[0])

    def test_disabled_or_unselected_never_touches_driver(self):
        with patch.dict('os.environ', {}, clear=True):
            replay(SimpleNamespace(), URL)
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': '1'}, clear=True):
            for url in ('/other.html', 'http://[invalid', None):
                replay(SimpleNamespace(), url)

    def test_replay_is_separate_bounded_and_sanitized(self):
        logs, calls = [], []
        selected = None
        def send(method, uri, body=None, timeout=None):
            nonlocal selected
            calls.append((method, uri, body, timeout))
            if uri == 'window' and method == 'GET':
                return 'original-private-handle'
            if uri == 'window/handles':
                return ['private-root', 'private-popup']
            if uri == 'window':
                selected = body['handle']
            if uri == 'execute/sync':
                if 'typeof add_iframe_js' in body['script']:
                    self.assertEqual(len(body['args']), 1)
                    self.assertEqual(str(uuid.UUID(body['args'][0])), body['args'][0])
                    return 'private task source'
                return 'root' if selected == 'private-root' else 'popup'
            if uri == 'execute/async':
                return {'settled': True, 'failed': True, 'exception_type': 'TypeError',
                        'body_present': True, 'frame_count': 0, 'script_count': 2.0,
                        'message': 'private error payload'}
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send), logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': '1'}, clear=True):
            replay(protocol, URL)
        self.assertEqual(calls[-1][2], {'handle': 'original-private-handle'})
        self.assertEqual(sum(uri == 'execute/async' for _, uri, _, _ in calls), 2)
        passive = [body for _, uri, body, _ in calls if uri == 'execute/async'][0]
        self.assertEqual(passive['args'], [])
        self.assertNotIn('eval', passive['script'])
        self.assertNotIn('appendChild', passive['script'])
        self.assertTrue(all(timeout in (2, 20) for _, _, _, timeout in calls))
        self.assertIn('"exception_type": "TypeError"', logs[0])
        self.assertIn('"failed": true', logs[0])
        self.assertNotIn('private', logs[0])

    def test_failure_does_not_escape_or_expose_payload(self):
        logs = []
        def send(*args, **kwargs):
            raise RuntimeError('private payload')
        protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=send), logger=SimpleNamespace(info=logs.append))
        with patch.dict('os.environ', {'ORYN_WPT_COMPLETION_REPLAY': '1'}, clear=True):
            replay(protocol, URL)
            replay(SimpleNamespace(), URL)
        self.assertIn('RuntimeError', logs[0])
        self.assertNotIn('private', logs[0])

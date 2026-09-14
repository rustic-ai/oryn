import io
import json
from pathlib import Path
import sys
import threading
import unittest
from collections import deque
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_session import WptSession


class SessionProtocolTests(unittest.TestCase):
    def read_messages(self, messages):
        session = WptSession.__new__(WptSession)
        session.process = Mock(stdout=io.StringIO(''.join(json.dumps(x) + '\n' for x in messages)))
        session.lock = threading.Lock()
        session.ready = threading.Event()
        session.shutdown = threading.Event()
        session.pending = {'case': {'event': threading.Event()}}
        session.errors = deque()
        session.failure = None
        session._read()
        return session

    def test_valid_protocol(self):
        session = self.read_messages([{'event': 'ready', 'schema_version': 1},
            {'event': 'result', 'id': 'case', 'page_id': 1, 'queue_ms': 0, 'run_ms': 1,
             'teardown_ms': 1, 'error': None, 'replacement_error': None, 'resource_error': None,
             'cleanup_errors': [], 'resources': [], 'resource_evidence_complete': False}, {'event': 'shutdown'}])
        self.assertIsNone(session.failure)
        self.assertTrue(session.pending['case']['event'].is_set())

    def test_bad_frames_fail_and_wake_waiters(self):
        ready = {'event': 'ready', 'schema_version': 1}
        for messages in ([[]], [dict(ready, schema_version=2)], [ready, ready],
                         [{'event': 'result', 'id': 'case'}],
                         [ready, {'event': 'result', 'id': 'wrong'}],
                         [ready, {'event': 'result', 'id': 'case'}, {'event': 'result', 'id': 'case'}],
                         [ready], [ready, {'event': 'shutdown'}, {'event': 'result', 'id': 'case'}]):
            session = self.read_messages(messages)
            self.assertIsNotNone(session.failure, messages)
            self.assertTrue(session.pending['case']['event'].is_set())

import io
import json
from pathlib import Path
import sys
import threading
import unittest
from collections import deque
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_session import WptSession, validate_diagnostic_evidence


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


class DiagnosticEvidenceTests(unittest.TestCase):
    def frame(self):
        events = [{'page_id': '8', 'sequence': 3, 'event': {'kind': 'script_diagnostic',
            'capability': 'scheduler.page_task', 'detail': 'child original λ\nmessage'}}]
        return {'diagnostic_events': events, 'diagnostic_audit': {'schema_version': 1, 'scope': 'pre_cleanup_live_pages',
            'coverage_snapshot': 'before_archive', 'expected_live_pages': ['8'],
            'captured_pages': ['8'], 'missing_pages': [],
            'complete': True, 'overflow': False, 'source_overflow': False,
            'recorded_events': 1, 'encoded_bytes': sum(len(json.dumps(row, ensure_ascii=False,
                separators=(',', ':')).encode()) for row in events),
            'dropped_events': 0, 'dropped_bytes': 0, 'event_limit': 256,
            'byte_limit': 1024 * 1024, 'collection_error': None}}

    def test_original_diagnostics_survive_protocol_validation(self):
        frame = self.frame()
        before = json.dumps(frame, ensure_ascii=False)
        validate_diagnostic_evidence(frame)
        self.assertEqual(json.dumps(frame, ensure_ascii=False), before)
        validate_diagnostic_evidence({})

    def test_malformed_or_incomplete_diagnostics_cannot_claim_complete(self):
        import copy
        original = self.frame()
        for key, value in [('recorded_events', 0), ('encoded_bytes', 0), ('event_limit', 999),
                           ('dropped_events', 1), ('source_overflow', True), ('scope', 'all_page_lifetime'),
                           ('collection_error', 'trace channel closed'), ('captured_pages', []),
                           ('expected_live_pages', ['8', '9']), ('missing_pages', ['9'])]:
            frame = copy.deepcopy(original)
            frame['diagnostic_audit'][key] = value
            with self.assertRaises(ValueError): validate_diagnostic_evidence(frame)
        frame = copy.deepcopy(original)
        frame['diagnostic_events'] *= 2
        frame['diagnostic_audit']['recorded_events'] = 2
        frame['diagnostic_audit']['encoded_bytes'] *= 2
        with self.assertRaises(ValueError): validate_diagnostic_evidence(frame)
        frame = self.frame()
        frame['diagnostic_audit'].update(complete=False, overflow=True, dropped_events=1, dropped_bytes=100)
        validate_diagnostic_evidence(frame)
        with self.assertRaises(ValueError): validate_diagnostic_evidence({'diagnostic_events': []})

    def test_missing_live_page_is_explicit_and_cannot_claim_complete(self):
        frame = self.frame()
        frame['diagnostic_audit'].update(expected_live_pages=['8', '9'], missing_pages=['9'], complete=False)
        validate_diagnostic_evidence(frame)
        frame['diagnostic_audit']['complete'] = True
        with self.assertRaises(ValueError):
            validate_diagnostic_evidence(frame)

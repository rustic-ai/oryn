"""Diagnostic boundaries preserve operation outcomes and bound exposed values."""
import json
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from oryn_wptrunner import timing


class TimingTests(unittest.TestCase):
    def setUp(self):
        self.records = []
        self.logger = SimpleNamespace(info=lambda text: self.records.append(json.loads(text.removeprefix('oryn-timing '))))
        self.local = patch.object(timing, '_LOCAL', threading.local())
        self.local.start()
        self.addCleanup(self.local.stop)

    def test_nested_monotonic_spans_preserve_return_and_parentage(self):
        sentinel = object()
        def work():
            with timing.span(self.logger, 'outer'):
                with timing.span(self.logger, 'inner'):
                    return sentinel
        with patch.object(timing.time, 'perf_counter_ns', side_effect=[10, 20, 35, 50]):
            self.assertIs(work(), sentinel)
        outer_begin, inner_begin, inner_end, outer_end = self.records
        self.assertEqual(inner_begin['parent_span_id'], outer_begin['span_id'])
        self.assertEqual(inner_end['elapsed_ns'], 15)
        self.assertEqual(outer_end['elapsed_ns'], 40)
        self.assertEqual(outer_end['outcome'], 'returned')
        self.assertIsNone(timing._LOCAL.active)

    def test_exception_and_logger_failure_do_not_replace_primary_or_leak_text(self):
        primary = RuntimeError('Bearer private-token script: private-source')
        calls = []
        def intermittent(text):
            calls.append(text)
            if len(calls) == 1:
                raise OSError('log sink failure')
            self.logger.info(text)
        with self.assertRaises(RuntimeError) as caught:
            with timing.span(SimpleNamespace(info=intermittent), 'session.teardown'):
                raise primary
        self.assertIs(caught.exception, primary)
        self.assertEqual(self.records[-1]['outcome'], 'exception')
        self.assertEqual(self.records[-1]['exception_type'], 'RuntimeError')
        self.assertEqual(self.records[-1]['prior_log_failures'], 1)
        self.assertNotIn('private-token', ''.join(calls))
        self.assertNotIn('private-source', ''.join(calls))
        self.assertIsNone(timing._LOCAL.active)

    def test_cancellation_is_not_swallowed(self):
        failure = KeyboardInterrupt()
        with self.assertRaises(KeyboardInterrupt) as caught:
            with timing.span(self.logger, 'test.total'):
                raise failure
        self.assertIs(caught.exception, failure)
        self.assertEqual(self.records[-1]['exception_type'], 'KeyboardInterrupt')

    def test_identity_copies_only_complete_already_bound_fields(self):
        identity = {key: 'bound-' + key for key in timing.IDENTITY_FIELDS}
        identity['authorization'] = 'Bearer secret'
        with timing.span(self.logger, 'test.total', identity=identity,
                         custody_id='a' * 32, test_id='/test.html'):
            identity['candidate_sha256'] = 'mutated-caller'
        self.assertEqual(self.records[-1]['identity']['candidate_sha256'], 'bound-candidate_sha256')
        self.assertNotIn('authorization', self.records[-1]['identity'])
        with timing.span(self.logger, 'test.total', identity={'candidate_sha256': 'partial'},
                         custody_id='invalid', test_id='x' * 4097):
            pass
        self.assertIsNone(self.records[-1]['identity'])
        self.assertIsNone(self.records[-1]['test_id'])
        self.assertIsNone(self.records[-1]['custody_id'])

    def test_contract_identity_uses_current_collection_without_revalidation(self):
        values = {key: 'bound-' + key for key in timing.IDENTITY_FIELDS if key != 'collection_id'}
        values['envelope'] = {'collection_ids': {'core': 'core-actual', 'canaries': 'canaries-actual'}}
        with patch.dict(timing.os.environ, {'ORYN_CORE_COLLECTION': 'core'}):
            result = timing.contract_identity({'profile_envelope': values})
        self.assertEqual(result['collection_id'], 'core-actual')
        self.assertEqual(set(result), set(timing.IDENTITY_FIELDS))
        with patch.dict(timing.os.environ, {'ORYN_CORE_COLLECTION': 'foreign'}):
            self.assertIsNone(timing.contract_identity({'profile_envelope': values}))

import unittest
from types import SimpleNamespace
from oryn_wptrunner.navigation_evidence import attach


class NavigationEvidenceTests(unittest.TestCase):
    def test_capture_preserves_raw_status_and_native_truncation(self):
        after = {"schema_version": 1, "pages": [{"page_id": 1, "snapshot": {"retired": True, "active": 0, "pruned_outcomes": 2, "outcomes": [{"id": 7, "result": {"Err": "bounded"}, "error_truncated": True}]}}]}
        driver = SimpleNamespace(send_session_command=lambda *args: after)
        protocol = SimpleNamespace(webdriver=driver, _navigation_baseline={"schema_version": 1, "pages": []}, _navigation_boundary="session-start")
        result = SimpleNamespace(status="OK", extra={"existing": 1})
        converted = (result, [])
        self.assertIs(attach(SimpleNamespace(protocol=protocol), SimpleNamespace(url="/test"), converted), converted)
        self.assertEqual(result.status, "OK")
        self.assertEqual(result.extra["existing"], 1)
        self.assertEqual(result.extra["oryn_navigation"]["after"], after)
        self.assertEqual(protocol._navigation_boundary, "/test")

    def test_collection_failure_is_evidence_not_an_upstream_pass(self):
        def fail(*args):
            raise RuntimeError("retired transport")
        result = SimpleNamespace(status="ERROR", extra={})
        attach(SimpleNamespace(protocol=SimpleNamespace(webdriver=SimpleNamespace(send_session_command=fail))), SimpleNamespace(url="/test"), (result, []))
        self.assertEqual(result.status, "ERROR")
        self.assertIn("collection_error", result.extra["oryn_navigation"]["after"])


class NavigationTimingTests(unittest.TestCase):
    def test_run_wrapper_preserves_upstream_result_and_exception(self):
        from unittest.mock import patch
        from oryn_wptrunner.navigation_evidence import NavigationEvidenceMixin
        from oryn_wptrunner import timing
        events = []
        class Upstream:
            def run_test(self, test):
                events.append(test)
                if getattr(test, 'failure', None):
                    raise test.failure
                return test.result
        class Executor(NavigationEvidenceMixin, Upstream):
            logger = SimpleNamespace(info=lambda text: None)
            protocol = SimpleNamespace(_custody_session='a' * 32)
        executor = Executor()
        result = object()
        test = SimpleNamespace(url='/actual.html', result=result)
        with patch.object(timing.time, 'perf_counter_ns', side_effect=[10, 30, 40, 90]):
            self.assertIs(executor.run_test(test), result)
            failure = RuntimeError('upstream failure')
            test.failure = failure
            with self.assertRaises(RuntimeError) as caught:
                executor.run_test(test)
        self.assertIs(caught.exception, failure)
        self.assertEqual(events, [test, test])

    def test_result_conversion_times_actual_snapshot_without_changing_status(self):
        import json
        from oryn_wptrunner.navigation_evidence import NavigationEvidenceMixin
        messages = []
        result = SimpleNamespace(status='TIMEOUT', extra={})
        converted = (result, [])
        class Upstream:
            def convert_result(self, test, value):
                self.converted_value = value
                return converted
        class Executor(NavigationEvidenceMixin, Upstream):
            logger = SimpleNamespace(info=messages.append)
            protocol = SimpleNamespace(webdriver=SimpleNamespace(send_session_command=lambda *args: {'schema_version': 1, 'pages': []}))
        executor = Executor()
        self.assertIs(executor.convert_result(SimpleNamespace(url='/timeout.html'), 'raw'), converted)
        self.assertEqual(executor.converted_value, 'raw')
        self.assertEqual(result.status, 'TIMEOUT')
        rows = [json.loads(text.removeprefix('oryn-timing ')) for text in messages]
        self.assertEqual([row['phase'] for row in rows], ['test.result_conversion', 'test.navigation_snapshot', 'test.navigation_snapshot', 'test.result_conversion'])
        self.assertEqual(rows[1]['parent_span_id'], rows[0]['span_id'])

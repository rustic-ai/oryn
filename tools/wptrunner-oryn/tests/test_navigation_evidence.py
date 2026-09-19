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

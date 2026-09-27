"""Regression coverage for informational navigation diagnostics in framework evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("framework_navigation_runner", ROOT / "scripts/run-oil-framework-outcomes.py")
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class NavigationDiagnosticTests(unittest.TestCase):
    def detail(self, **changes):
        return dict(dict(navigation_id=1, source_generation=4294967296, request_id=0,
                         phase="transaction_bound", error=None, elapsed_us=0,
                         deadline_remaining_us=30000000), **changes)

    def frames(self, detail=None, **kind_changes):
        kind = dict(kind="script_diagnostic", capability="navigation.io",
                    detail=json.dumps(self.detail()) if detail is None else detail)
        kind.update(kind_changes)
        return [dict(kind="observation"),
                dict(kind="action", result=dict(action_id=1, execution_domain="native")),
                dict(kind="structured", value="Items: 1"),
                dict(kind="trace", events=[dict(kind=kind)])]

    def verdict(self, frames):
        return RUNNER.validate("vue", 'click "Add item"', frames)

    def test_all_recorded_success_phases_are_informational(self):
        for phase in RUNNER.NAVIGATION_PROGRESS_PHASES:
            with self.subTest(phase=phase):
                self.assertEqual(self.verdict(self.frames(json.dumps(self.detail(phase=phase)))), [])

    def test_explicit_errors_unknown_phase_and_capability_still_fail(self):
        for changes in ({"error": "broker failed"}, {"error": False}, {"phase": "consumer_failed"},
                        {"phase": "diagnostics_dropped"}, {"phase": []}):
            with self.subTest(changes=changes):
                self.assertTrue(self.verdict(self.frames(json.dumps(self.detail(**changes)))))
        for capability in ("script", "navigation.other", "diagnostics.overflow", None):
            self.assertTrue(self.verdict(self.frames(capability=capability)))

    def test_malformed_missing_duplicate_or_extra_fields_fail_closed(self):
        missing = self.detail(); del missing["error"]
        extra = self.detail(); extra["dropped"] = 1
        detail = json.dumps(self.detail())
        for bad in ("broken", "[]", "null", json.dumps(missing), json.dumps(extra),
                    detail[:-1] + ', "error": null}', "[" * 2000, "x" * 16385):
            with self.subTest(bad=bad[:80]):
                self.assertTrue(self.verdict(self.frames(bad)))

    def test_identity_and_timing_types_are_checked(self):
        for key in ("navigation_id", "source_generation", "request_id", "elapsed_us", "deadline_remaining_us"):
            for value in (-1, True, "1", 1.5):
                with self.subTest(key=key, value=value):
                    self.assertTrue(self.verdict(self.frames(json.dumps(self.detail(**{key: value})))))
        self.assertEqual(self.verdict(self.frames(json.dumps(self.detail(deadline_remaining_us=None)))), [])

    def test_real_console_error_and_script_failure_remain_failures(self):
        self.assertTrue(self.verdict(self.frames(kind="console", level="error", message="failed")))
        self.assertTrue(self.verdict(self.frames("actual script exception", capability="script")))

    def test_informational_diagnostics_do_not_bypass_other_assertions(self):
        for change in ("state", "action", "observation", "duplicate_trace", "error_frame"):
            frames = self.frames()
            if change == "state": frames[2]["value"] = "Items: 0"
            elif change == "action": frames[1]["result"]["execution_domain"] = "chromium"
            elif change == "observation": frames.pop(0)
            elif change == "duplicate_trace": frames.append(copy.deepcopy(frames[-1]))
            else: frames.append(dict(kind="error", message="failed"))
            with self.subTest(change=change): self.assertTrue(self.verdict(frames))

    def test_malformed_trace_structure_is_a_recorded_failure(self):
        for bad in (None, {}, [None], [dict(kind=None)]):
            frames = self.frames(); frames[-1]["events"] = bad
            with self.subTest(bad=bad): self.assertTrue(self.verdict(frames))


if __name__ == "__main__":
    unittest.main()

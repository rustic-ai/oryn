import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("core_ledger", Path(__file__).parents[1] / "build-core-completion-ledger.py")
ledger = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ledger)


class CoreLedgerTests(unittest.TestCase):
    def setUp(self):
        self.coverage = [dict(id="testharness:/encoding/a.any.html", path="encoding/a.any.js",
            url="/encoding/a.any.html", tier="core", root="encoding", test_type="testharness",
            sha256="a" * 64, runner_id="encoding/a.any.js|window|", status="not_run"),
            dict(id="testharness:/encoding/a.any.serviceworker.html", path="encoding/a.any.js",
            url="/encoding/a.any.serviceworker.html", tier="core", root="encoding", test_type="testharness",
            sha256="a" * 64, status="unsupported")]
        self.observed = copy.deepcopy(self.coverage)
        self.observed[0].update(status="passed", qualification="review_candidate")
        self.observed[1].update(reason="runner_not_available:generated_global:a.any.serviceworker.html",
                               qualification="unsupported")

    def test_raw_pass_does_not_close_obligation_and_worker_gap_is_owned(self):
        entries = ledger.assign(self.coverage, self.observed)
        self.assertEqual(len(entries), 2)
        passed = next(row for row in entries if row["raw_status"] == "passed")
        self.assertEqual(passed["acceptance_status"], "requires_source_bound_qualification")
        self.assertIsNone(passed["confirmed_root_cause"])
        unsupported = next(row for row in entries if row["raw_status"] == "unsupported")
        self.assertEqual(unsupported["required_packages"], ["G", "L", "W"])

    def test_missing_duplicate_and_variant_substitution_are_rejected(self):
        for rows in (self.observed[:1], self.observed + self.observed[:1]):
            with self.assertRaises(ValueError):
                ledger.assign(self.coverage, rows)
        altered = copy.deepcopy(self.observed)
        altered[0]["url"] += "?different"
        with self.assertRaises(ValueError):
            ledger.assign(self.coverage, altered)

    def test_unsupported_cannot_be_reclassified_as_a_pass(self):
        self.observed[1]["status"] = "passed"
        with self.assertRaises(ValueError):
            ledger.assign(self.coverage, self.observed)

    def test_unknown_core_root_requires_explicit_assignment(self):
        for rows in (self.coverage, self.observed):
            rows[0]["root"] = "new-feature"
        with self.assertRaises(ValueError):
            ledger.assign(self.coverage, self.observed)

    def test_unfinished_observation_cannot_be_accepted(self):
        self.observed[0]["status"] = "not_run"
        with self.assertRaises(ValueError):
            ledger.assign(self.coverage, self.observed)


if __name__ == "__main__":
    unittest.main()

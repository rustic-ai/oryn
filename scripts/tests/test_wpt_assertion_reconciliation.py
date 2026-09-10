import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "reconcile", Path(__file__).resolve().parents[1] / "reconcile-wpt-assertions.py"
)
RECONCILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RECONCILE)


def assertion(name, status=0):
    return {"name": name, "status": status, "message": ""}


class ReconciliationTests(unittest.TestCase):
    def test_legacy_transport_preserves_escaped_newlines_and_string_edges(self):
        self.assertEqual(RECONCILE.legacy_name("  x\ny\u00a0 z  "), " x\ny z ")

    def test_legacy_name_recovery_is_not_a_missing_assertion(self):
        result = RECONCILE.compare_assertions(
            [assertion("a b")], [assertion("a  b", 1)], legacy_baseline=True
        )
        self.assertEqual(result["uniquely_matched"], 1)
        self.assertEqual(len(result["regressed"]), 1)
        self.assertEqual(result["missing"], [])

    def test_duplicate_names_are_ambiguous_even_when_counts_match(self):
        result = RECONCILE.compare_assertions(
            [assertion("a b"), assertion("a b", 1)],
            [assertion("a b", 1), assertion("a  b")], legacy_baseline=True,
        )
        self.assertEqual(result["ambiguous"], [{"name": "a b", "count": 2}])
        self.assertEqual(result["uniquely_matched"], 0)

    def test_duplicate_disappearance_is_reported(self):
        result = RECONCILE.compare_assertions(
            [assertion("x"), assertion("x")], [assertion("x")]
        )
        self.assertEqual(result["multiplicity_mismatch"], [{"name": "x", "before": 2, "after": 1}])

    def test_exact_comparison_does_not_normalize_new_evidence(self):
        result = RECONCILE.compare_assertions([assertion("a b")], [assertion("a  b")])
        self.assertEqual(result["missing"], ["a b"])
        self.assertEqual(result["added"], ["a  b"])


if __name__ == "__main__":
    unittest.main()

import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("project", Path(__file__).resolve().parents[1] / "reclassify-wpt-evidence.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ProjectionTests(unittest.TestCase):
    def test_exact_variants_and_unsupported_remain_distinct(self):
        coverage = [
            {"tier": "core", "runner_id": "a|window|?one", "status": "not_run"},
            {"tier": "core", "runner_id": "a|window|?two", "status": "not_run"},
            {"tier": "core", "status": "unsupported"},
        ]
        result = MODULE.project(coverage, [{"id": "a|window|?one", "status": "passed"}])
        self.assertEqual({row["status"]: row["count"] for row in result},
                         {"passed": 1, "not_run": 1, "unsupported": 1})

    def test_duplicate_and_unknown_evidence_are_rejected(self):
        row = {"id": "a", "status": "passed"}
        for rows in [[row], [row, row]]:
            with self.assertRaises(ValueError):
                MODULE.project([], rows)


if __name__ == "__main__":
    unittest.main()

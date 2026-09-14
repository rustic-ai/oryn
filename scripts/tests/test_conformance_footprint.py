from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "summarize_conformance_footprint",
    ROOT / "scripts/summarize-conformance-footprint.py",
)
assert SPEC is not None and SPEC.loader is not None
SUMMARY = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SUMMARY)


class ConformanceFootprintTests(unittest.TestCase):
    def test_prefixes_match_only_path_boundaries(self) -> None:
        self.assertTrue(SUMMARY.matches_prefix("dom/nodes/case.html", "dom"))
        self.assertTrue(SUMMARY.matches_prefix("dom", "dom"))
        self.assertFalse(SUMMARY.matches_prefix("domparsing/case.html", "dom"))

    def test_profile_exclusions_override_inclusions(self) -> None:
        profile = {
            "include_prefixes": ["html"],
            "exclude_prefixes": ["html/rendering"],
        }
        self.assertTrue(SUMMARY.matches_profile("html/forms/input.html", profile))
        self.assertFalse(SUMMARY.matches_profile("html/rendering/replaced.html", profile))

    def test_execution_count_expands_globals_and_variants(self) -> None:
        test = {"globals": ["window", "dedicatedworker"], "variants": ["", "?one"]}
        self.assertEqual(SUMMARY.execution_count(test), 4)


if __name__ == "__main__":
    unittest.main()

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from scripts.evidence_assertions import (
    EvidenceError,
    assertion_bound_cases,
    verified_regressions,
)


class AssertionEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = {
            "cases": [
                {"id": "one", "category": "runtime", "disposition": "required"},
                {
                    "id": "two",
                    "category": "layout",
                    "disposition": "declared_limitation",
                    "expected_capability": "layout.table",
                },
            ]
        }
        self.assertions = {
            "schema_version": 1,
            "assertions": [
                {
                    "id": "assert-one",
                    "case_id": "one",
                    "status": "passed",
                    "checks": [
                        {"name": "real check", "passed": True, "artifact": "one.json"}
                    ],
                    "artifacts": ["one.json"],
                },
                {
                    "id": "assert-two",
                    "case_id": "two",
                    "status": "expected_limitation",
                    "checks": [
                        {
                            "name": "diagnostic observed",
                            "passed": True,
                            "artifact": "two.json",
                        }
                    ],
                    "artifacts": ["two.json"],
                },
            ],
        }
        self.artifacts = {"one.json": "raw:one", "two.json": "raw:two"}

    def test_derives_case_status_from_assertions(self) -> None:
        cases = assertion_bound_cases(
            self.manifest, self.assertions, self.artifacts, include_layout_domains=True
        )
        self.assertEqual(
            [case["status"] for case in cases], ["passed", "expected_limitation"]
        )

    def test_missing_assertion_fails(self) -> None:
        self.assertions["assertions"].pop()
        with self.assertRaisesRegex(EvidenceError, "coverage mismatch"):
            assertion_bound_cases(self.manifest, self.assertions, self.artifacts)

    def test_failed_check_fails(self) -> None:
        self.assertions["assertions"][0]["checks"][0]["passed"] = False
        with self.assertRaisesRegex(EvidenceError, "failed check"):
            assertion_bound_cases(self.manifest, self.assertions, self.artifacts)

    def test_duplicate_case_mapping_fails(self) -> None:
        duplicate = dict(self.assertions["assertions"][0])
        duplicate["id"] = "duplicate"
        self.assertions["assertions"].append(duplicate)
        with self.assertRaisesRegex(EvidenceError, "multiple assertions"):
            assertion_bound_cases(self.manifest, self.assertions, self.artifacts)

    def test_regressions_must_match_exactly(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "regressions.json"
            artifact = Path(directory) / "rust.log"
            artifact.write_text("test result: ok")
            path.write_text(
                '{"schema_version":1,"suite":"test","regressions":{"rust":{"status":"passed",'
                '"checks":[{"name":"named test","passed":true}],'
                '"artifacts":["rust.log"]}}}'
            )
            self.assertEqual(verified_regressions(path, {"rust"}), {"rust": "passed"})
            with self.assertRaisesRegex(EvidenceError, "required keys"):
                verified_regressions(path, {"rust", "python"})

    def test_failed_regression_check_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "regressions.json"
            artifact = Path(directory) / "rust.log"
            artifact.write_text("failed")
            path.write_text(
                '{"schema_version":1,"suite":"test","regressions":{"rust":{"status":"failed",'
                '"checks":[{"name":"named test","passed":false}],'
                '"artifacts":["rust.log"]}}}'
            )
            with self.assertRaisesRegex(EvidenceError, "did not pass"):
                verified_regressions(path, {"rust"})


if __name__ == "__main__":
    unittest.main()

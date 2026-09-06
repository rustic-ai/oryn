from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "conformance_backlog", ROOT / "scripts/build-conformance-failure-backlog.py"
)
BACKLOG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BACKLOG)


class ConformanceBacklogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.inventory = Path(self.temporary.name) / "inventory.json"
        self.ledger = Path(self.temporary.name) / "ledger.json"
        self.inventory.write_text(json.dumps({"tests": [
            {"path": "dom/nodes/example.html", "globals": ["window"], "variants": ["", "?x"]}
        ]}))
        self.result = {
            "inventory_sha256": hashlib.sha256(self.inventory.read_bytes()).hexdigest(),
            "source": {"commit": "test", "dirty": False},
            "runtime_metadata": {"worker_binary_sha256": "a" * 64},
            "outcome": {"required_total": 2, "required_finished": 2,
                        "required_passed": 1, "status": "failed"},
            "assertions": [
                {"id": "dom/nodes/example.html|window|", "path": "dom/nodes/example.html",
                 "status": "passed", "result": {"complete": True, "harness": 0,
                                                  "tests": [{"name": "actual test", "status": 0}]}},
                {"id": "dom/nodes/example.html|window|?x", "path": "dom/nodes/example.html",
                 "status": "failed", "failure_domain": "assertion",
                 "result": {"tests": [{"name": "failing test", "status": 1, "message": "why"}]}},
            ],
        }

    def build(self, result):
        self.ledger.write_text(json.dumps(result))
        return BACKLOG.build_backlog(self.ledger, self.inventory)

    def test_one_source_preserves_variants_and_failure_evidence(self):
        report = self.build(self.result)
        self.assertEqual(report["open_execution_count"], 1)
        self.assertEqual(report["counts_by_workstream"], {"W1": 1})
        self.assertEqual(report["entries"][0]["id"], "dom/nodes/example.html|window|?x")
        self.assertEqual(report["entries"][0]["first_failure_message"], "why")
        self.assertIsNone(report["entries"][0]["confirmed_root_cause"])
        self.assertEqual(len(report["sources"]), 1)

    def test_rejects_incomplete_or_inconsistent_ledgers(self):
        for kind in ("duplicate", "missing", "extra", "fingerprint", "pass_count", "running"):
            with self.subTest(kind=kind):
                result = copy.deepcopy(self.result)
                if kind == "duplicate":
                    result["assertions"][1] = result["assertions"][0]
                elif kind == "missing":
                    result["assertions"].pop()
                elif kind == "extra":
                    result["assertions"].append({"id": "extra"})
                elif kind == "fingerprint":
                    result["inventory_sha256"] = "wrong"
                elif kind == "pass_count":
                    result["outcome"]["required_passed"] = 2
                else:
                    result["outcome"]["status"] = "running"
                with self.assertRaises(ValueError):
                    self.build(result)

    def test_rejects_unverified_passing_completion(self):
        for kind in ("incomplete", "harness", "assertion", "empty", "probe"):
            with self.subTest(kind=kind):
                result = copy.deepcopy(self.result)
                row = result["assertions"][0]
                if kind == "incomplete":
                    row["result"]["complete"] = False
                elif kind == "harness":
                    row["result"]["harness"] = 1
                elif kind == "assertion":
                    row["result"]["tests"][0]["status"] = 1
                elif kind == "empty":
                    row["result"]["tests"] = []
                else:
                    row["result_transport"] = "isolated_dom_probe"
                with self.assertRaises(SystemExit):
                    self.build(result)

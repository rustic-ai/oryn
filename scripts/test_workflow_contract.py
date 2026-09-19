"""Controls for expanded workflow sampling and immutable regression inputs."""
import copy
import json
import tempfile
import unittest
from pathlib import Path

from workflow_contract import DEFAULT_CONTRACT, FAULTS, load_contract, planned_attempts, validate_contract


class WorkflowContractTests(unittest.TestCase):
    def setUp(self):
        self.contract = json.loads(DEFAULT_CONTRACT.read_text())

    def test_sampling_keeps_every_attempt_and_all_recovery_controls(self):
        attempts = planned_attempts(self.contract)
        self.assertEqual(len(attempts), len({attempt["attempt_id"] for attempt in attempts}))
        for family in self.contract["families"]:
            for cell in family["cells"]:
                selected = [attempt for attempt in attempts if attempt["cell"] == cell["id"]]
                self.assertEqual(sum(attempt["mode"] == "serial" for attempt in selected), 3)
                self.assertEqual(sum(attempt["mode"] == "mixed_family" for attempt in selected), 9)
                self.assertEqual(sum(attempt["mode"] == "recovery_repeat" for attempt in selected),
                                 30 if cell["recovery_sensitive"] else 0)
                for contexts in (1, 5, 20):
                    self.assertEqual(sum(attempt["mode"] == "mixed_family" and attempt["context_count"] == contexts for attempt in selected), 3)
                faults = {attempt["fault"] for attempt in selected if attempt["mode"] == "fault_injection"}
                self.assertEqual(faults, set(FAULTS) if cell["recovery_sensitive"] else set())

    def test_missing_family_duplicate_cell_and_missing_oracle_reject(self):
        missing = copy.deepcopy(self.contract)
        missing["families"].pop()
        duplicate = copy.deepcopy(self.contract)
        duplicate["families"][0]["cells"].append(copy.deepcopy(duplicate["families"][0]["cells"][0]))
        no_oracle = copy.deepcopy(self.contract)
        no_oracle["families"][0]["cells"][0]["independent_outcome"] = ""
        weakened = copy.deepcopy(self.contract)
        weakened["sampling"]["recovery_repeat"] = 1
        for invalid in (missing, duplicate, no_oracle, weakened):
            with self.assertRaises(ValueError):
                validate_contract(invalid)

    def test_removing_a_behavior_cell_cannot_shrink_the_frozen_contract(self):
        changed = copy.deepcopy(self.contract)
        changed["families"][0]["cells"].pop()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "contract.json"
            path.write_text(json.dumps(changed, indent=2) + "\n")
            with self.assertRaisesRegex(ValueError, "frozen v1 identity"):
                load_contract(path)

    def test_changed_regression_source_rejects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for entry in self.contract["regression_inputs"]:
                path = root / entry["path"]
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("changed input")
            path = root / "contract.json"
            path.write_bytes(DEFAULT_CONTRACT.read_bytes())
            with self.assertRaisesRegex(ValueError, "regression input changed"):
                load_contract(path, root)

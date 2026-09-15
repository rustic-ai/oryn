"""Test the oracle independently of browser execution and displayed success text."""

import copy
import importlib.util
import json
from pathlib import Path
import unittest
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


receiver = load("forms_fixture", ROOT / "test-harness/workflows/forms_fixture.py")
runner = load("forms_runner", ROOT / "scripts/run-oil-forms-outcomes.py")
CONTRACT = json.loads((ROOT / "test-harness/workflows/forms-cases.json").read_text())
VALID_CASE = next(case for case in CONTRACT["cases"] if case["id"] == "valid_urlencoded")
ID = "a" * 32
ORIGIN = "http://127.0.0.1:12345"


class OracleTests(unittest.TestCase):
    def setUp(self):
        self.fixture = receiver.Fixture()
        self.fixture.command({"command": "create", "case_id": ID})
        values = CONTRACT["fields"]
        self.fields = [
            [key, values[key]]
            for key in ("name", "email", "role", "message", "quantity", "date", "time", "reference")
        ]
        self.fields += [["skill", skill] for skill in values["skill"]]
        self.fields += [["case_id", ID], ["intent", "apply"]]
        self.body = urlencode([tuple(pair) for pair in self.fields]).encode()
        self.frame = [
            {
                "kind": "observation",
                "observation": {
                    "page": {"url": ORIGIN + "/receipt/" + ID},
                    "nodes": [{"name": "Application received"}],
                },
            }
        ]

        self.frame = [
            {"kind": "action", "result": {"action_id": number + 1, "execution_domain": "native"}}
            for number in range(runner.expected_action_count(VALID_CASE, CONTRACT))
        ] + self.frame

    def valid_snapshot(self):
        self.assertEqual(
            self.fixture.receive(ID, "application/x-www-form-urlencoded", self.body), 303
        )
        snapshot = self.fixture.command({"command": "snapshot", "case_id": ID})
        snapshot["receipt_views"] = 2
        return snapshot

    def errors(self, snapshot, frame=None):
        return runner.validate_outcome(
            VALID_CASE,
            CONTRACT,
            ID,
            ORIGIN,
            self.frame if frame is None else frame,
            snapshot,
            {},
        )

    def test_success_text_without_a_server_receipt_cannot_pass(self):
        self.assertTrue(self.errors(self.fixture.command({"command": "snapshot", "case_id": ID})))

    def test_exact_fields_and_receipt_reload_pass(self):
        self.assertEqual(self.errors(self.valid_snapshot()), [])

    def test_numeric_api_adjustment_requires_exact_changed_server_fields(self):
        case = next(c for c in CONTRACT["cases"] if c["id"] == "native_numeric_api_adjustment")
        changed = [[key, case["expected_fields"].get(key, value)] for key, value in self.fields]
        body = urlencode([tuple(pair) for pair in changed]).encode()
        self.assertEqual(self.fixture.receive(ID, "application/x-www-form-urlencoded", body), 303)
        snapshot = self.fixture.command({"command": "snapshot", "case_id": ID})
        snapshot["receipt_views"] = 2
        frame = [
            {"kind": "action", "result": {"action_id": n + 1, "execution_domain": "native"}}
            for n in range(runner.expected_action_count(case, CONTRACT))
        ] + [self.frame[-1]]
        self.assertEqual(
            runner.validate_outcome(case, CONTRACT, ID, ORIGIN, frame, snapshot, {}), []
        )
        snapshot["receipts"][0]["fields"] = self.fields
        self.assertTrue(runner.validate_outcome(case, CONTRACT, ID, ORIGIN, frame, snapshot, {}))

    def test_numeric_and_calendar_oracle_rejects_invalid_payloads(self):
        for field, invalid in [
            ("quantity", "4"),
            ("quantity", "11"),
            ("date", "2026-09-30"),
            ("date", "2026-10-32"),
            ("time", "12:00"),
            ("time", "24:00"),
            ("reference", "bad123"),
            ("reference", "ABC123extra"),
            ("reference", "ABC١٢٣"),
            ("message", "abc"),
            ("message", "x" * 65),
            ("name", "👨‍👩‍👧‍👦"),
        ]:
            with self.subTest(field=field, invalid=invalid):
                fixture = receiver.Fixture()
                fixture.command({"command": "create", "case_id": ID})
                fields = [(key, invalid if key == field else value) for key, value in self.fields]
                self.assertEqual(
                    fixture.receive(
                        ID, "application/x-www-form-urlencoded", urlencode(fields).encode()
                    ),
                    422,
                )
                snapshot = fixture.command({"command": "snapshot", "case_id": ID})
                self.assertEqual(len(snapshot["requests"]), 1)
                self.assertEqual(snapshot["receipts"], [])

    def test_receipt_without_completed_actions_cannot_pass(self):
        snapshot = self.valid_snapshot()
        self.assertTrue(self.errors(snapshot, [self.frame[-1]]))

    def test_corrupted_receipt_and_missing_browser_observation_fail(self):
        snapshot = self.valid_snapshot()
        self.assertTrue(self.errors(snapshot, []))
        for change in ("fields", "files", "reload", "duplicate", "identity"):
            changed = copy.deepcopy(snapshot)
            if change == "fields":
                changed["receipts"][0]["fields"][0][1] = "incorrect"
            elif change == "files":
                changed["receipts"][0]["files"] = [{"sha256": "wrong"}]
            elif change == "reload":
                changed["receipt_views"] = 1
            elif change == "duplicate":
                changed["requests"].append(changed["requests"][0])
            else:
                changed["receipts"][0]["case_id"] = "b" * 32
            self.assertTrue(self.errors(changed), change)

    def test_duplicate_submit_is_visible_and_cannot_duplicate_receipt(self):
        self.valid_snapshot()
        self.assertEqual(
            self.fixture.receive(ID, "application/x-www-form-urlencoded", self.body), 409
        )
        snapshot = self.fixture.command({"command": "snapshot", "case_id": ID})
        self.assertEqual(len(snapshot["receipts"]), 1)
        self.assertEqual(len(snapshot["requests"]), 2)
        self.assertTrue(self.errors(snapshot))

    def test_rejected_request_cannot_disappear_from_recovery_evidence(self):
        bad = urlencode([tuple(pair) for pair in self.fields if pair[0] != "email"]).encode()
        self.assertEqual(self.fixture.receive(ID, "application/x-www-form-urlencoded", bad), 422)
        self.assertTrue(self.errors(self.valid_snapshot()))

    def test_control_case_identity_is_isolated_and_bounded(self):
        with self.assertRaises(ValueError):
            self.fixture.command({"command": "create", "case_id": ID})
        with self.assertRaises(ValueError):
            self.fixture.command({"command": "snapshot", "case_id": "b" * 32})
        self.assertEqual(
            self.fixture.receive("b" * 32, "application/x-www-form-urlencoded", self.body), 404
        )

    def test_multipart_preserves_binary_bytes_and_rejects_missing_boundary(self):
        data = b"\x00\xff\r\ncontent"
        body = (
            b'--boundary\r\nContent-Disposition: form-data; name="document"; filename="safe.bin"\r\n'
            b"Content-Type: application/octet-stream\r\n\r\n" + data + b"\r\n--boundary--\r\n"
        )
        fields, files = receiver.decode_submission("multipart/form-data; boundary=boundary", body)
        self.assertEqual(fields, [])
        self.assertEqual(files[0]["bytes"], len(data))
        self.assertEqual(files[0]["sha256"], receiver.hashlib.sha256(data).hexdigest())
        with self.assertRaises(ValueError):
            receiver.decode_submission("multipart/form-data; boundary=absent", body)


if __name__ == "__main__":
    unittest.main()

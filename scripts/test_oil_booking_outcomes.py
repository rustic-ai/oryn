"""Persisted controlled-booking invariants, independent of browser output."""

from concurrent.futures import ThreadPoolExecutor
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "booking_fixture", ROOT / "test-harness/workflows/booking_fixture.py"
)
booking = importlib.util.module_from_spec(spec)
spec.loader.exec_module(booking)
runner_spec = importlib.util.spec_from_file_location(
    "booking_runner", ROOT / "scripts/run-oil-booking-outcomes.py"
)
runner = importlib.util.module_from_spec(runner_spec)
runner_spec.loader.exec_module(runner)
CASE = "a" * 32


class BookingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = Path(self.directory.name) / "booking.sqlite3"
        self.fixture = booking.Fixture(self.path)

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def create(self, scenario="normal", case_id=CASE):
        self.fixture.command(dict(command="create", case_id=case_id, scenario=scenario))
        return self.fixture.db.execute(
            "SELECT session FROM cases WHERE case_id=?", (case_id,)
        ).fetchone()[0]

    def offer(self, cookie, slot="early", zone="America/New_York"):
        self.assertEqual(
            self.fixture.get(CASE, "review", {"slot": [slot], "zone": [zone]}, cookie)[
                0
            ],
            200,
        )
        return self.fixture.db.execute(
            "SELECT offer FROM offers ORDER BY rowid DESC"
        ).fetchone()[0]

    def reserve(self, cookie, offer, key="b" * 32, **overrides):
        fields = (
            dict(
                offer=offer, request_key=key, name="Zoë Test", email="zoe@example.test"
            )
            | overrides
        )
        return self.fixture.post(CASE, "reserve", urlencode(fields).encode(), cookie)[0]

    def test_dst_folds_retain_distinct_utc_instants_and_offsets(self):
        self.assertEqual(
            booking.local_time("early", "America/New_York"), "2026-11-01T01:30:00-04:00"
        )
        self.assertEqual(
            booking.local_time("late", "America/New_York"), "2026-11-01T01:30:00-05:00"
        )
        self.assertEqual(booking.local_time("late", "UTC"), "2026-11-01T06:30:00+00:00")
        with self.assertRaises(ValueError):
            booking.local_time("early", "unknown")

    def test_reservation_survives_writer_close(self):
        cookie = self.create()
        self.assertEqual(self.reserve(cookie, self.offer(cookie)), 303)
        self.fixture.db.close()
        data = booking.snapshot(self.path, CASE)
        self.assertEqual(len(data["reservations"]), 1)
        self.assertEqual(data["reservations"][0]["utc"], booking.SLOTS["early"])
        self.assertEqual(data["reservations"][0]["local"], "2026-11-01T01:30:00-04:00")
        self.assertNotIn("session", data["cases"][0])

    def test_stale_offer_rejected_then_recovery_uses_another_slot(self):
        cookie = self.create("stale")
        self.assertEqual(self.reserve(cookie, self.offer(cookie)), 409)
        self.assertEqual(booking.snapshot(self.path, CASE)["reservations"], [])
        self.assertEqual(
            self.reserve(cookie, self.offer(cookie, "late"), "c" * 32), 303
        )
        data = booking.snapshot(self.path, CASE)
        self.assertEqual([r["status"] for r in data["requests"]], [409, 303])
        self.assertEqual(data["reservations"][0]["slot"], "late")

    def test_concurrent_duplicate_requests_are_idempotent(self):
        cookie = self.create()
        offer = self.offer(cookie)
        with ThreadPoolExecutor(max_workers=2) as executor:
            self.assertEqual(
                list(executor.map(lambda _: self.reserve(cookie, offer), range(2))),
                [303, 303],
            )
        self.assertEqual(len(booking.snapshot(self.path, CASE)["reservations"]), 1)
        self.assertEqual(self.reserve(cookie, offer, name="Different"), 409)

    def test_competing_keys_cannot_double_book(self):
        cookie = self.create()
        offer = self.offer(cookie)
        with ThreadPoolExecutor(max_workers=2) as executor:
            outcomes = list(
                executor.map(
                    lambda key: self.reserve(cookie, offer, key), ["b" * 32, "c" * 32]
                )
            )
        self.assertEqual(sorted(outcomes), [303, 409])
        self.assertEqual(len(booking.snapshot(self.path, CASE)["reservations"]), 1)

    def test_case_isolation_and_validation_fail_closed(self):
        cookie = self.create()
        other = self.create(case_id="d" * 32)
        offer = self.offer(cookie)
        self.assertEqual(self.reserve(other, offer), 403)
        self.assertEqual(self.reserve(cookie, offer, email="invalid"), 400)
        self.assertEqual(self.reserve(cookie, offer, name=""), 400)
        data = booking.snapshot(self.path, CASE)
        self.assertEqual(data["reservations"], [])
        self.assertEqual(json.loads(data["requests"][0]["fields"])["offer"], [offer])

    def test_browser_validator_rejects_missing_actions_and_forged_outcomes(self):
        contract = json.loads(
            (ROOT / "test-harness/workflows/booking-cases.json").read_text()
        )
        case = contract["cases"][0]
        base = "http://127.0.0.1:12345/booking/" + CASE
        lines = runner.task_lines(contract, case, base)
        cookie = self.create()
        self.assertEqual(self.reserve(cookie, self.offer(cookie)), 303)
        for _ in range(2):
            self.assertEqual(self.fixture.get(CASE, "confirmation", {}, cookie)[0], 200)
        data = booking.snapshot(self.path, CASE)
        count = sum(line.split()[0] in ("type", "select", "click") for line in lines)
        frames = [
            {
                "kind": "action",
                "result": {"action_id": i + 1, "execution_domain": "native"},
            }
            for i in range(count)
        ]
        frames += [
            {
                "kind": "structured",
                "value": booking.local_time("early", "America/New_York"),
            }
        ] * 2
        frames += [
            {
                "kind": "observation",
                "observation": {"page": {"url": base + "/confirmation"}},
            }
        ]
        self.assertEqual(runner.validate(case, lines, base, frames, data), [])
        for change in ("null_action", "list_action", "list_identity", "bool_identity",
                       "null_observation", "null_page", "list_url"):
            with self.subTest(malformed_frame=change):
                altered = copy.deepcopy(frames)
                if change == "null_action":
                    altered[0]["result"] = None
                elif change == "list_action":
                    altered[0]["result"] = []
                elif change == "list_identity":
                    altered[0]["result"]["action_id"] = []
                elif change == "bool_identity":
                    altered[0]["result"]["action_id"] = True
                elif change == "null_observation":
                    altered[-1]["observation"] = None
                elif change == "null_page":
                    altered[-1]["observation"]["page"] = None
                else:
                    altered[-1]["observation"]["page"]["url"] = []
                self.assertTrue(runner.validate(case, lines, base, altered, data))
        self.assertTrue(runner.validate(case, lines, base, frames[1:], data))
        altered = copy.deepcopy(frames)
        altered[-2]["value"] = "2026-11-01T01:30:00-05:00"
        self.assertTrue(runner.validate(case, lines, base, altered, data))
        for change in ("missing", "duplicate", "wrong_utc", "wrong_status"):
            altered = copy.deepcopy(data)
            if change == "missing":
                altered["reservations"] = []
            elif change == "duplicate":
                altered["reservations"] *= 2
            elif change == "wrong_utc":
                altered["reservations"][0]["utc"] = booking.SLOTS["late"]
            else:
                altered["requests"][0]["status"] = 409
            self.assertTrue(runner.validate(case, lines, base, frames, altered), change)


    def test_durable_snapshot_rejects_malformed_and_incomplete_evidence(self):
        cookie = self.create()
        self.assertEqual(self.reserve(cookie, self.offer(cookie)), 303)
        data = booking.snapshot(self.path, CASE)
        self.assertEqual(runner.durable_booking_ownership_errors(data, CASE), [])
        contract = json.loads((ROOT / "test-harness/workflows/booking-cases.json").read_text())
        case = contract["cases"][0]
        base = "http://127.0.0.1:12345/booking/" + CASE
        lines = runner.task_lines(contract, case, base)
        for change in ("not_object", "missing_table", "null_table", "non_row",
                       "missing_field", "bad_json", "scalar_json", "invalid_form_values",
                       "boolean_version", "invalid_cookie", "unhashable_identity",
                       "empty_offer", "missing_case", "duplicate_case", "missing_slot",
                       "extra_slot"):
            with self.subTest(change=change):
                altered = copy.deepcopy(data)
                if change == "not_object":
                    altered = []
                elif change == "missing_table":
                    del altered["requests"]
                elif change == "null_table":
                    altered["views"] = None
                elif change == "non_row":
                    altered["requests"] = [None]
                elif change == "missing_field":
                    del altered["reservations"][0]["fields"]
                elif change in ("bad_json", "scalar_json", "invalid_form_values"):
                    altered["reservations"][0]["fields"] = {
                        "bad_json": "{", "scalar_json": "42",
                        "invalid_form_values": '{"offer":42}',
                    }[change]
                elif change == "boolean_version":
                    altered["offers"][0]["version"] = True
                elif change == "invalid_cookie":
                    altered["requests"][0]["cookie_valid"] = 2
                elif change == "unhashable_identity":
                    altered["offers"][0]["offer"] = []
                elif change == "empty_offer":
                    altered["offers"][0]["offer"] = ""
                elif change == "missing_case":
                    altered["cases"] = []
                elif change == "duplicate_case":
                    altered["cases"] *= 2
                elif change == "missing_slot":
                    altered["slots"] = [row for row in altered["slots"] if row["slot"] != "late"]
                else:
                    altered["slots"].append(dict(case_id=CASE, slot="extra", available=1, version=1))
                errors = runner.durable_booking_ownership_errors(altered, CASE)
                self.assertTrue(errors)
                # Full outcome validation must report the same evidence failure,
                # rather than throwing later while decoding or joining rows.
                reported = runner.validate(case, lines, base, [{"kind": "observation"}], altered)
                self.assertEqual(reported, errors)

    def test_durable_join_rejects_unlinked_offers_requests_and_slot_versions(self):
        cookie = self.create()
        offer = self.offer(cookie)
        self.assertEqual(self.reserve(cookie, offer), 303)
        # An idempotent replay has two accepted requests and one reservation.
        self.assertEqual(self.reserve(cookie, offer), 303)
        data = booking.snapshot(self.path, CASE)
        self.assertEqual(runner.durable_booking_ownership_errors(data, CASE), [])
        for change in ("foreign_case", "offer_slot", "offer_time", "request_body",
                       "reservation_key", "slot_version", "duplicate_offer",
                       "missing_request", "unbacked_acceptance"):
            with self.subTest(change=change):
                altered = copy.deepcopy(data)
                if change == "foreign_case":
                    altered["reservations"][0]["case_id"] = "c" * 32
                elif change == "offer_slot":
                    altered["offers"][0]["slot"] = "late"
                elif change == "offer_time":
                    altered["offers"][0]["utc"] = booking.SLOTS["late"]
                elif change == "request_body":
                    for row in altered["requests"]:
                        value = json.loads(row["fields"])
                        value["email"] = ["other@example.test"]
                        row["fields"] = json.dumps(value)
                elif change == "reservation_key":
                    altered["reservations"][0]["request_key"] = "d" * 32
                elif change == "slot_version":
                    slot = altered["reservations"][0]["slot"]
                    next(row for row in altered["slots"] if row["slot"] == slot)["version"] += 1
                elif change == "duplicate_offer":
                    altered["offers"].append(copy.deepcopy(altered["offers"][0]))
                elif change == "missing_request":
                    altered["requests"] = []
                else:
                    altered["reservations"] = []
                self.assertTrue(runner.durable_booking_ownership_errors(altered, CASE))


if __name__ == "__main__":
    unittest.main()

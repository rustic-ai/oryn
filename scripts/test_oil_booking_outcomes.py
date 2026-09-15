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


if __name__ == "__main__":
    unittest.main()

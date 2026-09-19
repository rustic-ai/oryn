"""Independent server controls for the client-timezone booking frontend."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("booking_v2", ROOT / "test-harness/workflows/booking_fixture_v2.py")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)

runner_spec = importlib.util.spec_from_file_location("booking_outcome_runner", ROOT / "scripts/run-oil-booking-outcomes.py")
runner = importlib.util.module_from_spec(runner_spec)
runner_spec.loader.exec_module(runner)


class BookingFrontendTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / "booking.sqlite3"
        self.fixture = fixture.Fixture(self.database)
        self.case = "a" * 32
        self.fixture.command(dict(command="create", case_id=self.case))
        self.cookie = self.fixture.db.execute("SELECT session FROM cases").fetchone()[0]

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def review(self, local, instant, slot="early"):
        return self.fixture.get(self.case, "review", dict(client_local=[local],
            client_utc=[instant], zone=["America/New_York"], slot=[slot]), self.cookie)

    def test_both_fold_occurrences_are_independently_verified(self):
        self.assertEqual(self.review("2026-11-01T01:30", "2026-11-01T05:30:00.000Z")[0], 200)
        self.assertEqual(self.review("2026-11-01T01:30", "2026-11-01T06:30:00.000Z", "late")[0], 200)
        persisted = fixture.snapshot(self.database, self.case)
        self.assertEqual([row["valid"] for row in persisted["client_selections"]], [1, 1])
        self.assertEqual(len(persisted["offers"]), 2)
        self.assertEqual(persisted["reservations"], [])

    def test_gap_and_forged_conversion_cannot_create_an_offer(self):
        self.assertEqual(self.review("2026-03-08T02:30", "2026-11-01T05:30:00.000Z")[0], 400)
        self.assertEqual(self.review("2026-11-01T01:30", "2026-11-01T06:30:00.000Z")[0], 400)
        persisted = fixture.snapshot(self.database, self.case)
        self.assertEqual([row["valid"] for row in persisted["client_selections"]], [0, 0])
        self.assertEqual(runner.booking_snapshot_shape_errors(persisted), [])
        self.assertEqual(persisted["offers"], [])
        self.assertEqual(persisted["reservations"], [])

    def test_availability_uses_client_module_and_authenticated_data(self):
        status, body, _ = self.fixture.get(self.case, "availability", {}, self.cookie)
        self.assertEqual(status, 200)
        self.assertIn('type="datetime-local"', body)
        self.assertIn('type="module"', body)
        self.assertEqual(self.fixture.get(self.case, "availability-data", {}, "")[0], 403)
        status, _, headers = self.fixture.get(self.case, "timezone-resolution.mjs", {}, self.cookie)
        self.assertEqual(status, 200)
        self.assertIn(("Content-Type", "text/javascript; charset=utf-8"), headers)

    def reserve(self, offer, key="b" * 32):
        from urllib.parse import urlencode
        return self.fixture.post(self.case, "reserve", urlencode(dict(offer=offer,
            request_key=key, name="Zoë Test", email="zoe@example.test")).encode(), self.cookie)

    def latest_offer(self):
        return self.fixture.db.execute("SELECT offer FROM offers ORDER BY rowid DESC LIMIT 1").fetchone()[0]

    def test_cancelled_offer_cannot_reserve_but_fresh_review_can(self):
        from urllib.parse import urlencode
        self.review("2026-11-01T01:30", "2026-11-01T05:30:00.000Z")
        old = self.latest_offer()
        body = urlencode(dict(offer=old)).encode()
        self.assertEqual(self.fixture.post(self.case, "cancel", body, self.cookie)[0], 303)
        self.assertEqual(self.fixture.post(self.case, "cancel", body, self.cookie)[0], 303)
        self.assertEqual(self.reserve(old)[0], 409)
        self.assertEqual(fixture.snapshot(self.database, self.case)["reservations"], [])
        self.review("2026-11-01T01:30", "2026-11-01T05:30:00.000Z")
        fresh = self.latest_offer()
        self.assertNotEqual(old, fresh)
        self.assertEqual(self.reserve(fresh)[0], 303)
        self.assertEqual(self.reserve(fresh)[0], 303)
        data = fixture.snapshot(self.database, self.case)
        self.assertEqual(len(data["reservations"]), 1)
        self.assertEqual(data["reservations"][0]["offer"], fresh)
        self.assertEqual([row["status"] for row in data["requests"]], [409, 303, 303])
        self.assertEqual(len(data["cancellations"]), 1)
        self.assertEqual(runner.durable_booking_ownership_errors(data, self.case), [])
        self.assertEqual(self.fixture.post(self.case, "cancel", urlencode(dict(offer=fresh)).encode(), self.cookie)[0], 409)

    def test_options_survive_confirmation_and_duplicate_request(self):
        query = dict(client_local=["2026-11-01T01:30"], client_utc=["2026-11-01T05:30:00.000Z"],
                     slot=["early"], zone=["America/New_York"], appointment_option=["phone"])
        status, body, _ = self.fixture.get(self.case, "review", query, self.cookie)
        self.assertEqual(status, 200)
        self.assertIn('id="appointment-option">phone', body)
        offer = self.latest_offer()
        self.assertEqual(self.reserve(offer)[0], 303)
        self.assertEqual(self.reserve(offer)[0], 303)
        status, body, _ = self.fixture.get(self.case, "confirmation", {}, self.cookie)
        self.assertEqual(status, 200)
        self.assertIn('id="appointment-option">phone', body)
        data = fixture.snapshot(self.database, self.case)
        self.assertEqual(data["offer_options"], [dict(case_id=self.case, offer=offer, appointment_option="phone")])
        self.assertEqual(runner.durable_booking_ownership_errors(data, self.case), [])
        self.assertEqual(len(data["reservations"]), 1)

    def test_invalid_option_and_duplicate_parameters_create_no_offer(self):
        query = dict(client_local=["2026-11-01T01:30"], client_utc=["2026-11-01T05:30:00.000Z"],
                     slot=["early"], zone=["America/New_York"], appointment_option=["unknown"])
        self.assertEqual(self.fixture.get(self.case, "review", query, self.cookie)[0], 400)
        query["appointment_option"] = ["video", "phone"]
        self.assertEqual(self.fixture.get(self.case, "review", query, self.cookie)[0], 400)
        self.assertEqual(fixture.snapshot(self.database, self.case)["offers"], [])

    def test_foreign_cookie_cannot_cancel_owned_offer(self):
        from urllib.parse import urlencode
        self.review("2026-11-01T01:30", "2026-11-01T05:30:00.000Z")
        offer = self.latest_offer()
        other = "c" * 32
        self.fixture.command(dict(command="create", case_id=other))
        cookie = self.fixture.db.execute("SELECT session FROM cases WHERE case_id=?", (other,)).fetchone()[0]
        self.assertEqual(self.fixture.post(self.case, "cancel", urlencode(dict(offer=offer)).encode(), cookie)[0], 403)
        self.assertEqual(fixture.snapshot(self.database, self.case)["cancellations"], [])

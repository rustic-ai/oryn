"""Independent persisted-oracle rejection and transaction regression tests."""

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
    "shopping_runner", ROOT / "scripts/run-oil-shopping-outcomes.py"
)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
CONTRACT = json.loads((ROOT / "test-harness/workflows/shopping-cases.json").read_text())
CASE_ID = "a" * 32
BASE = "http://127.0.0.1:12345/shop/" + CASE_ID


class ShoppingOracleTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / "shop.sqlite3"
        self.fixture = runner.receiver.Fixture(self.database)

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def create(self, scenario="normal", case_id=CASE_ID):
        self.fixture.command(dict(command="create", case_id=case_id, scenario=scenario))
        return self.fixture.db.execute(
            "SELECT session FROM cases WHERE case_id=?", (case_id,)
        ).fetchone()[0]

    def post(self, action, fields, cookie, case_id=CASE_ID):
        return self.fixture.post(case_id, action, urlencode(fields).encode(), cookie)[0]

    def cart(self, cookie, variant="blue", quantity="2"):
        self.assertEqual(self.post("cart", dict(variant=variant, quantity=quantity), cookie), 303)

    def quote(self, cookie):
        self.assertEqual(self.fixture.get(CASE_ID, "review", {}, cookie)[0], 200)
        return self.fixture.db.execute("SELECT quote FROM quotes ORDER BY rowid DESC").fetchone()[0]

    def checkout(self, cookie, quote):
        return self.post(
            "checkout", dict(quote=quote, name="Zoë Test", email="zoe@example.test"), cookie
        )

    def test_persisted_rows_survive_writer_close(self):
        cookie = self.create()
        self.cart(cookie)
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 303)
        self.fixture.db.close()
        data = runner.receiver.snapshot(self.database, CASE_ID)
        self.assertEqual(data["orders"][0]["total"], 5000)
        self.assertEqual(data["cases"][0]["blue_stock"], 8)
        self.assertNotIn("session", data["cases"][0])

    def test_price_revalidation_requires_a_new_quote(self):
        cookie = self.create("price")
        self.cart(cookie)
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 409)
        self.assertEqual(runner.receiver.snapshot(self.database, CASE_ID)["orders"], [])
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 303)
        self.assertEqual(
            runner.receiver.snapshot(self.database, CASE_ID)["orders"][0]["total"], 6000
        )

    def test_stock_recovery_requires_available_variant(self):
        cookie = self.create("stock")
        self.cart(cookie)
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 409)
        self.cart(cookie, "green")
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 303)
        data = runner.receiver.snapshot(self.database, CASE_ID)
        self.assertEqual((data["cases"][0]["blue_stock"], data["cases"][0]["green_stock"]), (0, 8))

    def test_concurrent_duplicate_checkout_depletes_inventory_once(self):
        cookie = self.create()
        self.cart(cookie)
        quote = self.quote(cookie)
        with ThreadPoolExecutor(max_workers=2) as executor:
            outcomes = list(executor.map(lambda _: self.checkout(cookie, quote), range(2)))
        self.assertEqual(outcomes, [303, 303])
        data = runner.receiver.snapshot(self.database, CASE_ID)
        self.assertEqual(len(data["orders"]), 1)
        self.assertEqual(data["cases"][0]["blue_stock"], 8)

    def test_cart_change_invalidates_earlier_quote(self):
        cookie = self.create()
        self.cart(cookie)
        quote = self.quote(cookie)
        self.cart(cookie, quantity="3")
        self.assertEqual(self.checkout(cookie, quote), 409)
        self.assertEqual(runner.receiver.snapshot(self.database, CASE_ID)["orders"], [])

    def test_cookie_and_quote_are_scoped_to_case(self):
        first, second = self.create(), self.create(case_id="b" * 32)
        self.assertNotEqual(first, second)
        self.assertEqual(self.fixture.get(CASE_ID, "cart", {}, second)[0], 401)
        self.assertEqual(self.post("cart", dict(variant="blue", quantity="2"), second), 401)
        self.cart(first)
        self.assertEqual(self.checkout(first, "f" * 32), 409)
        self.assertEqual(runner.receiver.snapshot(self.database, CASE_ID)["orders"], [])

    def test_invalid_cart_payload_never_creates_cart(self):
        cookie = self.create()
        for fields in [
            dict(variant="blue", quantity="6"),
            dict(variant="unknown", quantity="2"),
            dict(variant="blue", quantity="2", total="1"),
        ]:
            self.assertEqual(self.post("cart", fields, cookie), 400)
        self.assertEqual(runner.receiver.snapshot(self.database, CASE_ID)["carts"], [])

    def valid_outcome(self):
        cookie = self.create()
        self.cart(cookie, quantity="1")
        self.cart(cookie)
        self.assertEqual(self.checkout(cookie, self.quote(cookie)), 303)
        self.fixture.get(CASE_ID, "order", {}, cookie)
        self.fixture.get(CASE_ID, "order", {}, cookie)
        case = CONTRACT["cases"][0]
        lines = runner.task_lines(CONTRACT, case, BASE)
        count = sum(line.split()[0] in ("click", "type", "select") for line in lines)
        frames = [
            dict(kind="action", result=dict(action_id=n + 1, execution_domain="native"))
            for n in range(count)
        ]
        for suffix, nodes in [
            ("product/desk", []),
            ("catalog?search=field&category=outdoor", [dict(name="Field kit")]),
            ("product/field", []),
            ("cart", [dict(name="Cart quantity", value="2")]),
            ("order", []),
        ]:
            frames.append(
                dict(
                    kind="observation",
                    observation=dict(page=dict(url=BASE + "/" + suffix), nodes=nodes),
                )
            )
        return case, lines, frames, runner.receiver.snapshot(self.database, CASE_ID)

    def test_complete_persisted_oracle_passes(self):
        case, lines, frames, data = self.valid_outcome()
        self.assertEqual(runner.validate(case, lines, BASE, frames, data), [])

    def test_confirmation_and_action_frames_cannot_replace_correct_database(self):
        case, lines, frames, original = self.valid_outcome()
        for corruption in (
            "total",
            "quantity",
            "inventory",
            "duplicate",
            "reload",
            "request",
            "cookie",
            "missing",
        ):
            with self.subTest(corruption=corruption):
                data = copy.deepcopy(original)
                if corruption in ("total", "quantity"):
                    data["orders"][0][corruption] = 1
                elif corruption == "inventory":
                    data["cases"][0]["blue_stock"] = 6
                elif corruption == "duplicate":
                    data["orders"].append(data["orders"][0].copy())
                elif corruption == "reload":
                    data["views"] = []
                elif corruption == "request":
                    data["requests"].pop()
                elif corruption == "cookie":
                    data["requests"][0]["cookie_valid"] = 0
                else:
                    data["orders"] = []
                self.assertTrue(runner.validate(case, lines, BASE, frames, data))


if __name__ == "__main__":
    unittest.main()

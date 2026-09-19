"""Native framed checkout's independent authority and persistence controls."""
import importlib.util
import copy
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('shopping_frontend', ROOT/'test-harness/workflows/shopping_fixture_v2.py')
frontend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(frontend)

runner_spec = importlib.util.spec_from_file_location('shopping_ownership_runner', ROOT/'scripts/run-oil-shopping-outcomes.py')
runner = importlib.util.module_from_spec(runner_spec)
runner_spec.loader.exec_module(runner)


class FramedCheckoutTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.database = Path(self.temp.name)/'shop.sqlite3'
        self.fixture = frontend.Fixture(self.database)
        self.fixture.payment_origin = 'http://127.0.0.1:4101'
        self.fixture.shop_origin = 'http://127.0.0.1:4100'
        self.case = 'a'*32
        self.fixture.command(dict(command='create', case_id=self.case))
        self.cookie = self.fixture.db.execute('SELECT session FROM cases').fetchone()[0]
        self.fixture.post(self.case, 'cart', b'variant=blue&quantity=2', self.cookie)
        self.fixture.get(self.case, 'review', {}, self.cookie)
        self.payment = self.fixture.db.execute('SELECT * FROM payments').fetchone()
        self.body = urlencode(dict(quote=self.payment['quote'], name='Zoë Test', email='zoe@example.test')).encode()

    def tearDown(self):
        self.fixture.db.close()
        self.temp.cleanup()

    def test_checkout_requires_authorization_then_retries_keep_one_order(self):
        self.assertEqual(self.fixture.post(self.case, 'checkout', self.body, self.cookie)[0], 409)
        self.assertEqual(frontend.snapshot(self.database, self.case)['orders'], [])
        self.assertEqual(self.fixture.payment(self.payment['token'], True, self.fixture.payment_origin)[0], 200)
        for _ in range(2):
            self.assertEqual(self.fixture.post(self.case, 'checkout', self.body, self.cookie)[0], 303)
        data = frontend.snapshot(self.database, self.case)
        self.assertEqual(len(data['orders']), 1)
        self.assertEqual(data['cases'][0]['blue_stock'], 8)
        self.assertEqual(data['orders'][0]['quote'], data['payments'][0]['quote'])
        self.assertNotIn('token', data['payments'][0])

    def test_foreign_origin_replay_and_unknown_token_reject(self):
        token = self.payment['token']
        self.assertEqual(self.fixture.payment(token, True, self.fixture.shop_origin)[0], 403)
        self.assertEqual(self.fixture.payment('b'*64, True, self.fixture.payment_origin)[0], 404)
        self.assertEqual(self.fixture.payment(token, True, self.fixture.payment_origin)[0], 200)
        self.assertEqual(self.fixture.payment(token, True, self.fixture.payment_origin)[0], 409)
        self.assertEqual(frontend.snapshot(self.database, self.case)['orders'], [])

    def test_quote_authority_does_not_transfer_to_another_case(self):
        foreign = 'b'*32
        self.fixture.command(dict(command='create', case_id=foreign))
        foreign_cookie = self.fixture.db.execute('SELECT session FROM cases WHERE case_id=?', (foreign,)).fetchone()[0]
        self.fixture.payment(self.payment['token'], True, self.fixture.payment_origin)
        self.assertEqual(self.fixture.post(foreign, 'checkout', self.body, foreign_cookie)[0], 409)
        self.assertEqual(frontend.snapshot(self.database, foreign)['orders'], [])

    def test_authorization_does_not_override_stock_or_price(self):
        self.fixture.payment(self.payment['token'], True, self.fixture.payment_origin)
        with self.fixture.db:
            self.fixture.db.execute('UPDATE cases SET unit_price=3000 WHERE case_id=?', (self.case,))
        self.assertEqual(self.fixture.post(self.case, 'checkout', self.body, self.cookie)[0], 409)
        self.assertEqual(frontend.snapshot(self.database, self.case)['orders'], [])

    def test_payment_evidence_joins_exact_owned_quotes_and_receipts(self):
        token = self.payment['token']
        self.fixture.payment(token)
        self.fixture.payment(token, True, self.fixture.payment_origin)
        self.fixture.post(self.case, 'checkout', self.body, self.cookie)
        self.fixture.post(self.case, 'checkout', self.body, self.cookie)
        data = frontend.snapshot(self.database, self.case)
        case = dict(scenario='normal', frontend_v2=True)
        base = self.fixture.shop_origin + '/shop/' + self.case
        self.assertEqual(runner.validate_persisted_ownership(case, base, data), [])
        for table, field, value in (
                ('payments', 'case_id', 'b' * 32),
                ('payments', 'quote', 'b' * 32),
                ('payments', 'authorized', True),
                ('payments', 'receipt', None),
                ('payment_events', 'case_id', 'b' * 32),
                ('payment_events', 'quote', 'b' * 32)):
            with self.subTest(table=table, field=field):
                changed = copy.deepcopy(data)
                changed[table][0][field] = value
                self.assertTrue(runner.validate_persisted_ownership(case, base, changed))
        changed = copy.deepcopy(data)
        changed['payments'].append(copy.deepcopy(changed['payments'][0]))
        self.assertTrue(runner.validate_persisted_ownership(case, base, changed))

    def test_payment_history_cannot_be_reassigned_between_recovery_quotes(self):
        # Each review owns its own authorization history even if aggregate event
        # counts and action names still match the expected two-review sequence.
        self.fixture.payment(self.payment['token'])
        self.fixture.payment(self.payment['token'], True, self.fixture.payment_origin)
        self.fixture.get(self.case, 'review', {}, self.cookie)
        second = self.fixture.db.execute('SELECT * FROM payments ORDER BY rowid DESC').fetchone()
        self.fixture.payment(second['token'])
        self.fixture.payment(second['token'], True, self.fixture.payment_origin)
        body = urlencode(dict(quote=second['quote'], name='Zoë Test', email='zoe@example.test')).encode()
        self.fixture.post(self.case, 'checkout', body, self.cookie)
        data = frontend.snapshot(self.database, self.case)
        case = dict(scenario='normal', frontend_v2=True)
        base = self.fixture.shop_origin + '/shop/' + self.case
        self.assertEqual(runner.validate_persisted_ownership(case, base, data), [])
        changed = copy.deepcopy(data)
        changed['payment_events'][1]['quote'] = second['quote']
        self.assertTrue(runner.validate_persisted_ownership(case, base, changed))
        changed = copy.deepcopy(data)
        changed['payments'][1]['receipt'] = changed['payments'][0]['receipt']
        self.assertTrue(runner.validate_persisted_ownership(case, base, changed))

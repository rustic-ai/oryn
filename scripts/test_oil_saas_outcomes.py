"""Persisted authenticated-record fixture tests; not browser qualification."""
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import copy
import json
from http.cookiejar import CookieJar
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch
from urllib.parse import urlencode
from urllib.request import build_opener, HTTPCookieProcessor
from urllib.error import HTTPError

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("saas_fixture", ROOT / "test-harness/workflows/saas_fixture.py")
receiver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(receiver)
CASE = "a" * 32

support_spec = importlib.util.spec_from_file_location("saas_http_support", ROOT / "scripts/run-oil-forms-outcomes.py")
support = importlib.util.module_from_spec(support_spec)
support_spec.loader.exec_module(support)

runner_spec = importlib.util.spec_from_file_location("saas_runner", ROOT / "scripts/run-oil-saas-outcomes.py")
runner = importlib.util.module_from_spec(runner_spec)
runner_spec.loader.exec_module(runner)


class SaaSValidatorTests(unittest.TestCase):
    def setUp(self):
        contract = json.loads((ROOT / "test-harness/workflows/saas-cases.json").read_text())
        self.case = contract["cases"][0]
        self.base = "http://127.0.0.1:12345/saas/" + CASE
        self.lines = runner.task_lines(contract, self.case, self.base)
        count = sum(line.split()[0] in ("click", "type", "select") for line in self.lines)
        self.frames = [{"kind": "action", "result": {"action_id": index + 1, "execution_domain": "native"}} for index in range(count)]
        self.frames += [{"kind": "structured", "value": value} for value in self.case["expected_extractions"]]
        self.frames += [{"kind": "observation", "observation": {"page": {"url": self.base + "/login"}}}]
        self.data = {
            "cases": [{"case_id": CASE, "authenticated": 0, "logins": 1}],
            "records": [dict(case_id=CASE, **self.case["expected_record"])],
            "requests": [dict(case_id=CASE, body_sha256="0" * 64, **row) for row in self.case["expected_requests"]],
            "views": [dict(case_id=CASE, **row) for row in self.case["expected_views"]],
        }

    def test_consistent_contract_is_accepted(self):
        self.assertEqual(runner.validate(self.case, self.lines, self.base, self.frames, self.data), [])

    def test_green_browser_output_does_not_replace_persisted_writes(self):
        data = copy.deepcopy(self.data)
        data["records"] = []
        self.assertIn("persisted record/version mismatch", runner.validate(self.case, self.lines, self.base, self.frames, data))

    def test_unauthorized_write_and_wrong_final_session_are_rejected(self):
        data = copy.deepcopy(self.data)
        data["requests"][1]["authorized"] = 0
        data["cases"][0]["authenticated"] = 1
        errors = runner.validate(self.case, self.lines, self.base, self.frames, data)
        self.assertIn("persisted authorization/mutation sequence mismatch", errors)
        self.assertIn("persisted session lifecycle mismatch", errors)

    def test_missing_actions_and_forged_case_identity_are_rejected(self):
        data = copy.deepcopy(self.data)
        data["requests"][0]["case_id"] = "b" * 32
        errors = runner.validate(self.case, self.lines, self.base, self.frames[1:], data)
        self.assertIn("native action identity/count mismatch", errors)
        self.assertIn("persisted request identity mismatch", errors)

    def test_missing_page_authorization_evidence_is_rejected(self):
        data = copy.deepcopy(self.data)
        data["views"] = []
        self.assertIn("persisted page authorization sequence mismatch", runner.validate(self.case, self.lines, self.base, self.frames, data))


class SaaSOAuthValidatorTests(SaaSValidatorTests):
    def setUp(self):
        super().setUp()
        self.case['expected_oauth'] = dict(events=[dict(action='token_exchange', outcome=200)],
                                           codes=[dict(consumed=1)], tokens=[dict(revoked=0)])
        self.case['expected_login_events'] = [dict(action='authenticated', outcome=303)]
        self.case['expected_concurrent_edits'] = [dict(title='Concurrent', version=2)]
        self.data['oauth'] = copy.deepcopy(self.case['expected_oauth'])
        self.data['login_events'] = copy.deepcopy(self.case['expected_login_events'])
        self.data['concurrent_edits'] = copy.deepcopy(self.case['expected_concurrent_edits'])

    def test_missing_exchange_callback_or_concurrent_writer_cannot_qualify(self):
        for field in ('oauth', 'login_events', 'concurrent_edits'):
            data = copy.deepcopy(self.data)
            del data[field]
            self.assertTrue(runner.validate(self.case, self.lines, self.base, self.frames, data), field)

    def test_unconsumed_grant_or_revoked_token_cannot_qualify(self):
        for field, value in (('codes', [dict(consumed=0)]), ('tokens', [dict(revoked=1)]),
                             ('events', [dict(action='token_exchange', outcome=400)])):
            data = copy.deepcopy(self.data)
            data['oauth'][field] = value
            self.assertIn('persisted OAuth code/token/authorization sequence mismatch',
                          runner.validate(self.case, self.lines, self.base, self.frames, data))


class NumericLoopbackServerTests(unittest.TestCase):
    def test_numeric_bind_preserves_address_without_reverse_dns(self):
        with patch("socket.getfqdn", side_effect=AssertionError("unexpected DNS lookup")) as resolver:
            server = receiver.NumericLoopbackHTTPServer(("127.0.0.1", 0), receiver.BaseHTTPRequestHandler)
            try:
                self.assertEqual(server.server_name, "127.0.0.1")
                self.assertGreater(server.server_port, 0)
                self.assertEqual(server.server_address, (server.server_name, server.server_port))
                self.assertEqual(server.socket.getsockname(), server.server_address)
                resolver.assert_not_called()
            finally:
                server.server_close()


class OracleStartupDiagnosticsTests(unittest.TestCase):
    def test_failed_startup_retains_stderr_and_reaps_before_error(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory).resolve()
            with patch.object(support, "check_execution_generation"), \
                    patch.object(support.subprocess, "Popen") as launch, \
                    patch.object(support.Oracle, "read", side_effect=ValueError("invalid fixture control frame")):
                launch.return_value.poll.return_value = 1
                launch.return_value.returncode = 1
                def started(*args, **kwargs):
                    kwargs["stderr"].write(b"fixture prerequisite unavailable")
                    kwargs["stderr"].flush()
                    return launch.return_value
                launch.side_effect = started
                with self.assertRaisesRegex(ValueError, "fixture prerequisite unavailable"):
                    support.Oracle(output)
                launch.return_value.stdin.close.assert_called_once()
                launch.return_value.stdout.close.assert_called_once()


class SaaSHTTPTests(unittest.TestCase):
    def test_cookie_login_persisted_update_expiry_and_sealing(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            database = output / "saas.sqlite3"
            oracle = support.Oracle(output, ROOT / "test-harness/workflows/saas_fixture.py", ("--database", database))
            try:
                oracle.request("create", CASE)
                browser = build_opener(HTTPCookieProcessor(CookieJar()))
                base = oracle.origin + "/saas/" + CASE

                def fetch(path, fields=None):
                    data = None if fields is None else urlencode(fields).encode()
                    with browser.open(base + "/" + path, data=data, timeout=5) as response:
                        return response.read().decode()

                def csrf(page):
                    return re.search(r'name=csrf value="([0-9a-f]+)"', page)[1]

                with self.assertRaises(HTTPError) as rejected:
                    fetch("records")
                rejected.exception.close()
                self.assertEqual(rejected.exception.code, 401)
                page = fetch("login", dict(username="agent@example.test", password="fixture-only-password"))
                token = csrf(page)
                page = fetch("create", dict(csrf=token, title="Draft", version="0"))
                self.assertIn("Draft", page)
                page = fetch("update", dict(csrf=csrf(page), title="Approved", version="1"))
                self.assertIn("Approved", page)
                oracle.request("expire", CASE)
                with self.assertRaises(HTTPError) as expired:
                    fetch("update", dict(csrf=token, title="Forbidden", version="2"))
                expired.exception.close()
                self.assertEqual(expired.exception.code, 401)
                self.assertEqual(oracle.request("seal", CASE), {"sealed": True})
                data = oracle.request("snapshot", CASE)
                self.assertEqual(data["records"], [{"case_id": CASE, "title": "Approved", "version": 2}])
                self.assertEqual([row["outcome"] for row in data["requests"]], [303, 303, 303, 401])
                with self.assertRaises(ValueError):
                    oracle.request("expire", CASE)
            finally:
                cleanup = oracle.close()
            self.assertEqual(cleanup, {"returncode": 0, "forced": False})
            self.assertEqual(receiver.snapshot(database, CASE)["records"], data["records"])


class SaaSFixtureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / "saas.sqlite3"
        self.fixture = receiver.Fixture(self.database)
        self.fixture.command({"command": "create", "case_id": CASE})

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def post(self, action, fields, cookie="", case=CASE):
        return self.fixture.post(case, action, urlencode(fields).encode(), cookie)[0]

    def login(self):
        self.assertEqual(self.post("login", {"username": "agent@example.test", "password": "fixture-only-password"}), 303)
        row = self.fixture.db.execute("SELECT session,csrf FROM cases WHERE case_id=?", (CASE,)).fetchone()
        return row["session"], row["csrf"]

    def test_create_update_persists_after_writer_close(self):
        session, csrf = self.login()
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Draft", version="0"), session), 303)
        self.assertEqual(self.post("update", dict(csrf=csrf, title="Approved", version="1"), session), 303)
        self.fixture.db.close()
        data = receiver.snapshot(self.database, CASE)
        self.assertEqual(data["records"], [{"case_id": CASE, "title": "Approved", "version": 2}])
        self.assertNotIn("session", data["cases"][0])
        self.assertNotIn("csrf", data["cases"][0])

    def test_expired_session_cannot_write_and_relogin_rotates_authority(self):
        session, csrf = self.login()
        self.fixture.command({"command": "expire", "case_id": CASE})
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Draft", version="0"), session), 401)
        new_session, new_csrf = self.login()
        self.assertNotEqual(session, new_session)
        self.assertNotEqual(csrf, new_csrf)
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Draft", version="0"), new_session), 403)
        self.assertEqual(self.post("create", dict(csrf=new_csrf, title="Draft", version="0"), new_session), 303)

    def test_scheduled_expiry_rejects_edit_without_losing_record(self):
        other = "c" * 32
        self.fixture.command(dict(command="create", case_id=other, scenario="expire_after_create"))
        self.assertEqual(self.post("login", dict(username="agent@example.test", password="fixture-only-password"), case=other), 303)
        row = self.fixture.db.execute("SELECT session,csrf FROM cases WHERE case_id=?", (other,)).fetchone()
        session, csrf = row["session"], row["csrf"]
        self.assertEqual(self.fixture.get(other, "records", {}, session)[0], 200)
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Draft", version="0"), session, other), 303)
        self.assertEqual(self.fixture.get(other, "records", {}, session)[0], 200)
        self.assertEqual(self.post("update", dict(csrf=csrf, title="Rejected", version="1"), session, other), 401)
        data = receiver.snapshot(self.database, other)
        self.assertEqual(data["records"], [dict(case_id=other, title="Draft", version=1)])
        self.assertEqual(data["expirations"], [dict(case_id=other, reason="after_create")])

    def test_case_isolation_and_duplicate_save(self):
        session, csrf = self.login()
        other = "b" * 32
        self.fixture.command({"command": "create", "case_id": other})
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Wrong case", version="0"), session, other), 401)
        fields = dict(csrf=csrf, title="Draft", version="0")
        with ThreadPoolExecutor(max_workers=2) as executor:
            statuses = list(executor.map(lambda _: self.post("create", fields, session), range(2)))
        self.assertEqual(sorted(statuses), [303, 409])
        self.assertEqual(len(receiver.snapshot(self.database, CASE)["records"]), 1)
        self.assertEqual(receiver.snapshot(self.database, other)["records"], [])

    def test_stale_update_and_logout(self):
        session, csrf = self.login()
        self.assertEqual(self.post("create", dict(csrf=csrf, title="Draft", version="0"), session), 303)
        self.assertEqual(self.post("update", dict(csrf=csrf, title="Lost update", version="0"), session), 409)
        self.assertEqual(self.post("logout", dict(csrf=csrf), session), 303)
        self.assertEqual(self.fixture.get(CASE, "records", {}, session)[0], 401)
        self.assertEqual(receiver.snapshot(self.database, CASE)["records"][0]["title"], "Draft")

    def test_invalid_login_does_not_authorize_or_leak_credentials(self):
        self.assertEqual(self.post("login", dict(username="agent@example.test", password="wrong")), 401)
        self.assertEqual(receiver.snapshot(self.database, CASE)["cases"][0]["authenticated"], 0)
        self.assertNotIn("wrong", str(receiver.snapshot(self.database, CASE)))

    def test_duplicate_fields_are_rejected(self):
        session, csrf = self.login()
        body = urlencode(dict(csrf=csrf, title="Draft", version="0")).encode() + b"&title=Overwrite"
        self.assertEqual(self.fixture.post(CASE, "create", body, session)[0], 403)
        self.assertEqual(receiver.snapshot(self.database, CASE)["records"], [])


if __name__ == "__main__":
    unittest.main()

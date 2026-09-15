"""Independent research request persistence and fail-closed outcome checks."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "research_runner", ROOT / "scripts/run-oil-research-outcomes.py"
)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
CASE = "a" * 32
BASE = "http://127.0.0.1:12345/research/" + CASE


class ResearchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / "research.sqlite3"
        self.fixture = runner.receiver.Fixture(self.database)
        self.fixture.command(dict(command="create", case_id=CASE))
        self.contract = json.loads(
            (ROOT / "test-harness/workflows/research-cases.json").read_text()
        )

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def records(self, case):
        for visit in case["visits"]:
            status, body, _ = self.fixture.get(CASE, visit["path"], visit["query"])
            self.assertEqual(status, visit["status"])
            self.assertTrue(body)
        return runner.receiver.snapshot(self.database, CASE)

    def frames(self, case):
        lines = runner.task_lines(self.contract, case, BASE)
        count = sum(line.split()[0] in ("click", "type", "select") for line in lines)
        frames = [
            dict(kind="action", result=dict(action_id=i + 1, execution_domain="native"))
            for i in range(count)
        ]
        expected = json.loads(
            json.dumps(case["expected"]).replace("{base}", "/research/" + CASE)
        )
        frames += [dict(kind="structured", value=value) for value in expected]
        frames += [
            dict(kind="observation", observation=dict(page=dict(url=BASE + "/" + path)))
            for path in case["sources"]
        ]
        frames += [
            dict(
                kind="observation",
                observation=dict(page=dict(url=BASE + "/" + case["final"])),
            )
        ]
        return frames

    def test_versioned_cases_have_consistent_oracles(self):
        for case in self.contract["cases"]:
            with self.subTest(case=case["id"]):
                self.fixture.db.execute("DELETE FROM visits")
                self.fixture.db.commit()
                data = self.records(case)
                self.assertEqual(
                    runner.validate(
                        case,
                        runner.task_lines(self.contract, case, BASE),
                        BASE,
                        self.frames(case),
                        data,
                    ),
                    [],
                )

    def test_validator_rejects_facts_sources_requests_and_action_corruption(self):
        case = self.contract["cases"][0]
        data, frames = self.records(case), self.frames(case)
        lines = runner.task_lines(self.contract, case, BASE)
        bad_facts = copy.deepcopy(frames)
        next(frame for frame in bad_facts if frame["kind"] == "structured")["value"] = (
            "invented fact"
        )
        bad_sources = copy.deepcopy(frames)
        for frame in bad_sources:
            if frame["kind"] == "observation":
                frame["observation"]["page"]["url"] = "https://wrong-source.invalid/"
        bad_actions = copy.deepcopy(frames)
        bad_actions[0]["result"]["action_id"] = True
        bad_link = copy.deepcopy(frames)
        next(
            frame
            for frame in bad_link
            if frame.get("value")
            == [
                {
                    "text": "Canonical source",
                    "href": "/research/" + CASE + "/article/alpha",
                }
            ]
        )["value"][0]["href"] = "/wrong"
        for malformed in (
            [],
            bad_facts,
            bad_sources,
            bad_actions,
            bad_link,
            frames + [{"kind": "unsupported"}],
        ):
            self.assertTrue(runner.validate(case, lines, BASE, malformed, data))
        for mutation in ("missing_visit", "wrong_status", "wrong_query", "wrong_case"):
            changed = copy.deepcopy(data)
            if mutation == "missing_visit":
                changed["visits"].pop()
            elif mutation == "wrong_status":
                changed["visits"][0]["status"] = 500
            elif mutation == "wrong_query":
                changed["visits"][1]["query"] = "{}"
            else:
                changed["cases"][0]["case_id"] = "b" * 32
            self.assertTrue(runner.validate(case, lines, BASE, frames, changed))

    def test_requests_persist_after_writer_closes_and_cases_are_isolated(self):
        self.fixture.get(CASE, "search", {"q": ["absent"]})
        self.fixture.command(dict(command="create", case_id="b" * 32))
        self.assertEqual(
            runner.receiver.snapshot(self.database, "b" * 32)["visits"], []
        )
        self.assertEqual(self.fixture.get("c" * 32, "start", {})[0], 404)
        self.fixture.db.close()
        self.assertEqual(
            len(runner.receiver.snapshot(self.database, CASE)["visits"]), 1
        )

    def test_redirect_and_dynamic_content_have_real_routes(self):
        status, _, headers = self.fixture.get(CASE, "old-alpha", {})
        self.assertEqual(status, 303)
        self.assertEqual(
            dict(headers)["Location"], "/research/" + CASE + "/article/alpha"
        )
        _, body, _ = self.fixture.get(CASE, "search", {"q": ["dynamic"]})
        self.assertIn("fetch(", body)
        _, fragment, _ = self.fixture.get(CASE, "api", {"q": ["energy"], "page": ["2"]})
        self.assertIn("Beta report", fragment)
        self.assertNotIn("Alpha report", fragment)


if __name__ == "__main__":
    unittest.main()

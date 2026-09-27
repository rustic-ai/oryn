"""Independent research request persistence and fail-closed outcome checks."""

import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

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
        extraction_steps=[(i,line) for i,line in enumerate(lines) if line.startswith("extract ")]
        self.assertEqual(len(expected),len(extraction_steps))
        frames += [dict(kind="structured", value=value,command_provenance=dict(phase="steps",step=i,oil=line,output=0)) for value,(i,line) in zip(expected,extraction_steps)]
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

    def test_extraction_missing_extra_duplicate_and_selection_snapshot_controls(self):
        case=self.contract['cases'][0];data=self.records(case);lines=runner.task_lines(self.contract,case,BASE);frames=self.frames(case)
        extract_indexes=[i for i,row in enumerate(frames) if row['kind']=='structured']
        for changed in [frames[:extract_indexes[0]]+frames[extract_indexes[0]+1:],frames+[copy.deepcopy(frames[extract_indexes[0]])]]:
            self.assertTrue(runner.validate(case,lines,BASE,changed,data))
        changed=copy.deepcopy(frames);extra=copy.deepcopy(changed[extract_indexes[-1]])
        extra['command_provenance']['output']=1;changed.insert(extract_indexes[-1]+1,extra)
        self.assertTrue(runner.validate(case,lines,BASE,changed,data))
        unattributed=copy.deepcopy(frames);del unattributed[extract_indexes[0]]['command_provenance']
        self.assertTrue(runner.validate(case,lines,BASE,unattributed,data))


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



class StandaloneProvenanceTests(unittest.TestCase):
    def test_standalone_uses_native_step_evidence_and_admitted_worker_hash(self):
        import workflow_native_adapter as adapter
        case=dict(id='probe',scenario='normal',steps=['frame css(#child)','extract table'],expected=[{'rows':[[1]]}])
        session=dict(id='research',errors=[],cleanup_errors=[],workers_retired=True,steps=[dict(index=0,outputs=[dict(kind='structured',value={'selected':1})]),dict(index=1,outputs=[dict(kind='structured',value={'rows':[[1]]})])])
        evidence=dict(sessions=[session],resource_accounting=dict(complete=True,errors=[]))
        def execute(binary,worker,plan,directory,stop,worker_sha256=None,timeout_seconds=180):
            self.assertEqual(binary,Path('/executor'));self.assertEqual(worker_sha256,'a'*64);self.assertEqual(timeout_seconds,60)
            self.assertEqual([step['oil'] for step in plan['sessions'][0]['steps']],case['steps'])
            (directory/'native-process-cleanup.json').write_text(json.dumps(dict(group_settled=True,errors=[])))
            return evidence
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(adapter,'execute_native',side_effect=execute),mock.patch.object(runner,'validate',return_value=[]) as validate:
            oracle=mock.Mock(origin='http://127.0.0.1:1');oracle.request.return_value={}
            record=runner.run_case(oracle,{},case,1,Path('/oryn'),Path('/worker'),Path(tmp),Path('/executor'),'a'*64)
            self.assertTrue(record['functional_passed']);self.assertTrue(record['resource_complete']);self.assertTrue(record['passed'])
            frames=validate.call_args.args[3]
            self.assertEqual(runner.extraction_values(frames,case['steps']),case['expected'])
            saved=[json.loads(row) for row in (Path(tmp)/'probe-01/stdout.jsonl').read_text().splitlines()]
            self.assertEqual(saved,frames)

    def test_standalone_preserves_timeout_as_primary_when_cleanup_fails(self):
        import workflow_native_adapter as adapter
        from workflow_cleanup import CleanupFailure,NativeExecutionFailure
        case=dict(id='probe',scenario='normal',steps=['observe'])
        failure=CleanupFailure(NativeExecutionFailure('initiating deadline',timed_out=True),['permission denied'],True)
        with tempfile.TemporaryDirectory() as tmp,mock.patch.object(adapter,'execute_native',side_effect=failure):
            oracle=mock.Mock(origin='http://127.0.0.1:1')
            record=runner.run_case(oracle,{},case,1,Path('/oryn'),Path('/worker'),Path(tmp),Path('/executor'),'a'*64)
            self.assertFalse(record['passed']);self.assertTrue(record['timed_out'])
            self.assertEqual(record['primary_error'],'initiating deadline')
            self.assertEqual(record['cleanup_failure_details'],['permission denied'])
            self.assertFalse(record['cleanup_complete'])

if __name__ == "__main__":
    unittest.main()

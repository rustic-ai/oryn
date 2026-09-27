#!/usr/bin/env python3
"""Versioned OIL research outcomes with independent persisted request and source checks."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
from evidence_protocols import validate_protocol_pair
import secrets
import signal
from generation_gate import check_execution_generation
import subprocess
import sys
import time
import threading
from workflow_outcome_evidence import command_frames, extraction_values, terminal_verdict

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "test-harness/workflows"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


support = load_module(
    "research_outcome_support", ROOT / "scripts/run-oil-forms-outcomes.py"
)
receiver = load_module("research_receiver", FIXTURES / "research_fixture.py")


def task_lines(contract, case, base):
    return [line.replace("{base}", base) for line in case["steps"]]


def validate(case, lines, base, frames, data):
    from urllib.parse import urlsplit

    if not frames or any(
        not isinstance(frame, dict) or frame.get("kind") in ("error", "unsupported")
        for frame in frames
    ):
        return ["missing or failed OIL frames"]
    errors = []
    actions = [
        frame.get("result", {}) for frame in frames if frame.get("kind") == "action"
    ]
    if any(not isinstance(action, dict) for action in actions):
        return ["malformed action output"]
    ids = [(action.get("page_id"),action.get("action_id")) for action in actions]
    expected_count = case.get("expected_action_count",sum(
        line.split()[0] in ("click", "type", "select") for line in lines
    ))
    if (
        len(actions) != expected_count
        or any(type(identity) is not int or identity < 1 for _,identity in ids)
        or len(set(ids)) != len(ids)
        or any(action.get("execution_domain") != "native" for action in actions)
    ):
        errors.append("native action identity/count mismatch")
    expected = json.loads(
        json.dumps(case["expected"]).replace("{base}", urlsplit(base).path)
    )
    try:
        if extraction_values(frames, lines) != expected:
            errors.append("extracted facts/tables/source links mismatch")
    except ValueError as error:
        errors.append(str(error))
    observations = [
        frame.get("observation", {})
        for frame in frames
        if frame.get("kind") == "observation"
    ]
    if any(
        not isinstance(obs, dict) or not isinstance(obs.get("page"), dict)
        for obs in observations
    ):
        return errors + ["malformed observation"]
    urls = [obs["page"].get("url", "") for obs in observations]
    if not urls or urls[-1] != base + "/" + case["final"]:
        errors.append("final source location mismatch")
    sources = list(
        dict.fromkeys(url for url in urls if url.startswith(base + "/article/"))
    )
    if sources != [base + "/" + path for path in case["sources"]]:
        errors.append("canonical source URL provenance mismatch")
    visits = data.get("visits", [])
    actual_visits = [
        dict(path=row["path"], query=json.loads(row["query"]), status=row["status"])
        for row in visits
    ]
    if actual_visits != case["visits"]:
        errors.append("persisted request/redirect/recovery sequence mismatch")
    case_id = base.rsplit("/", 1)[-1]
    if data.get("cases") != [{"case_id": case_id}] or any(
        row["case_id"] != case_id or len(row.get("body_sha256", "")) != 64
        for row in visits
    ):
        errors.append("persisted case/body identity mismatch")
    return errors


def run_case(oracle, contract, case, sample, binary, worker, output, executor, worker_sha256):
    case_id = secrets.token_hex(16)
    directory = output / f"{case['id']}-{sample:02d}"
    directory.mkdir()
    base = f"{oracle.origin}/research/{case_id}"
    lines = task_lines(contract, case, base)
    task = directory / "task.oil"
    task.write_text("\n".join(lines) + "\n")
    record = dict(
        case=case["id"],
        sample=sample,
        case_id=case_id,
        base=base,
        errors=[],
        timed_out=False,
        passed=False,
        task_sha256=support.digest(task),
    )
    started = time.monotonic()
    try:
        oracle.request("create", case_id, scenario=case["scenario"])
        from workflow_native_adapter import execute_native
        from workflow_cleanup import failure_details
        plan=dict(schema_version=1,attempt_id=f"research:{case_id}:{sample}",sessions=[dict(id='research',steps=[dict(oil=line,**case.get('step_options',{}).get(str(index),{})) for index,line in enumerate(lines)],uploads=[],fault=None)])
        evidence=execute_native(executor,worker,plan,directory,threading.Event(),worker_sha256=worker_sha256,timeout_seconds=60)
        sessions=evidence.get('sessions',[])
        if len(sessions)!=1 or sessions[0].get('id')!='research':
            raise ValueError('standalone research session identity mismatch')
        session=sessions[0]
        record['errors'].extend(session.get('errors',[]))
        frames=command_frames(session['steps'],lines)
        (directory/'stdout.jsonl').write_text(''.join(json.dumps(frame)+'\n' for frame in frames))
        cleanup=json.loads((directory/'native-process-cleanup.json').read_text())
        if session.get('cleanup_errors') or session.get('workers_retired') is not True:
            cleanup=dict(cleanup,group_settled=False,errors=[*cleanup.get('errors',[]),*session.get('cleanup_errors',[]),'session workers not retired'])
        record['resource_accounting']=evidence['resource_accounting']
        record['process_cleanup']=cleanup
        data = oracle.request("snapshot", case_id)
        (directory / "oracle.json").write_text(json.dumps(data, indent=2) + "\n")
        record["errors"] += validate(case, lines, base, frames, data)
        record.update(terminal_verdict(record["errors"],[record["resource_accounting"]],[record["process_cleanup"]]))
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
        RuntimeError,
    ) as error:
        record["errors"].append(str(error))
        from workflow_cleanup import failure_details
        detail=failure_details(error)
        record.update(terminal_verdict(record["errors"],[],[]))
        record.update(detail)
        record["cleanup_failure_details"]=detail["cleanup_errors"]
    record["elapsed_seconds"] = round(time.monotonic() - started, 3)
    (directory / "preliminary-result.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )
    print(json.dumps(dict(event="provisional_result", **record)), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/oryn")
    parser.add_argument(
        "--worker",
        type=Path,
        default=ROOT
        / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker",
    )
    parser.add_argument("--executor", type=Path, help="oryn-workflow-qualification executable; defaults beside --binary")
    parser.add_argument("--output", type=Path, required=True)
    frontend=parser.add_mutually_exclusive_group()
    frontend.add_argument("--frontend-v2", action="store_true")
    frontend.add_argument("--frontend-v3", action="store_true")
    parser.add_argument("--samples", type=int, choices=range(1, 9), default=3)
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=3)
    args = parser.parse_args()
    from generation_gate import inherited_generation_lease
    admission = inherited_generation_lease()
    args.output, args.binary, args.worker = (
        args.output.resolve(),
        args.binary.resolve(),
        args.worker.resolve(),
    )
    args.executor=(args.executor or args.binary.with_name("oryn-workflow-qualification")).resolve()
    if not args.executor.is_file() or not os.access(args.executor,os.X_OK):
        raise ValueError("missing executable native workflow executor: "+str(args.executor))
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    database = args.output / "research.sqlite3"
    suffix = "-v3" if args.frontend_v3 else "-v2" if args.frontend_v2 else ""
    fixture_path = FIXTURES / f"research_fixture{suffix.replace('-', '_')}.py"
    contract_path = FIXTURES / f"research-cases{suffix}.json"
    contract = json.loads(contract_path.read_text())
    report = dict(
        schema_version=1,
        goal_complete=False,
        family_qualified=False,
        qualification="controlled SQLite research diagnostics; not full research/framework/release qualification",
        planned=len(contract["cases"]) * args.samples,
        jobs=args.jobs,
        records=[],
    )
    oracle = None
    try:
        report["executor_sha256"]=support.digest(args.executor)
        report["executor_path"]=str(args.executor)
        report["parent_sha256"], report["worker_sha256"] = (
            support.digest(args.binary),
            support.digest(args.worker),
        )
        report["source_tree_sha256"] = subprocess.check_output(
            [str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True
        ).strip()
        manifest = args.worker.parent.parent / "Resources/worker-manifest.json"
        report["worker_manifest"] = json.loads(manifest.read_text())
        report["source_matches_signed_build"] = (
            report["source_tree_sha256"]
            == report["worker_manifest"]["source_tree_sha256"]
        )
        report["input_hashes"] = {
            str(path.relative_to(ROOT)): support.digest(path)
            for path in [
                Path(__file__),
                ROOT / "scripts/workflow_native_adapter.py",
                ROOT / "scripts/workflow_outcome_evidence.py",
                ROOT / "scripts/run-oil-forms-outcomes.py",
                FIXTURES / "research_fixture.py",
                contract_path,
                fixture_path,
            ]
        }
        if args.frontend_v2 or args.frontend_v3:
            report["input_hashes"].update({str(path.relative_to(ROOT)): support.digest(path)
                for path in sorted((FIXTURES / "v2").glob("research-*.mjs"))})
        if args.frontend_v3:
            report["input_hashes"].update({str(path.relative_to(ROOT)):support.digest(path) for path in [FIXTURES/"research_fixture_v2.py",FIXTURES/"workflow_faults.py"]})
        runtime = subprocess.run(
            [str(args.binary), "native", "--runtime-info"],
            env=dict(os.environ, ORYN_PAGE_WORKER=str(args.worker)),
            capture_output=True,
            timeout=30,
            check=False,
        )
        (args.output / "runtime-info.stdout").write_bytes(runtime.stdout)
        (args.output / "runtime-info.stderr").write_bytes(runtime.stderr)
        runtime.check_returncode()
        report["runtime"] = json.loads(runtime.stdout)
        validate_protocol_pair(report["runtime"], report["worker_manifest"], ROOT)
        if (
            not report["runtime"].get("sandboxed")
            or report["runtime"].get("worker_binary_sha256") != report["worker_sha256"]
        ):
            raise ValueError("signed runtime identity mismatch")
        oracle = support.Oracle(
            args.output, fixture_path, ["--database", database]
        )
        report["origin"] = oracle.origin
        with ThreadPoolExecutor(max_workers=args.jobs) as executor:
            futures = [
                executor.submit(
                    run_case,
                    oracle,
                    contract,
                    case,
                    sample,
                    args.binary,
                    args.worker,
                    args.output,
                    args.executor,
                    report["worker_sha256"],
                )
                for sample in range(1, args.samples + 1)
                for case in contract["cases"]
            ]
            report["records"] = [future.result() for future in futures]
        report["oracle_seal"] = oracle.request("seal", "0" * 32)
        report["fixture_cleanup"] = oracle.close()
        oracle = None
        # Fresh read-only connections after server shutdown prove durable state,
        # independent of HTTP responses and the fixture's control snapshots.
        cases = {case["id"]: case for case in contract["cases"]}
        for record in report["records"]:
            directory = args.output / f"{record['case']}-{record['sample']:02d}"
            data = receiver.snapshot(database, record["case_id"])
            (directory / "final-database.json").write_text(
                json.dumps(data, indent=2) + "\n"
            )
            lines=task_lines(contract,cases[record['case']],record['base'])
            frames=[]
            try:
                native=json.loads((directory/'native-evidence.json').read_text())
                plan=json.loads((directory/'native-plan.json').read_text())
                if (native.get('attempt_id')!=plan.get('attempt_id') or len(native.get('sessions',[]))!=1
                        or [row.get('oil') for row in plan['sessions'][0]['steps']]!=lines):
                    raise ValueError('standalone preserved command provenance mismatch')
                frames=command_frames(native['sessions'][0]['steps'],lines)
            except (OSError,ValueError,KeyError,TypeError) as error:
                record['operation_errors']=record.get('operation_errors',[])+[str(error)]
            errors = validate(
                cases[record["case"]],
                task_lines(contract, cases[record["case"]], record["base"]),
                record["base"],
                frames,
                data,
            )
            record["errors"] = list(dict.fromkeys(record["errors"] + errors))
            operation_errors=list(dict.fromkeys(record.get("operation_errors",record["errors"])+errors))
            fixture_settled=report.get("fixture_cleanup")==dict(returncode=0,forced=False)
            record.update(terminal_verdict(operation_errors,[record.get("resource_accounting")],[record.get("process_cleanup"),dict(group_settled=fixture_settled,errors=[] if fixture_settled else ["fixture cleanup failed"])]))
            record["final_database_verified"] = True
            (directory / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        report["database_sha256"] = support.digest(database)
        report["artifact_identities_unchanged"] = (
            report["parent_sha256"] == support.digest(args.binary)
            and report["executor_sha256"] == support.digest(args.executor)
            and report["worker_sha256"] == support.digest(args.worker)
            and all(
                support.digest(ROOT / path) == value
                for path, value in report["input_hashes"].items()
            )
        )
        if not report["artifact_identities_unchanged"]:
            raise ValueError("candidate or fixture artifacts changed during run")
    except Exception as error:
        report["infrastructure_error"] = str(error)
    finally:
        if oracle is not None:
            report["fixture_cleanup"] = oracle.close()
    report["passed"] = sum(
        r["passed"] and r.get("final_database_verified", False)
        for r in report["records"]
    )
    report["status"] = (
        "passed"
        if report["passed"] == report["planned"]
        and not report.get("infrastructure_error")
        and report.get("fixture_cleanup") == {"returncode": 0, "forced": False}
        else "failed"
    )
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps({k: report[k] for k in ("status", "passed", "planned")}), flush=True
    )
    return int(report["status"] != "passed")


if __name__ == "__main__":
    sys.exit(main())

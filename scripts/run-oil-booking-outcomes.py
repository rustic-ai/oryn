#!/usr/bin/env python3
"""Versioned OIL booking outcomes with independent persisted reservation checks."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "test-harness/workflows"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


support = load_module(
    "booking_outcome_support", ROOT / "scripts/run-oil-forms-outcomes.py"
)
receiver = load_module("booking_receiver", FIXTURES / "booking_fixture.py")


def task_lines(contract, case, base):
    def review(slot):
        return [
            f'select "Time zone" "{case["zone"]}"',
            f'select "Appointment" "{slot}"',
            'click "Review appointment"',
            "wait visible css(#appointment-time)",
            'extract text --selector "#appointment-time"',
        ]

    details = ['type "Attendee" "Zoë Test"', 'type "Email" "zoe@example.test"']
    lines = [f'goto "{base}/start"'] + review(case["slot"])
    if case["fill"]:
        lines += details
    lines += ['click "Reserve test appointment"']
    if case["scenario"] == "stale":
        lines += ["wait visible css(#conflict)", "observe"]
    if case["recovery"]:
        lines += ['click "Search availability"'] + review(case["final_slot"])
        lines += details + ['click "Reserve test appointment"']
    if case["reservations"]:
        lines += ["wait visible css(#reservation)"]
        if case["duplicate"]:
            lines += [
                'click "Repeat reservation request"',
                "wait visible css(#reservation)",
            ]
        lines += [
            'click "Reload reservation"',
            "wait visible css(#reservation)",
            'extract text --selector "#appointment-time"',
        ]
    return lines + ["observe"]


def validate(case, lines, base, frames, data):
    errors = []
    if not frames or any(
        not isinstance(f, dict) or f.get("kind") in ("error", "unsupported")
        for f in frames
    ):
        return ["missing or failed OIL frames"]
    actions = [f.get("result", {}) for f in frames if f.get("kind") == "action"]
    ids = [a.get("action_id") for a in actions]
    expected = sum(line.split()[0] in ("click", "type", "select") for line in lines)
    if (
        len(actions) != expected
        or len(set(ids)) != len(ids)
        or any(type(i) is not int or i < 1 for i in ids)
        or any(a.get("execution_domain") != "native" for a in actions)
    ):
        errors.append("native action identity/count mismatch")
    observations = [
        f.get("observation", {}) for f in frames if f.get("kind") == "observation"
    ]
    final = (
        "confirmation"
        if case["reservations"]
        else "review"
        if not case["fill"]
        else "reserve"
    )
    final_url = observations[-1].get("page", {}).get("url", "") if observations else ""
    if final_url.split("?")[0] != base + "/" + final:
        errors.append("final location mismatch")
    times = [receiver.local_time(case["slot"], case["zone"])]
    if case["recovery"]:
        times.append(receiver.local_time(case["final_slot"], case["zone"]))
    if case["reservations"]:
        times.append(receiver.local_time(case["final_slot"], case["zone"]))
    if [f.get("value") for f in frames if f.get("kind") == "structured"] != times:
        errors.append("extracted appointment times mismatch")
    if [r["status"] for r in data["requests"]] != case["statuses"] or any(
        not r["cookie_valid"] for r in data["requests"]
    ):
        errors.append("persisted acceptance/rejection sequence mismatch")
    if len(data["reservations"]) != case["reservations"]:
        errors.append("reservation count mismatch")
    offers = {r["offer"]: r for r in data["offers"]}
    if len(offers) != (2 if case["recovery"] else 1):
        errors.append("review count mismatch")
    for row in data["reservations"]:
        fields = json.loads(row["fields"])
        if (
            (row["slot"], row["utc"], row["local"], row["zone"])
            != (
                case["final_slot"],
                receiver.SLOTS[case["final_slot"]],
                times[-1],
                case["zone"],
            )
            or fields.get("name") != ["Zoë Test"]
            or fields.get("email") != ["zoe@example.test"]
            or fields.get("offer") != [row["offer"]]
            or row["offer"] not in offers
        ):
            errors.append("persisted reservation identity/time/attendee mismatch")
    expected_unavailable = set()
    if case["scenario"] == "stale":
        expected_unavailable.add(case["slot"])
    if case["reservations"]:
        expected_unavailable.add(case["final_slot"])
    if {r["slot"] for r in data["slots"] if not r["available"]} != expected_unavailable:
        errors.append("slot availability mismatch")
    confirmations = sum(row["path"] == "confirmation" for row in data["views"])
    if confirmations != (3 if case["duplicate"] else 2) * case["reservations"] or any(
        not row["cookie_valid"] for row in data["views"]
    ):
        errors.append("persisted confirmation/reload/session mismatch")
    return errors


def run_case(oracle, contract, case, sample, binary, worker, output):
    case_id = secrets.token_hex(16)
    directory = output / f"{case['id']}-{sample:02d}"
    directory.mkdir()
    base = f"{oracle.origin}/booking/{case_id}"
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
        process = subprocess.Popen(
            [str(binary), "--file", str(task), "native", "--allow-loopback"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=dict(os.environ, ORYN_PAGE_WORKER=str(worker)),
            start_new_session=True,
        )
        try:
            stdout, stderr = process.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            record["timed_out"] = True
            os.killpg(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate(timeout=10)
        (directory / "stdout.jsonl").write_bytes(stdout)
        (directory / "stderr.txt").write_bytes(stderr)
        if stderr.strip():
            record["errors"].append("nonempty OIL stderr")
        record["returncode"] = process.returncode
        if process.returncode != 0 or record["timed_out"]:
            record["errors"].append("OIL process failed or timed out")
        data = oracle.request("snapshot", case_id)
        (directory / "oracle.json").write_text(json.dumps(data, indent=2) + "\n")
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        record["errors"] += validate(case, lines, base, frames, data)
        record["passed"] = not record["errors"]
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        subprocess.SubprocessError,
    ) as error:
        record["errors"].append(str(error))
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
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--samples", type=int, choices=range(1, 9), default=3)
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=3)
    args = parser.parse_args()
    args.output, args.binary, args.worker = (
        args.output.resolve(),
        args.binary.resolve(),
        args.worker.resolve(),
    )
    args.output.mkdir(parents=True, exist_ok=False)
    database = args.output / "booking.sqlite3"
    contract = json.loads((FIXTURES / "booking-cases.json").read_text())
    report = dict(
        schema_version=1,
        goal_complete=False,
        family_qualified=False,
        qualification="controlled SQLite booking diagnostics; not general booking/framework/release qualification",
        planned=len(contract["cases"]) * args.samples,
        jobs=args.jobs,
        records=[],
    )
    oracle = None
    try:
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
                ROOT / "scripts/run-oil-forms-outcomes.py",
                FIXTURES / "booking_fixture.py",
                FIXTURES / "booking-cases.json",
            ]
        }
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
        if (
            not report["runtime"].get("sandboxed")
            or report["runtime"].get("worker_binary_sha256") != report["worker_sha256"]
        ):
            raise ValueError("signed runtime identity mismatch")
        oracle = support.Oracle(
            args.output, FIXTURES / "booking_fixture.py", ["--database", database]
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
            frames = [
                json.loads(line)
                for line in (directory / "stdout.jsonl").read_text().splitlines()
                if line.strip()
            ]
            errors = validate(
                cases[record["case"]],
                task_lines(contract, cases[record["case"]], record["base"]),
                record["base"],
                frames,
                data,
            )
            record["errors"] = list(dict.fromkeys(record["errors"] + errors))
            record["passed"] = not record["errors"]
            record["final_database_verified"] = True
            (directory / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        report["database_sha256"] = support.digest(database)
        report["artifact_identities_unchanged"] = (
            report["parent_sha256"] == support.digest(args.binary)
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

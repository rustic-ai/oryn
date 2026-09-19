#!/usr/bin/env python3
"""Versioned OIL booking outcomes with independent persisted reservation checks."""

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
    if "steps" in case:
        return [line.replace("{base}", base) for line in case["steps"]]
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



def booking_snapshot_shape_errors(data):
    """Reject malformed evidence before any join or outcome comparisons."""
    if not isinstance(data, dict):
        return ["persisted booking snapshot is not an object"]
    schemas = {
        "cases": ("case_id scenario", "reviews"),
        "slots": ("case_id slot", "available version"),
        "offers": ("case_id offer slot zone utc local", "version"),
        "reservations": ("case_id reservation slot offer request_key fields utc local zone", ""),
        "requests": ("case_id fields", "status cookie_valid"),
        "views": ("case_id path", "cookie_valid"),
        "client_selections": ("case_id local zone utc", "valid"),
        "offer_options": ("case_id offer appointment_option", ""),
        "cancellations": ("case_id offer", ""),
    }
    optional = {"client_selections", "offer_options", "cancellations"}
    errors = []
    for table, (strings, integers) in schemas.items():
        rows = data.get(table, [] if table in optional else None)
        if not isinstance(rows, list):
            errors.append(f"persisted {table} table is missing or malformed")
            continue
        for row in rows:
            if (not isinstance(row, dict)
                    or any(not isinstance(row.get(key), str) for key in strings.split())
                    or any(type(row.get(key)) is not int for key in integers.split())):
                errors.append(f"persisted {table} row has missing or malformed fields")
                continue
            if any(row[key] not in (0, 1) for key in integers.split()
                   if key in ("available", "cookie_valid", "valid")):
                errors.append(f"persisted {table} row has invalid boolean evidence")
            if "fields" in row:
                try:
                    fields = json.loads(row["fields"])
                except (TypeError, ValueError):
                    fields = None
                if (not isinstance(fields, dict)
                        or any(not isinstance(values, list)
                               or any(not isinstance(value, str) for value in values)
                               for values in fields.values())):
                    errors.append(f"persisted {table} request body is malformed")
    return errors


def durable_booking_ownership_errors(data, case_id):
    """Join independently persisted offers, requests, reservations and slot state."""
    errors = booking_snapshot_shape_errors(data)
    if errors:
        return errors
    if len(data["cases"]) != 1:
        errors.append("persisted booking snapshot must identify exactly one case")
    tables = ("cases", "slots", "offers", "reservations", "requests", "views",
              "client_selections", "offer_options", "cancellations")
    if any(row.get("case_id") != case_id for table in tables for row in data.get(table, [])):
        errors.append("persisted booking record belongs to another case")
    offers = {row.get("offer"): row for row in data["offers"]}
    slots = {row.get("slot"): row for row in data["slots"]}
    if any(not key for key in offers) or len(offers) != len(data["offers"]):
        errors.append("issued offer identities are missing or duplicated")
    if any(not key for key in slots) or len(slots) != len(data["slots"]):
        errors.append("slot identities are missing or duplicated")

    def fields(row):
        try:
            value = json.loads(row["fields"])
        except (KeyError, TypeError, ValueError):
            return None
        return value if isinstance(value, dict) else None

    if set(slots) != set(receiver.SLOTS):
        errors.append("persisted booking slot inventory mismatch")

    accepted = [fields(row) for row in data["requests"] if row["status"] == 303]
    reservations = data["reservations"]
    for key in ("reservation", "request_key"):
        values = [row.get(key) for row in reservations]
        if any(not isinstance(value, str) or not value for value in values) or len(set(values)) != len(values):
            errors.append("reservation or idempotency identity is missing or duplicated")
    persisted = [fields(row) for row in reservations]
    if any(value is None or value not in persisted for value in accepted):
        errors.append("accepted booking request has no matching durable reservation")
    for row, value in zip(reservations, persisted):
        offer = offers.get(row.get("offer"))
        slot = slots.get(row.get("slot"))
        if (value is None or set(value) != {"offer", "request_key", "name", "email"}
                or value.get("offer") != [row.get("offer")]
                or value.get("request_key") != [row.get("request_key")]
                or value not in accepted):
            errors.append("reservation does not match its accepted request identity/body")
        if (offer is None or any(row.get(key) != offer.get(key) for key in ("slot", "utc", "local", "zone"))):
            errors.append("reservation does not match its issued offer")
        if (offer is None or slot is None or slot.get("available") != 0
                or type(offer.get("version")) is not int
                or slot.get("version") != offer["version"] + 1):
            errors.append("reservation does not own exactly one slot version change")
    return errors


def validate(case, lines, base, frames, data):
    errors = []
    if not frames or any(
        not isinstance(f, dict) or f.get("kind") in ("error", "unsupported")
        for f in frames
    ):
        return ["missing or failed OIL frames"]
    errors.extend(durable_booking_ownership_errors(data, base.rstrip("/").rsplit("/", 1)[-1]))
    if errors:
        return errors
    if data["cases"][0]["scenario"] != case["scenario"]:
        errors.append("persisted booking scenario mismatch")
    actions = [f.get("result") for f in frames if f.get("kind") == "action"]
    if any(not isinstance(action, dict) for action in actions):
        return errors + ["malformed native action evidence"]
    for frame in frames:
        if frame.get("kind") == "observation":
            observation = frame.get("observation")
            if (not isinstance(observation, dict)
                    or not isinstance(observation.get("page"), dict)
                    or not isinstance(observation["page"].get("url"), str)):
                return errors + ["malformed booking observation evidence"]
    ids = [a.get("action_id") for a in actions]
    expected = sum(line.split()[0] in ("click", "type", "select") for line in lines)
    if (
        len(actions) != expected
        or any(type(i) is not int or i < 1 for i in ids)
        or len(set(ids)) != len(ids)
        or any(a.get("execution_domain") != "native" for a in actions)
    ):
        errors.append("native action identity/count mismatch")
    observations = [
        f.get("observation", {}) for f in frames if f.get("kind") == "observation"
    ]
    if "client_selections" in case:
        actual = [{key: row[key] for key in ("local", "zone", "utc", "valid")}
                  for row in data.get("client_selections", [])]
        if actual != case["client_selections"]:
            errors.append("client timezone selection audit mismatch")
    if "appointment_options" in case:
        if [row.get("appointment_option") for row in data.get("offer_options", [])] != case["appointment_options"]:
            errors.append("persisted appointment options mismatch")
        if len(data.get("cancellations", [])) != case["cancellations"]:
            errors.append("persisted review cancellation mismatch")
        cancelled = {row["offer"] for row in data.get("cancellations", [])}
        if not cancelled.issubset({row["offer"] for row in data["offers"]}) or len(cancelled) != len(data.get("cancellations", [])):
            errors.append("cancellation does not identify a unique issued offer")
        if any(row["offer"] in cancelled for row in data["reservations"]):
            errors.append("cancelled offer created a reservation")
        option_offers = {row["offer"] for row in data.get("offer_options", [])}
        if option_offers != {row["offer"] for row in data["offers"]}:
            errors.append("offer options do not cover exactly the issued offers")
    if case.get("gap"):
        if any(data.get(table) for table in ("offers", "reservations", "requests", "client_selections")):
            errors.append("nonexistent local time reached reservation backend")
        if [frame.get("value") for frame in frames if frame.get("kind") == "structured"] != [
            "This local time does not exist. Choose another time."
        ]:
            errors.append("client did not explicitly reject DST gap")
        final_url = observations[-1].get("page", {}).get("url", "") if observations else ""
        if final_url != base + "/availability":
            errors.append("gap navigated away from date selection")
        return errors
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
    if confirmations != (3 if case["duplicate"] else 2) * case["reservations"] + case.get("extra_confirmation_views",0) or any(
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
        check_execution_generation()
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
    global receiver
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
    parser.add_argument("--frontend-v2", action="store_true")
    args = parser.parse_args()
    from generation_gate import inherited_generation_lease
    admission = inherited_generation_lease()
    fixture_path = FIXTURES / ("booking_fixture_v2.py" if args.frontend_v2 else "booking_fixture.py")
    contract_path = FIXTURES / ("booking-cases-v2.json" if args.frontend_v2 else "booking-cases.json")
    if args.frontend_v2:
        receiver = load_module("booking_receiver_v2", fixture_path)
    args.output, args.binary, args.worker = (
        args.output.resolve(),
        args.binary.resolve(),
        args.worker.resolve(),
    )
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    database = args.output / "booking.sqlite3"
    contract = json.loads(contract_path.read_text())
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
                fixture_path, contract_path,
                *([FIXTURES / "v2/booking-client.mjs", FIXTURES / "v2/timezone-resolution.mjs"] if args.frontend_v2 else []),
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

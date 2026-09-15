#!/usr/bin/env python3
"""Versioned OIL shopping outcomes verified against persisted SQLite rows."""

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
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "test-harness/workflows"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


support = load_module("forms_outcome_support", ROOT / "scripts/run-oil-forms-outcomes.py")
receiver = load_module("shopping_receiver", FIXTURES / "shopping_fixture.py")


def task_lines(contract, case, base):
    lines = [line.replace("{base}", base) for line in contract["browse"]]
    if case["fill"]:
        lines += contract["details"]
    lines += ['click "Place test order"']
    if case["scenario"] != "normal":
        lines += ["wait visible css(#conflict)", "observe"]
    if case["recovery"] == "price":
        lines += ['click "Refresh review"']
    elif case["recovery"] == "stock":
        lines += [
            'click "Edit cart"',
            'select "Cart variant" "green"',
            'click "Update cart"',
            'click "Review checkout"',
        ]
    if case["recovery"]:
        lines += contract["details"] + ['click "Place test order"']
    if case["orders"]:
        lines += ["wait visible css(#order-total)"]
        if case["duplicate"]:
            lines += ['click "Repeat order request"', "wait visible css(#order-total)"]
        lines += ['click "Reload order"', "wait visible css(#order-total)"]
    return lines + ["observe"]


def validate(case, lines, base, frames, data):
    errors = []
    if not frames or any(
        not isinstance(f, dict) or f.get("kind") in ("error", "unsupported") for f in frames
    ):
        errors.append("missing or failed OIL frames")
    actions = [
        f.get("result", {}) for f in frames if isinstance(f, dict) and f.get("kind") == "action"
    ]
    expected = sum(line.split()[0] in ("click", "type", "select") for line in lines)
    ids = [action.get("action_id") for action in actions]
    if (
        len(actions) != expected
        or len(set(ids)) != len(ids)
        or any(type(i) is not int or i < 1 for i in ids)
        or any(a.get("execution_domain") != "native" for a in actions)
    ):
        errors.append("native OIL action identity/count mismatch")
    observations = [
        f["observation"]
        for f in frames
        if isinstance(f, dict)
        and f.get("kind") == "observation"
        and isinstance(f.get("observation"), dict)
    ]
    final = "order" if case["orders"] else "review" if not case["fill"] else "checkout"
    if not observations or observations[-1].get("page", {}).get("url") != base + "/" + final:
        errors.append("final location mismatch")
    urls = [o.get("page", {}).get("url", "") for o in observations]
    if base + "/product/desk" not in urls or base + "/product/field" not in urls:
        errors.append("product comparison pages not observed")
    filtered = [
        o
        for o in observations
        if parse_qs(urlsplit(o.get("page", {}).get("url", "")).query)
        == {"search": ["field"], "category": ["outdoor"]}
    ]
    if (
        not filtered
        or not any(n.get("name") == "Field kit" for n in filtered[-1].get("nodes", []))
        or any(n.get("name") == "Desk set" for n in filtered[-1].get("nodes", []))
    ):
        errors.append("filtered catalog outcome mismatch")
    carts = [o for o in observations if o.get("page", {}).get("url") == base + "/cart"]
    if not carts or not any(
        n.get("name") == "Cart quantity" and str(n.get("value")) == "2"
        for n in carts[-1].get("nodes", [])
    ):
        errors.append("reloaded browser cart quantity mismatch")
    requests, orders = data["requests"], data["orders"]
    checkout = [r for r in requests if r["action"] == "checkout"]
    cart_requests = [r for r in requests if r["action"] == "cart"]
    expected_carts = [
        {"quantity": ["1"], "variant": ["blue"]},
        {"quantity": ["2"], "variant": ["blue"]},
    ]
    if case["recovery"] == "stock":
        expected_carts += [{"quantity": ["2"], "variant": ["green"]}]
    if [json.loads(r["fields"]) for r in cart_requests] != expected_carts or any(
        r["outcome"] != 303 for r in cart_requests
    ):
        errors.append("persisted cart update sequence mismatch")
    if [r["outcome"] for r in checkout] != case["checkout_outcomes"] or len(requests) != len(
        expected_carts
    ) + len(checkout):
        errors.append("checkout rejection/acceptance sequence mismatch")
    if any(not r["cookie_valid"] for r in requests + data["views"]):
        errors.append("session cookie not preserved")
    if len(data["carts"]) != 1 or (data["carts"][0]["variant"], data["carts"][0]["quantity"]) != (
        case["variant"],
        2,
    ):
        errors.append("persisted final cart mismatch")
    if len(orders) != case["orders"]:
        errors.append("wrong order count, including duplicate prevention")
    if len(data["quotes"]) != (2 if case["recovery"] else 1):
        errors.append("quote/review count mismatch")
    for order in orders:
        if (
            order["variant"],
            order["quantity"],
            order["unit_price"],
            order["total"],
            order["name"],
            order["email"],
        ) != (
            case["variant"],
            2,
            case["unit_price"],
            case["unit_price"] * 2,
            "Zoë Test",
            "zoe@example.test",
        ):
            errors.append("persisted order fields/price mismatch")
        if order["quote"] not in [q["quote"] for q in data["quotes"]] or not any(
            json.loads(r["fields"]).get("quote") == [order["quote"]] and r["outcome"] == 303
            for r in checkout
        ):
            errors.append("order not tied to submitted quote")
    case_row = data["cases"][0]
    expected_stock = {"blue_stock": 0 if case["scenario"] == "stock" else 10, "green_stock": 10}
    if case["orders"]:
        expected_stock[case["variant"] + "_stock"] -= 2
    if (
        any(case_row[k] != v for k, v in expected_stock.items())
        or case_row["unit_price"] != case["unit_price"]
    ):
        errors.append("inventory depletion or price mismatch")
    order_views = sum(v["page"] == "order" for v in data["views"])
    if order_views != (3 if case["duplicate"] else 2) * case["orders"]:
        errors.append("order redirect/reload count mismatch")
    return errors


def run_case(oracle, contract, case, sample, binary, worker, output):
    case_id = secrets.token_hex(16)
    directory = output / f"{case['id']}-{sample:02d}"
    directory.mkdir()
    base = f"{oracle.origin}/shop/{case_id}"
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
        record["returncode"] = process.returncode
        if process.returncode != 0 or record["timed_out"]:
            record["errors"].append("OIL process failed or timed out")
        data = oracle.request("snapshot", case_id)
        (directory / "oracle.json").write_text(json.dumps(data, indent=2) + "\n")
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        record["errors"] += validate(case, lines, base, frames, data)
        record["passed"] = not record["errors"]
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        record["errors"].append(str(error))
    record["elapsed_seconds"] = round(time.monotonic() - started, 3)
    (directory / "preliminary-result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(dict(event="provisional_result", **record)), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/oryn")
    parser.add_argument(
        "--worker",
        type=Path,
        default=ROOT / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker",
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
    database = args.output / "shopping.sqlite3"
    contract = json.loads((FIXTURES / "shopping-cases.json").read_text())
    report = dict(
        schema_version=1,
        goal_complete=False,
        family_qualified=False,
        qualification="controlled SQLite shopping diagnostics; not general shopping/framework/release qualification",
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
            report["source_tree_sha256"] == report["worker_manifest"]["source_tree_sha256"]
        )
        report["input_hashes"] = {
            str(path.relative_to(ROOT)): support.digest(path)
            for path in [
                Path(__file__),
                ROOT / "scripts/run-oil-forms-outcomes.py",
                FIXTURES / "shopping_fixture.py",
                FIXTURES / "shopping-cases.json",
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
            args.output, FIXTURES / "shopping_fixture.py", ["--database", database]
        )
        report["origin"] = oracle.origin
        with ThreadPoolExecutor(max_workers=args.jobs) as executor:
            futures = [
                executor.submit(
                    run_case, oracle, contract, case, sample, args.binary, args.worker, args.output
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
            (directory / "final-database.json").write_text(json.dumps(data, indent=2) + "\n")
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
    except Exception as error:
        report["infrastructure_error"] = str(error)
    finally:
        if oracle is not None:
            report["fixture_cleanup"] = oracle.close()
    report["passed"] = sum(
        r["passed"] and r.get("final_database_verified", False) for r in report["records"]
    )
    report["status"] = (
        "passed"
        if report["passed"] == report["planned"]
        and not report.get("infrastructure_error")
        and report.get("fixture_cleanup") == {"returncode": 0, "forced": False}
        else "failed"
    )
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ("status", "passed", "planned")}), flush=True)
    return int(report["status"] != "passed")


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Versioned OIL shopping outcomes verified against persisted SQLite rows."""

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
    lines += contract.get("frame_ready", [])
    if case["fill"]:
        lines += contract["details"] + contract.get("payment_steps", [])
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
        lines += contract["details"] + contract.get("payment_steps", []) + ['click "Place test order"']
    if case["orders"]:
        lines += ["wait visible css(#order-total)"]
        if case["duplicate"]:
            lines += ['click "Repeat order request"', "wait visible css(#order-total)"]
        lines += ['click "Reload order"', "wait visible css(#order-total)"]
    lines += ["observe"]
    if contract.get("frontend_v2"):
        synchronized, in_frame = [], False
        for line in lines:
            synchronized.append(line)
            if line.startswith("frame "):
                in_frame = line != "frame main"
            if not in_frame and line.split()[0] in ("goto", "click"):
                synchronized.append('wait visible css(#shop-app[data-state="ready"])')
        return synchronized
    return lines


def validate_persisted_ownership(case, base, data):
    """Reject malformed or unrelated durable evidence before projection/joins."""
    tables = {
        "cases": ("scenario", "unit_price", "blue_stock", "green_stock", "quote_count"),
        "carts": ("variant", "quantity"),
        "quotes": ("quote", "variant", "quantity", "unit_price", "total"),
        "orders": ("order_id", "quote", "variant", "quantity", "unit_price", "total", "name", "email"),
        "requests": ("action", "outcome", "body_sha256", "fields", "cookie_valid"),
        "views": ("page", "cookie_valid"),
    }
    if case.get("frontend_v2"):
        tables.update(payments=("quote", "receipt", "authorized"),
                      payment_events=("quote", "action", "outcome"))
    if not isinstance(data, dict):
        return ["malformed shopping database"]
    owner = urlsplit(base).path.rsplit("/", 1)[-1]
    integers = {"unit_price", "blue_stock", "green_stock", "quote_count", "quantity",
                "total", "outcome", "cookie_valid", "authorized"}
    for table, fields in tables.items():
        rows = data.get(table)
        if not isinstance(rows, list):
            return ["malformed shopping table: " + table]
        for row in rows:
            if not isinstance(row, dict) or row.get("case_id") != owner:
                return ["foreign or malformed shopping row: " + table]
            for field in fields:
                value = row.get(field)
                if field == "receipt" and value is None and row.get("authorized") == 0:
                    continue
                if field in integers:
                    if type(value) is not int or value < 0:
                        return ["malformed shopping integer: " + table + "." + field]
                    if field in ("cookie_valid", "authorized") and value not in (0, 1):
                        return ["malformed shopping boolean: " + table + "." + field]
                elif not isinstance(value, str) or not value:
                    return ["malformed shopping text: " + table + "." + field]
    if len(data["cases"]) != 1 or data["cases"][0]["scenario"] != case["scenario"]:
        return ["shopping case identity/scenario mismatch"]
    errors = []
    for table, field in (("quotes", "quote"), ("orders", "order_id"), ("orders", "quote")):
        identities = [row[field] for row in data[table]]
        if len(set(identities)) != len(identities):
            errors.append("duplicate shopping identity: " + table + "." + field)
    quotes = {row["quote"]: row for row in data["quotes"]}
    orders = {row["quote"]: row for row in data["orders"]}
    if data["cases"][0]["quote_count"] != len(quotes):
        errors.append("shopping quote counter mismatch")
    for row in data["quotes"]:
        if row["variant"] not in ("blue", "green") or row["quantity"] < 1 or row["total"] != row["quantity"] * row["unit_price"]:
            errors.append("shopping quote arithmetic mismatch")
    accepted = set()
    for request in data["requests"]:
        try:
            fields = json.loads(request["fields"])
        except ValueError:
            return errors + ["malformed shopping request JSON"]
        if not isinstance(fields, dict) or any(not isinstance(values, list) or not values
                or any(not isinstance(value, str) for value in values) for values in fields.values()):
            return errors + ["malformed shopping request fields"]
        if request["action"] == "checkout" and request["outcome"] == 303:
            quote = fields.get("quote", [])
            order = orders.get(quote[0]) if len(quote) == 1 else None
            if (order is None or request["cookie_valid"] != 1
                    or fields != {"quote": [order["quote"]], "name": [order["name"]], "email": [order["email"]]}):
                errors.append("accepted checkout has no matching owned order")
            else:
                accepted.add(order["quote"])
    for order in data["orders"]:
        quote = quotes.get(order["quote"])
        if (quote is None or order["quote"] not in accepted
                or any(order[field] != quote[field] for field in ("variant", "quantity", "unit_price", "total"))):
            errors.append("shopping order/quote ownership mismatch")
    if case.get("frontend_v2"):
        payments = data["payments"]
        if len(payments) != len(quotes) or {row["quote"] for row in payments} != set(quotes):
            errors.append("payment/quote inventory mismatch")
        receipts = [row["receipt"] for row in payments if row["authorized"] == 1]
        if len(set(receipts)) != len(receipts):
            errors.append("duplicate payment receipt")
        for payment in payments:
            events = [row for row in data["payment_events"] if row["quote"] == payment["quote"]]
            expected = ["frame_view", "authorized"] if payment["authorized"] else ["frame_view"]
            if [row["action"] for row in events] != expected:
                errors.append("payment authority history belongs to another quote")
        if any(row["quote"] not in quotes for row in data["payment_events"]):
            errors.append("payment event has no owned quote")
    return errors


def validate(case, lines, base, frames, data):
    errors = validate_persisted_ownership(case, base, data)
    if errors:
        return errors
    if not isinstance(frames, list):
        return ["malformed OIL frames"]
    for frame in frames:
        if not isinstance(frame, dict):
            return ["malformed OIL frame"]
        if frame.get("kind") == "action" and not isinstance(frame.get("result"), dict):
            return ["malformed OIL action result"]
        if frame.get("kind") == "observation":
            observation = frame.get("observation")
            if (not isinstance(observation, dict) or not isinstance(observation.get("page"), dict)
                    or not isinstance(observation["page"].get("url"), str)
                    or not isinstance(observation.get("nodes"), list)
                    or any(not isinstance(node, dict) for node in observation["nodes"])):
                return ["malformed OIL observation"]
    if not frames or any(
        not isinstance(f, dict) or f.get("kind") in ("error", "unsupported") for f in frames
    ):
        errors.append("missing or failed OIL frames")
    actions = [
        f.get("result", {}) for f in frames if isinstance(f, dict) and f.get("kind") == "action"
    ]
    expected = sum(line.split()[0] in ("click", "type", "select") for line in lines)
    ids = [(action.get("page_id"), action.get("action_id")) for action in actions]
    if (
        len(actions) != expected
        or any(type(identity) is not int or identity < 1 or (page is not None and type(page) is not int) for page, identity in ids)
        or len({(page, identity) for page, identity in ids if type(identity) is int and (page is None or type(page) is int)}) != len(ids)
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
    if order_views != (3 if case["duplicate"] else 2) * case["orders"] + case.get("extra_order_views",0):
        errors.append("order redirect/reload count mismatch")
    if case.get("frontend_v2"):
        payments = data.get("payments", [])
        events = data.get("payment_events", [])
        expected_quotes = len(data.get("quotes", []))
        authorized = expected_quotes if case["fill"] else 0
        if len(payments) != expected_quotes or sum(row.get("authorized") == 1 for row in payments) != authorized:
            errors.append("controlled payment authorization count mismatch")
        if [event.get("action") for event in events] != [action for _ in range(expected_quotes) for action in
                (("frame_view", "authorized") if case["fill"] else ("frame_view",))]:
            errors.append("controlled payment authority history mismatch")
        if any(event.get("outcome") != 200 for event in events):
            errors.append("controlled payment authority failure")
        for order in orders:
            if sum(payment.get("quote") == order["quote"] and payment.get("authorized") == 1
                   and isinstance(payment.get("receipt"), str) and len(payment["receipt"]) == 32
                   for payment in payments) != 1:
                errors.append("order lacks its unique controlled payment receipt")
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
    parser.add_argument("--frontend-v2", action="store_true")
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
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    database = args.output / "shopping.sqlite3"
    contract_path = FIXTURES / ("shopping-cases-v2.json" if args.frontend_v2 else "shopping-cases.json")
    fixture_path = FIXTURES / ("shopping_fixture_v2.py" if args.frontend_v2 else "shopping_fixture.py")
    selected_receiver = load_module("shopping_selected_receiver", fixture_path) if args.frontend_v2 else receiver
    contract = json.loads(contract_path.read_text())
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
                contract_path, fixture_path,
            ]
        }
        if args.frontend_v2:
            report["input_hashes"]["test-harness/workflows/v2/shopping-client.mjs"] = support.digest(FIXTURES / "v2/shopping-client.mjs")
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
            data = selected_receiver.snapshot(database, record["case_id"])
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
        report["artifact_identities_unchanged"] = (
            report["parent_sha256"] == support.digest(args.binary)
            and report["worker_sha256"] == support.digest(args.worker)
            and all(support.digest(ROOT / path) == value for path, value in report["input_hashes"].items()))
        if not report["artifact_identities_unchanged"]:
            raise ValueError("candidate or fixture artifacts changed during run")
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

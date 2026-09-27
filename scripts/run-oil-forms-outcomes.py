#!/usr/bin/env python3
"""Controlled OIL outcomes with an independent, non-HTTP server oracle."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
from evidence_protocols import validate_protocol_pair
import secrets
import select
import signal
from generation_gate import check_execution_generation
import subprocess
import sys
import threading
import time
from workflow_cleanup import CleanupFailure, settle_group

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "test-harness/workflows"


def digest(path):
    with open(path, "rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def expected_action_count(case, contract):
    commands = case["before_fill"] + (contract["fill"] if case["fill"] else []) + case["after_fill"]
    return sum(line.split()[0] in ("click", "type", "select", "check", "uncheck", "upload") for line in commands) + int(case["submit"]) + int(case.get("expected_receipts", 1) > 0)


def validate_outcome(case, contract, case_id, origin, frames, snapshot, upload):
    errors = []
    if not frames or any(
        not isinstance(frame, dict) or frame.get("kind") in ("unsupported", "error")
        for frame in frames
    ):
        errors.append("missing or failed OIL outputs")
    observations = [
        frame["observation"]
        for frame in frames
        if isinstance(frame, dict)
        and frame.get("kind") == "observation"
        and isinstance(frame.get("observation"), dict)
    ]
    expected_receipts = case.get("expected_receipts", 1)
    actions = [
        frame.get("result", {})
        for frame in frames
        if isinstance(frame, dict) and frame.get("kind") == "action"
    ]
    if len(actions) != expected_action_count(case, contract):
        errors.append("OIL action count does not match the versioned task")
    identities = [action.get("action_id") for action in actions]
    if (
        any(type(value) is not int or value < 1 for value in identities)
        or len(set(identities)) != len(identities)
        or any(action.get("execution_domain") != "native" for action in actions)
    ):
        errors.append("invalid native action identities")
    requests = snapshot.get("requests", [])
    receipts = snapshot.get("receipts", [])
    if len(requests) != expected_receipts or len(receipts) != expected_receipts:
        errors.append(
            f"expected {expected_receipts} POST/receipt, got {len(requests)}/{len(receipts)}"
        )
    expected_path = "/receipt/" if expected_receipts else "/form/"
    expected_url = origin + expected_path + case_id
    if not observations or observations[-1].get("page", {}).get("url") != expected_url:
        errors.append("final browser location does not match expected outcome")
    if expected_receipts and receipts:
        values = {**contract["fields"], **case.get("expected_fields", {})}
        expected_fields = [
            [key, values[key]]
            for key in ("name", "email", "role", "message", "quantity", "date", "time", "reference")
        ]
        expected_fields += [["skill", value] for value in values["skill"]]
        expected_fields += [["case_id", case_id], ["intent", "apply"]]
        receipt = receipts[0]
        if receipt.get("fields") != expected_fields or receipt.get("case_id") != case_id:
            errors.append("server receipt field values/order differ from contract")
        expected_files = [upload] if case["upload"] else []
        if receipt.get("files") != expected_files:
            errors.append("server upload bytes/name/identity differ from authorized file")
        if (
            not requests
            or requests[0].get("status") != 303
            or receipt.get("request_sha256") != requests[0].get("sha256")
        ):
            errors.append("receipt does not correspond to the accepted POST")
        if snapshot.get("receipt_views", 0) < 2:
            errors.append("receipt did not survive independent GET reload")
        nodes = observations[-1].get("nodes", []) if observations else []
        if not any(node.get("name") == "Application received" for node in nodes):
            errors.append("browser did not observe receipt confirmation")
    if "expected_validations" in case and snapshot.get("validations") != case["expected_validations"]:
        errors.append("async validation request history mismatch")
    return errors


class Oracle:
    def __init__(self, output, fixture=None, arguments=()):
        check_execution_generation()
        self.lock = threading.Lock()
        self.stderr = open(output / "fixture.stderr", "wb")
        self.process = None
        try:
            check_execution_generation()
            self.process = subprocess.Popen(
                [sys.executable, str(fixture or FIXTURES / "forms_fixture.py"), *map(str, arguments)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
                text=True, bufsize=1, start_new_session=True,
            )
            ready = self.read()
            if ready.get("event") != "ready" or type(ready.get("port")) is not int:
                raise ValueError("invalid fixture startup")
            self.origin = f"http://127.0.0.1:{ready['port']}"
        except BaseException as error:
            cleanup = self.close()
            try:
                diagnostic = (output / "fixture.stderr").read_text(errors="replace")[-8192:]
            except Exception as read_error:
                diagnostic = 'stderr unavailable: '+str(read_error)
            settled = cleanup == {"returncode": 0, "forced": False} or (cleanup.get('group_settled') is True and cleanup.get('resources_settled') is True)
            raise CleanupFailure(error, [f"fixture startup cleanup={cleanup}; stderr={diagnostic}"],
                                 not settled, (self,) if not settled else ()) from error

    def read(self):
        if not select.select([self.process.stdout], [], [], 15)[0]:
            raise TimeoutError("fixture control deadline exceeded")
        line = self.process.stdout.readline(1024 * 1024 + 1)
        if not line or len(line) > 1024 * 1024:
            raise ValueError("invalid fixture control frame")
        return json.loads(line)

    def request(self, command, case_id, **kwargs):
        with self.lock:
            self.process.stdin.write(
                json.dumps(dict(command=command, case_id=case_id, **kwargs)) + "\n"
            )
            self.process.stdin.flush()
            response = self.read()
            if response.get("ok") is not True:
                raise ValueError("fixture control failed: " + str(response.get("error")))
            return response["result"]

    def close(self):
        forced, errors = False, []
        pipes_settled = True
        process = self.process
        settled = process is None
        if process is not None:
            try:
                if process.poll() is None:
                    process.stdin.write('{"command":"shutdown"}\n')
                    process.stdin.flush()
                    process.wait(timeout=10)
            except BaseException as error:
                forced = True
                errors.append('fixture shutdown: '+repr(error))
            try:
                receipt = settle_group(process, timeout=10)
                settled = receipt['group_settled'] and type(receipt['returncode']) is int
                errors.extend(receipt['errors'])
            except BaseException as error:
                settled = False
                errors.append('fixture settlement: '+repr(error))
        for pipe in ((process.stdin, process.stdout) if process is not None else ()) + (self.stderr,):
            try:
                if pipe is not None:
                    pipe.close()
            except BaseException as error:
                errors.append('fixture pipe close: '+repr(error))
                pipes_settled = False
        result = dict(returncode=process.returncode if process is not None else 0, forced=forced)
        if errors or not settled or result['returncode'] != 0 or forced:
            result.update(errors=errors, group_settled=settled, resources_settled=settled and pipes_settled)
        return result


def run_case(oracle, case, sample, contract, binary, worker, output):
    case_id = secrets.token_hex(16)
    name = f"{case['id']}-{sample:02d}"
    directory = output / name
    directory.mkdir()
    record = {
        "case": case["id"],
        "sample": sample,
        "case_id": case_id,
        "passed": False,
        "errors": [],
        "timed_out": False,
    }
    started = time.monotonic()
    try:
        oracle.request("create", case_id, upload=case["upload"])
        before = oracle.request("snapshot", case_id)
        (directory / "before.json").write_text(json.dumps(before, indent=2) + "\n")
        if before["requests"] or before["receipts"]:
            raise ValueError("fixture is not isolated and empty")
        lines = [f'goto "{oracle.origin}/form/{case_id}"', "observe"]
        lines += case["before_fill"]
        if case["fill"]:
            lines += contract["fill"]
        lines += case["after_fill"]
        if case["submit"]:
            lines += ['click "Submit application"']
        if case.get("expected_receipts", 1):
            lines += [
                "wait visible css(#receipt)",
                'click "Reload receipt"',
                "wait visible css(#receipt)",
            ]
        lines += ["observe"]
        task = directory / "task.oil"
        task.write_text("\n".join(lines) + "\n")
        command = [str(binary), "--file", str(task), "native", "--allow-loopback"]
        document = FIXTURES / "authorized-document.txt"
        upload = {
            "name": "document",
            "filename": document.name,
            "bytes": document.stat().st_size,
            "sha256": digest(document),
        }
        if case["upload"]:
            command += ["--upload", f"authorized={document}"]
        check_execution_generation()
        process = subprocess.Popen(
            command,
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
        record.update(returncode=process.returncode, task_sha256=digest(task))
        if process.returncode != 0 or record["timed_out"]:
            record["errors"].append("OIL process failed or timed out")
        snapshot = oracle.request("snapshot", case_id)
        (directory / "oracle.json").write_text(json.dumps(snapshot, indent=2) + "\n")
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        record["errors"] += validate_outcome(
            case, contract, case_id, oracle.origin, frames, snapshot, upload
        )
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
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=2)
    args = parser.parse_args()
    from generation_gate import inherited_generation_lease
    admission = inherited_generation_lease()
    args.output = args.output.resolve()
    args.binary, args.worker = args.binary.resolve(), args.worker.resolve()
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    contract = json.loads((FIXTURES / ("forms-cases-v2.json" if args.frontend_v2 else "forms-cases.json")).read_text())
    report = {
        "schema_version": 1,
        "goal_complete": False,
        "family_qualified": False,
        "qualification": "controlled form outcome diagnostics, not general website/family/release qualification",
        "planned": len(contract["cases"]) * args.samples,
        "jobs": args.jobs,
        "records": [],
    }
    oracle = None
    try:
        report["parent_sha256"], report["worker_sha256"] = digest(args.binary), digest(args.worker)
        report["source_tree_sha256"] = subprocess.check_output(
            [str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True
        ).strip()
        report["fixture_hashes"] = {
            path.name: digest(path) for path in sorted(FIXTURES.glob("*")) if path.is_file()
        }
        report["runner_sha256"] = digest(__file__)
        manifest = args.worker.parent.parent / "Resources/worker-manifest.json"
        report["worker_manifest"] = json.loads(manifest.read_text())
        report["source_matches_signed_build"] = (
            report["source_tree_sha256"] == report["worker_manifest"]["source_tree_sha256"]
        )
        result = subprocess.run(
            [str(args.binary), "native", "--runtime-info"],
            env=dict(os.environ, ORYN_PAGE_WORKER=str(args.worker)),
            capture_output=True,
            timeout=30,
            check=False,
        )
        (args.output / "runtime-info.stdout").write_bytes(result.stdout)
        (args.output / "runtime-info.stderr").write_bytes(result.stderr)
        result.check_returncode()
        report["runtime"] = json.loads(result.stdout)
        validate_protocol_pair(report["runtime"], report["worker_manifest"], ROOT)
        if (
            not report["runtime"].get("sandboxed")
            or report["runtime"].get("worker_binary_sha256") != report["worker_sha256"]
        ):
            raise ValueError("signed runtime identity mismatch")
        if args.frontend_v2:
            report["frontend_module_sha256"] = digest(FIXTURES / "v2/forms-client.mjs")
        oracle = Oracle(args.output, FIXTURES / "forms_fixture_v2.py" if args.frontend_v2 else None)
        report["origin"] = oracle.origin
        jobs = [
            (case, sample) for sample in range(1, args.samples + 1) for case in contract["cases"]
        ]
        with ThreadPoolExecutor(max_workers=args.jobs) as executor:
            futures = [
                executor.submit(
                    run_case, oracle, case, sample, contract, args.binary, args.worker, args.output
                )
                for case, sample in jobs
            ]
            report["records"] = [future.result() for future in futures]
        # Stop accepting HTTP and join handlers before final snapshots. This
        # prevents a late rejected/duplicate POST from escaping the denominator.
        report["oracle_seal"] = oracle.request("seal", "0" * 32)
        document = FIXTURES / "authorized-document.txt"
        upload = {
            "name": "document",
            "filename": document.name,
            "bytes": document.stat().st_size,
            "sha256": digest(document),
        }
        cases = {case["id"]: case for case in contract["cases"]}
        for record in report["records"]:
            directory = args.output / f"{record['case']}-{record['sample']:02d}"
            snapshot = oracle.request("snapshot", record["case_id"])
            (directory / "final-oracle.json").write_text(json.dumps(snapshot, indent=2) + "\n")
            frames = [
                json.loads(line)
                for line in (directory / "stdout.jsonl").read_text().splitlines()
                if line.strip()
            ]
            errors = validate_outcome(
                cases[record["case"]],
                contract,
                record["case_id"],
                oracle.origin,
                frames,
                snapshot,
                upload,
            )
            record["errors"] = list(dict.fromkeys(record["errors"] + errors))
            record["passed"] = not record["errors"]
            record["final_oracle_verified"] = True
            (directory / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        report["artifact_identities_unchanged"] = (
            report["parent_sha256"] == digest(args.binary)
            and report["worker_sha256"] == digest(args.worker)
            and report["runner_sha256"] == digest(__file__)
            and all(digest(FIXTURES / name) == value for name, value in report["fixture_hashes"].items())
            and (not args.frontend_v2 or report["frontend_module_sha256"] == digest(FIXTURES / "v2/forms-client.mjs")))
        if not report["artifact_identities_unchanged"]:
            raise ValueError("candidate or fixture artifacts changed during run")
    except Exception as error:
        report["infrastructure_error"] = str(error)
    finally:
        if oracle is not None:
            report["fixture_cleanup"] = oracle.close()
    report["passed"] = sum(
        record["passed"] and record.get("final_oracle_verified", False)
        for record in report["records"]
    )
    report["status"] = (
        "passed"
        if (
            report["passed"] == report["planned"]
            and not report.get("infrastructure_error")
            and report.get("fixture_cleanup") == {"returncode": 0, "forced": False}
        )
        else "failed"
    )
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {"status": report["status"], "passed": report["passed"], "planned": report["planned"]}
        )
    )
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())

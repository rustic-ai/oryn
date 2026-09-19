#!/usr/bin/env python3
"""Run every locked framework task against a verified, immutable build snapshot."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
from evidence_protocols import validate_protocol_pair
import signal
import subprocess
import sys
import threading
import time
from urllib.parse import unquote, urlsplit

from generation_gate import generation_gate, GenerationLease, inherited_generation_lease, check_execution_generation
from completion_provenance import read_record

ROOT = Path(__file__).resolve().parents[1]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


build = module("framework_build", ROOT / "scripts/build_app_compatibility_framework_corpus.py")


def validate(framework, task, frames):
    errors = []
    if not frames or any(
        not isinstance(f, dict) or f.get("kind") in ("error", "unsupported") for f in frames
    ):
        return ["missing or failed OIL frames"]
    expected = ["Items: 1"]
    if framework == "react":
        expected += ["Surface: oryn|tools|2026-08-27|2|1|fixture.txt:7|loaded|closed"]
    if [f.get("value") for f in frames if f.get("kind") == "structured"] != expected:
        errors.append("framework-owned state outcome mismatch")
    actions = [f.get("result", {}) for f in frames if f.get("kind") == "action"]
    expected_count = sum(
        line.split()[0] in ("type", "select", "click") for line in task.splitlines() if line.strip()
    )
    ids = [a.get("action_id") for a in actions]
    if (
        len(actions) != expected_count
        or any(type(i) is not int or i < 1 for i in ids)
        or len(set(ids)) != len(ids)
        or any(a.get("execution_domain") != "native" for a in actions)
    ):
        errors.append("native action identity/count mismatch")
    traces = [f for f in frames if f.get("kind") == "trace"]
    if len(traces) != 1:
        errors.append("missing or duplicate diagnostic trace")
    for trace in traces:
        for event in trace.get("events", []):
            kind = event.get("kind", {})
            if kind.get("kind") == "script_diagnostic" or (
                kind.get("kind") == "console" and kind.get("level") == "error"
            ):
                errors.append(
                    "framework emitted script/console error: "
                    + str(kind.get("detail", kind.get("message")))
                )
    if not any(f.get("kind") == "observation" for f in frames):
        errors.append("no native observation")
    return errors


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, roots, **kwargs):
        self.roots = roots
        super().__init__(*args, **kwargs)

    def setup(self):
        super().setup()
        self.connection.settimeout(10)

    def log_message(self, *_args):
        pass

    def translate_path(self, path):
        parts = unquote(urlsplit(path).path).split("/")
        if (
            len(parts) < 4
            or parts[1:3] != ["app-compatibility", "frameworks"]
            or parts[3] not in self.roots
        ):
            return str(ROOT / "artifacts/nonexistent-framework-route")
        root = self.roots[parts[3]]
        if any(part in ("..", ".") or "\\" in part for part in parts[4:]):
            return str(root / ".invalid-route")
        target = root.joinpath(*parts[4:])
        if not target.exists() and target.with_suffix(target.suffix + ".html").is_file():
            target = target.with_suffix(target.suffix + ".html")
        return str(target)


def finish_case_process(process):
    """Reap the direct child and observe its separate group before releasing it."""
    result = dict(parent_reaped=False, descendants_clean=False, forced=False, errors=[])
    if process is None:
        result['errors'].append('framework process was not created')
        return result
    try:
        if process.poll() is None:
            result['forced'] = True
            os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=10)
        result['parent_reaped'] = True
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            result['descendants_clean'] = True
        else:
            # Keep the original leak observable even if emergency cleanup works.
            result['forced'] = True
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    except (OSError, subprocess.SubprocessError) as error:
        result['errors'].append(str(error))
    return result


def validate_process_record(record):
    cleanup = record.get('process_cleanup')
    if (type(record.get('returncode')) is not int or record['returncode'] != 0
            or record.get('timed_out') is not False or not isinstance(cleanup, dict)
            or cleanup.get('parent_reaped') is not True
            or cleanup.get('descendants_clean') is not True
            or cleanup.get('forced') is not False or cleanup.get('errors') != []):
        return ['framework process exit/descendant cleanup incomplete']
    return []


def check_generation(args):
    """Official component mode keeps the original declaration through each child.

    Standalone historical diagnostics carry no completion receipt, but require
    the same explicit or inherited frozen generation declaration.
    """
    lease = getattr(args, "generation_admission", None)
    if lease is not None: lease.check()
    gate = getattr(args, 'generation_gate', None)
    if gate is None:
        check_execution_generation()
        return
    if gate is not None:
        failed = getattr(args, 'generation_failure', None)
        if failed is not None and failed.is_set():
            raise ValueError('generation custody already failed; later launches cancelled')
        try:
            generation_gate(gate)
            read_record(args.generation_identity)
        except Exception:
            if failed is not None:
                failed.set()
            raise


def run_case(framework, sample, origin, args):
    directory = args.output / f"{framework}-{sample:02d}"
    directory.mkdir()
    original = build.CORPUS / (
        "framework-react-surface.oil" if framework == "react" else "framework-action-smoke.oil"
    )
    task = original.read_text()
    (directory / "task.oil").write_text(task)
    record = dict(
        framework=framework,
        sample=sample,
        errors=[],
        timed_out=False,
        task_sha256=build.digest(original),
    )
    started = time.monotonic()
    process = None
    try:
        check_generation(args)
        process = subprocess.Popen(
            [
                str(args.binary),
                "--file",
                str(directory / "task.oil"),
                "native",
                "--url",
                f"{origin}/app-compatibility/frameworks/{framework}/",
                "--allow-loopback",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=dict(os.environ, ORYN_PAGE_WORKER=str(args.worker)),
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
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        record["errors"] += validate(framework, task, frames)
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        record["errors"].append(str(error))
    finally:
        record['process_cleanup'] = finish_case_process(process)
    try:
        check_generation(args)
    except (OSError, ValueError, TypeError, KeyError) as error:
        record['errors'].append('generation custody failed: ' + str(error))
    record['errors'] += validate_process_record(record)
    record["elapsed_seconds"] = round(time.monotonic() - started, 3)
    record["passed"] = not record["errors"]
    (directory / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record), flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--generation-gate", type=Path, help="Complete generation declaration; inherited frozen admission required when omitted")
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/oryn")
    parser.add_argument(
        "--worker",
        type=Path,
        default=ROOT / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker",
    )
    parser.add_argument("--samples", type=int, choices=range(1, 9), default=3)
    parser.add_argument("--jobs", type=int, choices=range(1, 5), default=3)
    args = parser.parse_args()
    admission = GenerationLease(args.generation_gate, validator=generation_gate) if args.generation_gate is not None else inherited_generation_lease()
    args.generation_admission = admission
    args.generation_gate = admission.path
    if args.generation_gate is not None:
        args.generation_failure = threading.Event()
        generation_gate(args.generation_gate)
        args.generation_identity = dict(path=str(args.generation_gate.resolve()), sha256=build.digest(args.generation_gate))
        check_generation(args)
    args.output, args.binary, args.worker, args.build = (
        p.resolve() for p in (args.output, args.binary, args.worker, args.build)
    )
    admission.check()
    args.output.mkdir(parents=True, exist_ok=False)
    report = dict(
        schema_version=1,
        goal_complete=False,
        family_qualified=False,
        planned=8 * args.samples,
        records=[],
        status="failed",
        jobs=args.jobs,
    )
    server = None
    thread = None
    try:
        manifest = json.loads(args.build.read_text())
        if manifest["status"] != "passed" or set(manifest["builds"]) != set(build.OUTPUTS):
            raise ValueError("incomplete framework build corpus")
        roots = {}
        for framework, record in manifest["builds"].items():
            root = (args.build.parent / record["artifact"]).resolve()
            if not root.is_relative_to(args.build.parent) or build.inventory(root) != {
                k: record[k] for k in ("sha256", "files")
            }:
                raise ValueError("framework build identity mismatch: " + framework)
            roots[framework] = root
        report["build_manifest_sha256"] = build.digest(args.build)
        report["build_manifest"] = str(args.build.relative_to(ROOT))
        report["parent_sha256"], report["worker_sha256"] = (
            build.digest(args.binary),
            build.digest(args.worker),
        )
        report["source_tree_sha256"] = subprocess.check_output(
            [str(ROOT / "scripts/source-tree-hash.sh")], cwd=ROOT, text=True
        ).strip()
        report["worker_manifest"] = json.loads(
            (args.worker.parent.parent / "Resources/worker-manifest.json").read_text()
        )
        report["source_matches_signed_build"] = (
            report["source_tree_sha256"] == report["worker_manifest"]["source_tree_sha256"]
        )
        check_generation(args)
        runtime = subprocess.run(
            [str(args.binary), "native", "--runtime-info"],
            env=dict(os.environ, ORYN_PAGE_WORKER=str(args.worker)),
            capture_output=True,
            timeout=30,
            check=True,
        )
        (args.output / "runtime-info.stdout").write_bytes(runtime.stdout)
        report["runtime"] = json.loads(runtime.stdout)
        validate_protocol_pair(report["runtime"], report["worker_manifest"], ROOT)
        if (
            not report["runtime"].get("sandboxed")
            or report["runtime"].get("worker_binary_sha256") != report["worker_sha256"]
        ):
            raise ValueError("signed runtime identity mismatch")
        check_generation(args)
        server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Handler, roots=roots))
        server.daemon_threads = False
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        origin = f"http://127.0.0.1:{server.server_port}"
        report["origin"] = origin
        with ThreadPoolExecutor(max_workers=args.jobs) as executor:
            futures = [
                executor.submit(run_case, framework, sample, origin, args)
                for sample in range(1, args.samples + 1)
                for framework in build.OUTPUTS
            ]
            report["records"] = [future.result() for future in futures]
        server.shutdown()
        server.server_close()
        thread.join(timeout=10)
        report["fixture_cleanup"] = not thread.is_alive()
        server = None
        check_generation(args)
        report["artifact_identities_unchanged"] = all(
            build.inventory(roots[name])["sha256"] == record["sha256"]
            for name, record in manifest["builds"].items()
        )
    except Exception as error:
        report["infrastructure_error"] = str(error)
    finally:
        if server is not None:
            server.shutdown()
            server.server_close()
            if thread is not None:
                thread.join(timeout=10)
    report["passed"] = sum(r["passed"] for r in report["records"])
    if (
        report["passed"] == report["planned"]
        and report.get("fixture_cleanup")
        and report.get("artifact_identities_unchanged")
        and not report.get("infrastructure_error")
    ):
        report["status"] = "passed"
    report["scope"] = (
        "Locked framework hydration/state-action diagnostics. Not persisted server outcomes, full routing/storage coverage, or workflow-family qualification. Failures remain in the eight-framework denominator."
    )
    (args.output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ("status", "passed", "planned")}), flush=True)
    return int(report["status"] != "passed")


if __name__ == "__main__":
    sys.exit(main())

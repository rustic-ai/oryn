#!/usr/bin/env python3
"""Run pinned, applicable upstream WPT testharness cases through native Oryn."""

from __future__ import annotations

import argparse
import hashlib
import http.server
import json
import os
import subprocess
import tempfile
import threading
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-completion-v1.json"
EXPANDED_MANIFEST = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1.json"
EXPANDED_INVENTORY = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1-inventory.json"
COMPLETION = r"""
<script>
globalThis.__oryn_wpt_results=[];
function __oryn_publish_wpt(harness, complete) {
  var value = {complete:!!complete,harness:harness && typeof harness.status === 'number' ? harness.status : 0,tests:globalThis.__oryn_wpt_results};
  var output = document.getElementById('oryn-wpt-result') || document.createElement('pre');
  output.id='oryn-wpt-result'; output.textContent='__ORYN_WPT_DONE__'+JSON.stringify(value);
  if (!output.parentNode) (document.body || document.documentElement).appendChild(output);
}
add_result_callback(function(test) {
  globalThis.__oryn_wpt_results.push({name:String(test.name || ''),status:Number(test.status),message:String(test.message || '')});
  __oryn_publish_wpt(null, false);
});
add_completion_callback(function(tests, harness) {
  __oryn_publish_wpt(harness, true);
});
</script>
"""
WORKER_COMPLETION = r"""
self.__oryn_wpt_results=[];
let __oryn_wpt_result_generation=0;
function __oryn_publish_wpt(harness, complete) {
  self.postMessage({__oryn_wpt:true,value:{complete:!!complete,harness:harness && typeof harness.status === 'number' ? harness.status : 0,tests:self.__oryn_wpt_results}});
}
add_result_callback(function(test) {
  self.__oryn_wpt_results.push({name:String(test.name || ''),status:Number(test.status),message:String(test.message || '')});
  __oryn_publish_wpt(null, false);
  const generation=++__oryn_wpt_result_generation;
  queueMicrotask(function(){
    if(generation===__oryn_wpt_result_generation)__oryn_publish_wpt(null, true);
  });
});
add_completion_callback(function(tests, harness) { __oryn_publish_wpt(harness, true); });
"""


class Handler(http.server.SimpleHTTPRequestHandler):
    root: Path

    def translate_path(self, path: str) -> str:
        relative = urllib.parse.urlparse(path).path.lstrip("/")
        candidate = (self.root / relative).resolve()
        if self.root not in candidate.parents and candidate != self.root:
            return str(self.root / "__denied__")
        return str(candidate)

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/__oryn_any_runner.html":
            source = urllib.parse.parse_qs(parsed.query).get("source", [""])[0]
            body = (
                "<!doctype html><meta charset=utf-8><title>Oryn WPT</title>"
                '<script src="/resources/testharness.js"></script>'
                + COMPLETION
                + f'<script src="/{source}"></script>'
            ).encode()
            self.send_response(200); self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        if parsed.path == "/__oryn_worker_runner.html":
            source = urllib.parse.parse_qs(parsed.query).get("source", [""])[0]
            variant = urllib.parse.parse_qs(parsed.query).get("variant", [""])[0]
            global_name = urllib.parse.parse_qs(parsed.query).get(
                "global", ["dedicatedworker"]
            )[0]
            worker_query = urllib.parse.urlencode({"source": source, "variant": variant})
            constructor = {
                "dedicatedworker": f"new Worker('/__oryn_worker_bootstrap.js?{worker_query}')",
                "sharedworker": f"new SharedWorker('/__oryn_worker_bootstrap.js?{worker_query}').port",
                "serviceworker": f"new ServiceWorker('/__oryn_worker_bootstrap.js?{worker_query}')",
            }.get(global_name)
            if constructor is None:
                self.send_error(400, "unsupported worker global")
                return
            body = (
                "<!doctype html><meta charset=utf-8><title>Oryn worker WPT</title>"
                '<pre id="oryn-wpt-result"></pre><script>'
                f"const worker={constructor};"
                "if(worker.start)worker.start();"
                "worker.onmessage=event=>{if(event.data&&event.data.__oryn_wpt){document.getElementById('oryn-wpt-result').textContent='__ORYN_WPT_DONE__'+JSON.stringify(event.data.value)}};"
                "worker.onerror=event=>{document.getElementById('oryn-wpt-result').textContent='__ORYN_WPT_DONE__'+JSON.stringify({complete:true,harness:1,tests:[{name:'worker bootstrap',status:1,message:String(event.message||'worker error')}]})};"
                "</script>"
            ).encode()
            self.send_response(200); self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        if parsed.path == "/__oryn_worker_bootstrap.js":
            source = urllib.parse.parse_qs(parsed.query).get("source", [""])[0]
            source_path = (self.root / source).resolve()
            if self.root not in source_path.parents or not source_path.is_file():
                self.send_error(404)
                return
            harness = (self.root / "resources/testharness.js").read_text(
                encoding="utf-8", errors="replace"
            )
            test_source = source_path.read_text(encoding="utf-8", errors="replace")
            dependencies = []
            for line in test_source.splitlines():
                if not line.startswith("// META: script="):
                    continue
                dependency = line.split("=", 1)[1].strip()
                dependency_path = (
                    (self.root / dependency.lstrip("/"))
                    if dependency.startswith("/")
                    else (source_path.parent / dependency)
                ).resolve()
                if self.root in dependency_path.parents and dependency_path.is_file():
                    dependencies.append(
                        dependency_path.read_text(encoding="utf-8", errors="replace")
                    )
            body = (
                harness
                + "\n"
                + WORKER_COMPLETION
                + "\n"
                + "\n".join(dependencies)
                + "\n"
                + test_source
            ).encode()
            self.send_response(200); self.send_header("content-type", "text/javascript; charset=utf-8")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        path = Path(self.translate_path(self.path))
        if path.suffix in {".html", ".htm"} and path.is_file():
            body = path.read_text(encoding="utf-8", errors="replace")
            body = body.replace('<script src=/resources/testharnessreport.js></script>', '')
            lowered = body.lower()
            harness = lowered.find("testharness.js")
            index = lowered.find("</script>", harness) + len("</script>") if harness >= 0 else -1
            body = body[:index] + COMPLETION + body[index:] if index >= len("</script>") else body + COMPLETION
            encoded = body.encode()
            self.send_response(200); self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(encoded))); self.end_headers(); self.wfile.write(encoded)
            return
        super().do_GET()

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def applicable_tests(checkout: Path) -> list[tuple[Path, str, str, str]]:
    manifest = json.loads(MANIFEST.read_text())
    roots: list[tuple[str, str]] = []
    for entry in manifest["classifications"]:
        if entry["disposition"] == "required":
            roots.extend((part.strip(), entry["id"]) for part in entry["path"].split(","))
    selected: dict[Path, tuple[int, str]] = {}
    for root, classification in roots:
        directory = checkout / root
        if not directory.exists():
            continue
        for path in directory.rglob("*"):
            name = path.name
            if path.is_file() and (
                name.endswith(".any.js") or name.endswith(".window.js") or path.suffix in {".html", ".htm"}
            ) and not any(token in name for token in ["-ref.", ".manual."]):
                relative = path.relative_to(checkout)
                candidate = (len(Path(root).parts), classification)
                if relative not in selected or candidate[0] > selected[relative][0]:
                    selected[relative] = candidate
    return sorted(
        ((path, value[1], "window", "") for path, value in selected.items()),
        key=lambda item: item[0].as_posix(),
    )


def inventoried_tests(checkout: Path, inventory_path: Path) -> list[tuple[Path, str, str, str]]:
    inventory = json.loads(inventory_path.read_text(encoding="utf-8"))
    expected_revision = json.loads(EXPANDED_MANIFEST.read_text(encoding="utf-8"))["upstream"]["revision"]
    if inventory.get("upstream_revision") != expected_revision:
        raise SystemExit("expanded inventory has the wrong WPT revision")
    tests: list[tuple[Path, str, str, str]] = []
    for record in inventory["tests"]:
        source = checkout / record["path"]
        actual = __import__("hashlib").sha256(source.read_bytes()).hexdigest()
        if actual != record["sha256"]:
            raise SystemExit(f"WPT source digest mismatch: {record['path']}")
        for global_name in record["globals"]:
            for variant in record["variants"]:
                tests.append((Path(record["path"]), record["workstream"], global_name, variant))
    return tests


def run_case(
    binary: Path,
    port: int,
    test: Path,
    classification: str,
    global_name: str,
    variant: str,
    timeout: int,
    in_process_probe: bool,
) -> dict[str, object]:
    identifier = f"{test.as_posix()}|{global_name}|{variant}"
    if global_name not in {"window", "dedicatedworker", "sharedworker", "serviceworker"}:
        return {
            "id": identifier,
            "path": test.as_posix(),
            "classification": classification,
            "global": global_name,
            "variant": variant,
            "status": "failed",
            "diagnostics": [f"WPT global {global_name!r} is not yet installed by the native harness"],
        }
    if global_name in {"dedicatedworker", "sharedworker", "serviceworker"}:
        query = urllib.parse.urlencode(
            {"source": test.as_posix(), "variant": variant, "global": global_name}
        )
        url = f"http://127.0.0.1:{port}/__oryn_worker_runner.html?{query}"
    elif test.name.endswith((".any.js", ".window.js")):
        query = urllib.parse.urlencode({"source": test.as_posix()})
        variant_suffix = ("&" + variant[1:]) if variant.startswith("?") else variant
        url = f"http://127.0.0.1:{port}/__oryn_any_runner.html?{query}{variant_suffix}"
    else:
        url = f"http://127.0.0.1:{port}/{urllib.parse.quote(test.as_posix())}{variant}"
    with tempfile.NamedTemporaryFile("w", suffix=".oil", delete=False) as oil:
        oil.write('wait idle\nwait exists "#oryn-wpt-result"\nextract text --selector "#oryn-wpt-result"\n')
        oil_path = Path(oil.name)
    trace_path = oil_path.with_suffix(".trace.json")
    try:
        native = ["native", "--allow-loopback"]
        if in_process_probe:
            native.append("--in-process-probe")
        completed = subprocess.run(
            [str(binary), "--file", str(oil_path), *native, "--url", url, "--trace-out", str(trace_path)],
            cwd=ROOT, text=True, capture_output=True, timeout=timeout,
        )
        marker = "__ORYN_WPT_DONE__"
        payload = ""
        for line in completed.stdout.splitlines():
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                record = None
            value = record.get("value") if isinstance(record, dict) else None
            if isinstance(value, str) and marker in value:
                payload = value.rsplit(marker, 1)[-1].strip()
        if not payload and marker in completed.stdout:
            payload = completed.stdout.rsplit(marker, 1)[-1].strip()
        try:
            result = json.loads(payload)
        except (json.JSONDecodeError, IndexError):
            trace = trace_path.read_text(errors="replace") if trace_path.exists() else ""
            probe = subprocess.run(
                [str(binary), *native, "--url", url, "--evaluate", "typeof add_completion_callback+'|'+typeof test+'|'+document.documentElement.outerHTML"],
                cwd=ROOT, text=True, capture_output=True, timeout=timeout,
            )
            detail = (completed.stderr + "\n" + completed.stdout + "\n" + trace + "\nDOM_PROBE:\n" + probe.stdout + "\n" + probe.stderr)[-20000:].strip()
            return {"id":identifier,"path":test.as_posix(),"classification":classification,"global":global_name,"variant":variant,"status":"failed","diagnostics":[detail or "missing WPT completion payload"]}
        failed = (
            not result.get("complete", False)
            or result.get("harness") not in (0, None)
            or not result.get("tests")
            or any(item.get("status") != 0 for item in result.get("tests", []))
        )
        return {"id":identifier,"path":test.as_posix(),"classification":classification,"global":global_name,"variant":variant,"status":"failed" if failed else "passed","result":result}
    except subprocess.TimeoutExpired:
        return {"id":identifier,"path":test.as_posix(),"classification":classification,"global":global_name,"variant":variant,"status":"failed","diagnostics":[f"timeout after {timeout}s"]}
    finally:
        oil_path.unlink(missing_ok=True)
        trace_path.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument("--max-tests", type=int)
    parser.add_argument("--filter", help="substring filter applied to upstream test paths")
    parser.add_argument(
        "--global",
        dest="global_filter",
        choices=["window", "dedicatedworker", "sharedworker", "serviceworker"],
        help="run only one inventoried global (local diagnosis and sharding)",
    )
    parser.add_argument("--inventory", type=Path, help="run the immutable expanded inventory")
    parser.add_argument("--shard-count", type=int, default=1)
    parser.add_argument("--shard-index", type=int, default=0)
    parser.add_argument(
        "--in-process-probe",
        action="store_true",
        help="explicit local diagnostic only; release evidence must use the worker boundary",
    )
    args = parser.parse_args()
    revision_manifest = EXPANDED_MANIFEST if args.inventory else MANIFEST
    revision = json.loads(revision_manifest.read_text())["upstream"]["revision"]
    actual = subprocess.check_output(["git", "-C", args.checkout, "rev-parse", "HEAD"], text=True).strip()
    if actual != revision:
        raise SystemExit(f"WPT revision mismatch: expected {revision}, found {actual}")
    binary = args.binary.resolve()
    runtime_command = [str(binary), "native", "--runtime-info"]
    if args.in_process_probe:
        runtime_command.append("--in-process-probe")
    runtime_probe = subprocess.run(
        runtime_command, cwd=ROOT, text=True, capture_output=True, timeout=args.timeout
    )
    if runtime_probe.returncode != 0:
        raise SystemExit(
            "runtime preflight failed: "
            + (runtime_probe.stderr or runtime_probe.stdout).strip()
        )
    try:
        runtime_metadata = json.loads(runtime_probe.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"runtime preflight returned invalid JSON: {error}") from error
    source_commit = subprocess.check_output(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True
    ).strip()
    source_tree_dirty = bool(
        subprocess.check_output(
            ["git", "-C", str(ROOT), "status", "--porcelain", "--untracked-files=no"],
            text=True,
        ).strip()
    )
    Handler.root = args.checkout.resolve()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    if args.shard_count < 1 or not 0 <= args.shard_index < args.shard_count:
        raise SystemExit("invalid shard index/count")
    tests = inventoried_tests(args.checkout, args.inventory or EXPANDED_INVENTORY) if args.inventory else applicable_tests(args.checkout)
    if args.filter:
        tests = [item for item in tests if args.filter in item[0].as_posix()]
    if args.global_filter:
        tests = [item for item in tests if item[2] == args.global_filter]
    tests = [item for index, item in enumerate(tests) if index % args.shard_count == args.shard_index]
    if args.max_tests is not None: tests = tests[: args.max_tests]
    try:
        assertions = [
            run_case(
                binary,
                server.server_port,
                test,
                classification,
                global_name,
                variant,
                args.timeout,
                args.in_process_probe,
            )
            for test, classification, global_name, variant in tests
        ]
    finally:
        server.shutdown(); thread.join()
    result = {"schema_version":2 if args.inventory else 1,"upstream_revision":revision,"inventory_sha256":hashlib.sha256((args.inventory or EXPANDED_INVENTORY).read_bytes()).hexdigest() if args.inventory else None,"execution_mode":"in_process_probe" if args.in_process_probe else "sandboxed_worker","binary":{"sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"size_bytes":binary.stat().st_size},"runtime_metadata":runtime_metadata,"source":{"commit":source_commit,"dirty":source_tree_dirty},"shard":{"index":args.shard_index,"count":args.shard_count},"assertions":assertions,"outcome":{"status":"passed" if assertions and all(item["status"]=="passed" for item in assertions) else "failed","required_total":len(assertions),"required_passed":sum(item["status"]=="passed" for item in assertions)}}
    args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(result, indent=2, sort_keys=True)+"\n")
    print(json.dumps(result["outcome"], sort_keys=True))
    return 0 if result["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

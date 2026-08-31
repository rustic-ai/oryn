#!/usr/bin/env python3
"""Run pinned, applicable upstream WPT testharness cases through native Oryn."""

from __future__ import annotations

import argparse
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


def applicable_tests(checkout: Path) -> list[tuple[Path, str]]:
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
        ((path, value[1]) for path, value in selected.items()),
        key=lambda item: item[0].as_posix(),
    )


def run_case(binary: Path, port: int, test: Path, classification: str, timeout: int) -> dict[str, object]:
    if test.name.endswith((".any.js", ".window.js")):
        url = f"http://127.0.0.1:{port}/__oryn_any_runner.html?source={urllib.parse.quote(test.as_posix())}"
    else:
        url = f"http://127.0.0.1:{port}/{urllib.parse.quote(test.as_posix())}"
    with tempfile.NamedTemporaryFile("w", suffix=".oil", delete=False) as oil:
        oil.write('wait idle\nwait exists "#oryn-wpt-result"\nextract text --selector "#oryn-wpt-result"\n')
        oil_path = Path(oil.name)
    trace_path = oil_path.with_suffix(".trace.json")
    try:
        completed = subprocess.run(
            [str(binary), "--file", str(oil_path), "native", "--allow-loopback", "--url", url, "--trace-out", str(trace_path)],
            cwd=ROOT, text=True, capture_output=True, timeout=timeout,
        )
        marker = "__ORYN_WPT_DONE__"
        payload = completed.stdout.rsplit(marker, 1)[-1].strip() if marker in completed.stdout else ""
        try:
            result = json.loads(payload.splitlines()[0])
        except (json.JSONDecodeError, IndexError):
            trace = trace_path.read_text(errors="replace") if trace_path.exists() else ""
            probe = subprocess.run(
                [str(binary), "native", "--allow-loopback", "--url", url, "--evaluate", "typeof add_completion_callback+'|'+typeof test+'|'+document.documentElement.outerHTML"],
                cwd=ROOT, text=True, capture_output=True, timeout=timeout,
            )
            detail = (completed.stderr + "\n" + completed.stdout + "\n" + trace + "\nDOM_PROBE:\n" + probe.stdout + "\n" + probe.stderr)[-20000:].strip()
            return {"id":test.as_posix(),"classification":classification,"status":"failed","diagnostics":[detail or "missing WPT completion payload"]}
        failed = (
            not result.get("complete", False)
            or result.get("harness") not in (0, None)
            or not result.get("tests")
            or any(item.get("status") != 0 for item in result.get("tests", []))
        )
        return {"id":test.as_posix(),"classification":classification,"status":"failed" if failed else "passed","result":result}
    except subprocess.TimeoutExpired:
        return {"id":test.as_posix(),"classification":classification,"status":"failed","diagnostics":[f"timeout after {timeout}s"]}
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
    args = parser.parse_args()
    revision = json.loads(MANIFEST.read_text())["upstream"]["revision"]
    actual = subprocess.check_output(["git", "-C", args.checkout, "rev-parse", "HEAD"], text=True).strip()
    if actual != revision:
        raise SystemExit(f"WPT revision mismatch: expected {revision}, found {actual}")
    Handler.root = args.checkout.resolve()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    tests = applicable_tests(args.checkout)
    if args.filter:
        tests = [item for item in tests if args.filter in item[0].as_posix()]
    if args.max_tests is not None: tests = tests[: args.max_tests]
    try:
        assertions = [run_case(args.binary.resolve(), server.server_port, test, classification, args.timeout) for test, classification in tests]
    finally:
        server.shutdown(); thread.join()
    result = {"schema_version":1,"upstream_revision":revision,"assertions":assertions,"outcome":{"status":"passed" if assertions and all(item["status"]=="passed" for item in assertions) else "failed","required_total":len(assertions),"required_passed":sum(item["status"]=="passed" for item in assertions)}}
    args.output.parent.mkdir(parents=True, exist_ok=True); args.output.write_text(json.dumps(result, indent=2, sort_keys=True)+"\n")
    print(json.dumps(result["outcome"], sort_keys=True))
    return 0 if result["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

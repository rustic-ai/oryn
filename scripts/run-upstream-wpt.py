#!/usr/bin/env python3
"""Run pinned, applicable upstream WPT testharness cases through native Oryn."""

from __future__ import annotations

import argparse
import concurrent.futures
import collections
import hashlib
import html as html_module
import http.server
from html.parser import HTMLParser
import json
import os
import re
import subprocess
import tempfile
import threading
import urllib.parse
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "benchmarks/conformance/runtime-completion-v1.json"
EXPANDED_MANIFEST = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1.json"
EXPANDED_INVENTORY = ROOT / "benchmarks/conformance/runtime-api-style-semantics-v1-inventory.json"
COMPLETION = r"""
<script>
setup({output:false});
globalThis.__oryn_wpt_results=[];
function __oryn_publish_wpt(harness, complete) {
  var value = {complete:!!complete,harness:harness && typeof harness.status === 'number' ? harness.status : 0,harnessMessage:harness && harness.message ? String(harness.message) : '',harnessStack:harness && harness.stack ? String(harness.stack) : '',tests:globalThis.__oryn_wpt_results};
  var output = document.getElementById('oryn-wpt-result') || document.createElement('pre');
  output.id='oryn-wpt-result'; output.style.cssText='display:block!important;visibility:visible!important';
  output.textContent='__ORYN_WPT_DONE__'+JSON.stringify(value);
  // `setup({output:false})` hides testharness' body. Keep the private result
  // channel outside that hidden subtree so public OIL extraction can observe
  // it without weakening Oryn's visible-text semantics.
  if (output.parentNode !== document.documentElement) document.documentElement.appendChild(output);
}
add_result_callback(function(test) {
  globalThis.__oryn_wpt_results.push({name:String(test.name || ''),status:Number(test.status),message:String(test.message || '')});
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
  self.postMessage({__oryn_wpt:true,value:{complete:!!complete,harness:harness && typeof harness.status === 'number' ? harness.status : 0,harnessMessage:harness && harness.message ? String(harness.message) : '',harnessStack:harness && harness.stack ? String(harness.stack) : '',tests:self.__oryn_wpt_results}});
}
add_result_callback(function(test) {
  self.__oryn_wpt_results.push({name:String(test.name || ''),status:Number(test.status),message:String(test.message || '')});
  const generation=++__oryn_wpt_result_generation;
  queueMicrotask(function(){
    if(generation===__oryn_wpt_result_generation)__oryn_publish_wpt(null, true);
  });
});
add_completion_callback(function(tests, harness) { __oryn_publish_wpt(harness, true); });
"""


def meta_script_paths(root: Path, source_path: Path, source_text: str) -> list[str]:
    dependencies: list[str] = []
    for line in source_text.splitlines():
        if not line.startswith("// META: script="):
            continue
        declared = line.split("=", 1)[1].strip()
        dependency = (
            root / declared.lstrip("/")
            if declared.startswith("/")
            else source_path.parent / declared
        ).resolve()
        if root not in dependency.parents or not dependency.is_file():
            raise ValueError(f"invalid WPT META script: {declared}")
        dependencies.append("/" + dependency.relative_to(root).as_posix())
    return dependencies


def fs_path_assets(root: Path, source_path: Path) -> list[Path]:
    source = source_path.read_text(encoding="utf-8", errors="replace")
    assets = []
    for raw in re.findall(r"\{\{fs_path\(([^)]+)\)\}\}", source):
        relative = raw.strip()
        base = root if relative.startswith("/") else source_path.parent
        candidate = (base / relative.lstrip("/")).resolve()
        if root not in candidate.parents or not candidate.is_file():
            raise ValueError(f"invalid WPT fs_path substitution: {raw}")
        assets.append(candidate)
    return assets


class _HtmlVariantParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.variants: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.lower() != "meta":
            return
        values = {name.lower(): value or "" for name, value in attrs}
        if values.get("name", "").lower() == "variant":
            self.variants.append(values.get("content", ""))


def declared_html_variants(source: str) -> list[str]:
    """Return WPT `<meta name=variant content=...>` values in source order.

    The pinned inventory predates variant expansion for HTML tests and records
    an empty sentinel for those files. Executing that sentinel can silently
    turn a valid parametrized test into a different `null`-parameter test, so
    the runner expands the immutable source declaration at execution time.
    """
    parser = _HtmlVariantParser()
    parser.feed(source)
    return parser.variants


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def substitute_wpt_tokens(
    body: str,
    request_host: str,
    port: int,
    root: Path | None = None,
    source_path: Path | None = None,
) -> str:
    """Apply the deterministic wptserve substitutions used by this harness."""
    remote_host = "localhost" if request_host == "127.0.0.1" else "127.0.0.1"
    replacements = {
        "{{ports[http][0]}}": str(port),
        "{{ports[http][1]}}": str(port),
        "{{ports[https][0]}}": str(port),
        "{{ports[https][1]}}": str(port),
        "{{host}}": request_host,
        "{{domains[www2]}}": remote_host,
        "{{hosts[alt][]}}": remote_host,
        "{{hosts[alt][www2]}}": remote_host,
    }
    for token, value in replacements.items():
        body = body.replace(token, value)

    def replace_fs_path(match: re.Match[str]) -> str:
        relative = match.group(1).strip()
        base = root if relative.startswith("/") or source_path is None else source_path.parent
        candidate = (base / relative.lstrip("/")).resolve()
        if root not in candidate.parents or not candidate.is_file():
            raise ValueError(f"invalid WPT fs_path substitution: {relative}")
        return str(candidate)

    if root is not None:
        body = re.sub(r"\{\{fs_path\(([^)]+)\)\}\}", replace_fs_path, body)
    return body


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def outcome(assertions: list[dict[str, object]], expected_total: int, complete: bool) -> dict[str, object]:
    passed = sum(item["status"] == "passed" for item in assertions)
    finished = len(assertions)
    if not complete:
        status = "running"
    elif finished == expected_total and assertions and passed == finished:
        status = "passed"
    else:
        status = "failed"
    return {
        "status": status,
        "required_total": expected_total,
        "required_finished": finished,
        "required_passed": passed,
    }


class Handler(http.server.SimpleHTTPRequestHandler):
    root: Path
    dispatcher_condition = threading.Condition()
    dispatcher_queues: dict[str, collections.deque[bytes]] = {}

    def translate_path(self, path: str) -> str:
        relative = urllib.parse.urlparse(path).path.lstrip("/")
        candidate = (self.root / relative).resolve()
        if self.root not in candidate.parents and candidate != self.root:
            return str(self.root / "__denied__")
        return str(candidate)

    def do_POST(self) -> None:  # noqa: N802
        """Implement the deterministic subset of wptserve used by mapped tests."""
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/common/dispatcher/dispatcher.py":
            query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
            uuid = query.get("uuid", [""])[0]
            if not uuid:
                self.send_error(400, "dispatcher uuid is required")
                return
            length = int(self.headers.get("content-length", "0"))
            if length > 16 * 1024 * 1024:
                self.send_error(413)
                return
            body = self.rfile.read(length)
            with self.dispatcher_condition:
                self.dispatcher_queues.setdefault(uuid, collections.deque()).append(body)
                self.dispatcher_condition.notify_all()
            self._send_bytes(b"done", "text/plain; charset=utf-8")
            return
        if parsed.path != "/xhr/resources/content.py":
            self.send_error(404)
            return
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        query = urllib.parse.parse_qs(parsed.query)
        if "content" in query:
            body = query["content"][0].encode("latin-1")
        response_type = "text/plain"
        if "response_charset_label" in query:
            response_type += ";charset=" + query["response_charset_label"][0]
        self.send_response(200)
        self.send_header("content-type", response_type)
        self.send_header("x-request-method", "POST")
        self.send_header("x-request-query", parsed.query or "NO")
        self.send_header("x-request-content-length", self.headers.get("content-length", "NO"))
        self.send_header("x-request-content-type", self.headers.get("content-type", "NO"))
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/resources/testdriver-vendor.js":
            # Controlled native automation only. The page worker still has no
            # filesystem or generic automation operation: these calls reach
            # generated DOM behavior and explicitly authorized upload payloads.
            body = b"""'use strict';
window.__oryn_testdriver_log = [];
const __oryn_testdriver_public_click = window.test_driver.click.bind(window.test_driver);
window.test_driver.click = function(element) { __oryn_testdriver_log.push({operation:'public_click_start',target:element && (element.id || element.tagName)}); try { const result=__oryn_testdriver_public_click(element); Promise.resolve(result).then(()=>__oryn_testdriver_log.push({operation:'public_click_resolved'}),error=>__oryn_testdriver_log.push({operation:'public_click_rejected',error:String(error)})); return result; } catch(error) { __oryn_testdriver_log.push({operation:'public_click_threw',error:String(error)}); throw error; } };
window.test_driver_internal.click = element => { __oryn_testdriver_log.push({operation:'click',target:element && (element.id || element.tagName)}); return __oryn_testdriver_click(element); };
window.test_driver_internal.send_keys = (element, keys) => { __oryn_testdriver_log.push({operation:'send_keys',target:element && (element.id || element.tagName),keys:String(keys)}); return __oryn_testdriver_send_keys(element, keys); };
window.test_driver_internal.action_sequence = actions => { __oryn_testdriver_log.push({operation:'actions',count:Array.isArray(actions)?actions.length:0}); return __oryn_testdriver_actions(actions, document.activeElement); };
"""
            self._send_bytes(body, "application/javascript; charset=utf-8")
            return
        if parsed.path == "/common/dispatcher/dispatcher.py":
            query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
            uuid = query.get("uuid", [""])[0]
            if not uuid:
                self.send_error(400, "dispatcher uuid is required")
                return
            if "show-headers" in query:
                payload = json.dumps(
                    {
                        key.lower(): value
                        for key, value in self.headers.items()
                    },
                    sort_keys=True,
                ).encode()
                with self.dispatcher_condition:
                    self.dispatcher_queues.setdefault(uuid, collections.deque()).append(payload)
                    self.dispatcher_condition.notify_all()
                cache = "max-age=31536000" if "cacheable" in query else "no-store"
                self.send_response(200)
                self.send_header("content-type", "text/plain; charset=utf-8")
                self.send_header("cache-control", cache)
                self.send_header("content-length", "0")
                self.end_headers()
                return
            with self.dispatcher_condition:
                queue = self.dispatcher_queues.setdefault(uuid, collections.deque())
                payload = queue.popleft() if queue else b"not ready"
            self._send_bytes(payload, "text/plain; charset=utf-8")
            return
        if parsed.path.endswith(
            "/html/browsers/browsing-the-web/remote-context-helper/resources/executor-window.py"
        ):
            query = urllib.parse.parse_qs(parsed.query, keep_blank_values=True)
            uuid = query.get("uuid", [""])[0]
            if not uuid:
                self.send_error(400, "executor uuid is required")
                return
            scripts = "\n".join(
                f'<script src="{html_module.escape(script, quote=True)}"></script>'
                for script in query.get("script", [])
            )
            request_headers = "\n".join(
                "window.__requestHeaders.append("
                + json.dumps(name.lower())
                + ", "
                + json.dumps(value)
                + ");"
                for name, value in self.headers.items()
            )
            start_on = query.get("startOn", [None])[0]
            base_url = "http://" + self.headers.get("host", "localhost") + self.path
            body = f"""<!doctype html>
<base href="{html_module.escape(base_url, quote=True)}">
<script src="/common/dispatcher/dispatcher.js"></script>
<script src="./executor-common.js"></script>
<script src="./executor-window.js"></script>
{scripts}
<body><script>
window.__requestHeaders = new Headers();
{request_headers}
requestExecutor({json.dumps(uuid)}, {json.dumps(start_on)});
</script>""".encode()
            status = int(query.get("status", ["200"])[0])
            self._send_bytes(body, "text/html; charset=utf-8", status=status)
            return
        if parsed.path.endswith(".sub.js"):
            source = Path(self.translate_path(self.path))
            if not source.is_file():
                self.send_error(404)
                return
            request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
            body = source.read_text(encoding="utf-8", errors="replace")
            body = substitute_wpt_tokens(
                body, request_host, self.server.server_address[1], self.root, source
            )
            encoded = body.encode()
            self.send_response(200)
            self.send_header("content-type", "application/javascript; charset=utf-8")
            self.send_header("content-length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
            return
        if parsed.path == "/__oryn_any_runner.html":
            source = urllib.parse.parse_qs(parsed.query).get("source", [""])[0]
            source_path = (self.root / source).resolve()
            if self.root not in source_path.parents or not source_path.is_file():
                self.send_error(404)
                return
            source_text = source_path.read_text(encoding="utf-8", errors="replace")
            try:
                dependencies = meta_script_paths(self.root, source_path, source_text)
            except ValueError as error:
                self.send_error(400, str(error))
                return
            dependency_tags = "".join(
                f'<script src="{html_module.escape(path, quote=True)}"></script>'
                for path in dependencies
            )
            timeout_setup = (
                "<script>setup({timeout_multiplier:4});</script>"
                if "// META: timeout=long" in source_text
                else ""
            )
            body = (
                "<!doctype html><meta charset=utf-8><title>Oryn WPT</title>"
                '<script src="/resources/testharness.js"></script>'
                + timeout_setup
                + COMPLETION
                + dependency_tags
                + f'<script src="/{html_module.escape(source, quote=True)}"></script>'
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
            request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
            test_source = substitute_wpt_tokens(
                test_source,
                request_host,
                self.server.server_address[1],
                self.root,
                source_path,
            )
            dependencies = [
                (self.root / dependency.lstrip("/")).read_text(
                    encoding="utf-8", errors="replace"
                )
                for dependency in meta_script_paths(self.root, source_path, test_source)
            ]
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
            if path.name.endswith((".sub.html", ".sub.htm")):
                request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
                body = substitute_wpt_tokens(
                    body, request_host, self.server.server_address[1], self.root, path
                )
            body = re.sub(
                r"<script\b[^>]*\bsrc\s*=\s*(['\"]?)/resources/testharnessreport\.js\1[^>]*>\s*</script>",
                "",
                body,
                flags=re.IGNORECASE,
            )
            lowered = body.lower()
            harness = lowered.find("testharness.js")
            index = lowered.find("</script>", harness) + len("</script>") if harness >= 0 else -1
            timeout_setup = (
                "<script>setup({timeout_multiplier:20});</script>"
                if (
                    ("name=timeout" in lowered or 'name="timeout"' in lowered or "name='timeout'" in lowered)
                    and ("content=long" in lowered or 'content="long"' in lowered or "content='long'" in lowered)
                )
                else ""
            )
            if harness >= 0:
                injection = timeout_setup + COMPLETION
                body = body[:index] + injection + body[index:]
            encoded = body.encode()
            self.send_response(200); self.send_header("content-type", "text/html; charset=utf-8")
            self.send_header("content-length", str(len(encoded))); self.end_headers(); self.wfile.write(encoded)
            return
        super().do_GET()

    def _send_bytes(self, body: bytes, content_type: str, *, status: int = 200) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def is_testharness_html(path: Path) -> bool:
    """Return whether an HTML file is an independently runnable WPT test.

    Sparse upstream caches do not include MANIFEST.json, so HTML applicability
    must be derived from the page itself. Crash/manual fixtures and helper
    documents are not testharness tests and must not become synthetic failures.
    """
    if "resources" in path.parts or "crashtests" in path.parts:
        return False
    source = path.read_text(encoding="utf-8", errors="replace").lower()
    return "testharness.js" in source


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
                name.endswith(".any.js")
                or name.endswith(".window.js")
                or (path.suffix in {".html", ".htm"} and is_testharness_html(path))
            ) and not any(token in name for token in ["-ref.", ".manual."]):
                relative = path.relative_to(checkout)
                candidate = (len(Path(root).parts), classification)
                if relative not in selected or candidate[0] > selected[relative][0]:
                    selected[relative] = candidate
    tests: list[tuple[Path, str, str, str]] = []
    for path, value in selected.items():
        source = checkout / path
        variants = (
            declared_html_variants(source.read_text(encoding="utf-8", errors="replace"))
            if source.suffix in {".html", ".htm"}
            else []
        ) or [""]
        tests.extend((path, value[1], "window", variant) for variant in variants)
    return sorted(tests, key=lambda item: (item[0].as_posix(), item[3]))


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
        variants = record["variants"]
        if variants == [""] and source.suffix in {".html", ".htm"}:
            variants = declared_html_variants(
                source.read_text(encoding="utf-8", errors="replace")
            ) or variants
        for global_name in record["globals"]:
            for variant in variants:
                tests.append((Path(record["path"]), record["workstream"], global_name, variant))
    return tests


def run_case(
    binary: Path,
    checkout: Path,
    port: int,
    test: Path,
    classification: str,
    global_name: str,
    variant: str,
    timeout: int,
    in_process_probe: bool,
    script_wall_ms: int | None,
) -> dict[str, object]:
    def bounded_diagnostic(value: str, limit: int = 20_000) -> str:
        value = value.strip()
        if len(value) <= limit:
            return value
        head = limit // 5
        return value[:head] + "\n...[diagnostic truncated]...\n" + value[-(limit - head) :]

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
        url = f"http://localhost:{port}/__oryn_worker_runner.html?{query}"
    elif test.name.endswith((".any.js", ".window.js")):
        query = urllib.parse.urlencode({"source": test.as_posix()})
        variant_suffix = ("&" + variant[1:]) if variant.startswith("?") else variant
        url = f"http://localhost:{port}/__oryn_any_runner.html?{query}{variant_suffix}"
    else:
        url = f"http://localhost:{port}/{urllib.parse.quote(test.as_posix())}{variant}"
    with tempfile.NamedTemporaryFile("w", suffix=".oil", delete=False) as oil:
        oil.write('wait idle\nwait exists "#oryn-wpt-result"\nextract text --selector "#oryn-wpt-result"\n')
        oil_path = Path(oil.name)
    trace_path = oil_path.with_suffix(".trace.json")
    try:
        native = ["native", "--allow-loopback"]
        if in_process_probe:
            native.append("--in-process-probe")
        if script_wall_ms is not None:
            native.extend(["--script-wall-ms", str(script_wall_ms)])
        try:
            for asset in fs_path_assets(checkout, checkout / test):
                native.extend(["--upload", f"{asset}={asset}"])
        except ValueError as error:
            return {
                "id": identifier,
                "path": test.as_posix(),
                "classification": classification,
                "global": global_name,
                "variant": variant,
                "status": "failed",
                "failure_domain": "harness",
                "diagnostics": [str(error)],
            }
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
                [
                    str(binary),
                    *native,
                    "--url",
                    url,
                    "--evaluate",
                    "typeof add_completion_callback+'|'+typeof test"
                    "+'|results='+JSON.stringify(globalThis.__oryn_wpt_results||[])"
                    "+'|messages='+JSON.stringify(globalThis.__oryn_message_log||[])"
                    "+'|driver='+JSON.stringify(globalThis.__oryn_testdriver_log||[])"
                    "+'|frames='+JSON.stringify([...document.querySelectorAll('iframe')].map(frame=>{try{return {srcdoc:frame.srcdoc,document:frame.contentDocument?.documentElement?.outerHTML||null,origin:frame.contentWindow?.origin||null}}catch(error){return {error:String(error)}}}))"
                    "+'|document='+document.documentElement.outerHTML",
                ],
                cwd=ROOT, text=True, capture_output=True, timeout=timeout,
            )
            detail = bounded_diagnostic(
                completed.stderr
                + "\n"
                + completed.stdout
                + "\n"
                + trace
                + "\nDOM_PROBE:\n"
                + probe.stdout
                + "\n"
                + probe.stderr
            )
            return {"id":identifier,"path":test.as_posix(),"classification":classification,"global":global_name,"variant":variant,"status":"failed","diagnostics":[detail or "missing WPT completion payload"]}
        failed = (
            not result.get("complete", False)
            or result.get("harness") not in (0, None)
            or not result.get("tests")
            or any(item.get("status") != 0 for item in result.get("tests", []))
        )
        failure_domain = None
        diagnostics = []
        if failed:
            if not result.get("complete", False):
                failure_domain = "incomplete"
            elif result.get("harness") not in (0, None):
                failure_domain = "harness"
            elif not result.get("tests"):
                failure_domain = "no_assertions"
            else:
                failure_domain = "assertion"
            if trace_path.exists():
                trace = trace_path.read_text(encoding="utf-8", errors="replace").strip()
                if trace:
                    diagnostics.append(trace[-20000:])
            process_detail = (completed.stderr + "\n" + completed.stdout).strip()
            if process_detail:
                diagnostics.append(bounded_diagnostic(process_detail))
        record = {
            "id": identifier,
            "path": test.as_posix(),
            "classification": classification,
            "global": global_name,
            "variant": variant,
            "status": "failed" if failed else "passed",
            "failure_domain": failure_domain,
            "result": result,
        }
        if diagnostics:
            record["diagnostics"] = diagnostics
        return record
    except subprocess.TimeoutExpired as error:
        diagnostics = [f"timeout after {timeout}s"]
        partial = "\n".join(
            value.decode(errors="replace") if isinstance(value, bytes) else value
            for value in (error.stderr, error.stdout)
            if value
        ).strip()
        if partial:
            diagnostics.append(bounded_diagnostic(partial))
        if trace_path.exists():
            trace = trace_path.read_text(encoding="utf-8", errors="replace").strip()
            if trace:
                diagnostics.append(trace[-20000:])
        return {
            "id": identifier,
            "path": test.as_posix(),
            "classification": classification,
            "global": global_name,
            "variant": variant,
            "status": "failed",
            "failure_domain": "timeout",
            "diagnostics": diagnostics,
        }
    finally:
        oil_path.unlink(missing_ok=True)
        trace_path.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=30)
    parser.add_argument(
        "--script-wall-ms",
        type=int,
        help="explicit probe-only per-script wall limit for long upstream cases",
    )
    parser.add_argument("--max-tests", type=int)
    parser.add_argument("--filter", help="substring filter applied to upstream test paths")
    parser.add_argument(
        "--exclude",
        action="append",
        default=[],
        help="repeatable substring exclusion applied after --filter",
    )
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
        "--jobs",
        type=int,
        default=4,
        help="maximum concurrent native test processes (default: 4)",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="resume only assertions whose complete runtime and selection fingerprint matches",
    )
    parser.add_argument(
        "--in-process-probe",
        action="store_true",
        help="explicit local diagnostic only; release evidence must use the worker boundary",
    )
    args = parser.parse_args()
    if args.jobs < 1 or args.jobs > 32:
        raise SystemExit("jobs must be between 1 and 32")
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
    source_patch = subprocess.check_output(
        ["git", "-C", str(ROOT), "diff", "--binary", "HEAD"],
    )
    Handler.root = args.checkout.resolve()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    if args.shard_count < 1 or not 0 <= args.shard_index < args.shard_count:
        raise SystemExit("invalid shard index/count")
    tests = inventoried_tests(args.checkout, args.inventory or EXPANDED_INVENTORY) if args.inventory else applicable_tests(args.checkout)
    if args.filter:
        tests = [item for item in tests if args.filter in item[0].as_posix()]
    if args.exclude:
        tests = [
            item
            for item in tests
            if not any(pattern in item[0].as_posix() for pattern in args.exclude)
        ]
    if args.global_filter:
        tests = [item for item in tests if item[2] == args.global_filter]
    tests = [item for index, item in enumerate(tests) if index % args.shard_count == args.shard_index]
    if args.max_tests is not None: tests = tests[: args.max_tests]
    inventory_sha256 = (
        sha256_bytes((args.inventory or EXPANDED_INVENTORY).read_bytes())
        if args.inventory
        else None
    )
    binary_fingerprint = {
        "sha256": sha256_bytes(binary.read_bytes()),
        "size_bytes": binary.stat().st_size,
    }
    selection = {
        "filter": args.filter,
        "exclude": args.exclude,
        "global": args.global_filter,
        "shard_index": args.shard_index,
        "shard_count": args.shard_count,
        "max_tests": args.max_tests,
        "timeout_seconds": args.timeout,
        "script_wall_ms": args.script_wall_ms,
    }
    fingerprint_input = {
        "upstream_revision": revision,
        "inventory_sha256": inventory_sha256,
        "execution_mode": "in_process_probe" if args.in_process_probe else "sandboxed_worker",
        "binary": binary_fingerprint,
        "runtime_metadata": runtime_metadata,
        "source_commit": source_commit,
        "source_patch_sha256": sha256_bytes(source_patch),
        "selection": selection,
    }
    run_fingerprint = sha256_bytes(
        json.dumps(fingerprint_input, sort_keys=True, separators=(",", ":")).encode()
    )
    expected_ids = {
        f"{test.as_posix()}|{global_name}|{variant}"
        for test, _classification, global_name, variant in tests
    }
    completed_by_id: dict[str, dict[str, object]] = {}
    if args.resume and args.output.is_file():
        prior = json.loads(args.output.read_text(encoding="utf-8"))
        if prior.get("run_fingerprint") != run_fingerprint:
            raise SystemExit("refusing to resume: WPT run fingerprint changed")
        completed_by_id = {
            str(item["id"]): item
            for item in prior.get("assertions", [])
            if item.get("id") in expected_ids and item.get("status") in {"passed", "failed"}
        }

    def publish(complete: bool) -> dict[str, Any]:
        assertions = [completed_by_id[key] for key in sorted(completed_by_id)]
        result: dict[str, Any] = {
            "schema_version": 2 if args.inventory else 1,
            "upstream_revision": revision,
            "inventory_sha256": inventory_sha256,
            "execution_mode": fingerprint_input["execution_mode"],
            "run_fingerprint": run_fingerprint,
            "binary": binary_fingerprint,
            "runtime_metadata": runtime_metadata,
            "source": {
                "commit": source_commit,
                "dirty": source_tree_dirty,
                "patch_sha256": fingerprint_input["source_patch_sha256"],
            },
            "selection": selection,
            "shard": {"index": args.shard_index, "count": args.shard_count},
            "jobs": args.jobs,
            "assertions": assertions,
            "outcome": outcome(assertions, len(tests), complete),
        }
        atomic_write_json(args.output, result)
        return result

    pending = [
        item
        for item in tests
        if f"{item[0].as_posix()}|{item[2]}|{item[3]}" not in completed_by_id
    ]
    publish(complete=False)
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as executor:
            futures = {
                executor.submit(
                    run_case,
                    binary,
                    args.checkout.resolve(),
                    server.server_port,
                    test,
                    classification,
                    global_name,
                    variant,
                    args.timeout,
                    args.in_process_probe,
                    args.script_wall_ms,
                ): f"{test.as_posix()}|{global_name}|{variant}"
                for test, classification, global_name, variant in pending
            }
            for future in concurrent.futures.as_completed(futures):
                assertion = future.result()
                completed_by_id[str(assertion["id"])] = assertion
                publish(complete=False)
    finally:
        server.shutdown(); thread.join()
    result = publish(complete=True)
    print(json.dumps(result["outcome"], sort_keys=True))
    return 0 if result["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

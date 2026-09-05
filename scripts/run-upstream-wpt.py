#!/usr/bin/env python3
"""Run pinned, applicable upstream WPT testharness cases through native Oryn."""

from __future__ import annotations

import argparse
import concurrent.futures
import collections
import hashlib
import html as html_module
import http.server
import http.client
from html.parser import HTMLParser
import json
import os
import re
import select
import ssl
import subprocess
import sys
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
let __oryn_wpt_result_generation=0;
function __oryn_publish_wpt(harness, complete) {
  var value = {complete:!!complete,harness:harness && typeof harness.status === 'number' ? harness.status : 0,harnessMessage:harness && harness.message ? String(harness.message) : '',harnessStack:harness && harness.stack ? String(harness.stack) : '',tests:globalThis.__oryn_wpt_results,testdriver:globalThis.__oryn_testdriver_log || []};
  var output = document.getElementById('oryn-wpt-result');
  if (!output) {
    output = document.createElement('pre');
    output.setAttribute('id', 'oryn-wpt-result');
    document.documentElement.appendChild(output);
  }
  output.textContent='__ORYN_WPT_DONE__'+JSON.stringify(value);
}
add_result_callback(function(test) {
  globalThis.__oryn_wpt_results.push({name:String(test.name || ''),status:Number(test.status),message:String(test.message || '')});
  // Keep partial assertions private. Publishing them as complete allows the
  // OIL observer to race a script which is still registering later tests.
  ++__oryn_wpt_result_generation;
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
  ++__oryn_wpt_result_generation;
});
add_completion_callback(function(tests, harness) { __oryn_publish_wpt(harness, true); });
"""

EDITING_SUBSET_BOOTSTRAP = r"""
// The upstream editing pages perform expensive setup for every browserTests
// entry before subsetTest discards assertions outside the selected variant.
// Preselect entries whose assertion intervals intersect the immutable query,
// while preserving the original global subset-test indices.
(function(){
  const match=/(?:^\?|&)(\d+)-(\d+|last)(?:&|$)/.exec(location.search);
  // Use the classic-script global binding directly. In a multi-context V8
  // realm, the WindowProxy property view can lag the script's global lexical
  // environment until the current parser task completes.
  if(!match||typeof browserTests==='undefined'||!Array.isArray(browserTests))return;
  const first=Number(match[1]),last=match[2]==='last'?Infinity:Number(match[2]);
  let cursor=0,firstSelected=0,found=false,selected=[];
  for(const entry of browserTests){
    const count=(entry[1]||[]).length+2+6*Object.keys(entry[4]||{}).length;
    const low=cursor+1,high=cursor+count;
    if(high>=first&&low<=last){if(!found){firstSelected=cursor;found=true}selected.push(entry)}
    cursor=high;
  }
  browserTests=selected;
  let current=firstSelected;
  globalThis.subsetTest=function(testFunc,...args){
    ++current;
    return current>=first&&current<=last?testFunc(...args):null;
  };
})();
"""


def inject_editing_subset_bootstrap(body: str, relative_path: str) -> str:
    """Install the editing prefilter at its data-dependent script boundary.

    Oryn intentionally schedules parser-blocking external scripts through the
    page scheduler. A separate injected inline script can therefore execute
    before the preceding external data script has published ``browserTests``.
    The upstream page's final inline script already depends on that data, so
    inject at the start of that same script rather than creating a competing
    parser task.
    """
    if not relative_path.startswith("editing/run/"):
        return body
    final_script = body.rfind("<script>")
    if final_script < 0:
        return body
    insertion = final_script + len("<script>")
    return body[:insertion] + EDITING_SUBSET_BOOTSTRAP + body[insertion:]


# Mirror the stable path aliases installed by `wpt serve`. Tests intentionally
# use the historical WebIDL parser URL while the pinned source tree stores the
# current webidl2 implementation at the rewritten path.
WPT_SERVER_REWRITES = {
    "/resources/WebIDLParser.js": "/resources/webidl2/lib/webidl2.js",
}


def wpt_server_path(path: str) -> str:
    return WPT_SERVER_REWRITES.get(path, path)


def meta_script_paths(root: Path, source_path: Path, source_text: str) -> list[str]:
    dependencies: list[str] = []
    for line in source_text.splitlines():
        if not line.startswith("// META: script="):
            continue
        declared = line.split("=", 1)[1].strip()
        served = wpt_server_path(declared) if declared.startswith("/") else declared
        dependency = (
            root / served.lstrip("/")
            if served.startswith("/")
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
    http_port: int,
    root: Path | None = None,
    source_path: Path | None = None,
    https_port: int | None = None,
    request_query: str | None = None,
    request_headers: collections.abc.Mapping[str, str] | None = None,
) -> str:
    """Apply the deterministic wptserve substitutions used by this harness."""
    remote_host = "localhost" if request_host == "127.0.0.1" else "127.0.0.1"
    https_port = http_port if https_port is None else https_port
    replacements = {
        "{{ports[http][0]}}": str(http_port),
        "{{ports[http][1]}}": str(http_port),
        "{{ports[https][0]}}": str(https_port),
        "{{ports[https][1]}}": str(https_port),
        "{{ports[ws][0]}}": str(http_port),
        "{{ports[wss][0]}}": str(https_port),
        "{{host}}": request_host,
        "{{domains[www1]}}": remote_host,
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
    if request_query is not None:
        query = urllib.parse.parse_qs(request_query, keep_blank_values=True)
        body = re.sub(
            r"\{\{GET\[([^\]]+)\]\}\}",
            lambda match: query.get(match.group(1), [""])[0],
            body,
        )
    if request_headers is not None:
        headers = {name.lower(): value for name, value in request_headers.items()}
        body = re.sub(
            r"\{\{header_or_default\(([^,]+),\s*([^)]*)\)\}\}",
            lambda match: headers.get(match.group(1).strip().lower(), match.group(2).strip()),
            body,
        )
    return body


def atomic_write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def strip_nested_helper_harness(path: str, content_type: str, body: bytes) -> bytes:
    """Remove independent harness messaging from a nested WPT request helper."""
    if (
        "text/html" not in content_type
        or urllib.parse.urlparse(path).path
        != "/common/security-features/scope/document.py"
    ):
        return body
    decoded = body.decode("utf-8", errors="replace")
    decoded = re.sub(
        r"<script\b[^>]*\bsrc\s*=\s*(['\"])(?:https?://[^'\"]+)?/resources/testharness(?:report)?\.js\1[^>]*>\s*</script>",
        "",
        decoded,
        flags=re.IGNORECASE,
    )
    return decoded.encode("utf-8")


def rewrite_wpt_location(value: str, request_is_tls: bool) -> str:
    """Map wptserve's private-proxy authority back to public loopback origins."""
    parsed = urllib.parse.urlparse(value)
    host = parsed.hostname or ""
    if host.startswith("www1.") and host.removeprefix("www1.") in {
        "localhost",
        "127.0.0.1",
    }:
        host = "127.0.0.1"
    scheme = "https" if request_is_tls and parsed.scheme == "http" else parsed.scheme
    if host == (parsed.hostname or "") and scheme == parsed.scheme:
        return value
    authority = host
    if parsed.port is not None:
        authority += f":{parsed.port}"
    return urllib.parse.urlunparse(
        (scheme, authority, parsed.path, parsed.params, parsed.query, parsed.fragment)
    )


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


def probe_wpt_result(stdout: str) -> dict[str, Any] | None:
    """Recover typed testharness assertions from the isolated DOM probe."""
    match = re.search(r"\|results=(\[.*?\])\|messages=", stdout, re.DOTALL)
    if not match:
        return None
    try:
        tests = json.loads(match.group(1))
    except json.JSONDecodeError:
        return None
    if not isinstance(tests, list) or not tests:
        return None
    return {
        "complete": True,
        "harness": 0,
        "harnessMessage": "recovered from isolated DOM probe",
        "harnessStack": "",
        "tests": tests,
        "testdriver": [],
    }


class Handler(http.server.SimpleHTTPRequestHandler):
    root: Path
    official_wpt_port: int | None = None
    public_http_port: int | None = None
    public_https_port: int | None = None
    dispatcher_condition = threading.Condition()
    dispatcher_queues: dict[str, collections.deque[bytes]] = {}

    def translate_path(self, path: str) -> str:
        relative = urllib.parse.unquote(urllib.parse.urlparse(path).path).lstrip("/")
        candidate = (self.root / relative).resolve()
        if self.root not in candidate.parents and candidate != self.root:
            return str(self.root / "__denied__")
        return str(candidate)

    @staticmethod
    def _sidecar_headers(path: Path) -> list[tuple[str, str]]:
        """Retain adjacent WPT response headers for instrumented files."""
        sidecar = Path(str(path) + ".headers")
        if not sidecar.is_file():
            return []
        headers: list[tuple[str, str]] = []
        for raw_line in sidecar.read_text(encoding="utf-8", errors="replace").splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#") or ":" not in line:
                continue
            name, value = line.split(":", 1)
            name = name.strip()
            if name.lower() in {"content-length", "transfer-encoding", "connection"}:
                continue
            headers.append((name, value.strip()))
        return headers

    def _send_static_headers(self, path: Path, content_type: str, length: int) -> None:
        self.send_header("content-type", content_type)
        for name, value in self._sidecar_headers(path):
            if name.lower() != "content-type":
                self.send_header(name, value)
        self.send_header("content-length", str(length))

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
            self._proxy_to_wptserve()
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
        rewritten = wpt_server_path(parsed.path)
        if rewritten != parsed.path:
            self.path = urllib.parse.urlunparse(
                (parsed.scheme, parsed.netloc, rewritten, parsed.params, parsed.query, parsed.fragment)
            )
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
window.test_driver_internal.get_computed_label = element => Promise.resolve(__oryn_computed_accessibility(element).name);
window.test_driver_internal.get_computed_role = element => Promise.resolve(__oryn_computed_accessibility(element).role);
const __oryn_accessibility_nodes = new Map();
const __oryn_accessibility_ids = new WeakMap();
let __oryn_next_accessibility_id = 1;
const __oryn_accessibility_id = element => {
  let id = __oryn_accessibility_ids.get(element);
  if (!id) { id = `oryn-accessibility-${__oryn_next_accessibility_id++}`; __oryn_accessibility_ids.set(element, id); }
  return id;
};
const __oryn_accessibility_properties = (element, parent = null) => {
  const computed = __oryn_computed_accessibility(element);
  const accessibilityId = __oryn_accessibility_id(element);
  const properties = {
    accessibilityId,
    parent,
    role: computed.role,
    label: computed.name,
    children: [],
  };
  const role = String(computed.role || '');
  if (role === 'checkbox' || role === 'radio' || role === 'switch') {
    properties.checked = element.getAttribute('aria-checked') ?? (element.checked ? 'true' : 'false');
  }
  if (element.hasAttribute('aria-pressed')) properties.pressed = element.getAttribute('aria-pressed');
  for (const child of [...element.children]) {
    const childProperties = __oryn_accessibility_properties(child, accessibilityId);
    properties.children.push(childProperties.accessibilityId);
  }
  __oryn_accessibility_nodes.set(accessibilityId, properties);
  return properties;
};
window.test_driver_internal.get_accessibility_properties_for_element = element => Promise.resolve(__oryn_accessibility_properties(element));
window.test_driver_internal.get_accessibility_properties_for_accessibility_node = id => Promise.resolve(__oryn_accessibility_nodes.get(String(id)));
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
                body,
                request_host,
                self.public_http_port or self.server.server_address[1],
                self.root,
                source,
                self.public_https_port,
            )
            encoded = body.encode()
            self.send_response(200)
            self._send_static_headers(
                source, "application/javascript; charset=utf-8", len(encoded)
            )
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
                + "<body>"
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
                "<body>"
                "<script>"
                "const publish=value=>{const output=document.createElement('pre');"
                "output.id='oryn-wpt-result';"
                "output.textContent='__ORYN_WPT_DONE__'+JSON.stringify(value);"
                "document.body.appendChild(output)};"
                f"const worker={constructor};"
                "if(worker.start)worker.start();"
                "worker.onmessage=event=>{if(event.data&&event.data.__oryn_wpt)publish(event.data.value)};"
                "worker.onerror=event=>publish({complete:true,harness:1,tests:[{name:'worker bootstrap',status:1,message:String(event.message||'worker error')}]});"
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
                self.public_http_port or self.server.server_address[1],
                self.root,
                source_path,
                self.public_https_port,
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
                # The upstream `.any.js` worker wrapper calls `done()` after
                # importing the test file. WorkerTestEnvironment has no load
                # event and deliberately waits for this explicit end-of-file
                # signal, so concatenating the sources without it leaves even
                # fully synchronous tests pending forever.
                + "\n;done();\n"
            ).encode()
            self.send_response(200); self.send_header("content-type", "text/javascript; charset=utf-8")
            self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
            return
        path = Path(self.translate_path(self.path))
        if path.name.endswith(".sub.js") and path.is_file():
            request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
            body = substitute_wpt_tokens(
                path.read_text(encoding="utf-8", errors="replace"), request_host,
                self.public_http_port or self.server.server_address[1], self.root, path,
                self.public_https_port,
            ).encode()
            self.send_response(200)
            self._send_static_headers(
                path, "text/javascript; charset=utf-8", len(body)
            )
            self.end_headers()
            self.wfile.write(body)
            return
        if path.suffix in {".html", ".htm"} and path.is_file():
            body = path.read_text(encoding="utf-8", errors="replace")
            if path.name.endswith((".sub.html", ".sub.htm")):
                request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
                body = substitute_wpt_tokens(
                    body,
                    request_host,
                    self.public_http_port or self.server.server_address[1],
                    self.root,
                    path,
                    self.public_https_port,
                    parsed.query,
                    self.headers,
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
                "<script>setup({timeout_multiplier:4});</script>"
                if (
                    ("name=timeout" in lowered or 'name="timeout"' in lowered or "name='timeout'" in lowered)
                    and ("content=long" in lowered or 'content="long"' in lowered or "content='long'" in lowered)
                )
                else ""
            )
            if harness >= 0:
                injection = timeout_setup + COMPLETION
                body = body[:index] + injection + body[index:]
            relative_path = path.relative_to(self.root).as_posix()
            body = inject_editing_subset_bootstrap(body, relative_path)
            encoded = body.encode()
            self.send_response(200)
            self._send_static_headers(path, "text/html; charset=utf-8", len(encoded))
            self.end_headers(); self.wfile.write(encoded)
            return
        self._proxy_to_wptserve()

    def do_HEAD(self) -> None:  # noqa: N802
        self._proxy_to_wptserve(include_body=False)

    def do_OPTIONS(self) -> None:  # noqa: N802
        self._proxy_to_wptserve()

    def do_PUT(self) -> None:  # noqa: N802
        self._proxy_to_wptserve()

    def do_DELETE(self) -> None:  # noqa: N802
        self._proxy_to_wptserve()

    def _proxy_to_wptserve(self, *, include_body: bool = True) -> None:
        """Delegate WPT response semantics to the pinned upstream server.

        The public loopback server remains the browser-visible origin so Oryn's
        controlled policy and result wrappers stay unchanged.  The private
        upstream server supplies dynamic Python handlers, `.headers` sidecars,
        substitutions, and other WPT response behavior that SimpleHTTPServer
        cannot reproduce faithfully.
        """
        if self.official_wpt_port is None:
            self.send_error(503, "official wptserve is unavailable")
            return
        length = int(self.headers.get("content-length", "0"))
        if length > 24 * 1024 * 1024:
            self.send_error(413)
            return
        body = self.rfile.read(length) if length else None
        headers = {
            name: value
            for name, value in self.headers.items()
            if name.lower() not in {"content-length", "transfer-encoding"}
        }
        # Preserve the browser-visible authority for wptserve substitutions and
        # handler logic; only the TCP destination is private.
        headers["Host"] = self.headers.get("Host", f"localhost:{self.server.server_address[1]}")
        connection = http.client.HTTPConnection("127.0.0.1", self.official_wpt_port, timeout=30)
        response_started = False
        try:
            connection.request(self.command, self.path, body=body, headers=headers)
            response = connection.getresponse()
            response_body = response.read()
            content_type = response.getheader("content-type", "").lower()
            if os.environ.get("ORYN_WPT_DEBUG_REQUESTS"):
                sys.stderr.write(
                    f"WPT_RESPONSE {response.status} {content_type} {len(response_body)} "
                    f"{self.path}\n"
                )
                if "/common/security-features/subresource/xhr.py" in self.path:
                    sys.stderr.write(f"WPT_RESPONSE_BODY {response_body[:240]!r}\n")
                    sys.stderr.write(f"WPT_RESPONSE_HEADERS {response.getheaders()!r}\n")
            if urllib.parse.urlparse(self.path).path == "/common/security-features/scope/document.py":
                # This endpoint is a nested request helper, not an independent
                # test. testharness itself posts an infrastructure `type:
                # complete` record to the parent when the helper is slower
                # than the harness timeout. That record is
                # indistinguishable from the request result to upstream's
                # intentionally generic message listener. `wpt serve` does
                # not treat the helper as a separately reported test, so keep
                # the local proxy consistent with the static HTML path above.
                response_body = strip_nested_helper_harness(
                    self.path, content_type, response_body
                )
            if b"{{" in response_body and any(
                value in content_type
                for value in ("text/", "javascript", "json", "xml", "css", "html")
            ):
                request_host = self.headers.get("host", "127.0.0.1").split(":", 1)[0]
                source_path = self.root / urllib.parse.unquote(
                    self.path.split("?", 1)[0]
                ).lstrip("/")
                response_body = substitute_wpt_tokens(
                    response_body.decode("utf-8"),
                    request_host,
                    self.public_http_port or self.server.server_address[1],
                    self.root,
                    source_path,
                    self.public_https_port,
                    urllib.parse.urlparse(self.path).query,
                    self.headers,
                ).encode("utf-8")
            self.send_response(response.status, response.reason)
            response_started = True
            for name, value in response.getheaders():
                if name.lower() in {"connection", "content-length", "transfer-encoding"}:
                    continue
                if name.lower() == "location":
                    value = rewrite_wpt_location(
                        value, isinstance(self.connection, ssl.SSLSocket)
                    )
                self.send_header(name, value)
            self.send_header("content-length", str(len(response_body)))
            self.end_headers()
            if include_body and self.command != "HEAD":
                self.wfile.write(response_body)
        except (OSError, http.client.HTTPException) as error:
            # Aborted navigations and timed-out tests routinely close their
            # loopback connection while the upstream response is in flight.
            # Once headers have started, attempting to append an HTTP 502
            # corrupts the response and produces a second BrokenPipeError.
            if not response_started:
                self.send_error(502, f"wptserve proxy failure: {error}")
        finally:
            connection.close()

    def _send_bytes(self, body: bytes, content_type: str, *, status: int = 200) -> None:
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, message_format: str, *args: object) -> None:
        if os.environ.get("ORYN_WPT_DEBUG_REQUESTS"):
            sys.stderr.write("WPT_REQUEST " + (message_format % args) + "\n")


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
    _partitioned: bool = False,
) -> dict[str, object]:
    def bounded_diagnostic(value: str, limit: int = 20_000) -> str:
        value = value.strip()
        if len(value) <= limit:
            return value
        head = limit // 5
        return value[:head] + "\n...[diagnostic truncated]...\n" + value[-(limit - head) :]

    identifier = f"{test.as_posix()}|{global_name}|{variant}"
    source_text = (checkout / test).read_text(encoding="utf-8", errors="replace")
    lowered_source = source_text.lower()
    declared_long_timeout = (
        "// meta: timeout=long" in lowered_source
        or (
            (
                "name=timeout" in lowered_source
                or 'name="timeout"' in lowered_source
                or "name='timeout'" in lowered_source
            )
            and (
                "content=long" in lowered_source
                or 'content="long"' in lowered_source
                or "content='long'" in lowered_source
            )
        )
    )
    # WPT's `long` classification extends the harness deadline. The native
    # process deadline must cover that same interval or a valid, progressing
    # long test is misclassified as an engine hang by the outer runner.
    process_timeout = timeout * 4 if declared_long_timeout else timeout
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
    # The upstream editing corpus declares thousand-test query variants. A
    # single monolithic V8 task prevents scheduler checkpoints and can exceed
    # the per-script safety wall even though every individual assertion is
    # bounded. Execute those immutable numeric ranges as fresh 100-test
    # subranges, then fold their official testharness results back into the
    # original inventory cell. This changes neither source nor assertions and
    # records the partitioning explicitly in the result transport.
    large_variant = re.fullmatch(r"\?(\d+)-(\d+|last)", variant)
    if (
        not _partitioned
        and large_variant
        and test.as_posix().startswith("editing/run/")
    ):
        start = int(large_variant.group(1))
        end = (
            int(large_variant.group(2))
            if large_variant.group(2) != "last"
            else start + 999
        )
        if end - start + 1 > 100:
            parts = []
            for part_start in range(start, end + 1, 100):
                part_end = min(part_start + 99, end)
                part = run_case(
                        binary,
                        checkout,
                        port,
                        test,
                        classification,
                        global_name,
                        f"?{part_start}-{part_end}",
                        timeout,
                        in_process_probe,
                        script_wall_ms,
                        True,
                    )
                if (
                    large_variant.group(2) == "last"
                    and not (part.get("result") or {}).get("tests")
                ):
                    break
                parts.append(part)
            tests = [
                assertion
                for part in parts
                for assertion in (part.get("result") or {}).get("tests", [])
            ]
            passed = bool(tests) and all(part.get("status") == "passed" for part in parts)
            result = {
                "complete": all(
                    (part.get("result") or {}).get("complete", False) for part in parts
                ),
                "harness": 0,
                "harnessMessage": "",
                "harnessStack": "",
                "tests": tests,
                "testdriver": [],
            }
            record: dict[str, object] = {
                "id": identifier,
                "path": test.as_posix(),
                "classification": classification,
                "global": global_name,
                "variant": variant,
                "status": "passed" if passed else "failed",
                "failure_domain": None
                if passed
                else next(
                    (
                        part.get("failure_domain", "partition")
                        for part in parts
                        if part.get("status") != "passed"
                    ),
                    "partition",
                ),
                "result": result,
                "result_transport": "partitioned_testharness_range_v1",
                "partitions": [part["variant"] for part in parts],
            }
            diagnostics = [
                f"{part['variant']}: {diagnostic}"
                for part in parts
                if part.get("status") != "passed"
                for diagnostic in part.get("diagnostics", [])
            ]
            if diagnostics:
                record["diagnostics"] = diagnostics
            return record
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
        oil_path = Path(oil.name)
        oil.write('wait exists "#oryn-wpt-result"\n')
        oil.write('extract text --selector "#oryn-wpt-result"\n')
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
            [
                str(binary),
                "--file",
                str(oil_path),
                *native,
                "--url",
                url,
                "--trace-out",
                str(trace_path),
            ],
            cwd=ROOT, text=True, capture_output=True, timeout=process_timeout,
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
                cwd=ROOT, text=True, capture_output=True, timeout=process_timeout,
            )
            recovered = probe_wpt_result(probe.stdout)
            if recovered is not None:
                failed = any(item.get("status") != 0 for item in recovered["tests"])
                return {
                    "id": identifier,
                    "path": test.as_posix(),
                    "classification": classification,
                    "global": global_name,
                    "variant": variant,
                    "status": "failed" if failed else "passed",
                    "failure_domain": "assertion" if failed else None,
                    "result": recovered,
                    "result_transport": "isolated_dom_probe",
                }
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
        diagnostics = [f"timeout after {process_timeout}s"]
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
        "--variant-filter",
        help="exact WPT variant query to execute after source variant expansion",
    )
    parser.add_argument(
        "--variant-override",
        help="diagnostic query override; deduplicates the selected path/global pairs",
    )
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
    parser.add_argument(
        "--rerun-timeouts-from",
        type=Path,
        help="restrict execution to timeout IDs in a prior compatible result",
    )
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
    secure_server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    Handler.public_http_port = server.server_port
    Handler.public_https_port = secure_server.server_port
    certificate_root = args.checkout / "tools/certs"
    tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls_context.load_cert_chain(
        certificate_root / "web-platform.test.pem",
        certificate_root / "web-platform.test.key",
    )
    secure_server.socket = tls_context.wrap_socket(secure_server.socket, server_side=True)
    os.environ["ORYN_WPT_CA_CERT"] = str(certificate_root / "cacert.pem")
    os.environ["ORYN_WPT_ALLOW_INVALID_CERTS"] = "1"
    helper = ROOT / "scripts/wptserve-helper.py"
    venv_python = args.checkout / "_venv3/bin/python"
    helper_python = venv_python if venv_python.is_file() else Path(sys.executable)
    helper_errors = tempfile.NamedTemporaryFile("w+", suffix=".wptserve.log")
    official = subprocess.Popen(
        [
            str(helper_python),
            str(helper),
            "--checkout",
            str(args.checkout.resolve()),
            "--public-port",
            str(server.server_port),
            "--public-https-port",
            str(secure_server.server_port),
        ],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=helper_errors,
    )
    ready, _, _ = select.select([official.stdout], [], [], 20) if official.stdout else ([], [], [])
    if not ready:
        official.terminate()
        official.wait(timeout=5)
        helper_errors.seek(0)
        raise SystemExit("official wptserve did not start: " + helper_errors.read().strip())
    startup_line = official.stdout.readline() if official.stdout else ""
    try:
        Handler.official_wpt_port = int(json.loads(startup_line)["port"])
    except (json.JSONDecodeError, KeyError, TypeError, ValueError) as error:
        official.terminate()
        official.wait(timeout=5)
        helper_errors.seek(0)
        raise SystemExit(
            "official wptserve returned invalid startup metadata: "
            + startup_line.strip()
            + " "
            + helper_errors.read().strip()
        ) from error
    thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
    secure_thread = threading.Thread(target=secure_server.serve_forever, daemon=True)
    secure_thread.start()
    if args.shard_count < 1 or not 0 <= args.shard_index < args.shard_count:
        raise SystemExit("invalid shard index/count")
    tests = inventoried_tests(args.checkout, args.inventory or EXPANDED_INVENTORY) if args.inventory else applicable_tests(args.checkout)
    if args.filter:
        tests = [item for item in tests if args.filter in item[0].as_posix()]
    if args.variant_filter is not None:
        tests = [item for item in tests if item[3] == args.variant_filter]
    if args.variant_override is not None:
        selected = {}
        for path, classification, global_name, _variant in tests:
            selected.setdefault(
                (path, global_name),
                (path, classification, global_name, args.variant_override),
            )
        tests = list(selected.values())
    if args.exclude:
        tests = [
            item
            for item in tests
            if not any(pattern in item[0].as_posix() for pattern in args.exclude)
        ]
    if args.global_filter:
        tests = [item for item in tests if item[2] == args.global_filter]
    timeout_source_sha256 = None
    if args.rerun_timeouts_from:
        timeout_source = json.loads(args.rerun_timeouts_from.read_text(encoding="utf-8"))
        timeout_ids = {
            str(item["id"])
            for item in timeout_source.get("assertions", [])
            if item.get("failure_domain") == "timeout"
        }
        tests = [
            item
            for item in tests
            if f"{item[0].as_posix()}|{item[2]}|{item[3]}" in timeout_ids
        ]
        timeout_source_sha256 = sha256_bytes(args.rerun_timeouts_from.read_bytes())
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
        "variant_filter": args.variant_filter,
        "variant_override": args.variant_override,
        "exclude": args.exclude,
        "global": args.global_filter,
        "shard_index": args.shard_index,
        "shard_count": args.shard_count,
        "max_tests": args.max_tests,
        "timeout_seconds": args.timeout,
        "script_wall_ms": args.script_wall_ms,
        "timeout_source_sha256": timeout_source_sha256,
    }
    fingerprint_input = {
        "upstream_revision": revision,
        "inventory_sha256": inventory_sha256,
        "harness": {
            "server": "pinned_wptserve_proxy_v1",
            "helper_sha256": sha256_bytes(helper.read_bytes()),
        },
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
            "harness": fingerprint_input["harness"],
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
        secure_server.shutdown(); secure_thread.join()
        official.terminate()
        try:
            official.wait(timeout=5)
        except subprocess.TimeoutExpired:
            official.kill()
            official.wait(timeout=5)
        helper_errors.close()
    result = publish(complete=True)
    print(json.dumps(result["outcome"], sort_keys=True))
    return 0 if result["outcome"]["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())

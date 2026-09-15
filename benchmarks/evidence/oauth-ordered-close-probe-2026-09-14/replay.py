"""Diagnostic replay: run all five cases, preserving failures and exact inputs."""
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
ORIGIN = "http://127.0.0.1:3104"
ENV = dict(os.environ, ORYN_PAGE_WORKER=str(ROOT / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"))
CASES = {
    "popup-pkce": ("rp/popup", "Authenticated with PKCE"),
    "redirect": ("rp/redirect", "Redirect authenticated"),
    "mfa-handoff": ("rp/mfa", "MFA authenticated"),
    "popup-close-recovery": ("rp/recovery", "Popup closed; opener recovered"),
    "existing-session-sso": ("idp/session", "SSO authenticated"),
}

def run(item):
    name, sample = item
    route, expected = CASES[name]
    directory = OUT / f"{name}-{sample}"
    directory.mkdir()
    task = (ROOT / f"benchmarks/app-compatibility/oauth/{name}.oil").read_text().replace("http://127.0.0.1:3000", ORIGIN)
    (directory / "task.oil").write_text(task)
    command = [str(ROOT / "target/release/oryn"), "--file", str(directory / "task.oil"), "native", "--url", f"{ORIGIN}/app-compatibility/auth/{route}", "--allow-loopback"]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV, start_new_session=True)
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=60)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate(timeout=10)
    (directory / "stdout.jsonl").write_bytes(stdout)
    (directory / "stderr.txt").write_bytes(stderr)
    errors = []
    try:
        frames = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        actions = [f["result"] for f in frames if f.get("kind") == "action"]
        count = sum(line.split()[0] in ("click", "type", "select") for line in task.splitlines() if line.strip())
        if len(actions) != count or any(a.get("execution_domain") != "native" for a in actions):
            errors.append("action count/domain mismatch")
        if [f.get("value") for f in frames if f.get("kind") == "structured"] != [expected]:
            errors.append("outcome mismatch")
        traces = [f for f in frames if f.get("kind") == "trace"]
        if len(traces) != 1:
            errors.append("missing/duplicate trace")
        for trace in traces:
            for event in trace.get("events", []):
                kind = event.get("kind", {})
                if kind.get("kind") == "script_diagnostic" or (kind.get("kind") == "console" and kind.get("level") == "error"):
                    errors.append("script/console error")
    except (ValueError, KeyError, TypeError) as error:
        errors.append(str(error))
    if process.returncode or timed_out or stderr.strip():
        errors.append("process failure, timeout, or stderr")
    record = dict(case=name, sample=sample, passed=not errors, errors=errors, returncode=process.returncode, timed_out=timed_out, elapsed_seconds=time.monotonic()-started)
    (directory / "result.json").write_text(json.dumps(record, indent=2)+"\n")
    print(json.dumps(record), flush=True)
    return record

if __name__ == "__main__":
    with ThreadPoolExecutor(max_workers=3) as executor:
        records = list(executor.map(run, [(name, sample) for sample in range(1, 4) for name in CASES]))
    report = dict(scope="dirty-tree diagnostic, controlled auth mechanics only", family_qualified=False, records=records, passed=sum(r["passed"] for r in records), total=len(records))
    (OUT / "summary.json").write_text(json.dumps(report, indent=2)+"\n")
    raise SystemExit(0 if all(r["passed"] for r in records) else 1)

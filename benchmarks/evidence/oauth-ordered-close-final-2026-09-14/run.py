"""Repeat the locked canonical auth validator; fixture lifetime is owned by the caller."""
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
PARENT = ROOT / "target/release/oryn"
WORKER = ROOT / "target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker"
ENV = dict(os.environ, ORYN_PAGE_WORKER=str(WORKER))

def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def run(sample):
    destination = OUT / f"sample-{sample}"
    destination.mkdir()
    env = dict(ENV, ORYN_BIN=str(PARENT), ORYN_APP_COMPATIBILITY_DOMAIN="native",
               ORYN_APP_COMPATIBILITY_EXTERNAL_SERVER="1", ORYN_APP_COMPATIBILITY_PORT="3104",
               ORYN_APP_COMPATIBILITY_ARTIFACT_DIR=str(destination))
    started = time.monotonic()
    process = subprocess.Popen([str(ROOT / "scripts/validate-auth-flows.sh")], env=env,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    timed_out = False
    try:
        stdout, stderr = process.communicate(timeout=90)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(process.pid, signal.SIGKILL)
        stdout, stderr = process.communicate(timeout=10)
    (destination / "validator.stdout").write_bytes(stdout)
    (destination / "validator.stderr").write_bytes(stderr)
    count = stdout.count(b"application compatibility OAuth passed:")
    record = dict(sample=sample, cases_passed=count, planned=5, returncode=process.returncode,
                  timed_out=timed_out, elapsed_seconds=time.monotonic()-started,
                  passed=process.returncode == 0 and count == 5 and not timed_out and not stderr.strip())
    (destination / "result.json").write_text(json.dumps(record, indent=2)+"\n")
    print(json.dumps(record), flush=True)
    return record

if __name__ == "__main__":
    source = subprocess.check_output([str(ROOT / "scripts/source-tree-hash.sh")], text=True).strip()
    manifest = json.loads((WORKER.parent.parent / "Resources/worker-manifest.json").read_text())
    runtime = json.loads(subprocess.check_output([str(PARENT), "native", "--runtime-info"], env=ENV))
    parent_hash, worker_hash = digest(PARENT), digest(WORKER)
    assert source == manifest["source_tree_sha256"]
    assert runtime["sandboxed"] and runtime["worker_binary_sha256"] == worker_hash
    started = time.monotonic()
    with ThreadPoolExecutor(max_workers=3) as executor:
        records = list(executor.map(run, range(1, 4)))
    identities_unchanged = parent_hash == digest(PARENT) and worker_hash == digest(WORKER)
    report = dict(schema_version=1, classification="dirty-tree signed diagnostic; controlled auth mechanics only",
                  family_qualified=False, goal_complete=False, source_tree_sha256=source,
                  source_matches_signed_build=True, parent_sha256=parent_hash, worker_sha256=worker_hash,
                  worker_manifest=manifest, runtime=runtime, artifact_identities_unchanged=identities_unchanged,
                  fixture_lock_sha256=digest(ROOT / "benchmarks/app-compatibility/fixtures.lock.json"),
                  fixture_source_sha256=digest(ROOT / "test-harness/scenarios/auth-flows/api.js"),
                  fixture_ownership="caller-owned loopback server; cleanup recorded separately",
                  jobs=3, samples=3, planned=15, passed=sum(r["cases_passed"] for r in records),
                  records=records, elapsed_seconds=time.monotonic()-started)
    (OUT / "summary.json").write_text(json.dumps(report, indent=2)+"\n")
    raise SystemExit(0 if identities_unchanged and all(r["passed"] for r in records) else 1)

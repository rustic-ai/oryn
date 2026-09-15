"""Diagnostic only: run every existing OIL fixture and preserve all failures."""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

repo = Path('/Users/rohit/Work/oryn2/oryn-roadmap-next')
out = Path(tempfile.mkdtemp(prefix='oryn-controlled-all-', dir='/private/tmp'))
binary = repo / 'target/release/oryn'
worker = repo / 'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
env = dict(os.environ, ORYN_PAGE_WORKER=str(worker))
records = []
for task in sorted((repo / 'test-harness/scripts').glob('*.oil')):
    started = time.monotonic()
    proc = subprocess.Popen([str(binary), '--file', str(task), 'native', '--allow-loopback'],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            env=env, start_new_session=True)
    timed_out = False
    try:
        stdout, stderr = proc.communicate(timeout=90)
    except subprocess.TimeoutExpired:
        timed_out = True
        os.killpg(proc.pid, signal.SIGKILL)
        stdout, stderr = proc.communicate()
    (out / (task.stem + '.jsonl')).write_bytes(stdout)
    (out / (task.stem + '.stderr')).write_bytes(stderr)
    try:
        values = [json.loads(line) for line in stdout.splitlines() if line.strip()]
        valid = bool(values) and any(v.get('kind') == 'observation' for v in values)
        supported = all(v.get('kind') != 'unsupported' for v in values)
    except (ValueError, AttributeError):
        valid = supported = False
    record = dict(task=task.name, task_sha256=hashlib.sha256(task.read_bytes()).hexdigest(),
                  returncode=proc.returncode, timed_out=timed_out,
                  elapsed_seconds=round(time.monotonic()-started, 3),
                  smoke_pass=proc.returncode == 0 and valid and supported and not timed_out,
                  task_completion='not independently verified',
                  error=stderr.decode(errors='replace')[-3000:])
    records.append(record)
    print(json.dumps(record), flush=True)
report = dict(qualification='diagnostic smoke, not end-state or release qualification',
              parent_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
              worker_sha256=hashlib.sha256(worker.read_bytes()).hexdigest(), records=records)
(out / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
print('ARTIFACTS=' + str(out), flush=True)

#!/usr/bin/env python3
"""Run every prepared tier shard and continuously summarize diagnostic evidence."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
from collections import Counter
import json
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def publish_cohort_ledger(output, coverage, data, source):
    spec = importlib.util.spec_from_file_location('wpt_qualification', ROOT / 'scripts/qualify-wpt-evidence.py')
    qualification = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(qualification)
    rows = {row['id']: row for row in data.get('assertions', [])}
    counts, groups, tiers = Counter(), {}, {}
    temporary = output / 'ledger.tmp'
    with coverage.open() as entries, temporary.open('w') as destination:
        for line in entries:
            entry = json.loads(line)
            raw = rows.get(entry.get('runner_id'))
            if raw:
                state, reasons = qualification.classify(raw, current=data.get('schema_version', 0) >= 4)
                entry.update(status=raw['status'], qualification=state,
                    qualification_reasons=reasons, failure_domain=raw.get('failure_domain'),
                    diagnostic_tags=raw.get('diagnostic_tags', []), evidence=source)
            else:
                entry['qualification'] = 'unsupported' if entry['status'] == 'unsupported' else 'not_run'
            counts[entry['qualification']] += 1
            groups.setdefault(entry['root'], Counter())[entry['qualification']] += 1
            tiers.setdefault(entry['tier'], Counter())[entry['qualification']] += 1
            destination.write(json.dumps(entry, sort_keys=True) + '\n')
    ledger = output / 'ledger.jsonl'
    temporary.replace(ledger)
    return {'ledger_sha256': hashlib.sha256(ledger.read_bytes()).hexdigest(),
            'qualification_counts': dict(counts), 'by_root': groups, 'by_tier': tiers,
            'qualification_classifier_sha256': hashlib.sha256((ROOT / 'scripts/qualify-wpt-evidence.py').read_bytes()).hexdigest(),
            'qualification_helpers_sha256': hashlib.sha256((ROOT / 'scripts/wpt_evidence.py').read_bytes()).hexdigest()}


def write_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")
    temporary.replace(path)


def cohort_inventory(plan, shards):
    """Flatten exact executions, without merging variant/global cross products."""
    records, ids = [], set()
    for shard in shards:
        inventory = json.loads(Path(shard['inventory']).read_text())
        if inventory['upstream_revision'] != plan['upstream_revision']:
            raise ValueError('shard revision mismatch')
        count = 0
        for record in inventory['tests']:
            for global_name in record['globals']:
                for variant in record['variants']:
                    identifier = f"{record['path']}|{global_name}|{variant}"
                    if identifier in ids:
                        raise ValueError('duplicate cohort execution: ' + identifier)
                    ids.add(identifier)
                    records.append(dict(record, globals=[global_name], variants=[variant]))
                    count += 1
        if count != shard['executions']:
            raise ValueError('shard execution count mismatch')
    return {'schema_version': 1, 'upstream_revision': plan['upstream_revision'],
            'required_execution_total': len(ids), 'tests': records}, ids


def parent_integrity_errors(rows):
    errors = set()
    for row in rows:
        if row.get('fixture_error') or row.get('fixture', {}).get('error'):
            errors.add('fixture_integrity_failure')
        execution = row.get('execution', {})
        if execution.get('transport') != 'persistent_session':
            continue  # Preflight-blocked cases never enter the parent.
        shutdown = execution.get('parent_shutdown') or {}
        if (shutdown.get('returncode') != 0 or shutdown.get('error')
                or shutdown.get('shutdown_acknowledged') is not True):
            errors.add('persistent_parent_shutdown_unverified')
        if 'pool capacity' in str(execution.get('error', '')).lower():
            errors.add('primary_worker_pool_exhausted')
    return sorted(errors)


def run_cohort(args, plan, shards):
    inventory, expected = cohort_inventory(plan, shards)
    inventory_path = args.output / 'cohort-inventory.json'
    result_path = args.output / 'cohort.json'
    write_json(inventory_path, inventory)
    started = time.monotonic()
    status = {'complete': False, 'manifest_tests': plan['total_manifest_tests'],
              'runnable': plan['runnable'], 'unsupported': plan['unsupported'],
              'selected_executions': len(expected), 'selected_tier': args.tier,
              'official_manifest_sha256': plan['official_manifest_sha256'],
              'policy_sha256': plan['policy_sha256'], 'cohort': 'single persistent parent; fresh direct fixture per execution',
              'qualification': 'diagnostic; not qualified conformance or release attestation'}
    coverage = args.plan.parent / 'coverage.jsonl'
    if coverage.is_file():
        status['manifest_coverage'] = {'path': str(coverage.resolve()),
            'sha256': hashlib.sha256(coverage.read_bytes()).hexdigest()}
    status['plan_sha256'] = hashlib.sha256(args.plan.read_bytes()).hexdigest()
    write_json(args.output / 'summary.json', status)
    command = [sys.executable, str(ROOT / 'scripts/run-upstream-wpt.py'),
               '--checkout', str(args.checkout.resolve()), '--binary', str(args.binary.resolve()),
               '--inventory', str(inventory_path.resolve()), '--output', str(result_path.resolve()),
               '--jobs', str(args.jobs), '--harness-profile', 'upstream',
               '--parent-mode', 'persistent', '--resume']
    with (args.output / 'cohort.log').open('a') as log:
        process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
        while True:
            try:
                returncode = process.wait(timeout=30)
                break
            except subprocess.TimeoutExpired:
                status['elapsed_seconds'] = round(time.monotonic() - started, 1)
                if result_path.exists():
                    data = json.loads(result_path.read_text())
                    status['outcome'] = data.get('outcome')
                    status['parent_progress'] = data.get('parent_progress')
                write_json(args.output / 'summary.json', status)
                print(json.dumps(status), flush=True)
    data = json.loads(result_path.read_text()) if result_path.exists() else {}
    rows = data.get('assertions', [])
    status.update(elapsed_seconds=round(time.monotonic() - started, 1), returncode=returncode,
                  outcome=data.get('outcome'), parent_progress=data.get('parent_progress'))
    status['parent_integrity_errors'] = parent_integrity_errors(rows)
    status['complete'] = (returncode in (0, 1) and len(rows) == len(expected)
                          and not status['parent_integrity_errors']
                          and {row['id'] for row in rows} == expected
                          and data.get('outcome', {}).get('status') in ('passed', 'failed'))
    if result_path.exists():
        status['evidence_sources'] = [{'path': str(result_path.resolve()),
            'sha256': hashlib.sha256(result_path.read_bytes()).hexdigest(),
            'run_fingerprint': data.get('run_fingerprint')}]
    status['totals'] = dict(Counter(row['status'] for row in rows))
    status['error'] = None if status['complete'] else 'cohort incomplete; inspect cohort.log'
    if status['complete'] and coverage.is_file():
        status.update(publish_cohort_ledger(args.output, coverage, data, status['evidence_sources'][0]))
    write_json(args.output / 'summary.json', status)
    return 0 if status['complete'] else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--jobs", type=int, default=8)
    parser.add_argument('--execution-layout', choices=['cohort', 'legacy-shards'], default='cohort')
    parser.add_argument("--tier", choices=["core", "necessary", "extended", "platform_dependent"])
    parser.add_argument("--reuse-accepted-from", type=Path,
                        help="reuse only controller-accepted shards, preserving their original provenance")
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())
    selected_shards = [shard for shard in plan["shards"]
                       if args.tier is None or shard["tier"] == args.tier]
    if not selected_shards:
        parser.error("no shards match the selected tier")
    args.output.mkdir(parents=True, exist_ok=True)
    if args.execution_layout == 'cohort':
        if args.reuse_accepted_from:
            parser.error('a fresh cohort cannot reuse historical shard evidence')
        return run_cohort(args, plan, selected_shards)
    reused = {}
    if args.reuse_accepted_from:
        if args.output.resolve() == args.reuse_accepted_from.resolve():
            parser.error("recovery requires a separate output directory")
        previous = json.loads((args.reuse_accepted_from / "summary.json").read_text())
        for key in ("official_manifest_sha256", "policy_sha256"):
            if previous[key] != plan[key]:
                parser.error("prior " + key + " does not match plan")
        if previous.get("selected_tier") != args.tier:
            parser.error("prior selected tier differs")
        current_binary = hashlib.sha256(args.binary.read_bytes()).hexdigest()
        for shard in selected_shards[:previous["finished_shards"]]:
            path = args.reuse_accepted_from / (shard["name"] + ".json")
            data = json.loads(path.read_text())
            if data["binary"]["sha256"] != current_binary:
                parser.error("prior parent binary differs")
            if data["inventory_sha256"] != hashlib.sha256(Path(shard["inventory"]).read_bytes()).hexdigest():
                parser.error("prior inventory differs")
            reused[shard["name"]] = path
    started = time.time()
    totals = Counter()
    by_tier = {}
    by_root = {}
    finished = []
    status = {"complete": False, "qualification": "Signed diagnostic baseline; unsupported runner entries are not browser failures.",
              "manifest_tests": plan["total_manifest_tests"], "runnable": plan["runnable"],
              "unsupported": plan["unsupported"], "official_manifest_sha256": plan["official_manifest_sha256"]}
    status.update(selected_tier=args.tier,
                  selected_executions=sum(shard["executions"] for shard in selected_shards),
                  policy_sha256=plan["policy_sha256"])
    status["evidence_sources"] = []
    if args.reuse_accepted_from:
        status["prior_elapsed_seconds"] = previous["elapsed_seconds"]
        status["prior_summary_sha256"] = hashlib.sha256(
            (args.reuse_accepted_from / "summary.json").read_bytes()).hexdigest()
    status["qualification"] += " Recovery may contain multiple harness fingerprints; each source is retained explicitly."
    def publish(current=None, error=None):
        status.update(elapsed_seconds=round(time.time() - started, 1), totals=dict(totals),
                      by_tier=by_tier, by_root=by_root, finished_shards=len(finished), total_shards=len(selected_shards),
                      current_shard=current, error=error)
        write_json(args.output / "summary.json", status)
    for shard in selected_shards:
        name = shard["name"]
        publish(name)
        print(f"START {name}: {shard['executions']} executions", flush=True)
        result_path = args.output / (name + ".json")
        command = [sys.executable, str(ROOT / "scripts/run-upstream-wpt.py"),
                   "--checkout", str(args.checkout.resolve()), "--binary", str(args.binary.resolve()),
                   "--inventory", shard["inventory"], "--output", str(result_path.resolve()),
                   "--jobs", str(args.jobs), "--resume"]
        if name in reused:
            result_path = reused[name]
            returncode = 0
        else:
            with (args.output / (name + ".log")).open("a") as log:
                result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
            returncode = result.returncode
        data = json.loads(result_path.read_text()) if result_path.exists() else {}
        rows = data.get("assertions", [])
        inventory = json.loads(Path(shard["inventory"]).read_text())
        expected_ids = {f"{r['path']}|{g}|{v}" for r in inventory["tests"] for g in r["globals"] for v in r["variants"]}
        if returncode not in {0, 1} or len(rows) != shard["executions"] or {r["id"] for r in rows} != expected_ids or data.get("outcome", {}).get("status") not in {"passed", "failed"}:
            # Do not silently skip a failed harness process or stale resume.
            # Completion field is checked against the canonical runner below.
            publish(name, f"Shard incomplete or runner failed (exit {returncode}); inspect {name}.log")
            return 2
        status["evidence_sources"].append({"shard": name, "path": str(result_path.resolve()),
            "sha256": hashlib.sha256(result_path.read_bytes()).hexdigest(),
            "run_fingerprint": data.get("run_fingerprint"), "reused": name in reused})
        tier_counts = Counter(by_tier.get(shard["tier"], {}))
        root_counts = Counter(by_root.get(shard["root"], {}))
        for row in rows:
            key = row["status"] if row.get("status") in {"passed", "environment_blocked"} else "failed"
            subtests = row.get("result", {}).get("tests", [])
            updates = Counter({key: 1, "observed_subtests": len(subtests),
                               "passing_subtests": sum(t.get("status") == 0 for t in subtests)})
            if key == "failed":
                updates["failure_" + str(row.get("failure_domain") or "unclassified")] += 1
            if row.get("runner_limitation"):
                updates["runner_limitation_" + row["runner_limitation"]] += 1
            for tag in row.get("diagnostic_tags", []):
                updates["diagnostic_" + tag] += 1
            totals.update(updates)
            tier_counts.update(updates)
            root_counts.update(updates)
        by_tier[shard["tier"]] = dict(tier_counts)
        by_root[shard["root"]] = dict(root_counts)
        finished.append(name)
        publish()
        print(f"DONE {name}: {data['outcome']}", flush=True)
    status["complete"] = True
    publish()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

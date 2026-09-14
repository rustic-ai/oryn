#!/usr/bin/env python3
"""Prepare affected slices or reconcile a versioned exact-ID replacement ledger."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re

from wpt_evidence import diagnostics, script_dependencies


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_sources(summary):
    records, origins = {}, {}
    for source in summary["evidence_sources"]:
        path = Path(source["path"])
        if digest(path) != source["sha256"]:
            raise ValueError("evidence hash mismatch: " + str(path))
        data = json.loads(path.read_text())
        for row in data["assertions"]:
            if row["id"] in records:
                raise ValueError("duplicate ID: " + row["id"])
            records[row["id"]] = row
            origins[row["id"]] = {"path": str(path.resolve()), "sha256": source["sha256"],
                "run_fingerprint": data["run_fingerprint"], "binary": data["binary"],
                "worker_sha256": data["runtime_metadata"]["worker_binary_sha256"]}
    return records, origins


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["prepare", "merge"])
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--checkout", type=Path)
    parser.add_argument("--replacement", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("choose a new output path; evidence is immutable")
    old, origins = load_sources(json.loads(args.baseline.read_text()))
    plan = json.loads(args.plan.read_text())
    if args.mode == "prepare":
        if args.checkout is None:
            parser.error("prepare requires checkout")
        affected = set()
        for row in old.values():
            if row.get("result", {}).get("harnessMessage") == "Uncaught SyntaxError: Unexpected token ':'":
                affected.update(re.findall(r'https?://[^/\s]+(/[^\s"\\]+\.js)(?=: JavaScript execution failed)',
                    "\n".join(row.get("diagnostics", []))))
        reasons = defaultdict(list)
        dependency_cache = {}
        controls = set()
        for identifier, row in sorted(old.items()):
            path = row["path"]
            if path not in dependency_cache:
                dependency_cache[path] = script_dependencies(args.checkout.resolve(), path)
            if path.startswith("websockets/"):
                reasons[identifier].append("complete_websocket_slice")
            if row.get("result", {}).get("harnessMessage") == "Uncaught SyntaxError: Unexpected token ':'":
                reasons[identifier].append("recorded_colon_syntax_failure")
            if affected.intersection(dependency_cache[path]):
                reasons[identifier].append("shared_dependency_consumer")
            control = (path.split('/')[0], row.get('global'))
            if row["status"] == "passed" and control not in controls:
                reasons[identifier].append("passing_regression_control")
                controls.add(control)
        args.output.mkdir(parents=True)
        shards = []
        for shard in plan["shards"]:
            if shard["tier"] != "core":
                continue
            inventory = json.loads(Path(shard["inventory"]).read_text())
            tests = []
            for row in inventory["tests"]:
                for global_name in row["globals"]:
                    for variant in row["variants"]:
                        if f"{row['path']}|{global_name}|{variant}" in reasons:
                            tests.append(dict(row, globals=[global_name], variants=[variant]))
            if not tests:
                continue
            path = args.output / (shard["name"] + ".inventory.json")
            path.write_text(json.dumps(dict(inventory, tests=tests, required_execution_total=len(tests))) + '\n')
            shards.append(dict(shard, inventory=str(path.resolve()), executions=len(tests)))
        result = dict(plan, shards=shards, runnable=sum(s['executions'] for s in shards),
            repair_selection={"baseline_sha256": digest(args.baseline), "reasons": reasons,
                              "affected_resources": sorted(affected)})
        (args.output / 'plan.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({"executions": result['runnable'], "shards":len(shards), "resources":len(affected)}))
    else:
        if args.replacement is None:
            parser.error("merge requires replacement summary")
        if plan['repair_selection']['baseline_sha256'] != digest(args.baseline):
            parser.error("repair plan baseline hash mismatch")
        summary = json.loads(args.replacement.read_text())
        if not summary['complete']:
            parser.error("replacement run incomplete")
        new, new_origins = load_sources(summary)
        selected = set(plan['repair_selection']['reasons'])
        if set(new) != selected or not selected <= old.keys():
            parser.error("replacement selection mismatch")
        for identifier in new:
            if origins[identifier]['binary'] != new_origins[identifier]['binary'] or origins[identifier]['worker_sha256'] != new_origins[identifier]['worker_sha256']:
                parser.error("browser binary changed")
        combined = dict(old, **new)
        counts = Counter(row['status'] for row in combined.values())
        transitions = Counter((old[k]['status'], new[k]['status']) for k in new)
        ledger = []
        for identifier, row in sorted(combined.items()):
            ledger.append({"id":identifier,"status":row['status'],"reason":row.get('reason'),
                "diagnostic_tags":row.get('diagnostic_tags',diagnostics(row)),
                "original":origins[identifier],"replacement":new_origins.get(identifier)})
        args.output.mkdir(parents=True)
        with (args.output/'ledger.jsonl').open('w') as output:
            for row in ledger:
                output.write(json.dumps(row,sort_keys=True)+'\n')
        report={"qualification":"Versioned composite diagnostic evidence; not a single-harness release attestation.",
            "complete":True,"selected":len(old),"replaced":len(new),"counts":counts,
            "attempted":counts['passed']+counts['failed'],
            "transitions":[{"old":a,"new":b,"count":n} for (a,b),n in sorted(transitions.items())],
            "baseline_summary_sha256":digest(args.baseline),"replacement_summary_sha256":digest(args.replacement),
            "ledger_sha256":digest(args.output/'ledger.jsonl')}
        (args.output/'summary.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report,indent=2))


if __name__ == '__main__':
    main()

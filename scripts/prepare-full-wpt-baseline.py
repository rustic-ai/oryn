#!/usr/bin/env python3
"""Create tier/root execution shards from the official full WPT manifest.

All manifest test entries remain in coverage.jsonl, including unsupported runners.
No test count is inferred from filename suffixes or source-file counts.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("tiers", ROOT / "scripts/catalog-wpt-capabilities.py")
tiers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tiers)


def leaves(tree, prefix=""):
    for name, value in sorted(tree.items()):
        path = prefix + name
        if isinstance(value, dict):
            yield from leaves(value, path + "/")
        else:
            yield path, value


def runner_mapping(kind, path, url, metadata):
    if kind != "testharness":
        return None, "runner_not_available:" + kind
    if metadata.get("jsshell"):
        return None, "runner_not_available:jsshell"
    parsed = urlsplit(url)
    variant = ("?" + parsed.query if parsed.query else "") + ("#" + parsed.fragment if parsed.fragment else "")
    served = parsed.path.lstrip("/")
    if path.endswith(".any.js"):
        base = path[:-len(".any.js")]
        if served == base + ".any.html":
            return ("window", variant), None
        if served == base + ".any.worker.html":
            return ("dedicatedworker", variant), None
        # Existing service-worker adapter uses a nonstandard constructor and
        # cannot provide valid baseline evidence. Keep the whole family visible.
        return None, "runner_not_available:generated_global:" + served
    if path.endswith(".window.js") and served == path[:-len(".window.js")] + ".window.html":
        return ("window", variant), None
    if path.endswith((".html", ".htm")) and served == path:
        return ("window", variant), None
    return None, "runner_not_available:document_or_wrapper"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--policy", type=Path, default=ROOT / "benchmarks/conformance/wpt-capability-tiers-v2.json")
    args = parser.parse_args()
    policy_path = args.policy
    policy = json.loads(policy_path.read_text())
    revision = subprocess.check_output(["git", "-C", str(args.checkout), "rev-parse", "HEAD"], text=True).strip()
    if revision != policy["upstream_revision"]:
        raise SystemExit("WPT revision mismatch")
    manifest = json.loads(args.manifest.read_text())
    if manifest["version"] != 9:
        raise SystemExit("unsupported upstream manifest version")
    args.output.mkdir(parents=True, exist_ok=True)
    groups = defaultdict(list)
    counts = Counter()
    ids = set()
    coverage = []
    for kind, tree in sorted(manifest["items"].items()):
        if kind in {"support", "spec"}:
            continue
        for path, leaf in leaves(tree):
            tier, unclassified = tiers.classify(path, policy)
            root = path.split("/", 1)[0]
            digest = hashlib.sha256((args.checkout / path).read_bytes()).hexdigest()
            for entry in leaf[1:]:
                url = entry[0] or "/" + path
                identifier = kind + ":" + url
                if identifier in ids:
                    raise SystemExit("duplicate manifest ID: " + identifier)
                ids.add(identifier)
                metadata = entry[-1] if isinstance(entry[-1], dict) else {}
                mapping, reason = runner_mapping(kind, path, url, metadata)
                row = {"id": identifier, "url": url, "path": path, "tier": tier,
                       "root": root, "test_type": kind, "sha256": digest,
                       "needs_classification": unclassified, "metadata": metadata,
                       "status": "not_run" if mapping else "unsupported", "reason": reason}
                row["environment_requirements"] = sorted({
                    rule["requirement"] for rule in policy.get("environment_rules", [])
                    if any(path == prefix or path.startswith(prefix + "/")
                           for prefix in rule["prefixes"])
                })
                if mapping:
                    global_name, variant = mapping
                    row["runner_id"] = f"{path}|{global_name}|{variant}"
                    groups[(tier, root)].append({"path": path, "globals": [global_name],
                        "variants": [variant], "sha256": digest, "workstream": tier,
                        "kind": "script" if path.endswith(".js") else "html", "root": root})
                coverage.append(row)
                counts[(tier, kind, row["status"])] += 1
    with (args.output / "coverage.jsonl").open("w") as output:
        for row in coverage:
            output.write(json.dumps(row, sort_keys=True) + "\n")
    shards = []
    order = {name: index for index, name in enumerate(policy["tiers"])}
    for (tier, root), tests in sorted(groups.items(), key=lambda item: (order[item[0][0]], item[0][1])):
        # Bounded shards prevent quadratic checkpoint serialization on a full
        # inventory, while preserving every official execution variant.
        for offset in range(0, len(tests), 200):
            chunk = tests[offset:offset + 200]
            name = f"{tier}-{root}-{offset // 200:04d}"
            inventory_path = args.output / (name + ".inventory.json")
            inventory_path.write_text(json.dumps({"schema_version": 1, "upstream_revision": revision,
                "required_execution_total": len(chunk), "tests": chunk}, sort_keys=True) + "\n")
            shards.append({"name": name, "tier": tier, "root": root,
                           "executions": len(chunk), "inventory": str(inventory_path.resolve())})
    summary = {"upstream_revision": revision,
        "official_manifest_sha256": hashlib.sha256(args.manifest.read_bytes()).hexdigest(),
        "policy_sha256": hashlib.sha256(policy_path.read_bytes()).hexdigest(),
        "total_manifest_tests": len(coverage), "runnable": sum(len(v) for v in groups.values()),
        "unsupported": sum(row["status"] == "unsupported" for row in coverage),
        "counts": [{"tier": t, "test_type": k, "status": s, "count": n} for (t, k, s), n in sorted(counts.items())],
        "shards": shards}
    (args.output / "plan.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({key: value for key, value in summary.items() if key not in {"counts", "shards"}}, indent=2))


if __name__ == "__main__":
    main()

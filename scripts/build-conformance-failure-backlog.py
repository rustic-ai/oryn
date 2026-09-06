#!/usr/bin/env python3
"""Build a complete-inventory failure index from one verified full WPT ledger."""
import argparse
import collections
import hashlib
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def route(path):
    root = path.split("/")[0]
    if root == "editing":
        return "W7", path.split("/")[1]
    if root == "IndexedDB":
        return "W8", "indexeddb"
    if root in ("html", "referrer-policy"):
        return "W4", "history" if root == "html" else "referrer-inheritance"
    if root in ("accname", "wai-aria"):
        return "W9", root
    if root == "WebIDL":
        return "W2", "webidl"
    if root == "css":
        cluster = path.split("/")[1]
        return "W5" if cluster == "cssom" else "W6", cluster
    if path.startswith(("dom/events/", "dom/abort/")):
        return "W3", "events-and-abort"
    if path.startswith(("dom/ranges/", "dom/traversal/")):
        return "W1", path.split("/")[1]
    if (path.startswith(("dom/nodes/", "dom/collections/", "dom/lists/"))
            or path == "dom/attributes-are-nodes.html"):
        return "W1", "dom-core"
    return "W2", "dom-interface-and-observable"


def build_backlog(result_path, inventory_path):
    spec = importlib.util.spec_from_file_location(
        "conformance_validator", ROOT / "scripts/validate-runtime-api-style-semantics-result.py")
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    inventory_bytes = inventory_path.read_bytes()
    expected = validator.expected_assertions(json.loads(inventory_bytes))
    result_bytes = result_path.read_bytes()
    result = json.loads(result_bytes)
    assertions = result["assertions"]
    ids = [row["id"] for row in assertions]
    if len(ids) != len(set(ids)) or set(ids) != expected:
        raise ValueError("Ledger must cover the exact inventory once, without missing or extra IDs")
    outcome = result["outcome"]
    if (outcome["required_total"] != len(expected)
            or outcome["required_finished"] != len(expected)
            or outcome["status"] == "running"):
        raise ValueError("Ledger is not a finished full run")
    digest = hashlib.sha256(inventory_bytes).hexdigest()
    if result["inventory_sha256"] != digest:
        raise ValueError("Ledger inventory fingerprint does not match")
    passed = 0
    rows = []
    for row in sorted(assertions, key=lambda row: row["id"]):
        validator.validate_passed_assertion(row, row["id"])
        if row["status"] == "passed":
            passed += 1
            continue
        if row["status"] != "failed":
            raise ValueError(f"Unfinished execution: {row['id']}")
        details = row.get("result") or {}
        first = next((test for test in details.get("tests", []) if test.get("status") != 0), {})
        workstream, cluster = route(row["path"])
        rows.append({
            "id": row["id"], "workstream": workstream, "routing_cluster": cluster,
            "root": row["path"].split("/")[0], "source_ledger": result_path.name,
            "failure_domain": row.get("failure_domain", "unknown"),
            "first_failing_test": first.get("name", ""),
            "first_failure_message": (first.get("message", "") or details.get("harnessMessage", ""))[:300],
            "confirmed_root_cause": None,
        })
    if passed != outcome["required_passed"]:
        raise ValueError("Ledger pass count does not match its executions")
    def display_path(path):
        try:
            return str(path.resolve().relative_to(ROOT))
        except ValueError:
            return str(path.resolve())
    return {
        "schema_version": 1,
        "purpose": "Single full-run planning index; not release qualification. Routing does not establish root causes.",
        "inventory": display_path(inventory_path), "inventory_sha256": digest,
        "required_execution_count": len(expected),
        "selected_source_ids_exactly_cover_inventory": True,
        "source_selection": "One complete full ledger; no mixed-candidate overlays.",
        "sources": [{"ledger": display_path(result_path),
                     "sha256": hashlib.sha256(result_bytes).hexdigest(),
                     "worker_sha256": result["runtime_metadata"]["worker_binary_sha256"],
                     "source": result["source"], "outcome": outcome}],
        "open_execution_count": len(rows),
        "counts_by_root": dict(sorted(collections.Counter(row["root"] for row in rows).items())),
        "counts_by_workstream": dict(sorted(collections.Counter(row["workstream"] for row in rows).items())),
        "entries": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--inventory", type=Path,
                        default=ROOT / "benchmarks/conformance/runtime-api-style-semantics-v2-inventory.json")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = build_backlog(args.result, args.inventory)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({key: report[key] for key in
                     ("open_execution_count", "counts_by_root", "counts_by_workstream")}, indent=2))


if __name__ == "__main__":
    main()

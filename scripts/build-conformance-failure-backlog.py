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


def load_diagnoses(path, expected):
    if path is None:
        return {}, None
    contents = path.read_bytes()
    data = json.loads(contents)
    if data.get("schema_version") != 1:
        raise ValueError("Unsupported diagnosis ledger schema")
    diagnoses = {}
    for entry in data["entries"]:
        identifier = entry["id"]
        if identifier not in expected or identifier in diagnoses:
            raise ValueError(f"Unknown or duplicate diagnosis ID: {identifier}")
        if not entry.get("cause") or not entry.get("observations"):
            raise ValueError(f"Diagnosis needs a cause and observations: {identifier}")
        for observation in entry["observations"]:
            digest = observation.get("ledger_sha256", "")
            if (len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest)
                    or not observation.get("source_commit")
                    or not observation.get("evidence")
                    or not observation.get("assertion_names")):
                raise ValueError(f"Diagnosis lacks reproducible evidence: {identifier}")
        diagnoses[identifier] = entry
    return diagnoses, {"path": str(path), "sha256": hashlib.sha256(contents).hexdigest()}


def build_backlog(result_path, inventory_path, diagnoses_path=None):
    spec = importlib.util.spec_from_file_location(
        "conformance_validator", ROOT / "scripts/validate-runtime-api-style-semantics-result.py")
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    inventory_bytes = inventory_path.read_bytes()
    expected = validator.expected_assertions(json.loads(inventory_bytes))
    diagnoses, diagnosis_source = load_diagnoses(diagnoses_path, expected)
    result_bytes = result_path.read_bytes()
    ledger_digest = hashlib.sha256(result_bytes).hexdigest()
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
        diagnosis = diagnoses.get(row["id"])
        confirmed = False
        if diagnosis:
            failed_names = {test["name"] for test in details.get("tests", [])
                            if test.get("status") != 0}
            for observation in diagnosis["observations"]:
                if observation["ledger_sha256"] == ledger_digest:
                    if (observation["source_commit"] != result["source"]["commit"]
                            or not set(observation["assertion_names"]).issubset(failed_names)):
                        raise ValueError(f"Diagnosis contradicts its ledger: {row['id']}")
                    confirmed = True
        rows.append({
            "id": row["id"], "workstream": workstream, "routing_cluster": cluster,
            "root": row["path"].split("/")[0], "source_ledger": result_path.name,
            "failure_domain": row.get("failure_domain", "unknown"),
            "first_failing_test": first.get("name", ""),
            "first_failure_message": (first.get("message", "") or details.get("harnessMessage", ""))[:300],
            "confirmed_root_cause": diagnosis["cause"] if confirmed else None,
            **({"diagnosis": {**diagnosis,
                              "state": "confirmed_for_this_run" if confirmed else "needs_revalidation"}}
               if diagnosis else {}),
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
                     "sha256": ledger_digest,
                     "worker_sha256": result["runtime_metadata"]["worker_binary_sha256"],
                     "source": result["source"], "outcome": outcome}],
        "open_execution_count": len(rows),
        "counts_by_root": dict(sorted(collections.Counter(row["root"] for row in rows).items())),
        "counts_by_workstream": dict(sorted(collections.Counter(row["workstream"] for row in rows).items())),
        "diagnosis_source": ({**diagnosis_source, "path": display_path(diagnoses_path)}
                             if diagnosis_source else None),
        "entries": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--result", type=Path, required=True)
    parser.add_argument("--inventory", type=Path,
                        default=ROOT / "benchmarks/conformance/runtime-api-style-semantics-v2-inventory.json")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--diagnoses", type=Path,
                        default=ROOT / "docs/oryn2/CONFORMANCE-DIAGNOSES.json")
    args = parser.parse_args()
    report = build_backlog(args.result, args.inventory, args.diagnoses)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({key: report[key] for key in
                     ("open_execution_count", "counts_by_root", "counts_by_workstream")}, indent=2))


if __name__ == "__main__":
    main()

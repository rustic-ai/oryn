#!/usr/bin/env python3
"""Assign every pinned Core obligation without promoting diagnostics to conformance."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path

GROUPS = {
    "html": ("R", "L", "S", "V"),
    "dom": ("S", "L"),
    "encoding": ("L",),
    "network": ("L", "W"),
    "policy": ("L", "P", "W"),
    "components": ("S", "L", "W"),
    "interaction": ("S", "V"),
    "css": ("V",),
}
ROOTS = {
    "html": "html",
    **dict.fromkeys(("dom", "domparsing", "domxpath", "xml", "webidl"), "dom"),
    **dict.fromkeys(("encoding", "encoding-detection", "url", "mimesniff"), "encoding"),
    **dict.fromkeys(("fetch", "xhr", "cors", "websockets", "FileAPI", "streams"), "network"),
    **dict.fromkeys(("content-security-policy", "referrer-policy", "mixed-content",
                    "upgrade-insecure-requests", "permissions-policy", "trusted-types",
                    "secure-contexts", "x-frame-options", "subresource-integrity",
                    "cookies", "webstorage"), "policy"),
    **dict.fromkeys(("custom-elements", "shadow-dom", "import-maps", "webmessaging"), "components"),
    **dict.fromkeys(("selection", "focus", "inert", "uievents", "input-events",
                    "accname", "wai-aria", "html-aam"), "interaction"),
    "css": "css",
}


def digest(path):
    with Path(path).open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def index(rows):
    result = {}
    for row in rows:
        identifier = row["id"]
        if identifier in result:
            raise ValueError("duplicate manifest ID: " + identifier)
        result[identifier] = row
    return result


def assign(coverage, observations):
    """Join exact official IDs. Ownership buckets never establish root causes."""
    coverage, observations = index(coverage), index(observations)
    if set(coverage) != set(observations):
        raise ValueError("coverage and observed ledger IDs differ")
    entries = []
    for identifier, entry in sorted(coverage.items()):
        observed = observations[identifier]
        for field in ("path", "tier", "root", "test_type", "url", "sha256", "runner_id"):
            if entry.get(field) != observed.get(field):
                raise ValueError(f"ledger identity mismatch: {identifier}: {field}")
        if entry["tier"] != "core":
            continue
        group = ROOTS.get(entry["root"])
        if group is None:
            raise ValueError("unassigned Core root: " + entry["root"])
        status = observed["status"]
        if status not in ("passed", "failed", "unsupported", "environment_blocked"):
            raise ValueError(f"unfinished Core observation: {identifier}: {status}")
        if entry["status"] == "unsupported" and status != "unsupported":
            raise ValueError("unsupported coverage relabeled as executed: " + identifier)
        if entry.get("runner_id") and status == "unsupported":
            raise ValueError("mapped execution relabeled as unsupported: " + identifier)
        packages = list(GROUPS[group])
        if status in ("unsupported", "environment_blocked"):
            packages.insert(0, "G")
        reason = observed.get("reason") or ""
        if "generated_global" in reason and "W" not in packages:
            packages.append("W")
        if entry["test_type"] in ("reftest", "print-reftest", "visual", "manual") and "V" not in packages:
            packages.append("V")
        entries.append({
            "id": identifier,
            "path": entry["path"],
            "url": entry["url"],
            "test_type": entry["test_type"],
            "source_sha256": entry["sha256"],
            "runner_id": entry.get("runner_id"),
            "capability_group": group,
            "required_packages": packages,
            "raw_status": status,
            "observed_qualification": observed.get("qualification", "unresolved"),
            "failure_domain": observed.get("failure_domain"),
            "diagnostic_tags": observed.get("diagnostic_tags", []),
            "execution_gap": reason or None,
            "environment_requirements": entry.get("environment_requirements", []),
            "confirmed_root_cause": None,
            "implementation_status": "unassessed",
            "acceptance_status": "requires_source_bound_qualification",
        })
    return entries


def jsonlines(path):
    with path.open() as handle:
        for line in handle:
            yield json.loads(line)


def build(plan_path, run):
    plan = json.loads(plan_path.read_text())
    summary = json.loads((run / "summary.json").read_text())
    if (summary.get("complete") is not True or summary.get("error")
            or summary.get("parent_integrity_errors")
            or summary.get("selected_tier") not in (None, "core")):
        raise ValueError("requires an intact completed Core or all-tier run")
    for key in ("official_manifest_sha256", "policy_sha256"):
        if summary[key] != plan[key]:
            raise ValueError("run/plan mismatch: " + key)
    if summary["plan_sha256"] != digest(plan_path):
        raise ValueError("plan fingerprint mismatch")
    coverage_path = plan_path.parent / "coverage.jsonl"
    if summary["manifest_coverage"]["sha256"] != digest(coverage_path):
        raise ValueError("coverage fingerprint mismatch")
    ledger_path = run / "ledger.jsonl"
    if summary["ledger_sha256"] != digest(ledger_path):
        raise ValueError("observed ledger fingerprint mismatch")
    sources = summary["evidence_sources"]
    if len(sources) != 1:
        raise ValueError("requires a single candidate run; mixed evidence is not accepted")
    source = sources[0]
    source_path = Path(source["path"]).resolve()
    if not source_path.is_relative_to(run.resolve()) or digest(source_path) != source["sha256"]:
        raise ValueError("run evidence fingerprint/location mismatch")
    coverage = list(jsonlines(coverage_path))
    observed = list(jsonlines(ledger_path))
    if len(coverage) != plan["total_manifest_tests"]:
        raise ValueError("official manifest denominator mismatch")
    for row in observed:
        if row.get("evidence") and row["evidence"] != source:
            raise ValueError("mixed candidate evidence in observed ledger")
    entries = assign(coverage, observed)
    groups = {}
    for group in GROUPS:
        selected = [entry for entry in entries if entry["capability_group"] == group]
        groups[group] = dict(Counter(entry["raw_status"] for entry in selected))
        groups[group]["total"] = len(selected)
    return {
        "schema_version": 1,
        "purpose": "Complete Core implementation allocation; not conformance qualification",
        "goal_complete": False,
        "upstream_revision": plan["upstream_revision"],
        "official_manifest_sha256": plan["official_manifest_sha256"],
        "policy_sha256": plan["policy_sha256"],
        "plan_sha256": digest(plan_path),
        "coverage_sha256": digest(coverage_path),
        "observation_ledger_sha256": digest(ledger_path),
        "evidence": source,
        "required_core_total": len(entries),
        "raw_counts": dict(Counter(entry["raw_status"] for entry in entries)),
        "qualification_counts": dict(Counter(entry["observed_qualification"] for entry in entries)),
        "capability_groups": groups,
        "entries": entries,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = build(args.plan, args.run)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")
    print(json.dumps({key: report[key] for key in ("required_core_total", "raw_counts", "qualification_counts")}))


if __name__ == "__main__":
    main()

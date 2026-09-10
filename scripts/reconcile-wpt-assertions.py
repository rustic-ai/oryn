#!/usr/bin/env python3
"""Compare exact WPT assertions without silently dropping duplicate names.

Accepts canonical runner ledgers or isolated-native-context JSONL probe logs.
Legacy text extraction normalized whitespace inside serialized JSON strings;
--legacy-baseline models that transport, but never resolves name collisions by
position or treats an absent/ambiguous assertion as a pass.
"""

from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path


def legacy_name(name: str) -> str:
    return json.loads(" ".join(json.dumps(name, ensure_ascii=False).split()))


def read_executions(path: Path) -> dict:
    source = path.read_text()
    try:
        document = json.loads(source)
    except json.JSONDecodeError:
        document = None
    if isinstance(document, dict) and isinstance(document.get("assertions"), list):
        pairs = [(row["id"], row.get("result") or {}) for row in document["assertions"]]
    else:
        pairs = []
        for line in source.splitlines():
            if not line.startswith("{"):
                continue
            row = json.loads(line)
            if row.get("execution_mode") != "isolated_native_context_fixture":
                continue
            fixture, global_name = row["fixture"].rsplit("|", 1)
            fixture, separator, variant = fixture.partition("?")
            key = fixture + "|" + global_name + "|" + ("?" + variant if separator else "")
            pairs.append((key, row.get("result") or {}))
    if not pairs or len(dict(pairs)) != len(pairs):
        raise ValueError("execution evidence is empty or contains duplicate IDs")
    return dict(pairs)


def compare_assertions(before: list, after: list, *, legacy_baseline=False) -> dict:
    old, new = defaultdict(list), defaultdict(list)
    for assertion in before:
        old[assertion["name"]].append(assertion)
    for assertion in after:
        name = assertion["name"]
        new[legacy_name(name) if legacy_baseline else name].append(assertion)
    report = {key: [] for key in (
        "corrected", "regressed", "missing", "added", "ambiguous", "multiplicity_mismatch"
    )}
    report["uniquely_matched"] = 0
    for name in sorted(old.keys() | new.keys()):
        left, right = old[name], new[name]
        if not left:
            report["added"].append(name)
        elif not right:
            report["missing"].append(name)
        elif len(left) != len(right):
            report["multiplicity_mismatch"].append({
                "name": name, "before": len(left), "after": len(right)
            })
        elif len(left) != 1:
            report["ambiguous"].append({"name": name, "count": len(left)})
        else:
            report["uniquely_matched"] += 1
            if left[0]["status"] == 0 and right[0]["status"] != 0:
                report["regressed"].append(right[0])
            elif left[0]["status"] != 0 and right[0]["status"] == 0:
                report["corrected"].append(right[0]["name"])
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--legacy-baseline", action="store_true")
    parser.add_argument("--allow-subset", action="store_true",
                        help="Explicitly compare only candidate execution IDs")
    args = parser.parse_args()
    baseline, candidate = read_executions(args.baseline), read_executions(args.candidate)
    missing = sorted(baseline.keys() - candidate.keys())
    unexpected = sorted(candidate.keys() - baseline.keys())
    comparisons = {}
    incomplete = []
    for key in sorted(baseline.keys() & candidate.keys()):
        current = candidate[key]
        if current.get("complete") is not True or current.get("harness") != 0 or not current.get("tests"):
            incomplete.append(key)
        comparisons[key] = compare_assertions(
            baseline[key].get("tests", []), current.get("tests", []),
            legacy_baseline=args.legacy_baseline,
        )
    totals = {
        name: sum(len(row[name]) for row in comparisons.values())
        for name in ("corrected", "regressed", "missing", "added", "ambiguous", "multiplicity_mismatch")
    }
    clean = not (unexpected or incomplete or (missing and not args.allow_subset)
                 or any(totals[key] for key in ("regressed", "missing", "ambiguous", "multiplicity_mismatch")))
    result = {
        "schema_version": 1, "regression_free": clean,
        "legacy_baseline": args.legacy_baseline, "subset_comparison": args.allow_subset,
        "baseline_sha256": hashlib.sha256(args.baseline.read_bytes()).hexdigest(),
        "candidate_sha256": hashlib.sha256(args.candidate.read_bytes()).hexdigest(),
        "compared_executions": len(comparisons), "omitted_executions": missing,
        "unexpected_executions": unexpected, "incomplete_executions": incomplete,
        "totals": totals, "comparisons": comparisons,
    }
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=True) + "\n")
    print(json.dumps({key: result[key] for key in ("regression_free", "compared_executions", "totals")}))
    return 0 if clean else 1


if __name__ == "__main__":
    raise SystemExit(main())

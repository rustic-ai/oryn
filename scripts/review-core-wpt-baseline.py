#!/usr/bin/env python3
"""Summarize recorded Core outcomes without promoting symptoms to diagnoses."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import re
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    roots = defaultdict(Counter)
    messages = defaultdict(list)
    groups = defaultdict(Counter)
    sources = {}
    seen = set()
    script_errors = defaultdict(set)
    runtime_messages = defaultdict(list)
    for path in sorted(args.results.glob("core-*.json")):
        sources[path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
        data = json.loads(path.read_text())
        assert data["outcome"]["status"] in {"passed", "failed"}
        for row in data["assertions"]:
            assert row["id"] not in seen
            seen.add(row["id"])
            parts = row["path"].split("/")
            root = parts[0]
            group = "/".join(parts[:3] if root == "html" else parts[:2])
            status = "passed" if row["status"] == "passed" else row.get("failure_domain") or "unclassified"
            roots[root][status] += 1
            groups[group][status] += 1
            if status == "passed":
                continue
            result = row.get("result", {})
            first = next((t for t in result.get("tests", []) if t.get("status") != 0), {})
            message = result.get("harnessMessage") or first.get("message") or "[no assertion/harness message; inspect diagnostics]"
            messages[(status, message)].append(row["id"])
            diagnostics = "\n".join(row.get("diagnostics", []))
            for match in re.finditer(r"https?://[^/\s]+(/[^\s\"\\]+): JavaScript execution failed: Uncaught SyntaxError", diagnostics):
                script_errors[match.group(1)].add(row["id"])
            if status == "runtime":
                for line in diagnostics.splitlines():
                    if line.startswith("Error:"):
                        runtime_messages[line[:300]].append(row["id"])
                        break
    report = {"qualification": "Observed outcome domains and exact first-message clusters; not confirmed root causes.",
              "executions": len(seen), "roots": dict(sorted(roots.items())),
              "groups": dict(sorted(groups.items())), "source_sha256": sources,
              "syntax_error_resources": [{"path": path, "count": len(ids), "examples": sorted(ids)[:3]} for path, ids in sorted(script_errors.items(), key=lambda pair: -len(pair[1]))],
              "runtime_messages": [{"message": message, "count": len(ids), "examples": ids[:3]} for message, ids in sorted(runtime_messages.items(), key=lambda pair: -len(pair[1]))],
              "symptoms": [{"domain": domain, "message": message, "count": len(ids), "ids": ids}
                           for (domain, message), ids in sorted(messages.items(), key=lambda pair: -len(pair[1]))]}
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=True) + "\n")
    print(json.dumps({"executions": len(seen), "roots": report["roots"],
                      "top_symptoms": [{k: v for k, v in row.items() if k != "ids"} | {"examples": row["ids"][:2]} for row in report["symptoms"][:20]]}, indent=2))


if __name__ == "__main__":
    main()

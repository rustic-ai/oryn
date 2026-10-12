#!/usr/bin/env python3
"""Record a non-mutating three-way merge preview in local patch storage."""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    allowed = root.parent / ".local-patches"
    output = args.output.resolve()
    if not output.is_relative_to(allowed) or output == allowed:
        parser.error("output must be a batch directory under workspace .local-patches")
    output.mkdir(parents=True, exist_ok=True)
    def git(*argv):
        return subprocess.run(["git", "-C", str(root), *argv], capture_output=True, text=True)
    ours = git("rev-parse", "HEAD").stdout.strip()
    theirs = git("rev-parse", "recovery/native-split-2026-10-12").stdout.strip()
    result = git("merge-tree", "--write-tree", "--name-only", ours, theirs)
    (output / "merge-preview.txt").write_text(result.stdout + result.stderr)
    if result.returncode not in (0, 1):
        raise RuntimeError(result.stderr)
    lines = result.stdout.splitlines()
    conflicts = []
    for line in lines[1:]:
        if not line:
            break
        conflicts.append(line)
    manifest = {"integration_commit": ours, "recovery_commit": theirs,
                "merge_preview_tree": lines[0], "conflict_count": len(conflicts),
                "conflicts": conflicts, "state": "not-applied-not-reviewed",
                "policy": "Keep integration validation tooling and user layout changes. Reconcile native algorithms by component; regenerate bindings from reviewed IDL. Never prefer all ours or all theirs."}
    (output / "reconciliation.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"conflicts": len(conflicts), "manifest": str(output / "reconciliation.json")}))


if __name__ == "__main__":
    main()

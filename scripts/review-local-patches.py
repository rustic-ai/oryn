#!/usr/bin/env python3
"""Inventory preserved patches; applicability is not semantic qualification."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--batch", type=Path, required=True)
    parser.add_argument("--archive-duplicates", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    batch = args.batch.resolve(strict=True)
    if not batch.is_relative_to(root.parent / ".local-patches"):
        parser.error("batch must be under workspace .local-patches")
    output = batch / "patch-review"
    output.mkdir(exist_ok=True)
    head = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    seen = {}
    rows = []
    duplicates = []
    for patch in sorted((batch / "imported-tmp").glob("*.patch")):
        payload = patch.read_bytes()
        sha = hashlib.sha256(payload).hexdigest()
        row = {"name": patch.name, "sha256": sha, "bytes": len(payload),
               "paths": sorted({line[6:].split("\t")[0] for line in payload.decode("utf-8", errors="replace").splitlines()
                                if line.startswith("+++ b/")})}
        if sha in seen:
            row.update(state="exact-duplicate", canonical=seen[sha])
            duplicates.append(patch)
        else:
            seen[sha] = patch.name
            results = {}
            for label, flags in (("forward", []), ("reverse", ["--reverse"])):
                result = subprocess.run(["git", "-C", str(root), "apply", "--check", *flags, str(patch)],
                                        capture_output=True, text=True)
                results[label] = {"exit_code": result.returncode, "diagnostic": result.stderr}
            row["checks"] = results
            row["state"] = ("applicable-needs-semantic-review" if results["forward"]["exit_code"] == 0
                            else "reverse-applicable-needs-integration-proof" if results["reverse"]["exit_code"] == 0
                            else "base-conflict-or-invalid-needs-review")
        rows.append(row)
    report = {"integration_head": head, "patches": rows,
              "limits": "Textual application only. No correctness, regression or release qualification inferred."}
    (output / "inventory.json").write_text(json.dumps(report, indent=2) + "\n")
    if args.archive_duplicates and duplicates:
        archive_path = output / "exact-duplicates.tar.gz"
        if archive_path.exists():
            raise RuntimeError("refuse to overwrite existing archive")
        with tarfile.open(archive_path, "w:gz") as archive:
            for patch in duplicates:
                archive.add(patch, arcname=patch.name, recursive=False)
        with tarfile.open(archive_path, "r:gz") as archive:
            for patch in duplicates:
                saved = archive.extractfile(patch.name).read()
                if saved != patch.read_bytes():
                    raise RuntimeError(f"archive verification failed: {patch}")
        (output / "duplicate-retirement.json").write_text(json.dumps({
            "archive_sha256": hashlib.sha256(archive_path.read_bytes()).hexdigest(),
            "retired": [p.name for p in duplicates],
            "reason": "byte-identical canonical copy retained and archive verified; no semantic rejection"}, indent=2) + "\n")
        for patch in duplicates:
            patch.unlink()
    counts = {}
    for row in rows:
        counts[row["state"]] = counts.get(row["state"], 0) + 1
    print(json.dumps({"total": len(rows), "counts": counts, "report": str(output / "inventory.json")}))


if __name__ == "__main__":
    main()

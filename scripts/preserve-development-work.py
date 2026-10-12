#!/usr/bin/env python3
"""Preserve local source and temporary drafts without changing either Git index.

Explicit one-time recovery helper. Imported files are copied, never executed.
Outputs are local recovery evidence, not validation or release evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--batch", required=True)
    parser.add_argument("--finalize-existing", action="store_true",
                        help="Verify an existing recovery copy after interrupted bundle creation")
    args = parser.parse_args()
    if not args.batch or Path(args.batch).name != args.batch or args.batch in {".", ".."}:
        parser.error("batch must be a single directory name")
    workspace = args.workspace.resolve(strict=True)
    destination = workspace / ".local-patches" / args.batch
    if args.finalize_existing:
        records = []
        for target in sorted((destination / "imported-tmp").iterdir()):
            records.append({"source": str(Path("/private/tmp") / target.name),
                            "copy": str(target.relative_to(destination)),
                            "sha256": digest(target), "bytes": target.stat().st_size,
                            "state": "unreviewed-recovery-copy"})
        checkouts = {}
        for name in ("oryn", "oryn-roadmap-next"):
            saved = destination / name
            inventory = json.loads((saved / "inventory.json").read_text())
            with tarfile.open(saved / "source.tar.gz", "r:gz") as archive:
                for item in inventory:
                    if item.get("sha256"):
                        if hashlib.sha256(archive.extractfile(item["path"]).read()).hexdigest() != item["sha256"]:
                            raise RuntimeError(f"archive verification failed: {name}/{item['path']}")
            checkouts[name] = {"head": git(workspace / name, "rev-parse", "HEAD").decode().strip(),
                               "source_archive_sha256": digest(saved / "source.tar.gz"),
                               "inventory_entries": len(inventory)}
        subprocess.run(["git", "-C", str(workspace / "oryn-roadmap-next"), "bundle", "verify",
                        str(destination / "history.bundle")], check=True, capture_output=True)
        manifest = {"state": "preserved-not-integrated-not-qualified", "checkouts": checkouts,
                    "drafts": records, "history_bundle_sha256": digest(destination / "history.bundle"),
                    "limits": "No ignored input or temporary directory preservation; checkout HEAD observed at finalization. Copies may precede subsequent workflow edits."}
        (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print(json.dumps({"destination": str(destination), "draft_files": len(records),
                          "draft_bytes": sum(r["bytes"] for r in records), "checkouts": checkouts}))
        return
    destination.mkdir(parents=True, exist_ok=False)
    os.chmod(destination, 0o700)
    records = []
    drafts = destination / "imported-tmp"
    drafts.mkdir()
    for source in sorted(Path("/private/tmp").glob("oryn*")):
        if source.is_symlink() or not source.is_file():
            continue
        before = digest(source)
        target = drafts / source.name
        shutil.copy2(source, target)
        after = digest(source)
        copied = digest(target)
        if before != after or copied != before:
            raise RuntimeError(f"source changed during preservation: {source}")
        records.append({"source": str(source), "copy": str(target.relative_to(destination)),
                        "sha256": copied, "bytes": target.stat().st_size,
                        "state": "unreviewed-recovery-copy"})
    checkouts = {}
    for name in ("oryn", "oryn-roadmap-next"):
        root = workspace / name
        saved = destination / name
        saved.mkdir()
        head = git(root, "rev-parse", "HEAD").decode().strip()
        (saved / "tracked.patch").write_bytes(git(root, "diff", "--binary", "HEAD"))
        (saved / "index.patch").write_bytes(git(root, "diff", "--cached", "--binary"))
        (saved / "status.txt").write_bytes(git(root, "status", "--porcelain=v1"))
        names = git(root, "ls-files", "-z", "--cached", "--others", "--exclude-standard")
        members = sorted(set(os.fsdecode(x) for x in names.split(b"\0") if x))
        inventory = []
        with tarfile.open(saved / "source.tar.gz", "w:gz") as archive:
            for member in members:
                source = root / member
                if not source.exists() and not source.is_symlink():
                    inventory.append({"path": member, "deleted": True})
                    continue
                if source.is_dir():
                    inventory.append({"path": member, "nested_repository_or_directory": True})
                    continue
                archive.add(source, arcname=member, recursive=False)
                inventory.append({"path": member, "sha256": digest(source)
                                  if not source.is_symlink() else None,
                                  "symlink": os.readlink(source) if source.is_symlink() else None})
        with tarfile.open(saved / "source.tar.gz", "r:gz") as archive:
            for item in inventory:
                if item.get("sha256"):
                    extracted = archive.extractfile(item["path"])
                    if hashlib.sha256(extracted.read()).hexdigest() != item["sha256"]:
                        raise RuntimeError(f"archive verification failed: {name}/{item['path']}")
        (saved / "inventory.json").write_text(json.dumps(inventory, indent=2) + "\n")
        if git(root, "diff", "--binary", "HEAD") != (saved / "tracked.patch").read_bytes():
            raise RuntimeError(f"checkout changed during preservation: {name}")
        checkouts[name] = {"head": head, "branch": git(root, "branch", "--show-current").decode().strip(),
                           "source_archive_sha256": digest(saved / "source.tar.gz"),
                           "inventory_entries": len(inventory)}
    manifest = {"state": "source-preserved-history-bundle-pending", "checkouts": checkouts,
                "drafts": records,
                "limits": "Ignored build inputs and nested repository contents are not source archive members. Temporary directories are not imported. Original files remain untouched."}
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    subprocess.run(["git", "-C", str(workspace / "oryn-roadmap-next"), "bundle", "create",
                    str(destination / "history.bundle"), "--all"], check=True)
    subprocess.run(["git", "-C", str(workspace / "oryn-roadmap-next"), "bundle", "verify",
                    str(destination / "history.bundle")], check=True, capture_output=True)
    manifest["state"] = "preserved-not-integrated-not-qualified"
    manifest["history_bundle_sha256"] = digest(destination / "history.bundle")
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"destination": str(destination), "draft_files": len(records),
                      "draft_bytes": sum(r["bytes"] for r in records), "checkouts": checkouts}))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Apply the experimental WindowProxy patch only to its exact pinned inputs.

This does not qualify an engine archive or change the production runtime profile.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="isolated V8 source directory")
    parser.add_argument("--check", action="store_true", help="verify without applying")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    patch = root / "patches/v8/window-proxy-structural-semantics.patch"
    manifest = json.loads(patch.with_suffix(".json").read_text())
    if digest(patch) != manifest["patch_sha256"]:
        parser.error("patch differs from its manifest")
    source = args.source.resolve()
    actual = {name: digest(source / name) for name in manifest["base_files"]}
    if actual == manifest["patched_files"]:
        print("Exact patched sources verified; no changes needed.")
        return
    if actual != manifest["base_files"]:
        parser.error("source is neither the exact base nor the exact patched input")
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=source, check=True)
    if args.check:
        print("Exact base sources and patch applicability verified.")
        return
    subprocess.run(["git", "apply", str(patch)], cwd=source, check=True)
    actual = {name: digest(source / name) for name in manifest["patched_files"]}
    if actual != manifest["patched_files"]:
        raise RuntimeError("patched sources differ from the manifest")
    print("Exact patched sources verified; engine qualification remains required.")


if __name__ == "__main__":
    main()

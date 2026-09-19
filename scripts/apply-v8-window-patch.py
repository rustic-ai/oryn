#!/usr/bin/env python3
"""Apply an Oryn engine patch only to its exact pinned inputs.

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
    parser.add_argument("--patch", choices=("window-proxy-structural-semantics", "shared-memory-growth", "shared-memory-wait", "shared-wasm-size", "shared-wasm-backing", "shared-memory-factory", "frame-context-reattachment", "html-constructor-deferred-prototype"),
                        default="window-proxy-structural-semantics")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    patch = root / "patches/v8" / (args.patch + ".patch")
    manifest = json.loads(patch.with_suffix(".json").read_text())
    if digest(patch) != manifest["patch_sha256"]:
        parser.error("patch differs from its manifest")
    for required, expected in manifest.get("required_patch_sha256", {}).items():
        if digest(root / "patches/v8" / (required + ".patch")) != expected:
            parser.error("prerequisite patch hash changed: " + required)
    source = args.source.resolve()
    actual = {name: digest(source / name) for name in manifest["base_files"]}
    if actual == manifest["patched_files"]:
        print("Exact patched sources verified; no changes needed.")
        return
    # Later patches extend prerequisite files. Accept a fully verified complete
    # prefix (including the requested patch), never merely a subset of its touched files or an unrelated hash.
    chain = ("window-proxy-structural-semantics", "shared-memory-growth", "shared-memory-wait", "shared-wasm-size", "shared-wasm-backing", "shared-memory-factory", "frame-context-reattachment", "html-constructor-deferred-prototype")
    if args.patch in chain:
        final_files = {}
        valid = True
        for index, name in enumerate(chain):
            chain_patch = root / "patches/v8" / (name + ".patch")
            chain_manifest = json.loads(chain_patch.with_suffix(".json").read_text())
            valid &= digest(chain_patch) == chain_manifest["patch_sha256"]
            for prerequisite, expected in chain_manifest.get("required_patch_sha256", {}).items():
                valid &= digest(root / "patches/v8" / (prerequisite + ".patch")) == expected
            final_files.update(chain_manifest["patched_files"])
            if index >= chain.index(args.patch) and valid and all(
                    digest(source / relative) == expected for relative, expected in final_files.items()):
                print("Exact engine patch chain prefix verified; no changes needed.")
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

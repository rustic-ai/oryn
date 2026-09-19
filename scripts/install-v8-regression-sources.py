#!/usr/bin/env python3
"""Install Oryn native regression source into the exact pinned V8 cctest target.

This only prepares source. It neither builds nor executes any test.
"""
import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    args = parser.parse_args()
    fixtures = Path(__file__).resolve().parents[1] / "patches/v8/tests"
    manifest = json.loads((fixtures / "manifest.json").read_text())
    expected_files = {}
    visited = set()
    def require_patch(name, expected):
        patch = fixtures.parent / (name + ".patch")
        if digest(patch.read_bytes()) != expected:
            parser.error("regression prerequisite patch hash mismatch: " + name)
        metadata = json.loads(patch.with_suffix(".json").read_text())
        if metadata["patch_sha256"] != expected:
            parser.error("regression prerequisite manifest mismatch: " + name)
        if name in visited:
            return
        visited.add(name)
        for previous, previous_hash in metadata.get("required_patch_sha256", {}).items():
            require_patch(previous, previous_hash)
        expected_files.update(metadata["patched_files"])
    for name, expected in manifest.get("required_patch_sha256", {}).items():
        require_patch(name, expected)
    for relative, patched_hash in expected_files.items():
        if digest((args.source / relative).read_bytes()) != patched_hash:
            parser.error("regression requires patched engine source: " + relative)
    target = args.source / "test/cctest"
    build = target / "BUILD.gn"
    original = build.read_bytes()
    actual = digest(original)
    anchor = b'    "test-api-array-buffer.cc",\n'
    additions = b"".join(('    "' + name + '",\n').encode() for name in sorted(manifest["sources"]))
    if actual == manifest["cctest_build_base_sha256"]:
        base = original
    elif actual == manifest.get("previous_cctest_build_sha256"):
        # Upgrade only the exact previous source-only installation.
        base = original
        for name in manifest.get("previous_sources", ["test-oryn-shared-memory-growth.cc"]):
            base = base.replace(('    "' + name + '",\n').encode(), b"")
        if digest(base) != manifest["cctest_build_base_sha256"]:
            parser.error("previous cctest installation does not recover pinned base")
    elif actual == manifest["cctest_build_patched_sha256"]:
        base = None
    else:
        parser.error("cctest target differs from pinned base and generated target")
    if base is not None:
        if base.count(anchor) != 1:
            parser.error("missing unique pinned cctest source anchor")
        changed = base.replace(anchor, anchor + additions)
    else:
        changed = original
    if digest(changed) != manifest["cctest_build_patched_sha256"]:
        parser.error("generated cctest target hash mismatch")
    sources = {}
    for name, expected in manifest["sources"].items():
        data = (fixtures / name).read_bytes()
        if digest(data) != expected:
            parser.error("regression source hash mismatch: " + name)
        destination = target / name
        if (destination.exists() and destination.read_bytes() != data
                and digest(destination.read_bytes()) != manifest.get("previous_source_sha256", {}).get(name)):
            parser.error("refusing to overwrite divergent regression source: " + name)
        sources[destination] = data
    compatibility = []
    for name, expected in manifest.get("test_only_compatibility", {}).items():
        metadata_path = fixtures / (name + ".json")
        if digest(metadata_path.read_bytes()) != expected:
            parser.error("test compatibility manifest hash mismatch: " + name)
        metadata = json.loads(metadata_path.read_text())
        patch = fixtures / (name + ".patch")
        if digest(patch.read_bytes()) != metadata["patch_sha256"]:
            parser.error("test compatibility patch hash mismatch: " + name)
        if any(not path.startswith("test/cctest/") or ".." in Path(path).parts
               for path in metadata["base_files"]):
            parser.error("compatibility changes must remain in cctest")
        actual = {path: digest((args.source / path).read_bytes())
                  for path in metadata["base_files"]}
        if actual == metadata["patched_files"]:
            continue
        if actual != metadata["base_files"]:
            parser.error("divergent test compatibility source: " + name)
        checked = subprocess.run(["git", "apply", "--check", str(patch)], cwd=args.source,
                                 capture_output=True)
        if checked.returncode:
            parser.error("test compatibility patch check failed: " + name)
        compatibility.append((patch, metadata))
    for patch, metadata in compatibility:
        subprocess.run(["git", "apply", str(patch)], cwd=args.source, check=True)
        for path, expected in metadata["patched_files"].items():
            if digest((args.source / path).read_bytes()) != expected:
                parser.error("test compatibility output mismatch: " + path)
    for destination, data in sources.items():
        destination.write_bytes(data)
    build.write_bytes(changed)


if __name__ == "__main__":
    main()

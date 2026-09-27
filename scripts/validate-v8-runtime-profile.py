#!/usr/bin/env python3
"""Validate current native V8 build inputs; this does not certify conformance."""
import argparse
import hashlib
import json
import importlib.util
from pathlib import Path

PATCHES = {
    "v8_patch_sha256": "window-proxy-structural-semantics",
    "shared_memory_growth_patch_sha256": "shared-memory-growth",
    "shared_memory_wait_patch_sha256": "shared-memory-wait",
    "shared_wasm_size_patch_sha256": "shared-wasm-size",
    "shared_wasm_backing_patch_sha256": "shared-wasm-backing",
    "shared_memory_factory_patch_sha256": "shared-memory-factory",
    "frame_context_reattachment_patch_sha256": "frame-context-reattachment",
    "html_constructor_deferred_prototype_patch_sha256": "html-constructor-deferred-prototype",
}
ARTIFACTS = ("archive", "binding", "icu_data", "startup_snapshot", "external_references", "worker", "bridge_abi", "rusty_v8_source_manifest")

def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()

def validate(manifest, root, profile=None, target=None, *, allow_relocated_source_manifest=False):
    def require(condition, message):
        if not condition:
            raise ValueError(message)
    require(manifest.get("schema_version") == 6, "current V8 profile schema 6 is required")
    require(manifest.get("profile") in ("core-full", "core-lite"), "invalid runtime profile")
    if profile:
        require(manifest["profile"] == profile, "runtime profile mismatch")
    require(isinstance(manifest.get("target"), str) and manifest["target"], "missing target")
    if target:
        require(manifest["target"] == target, "runtime target mismatch")
    require(manifest.get("v8_revision") == "ac1e23989121713ca642f6650b34deff7b686896", "V8 revision mismatch")
    require(manifest.get("rusty_v8_revision") == "5c15a6995c9bb4bacd3e341b59fff32c909c80bf", "rusty_v8 revision mismatch")
    require(manifest.get("v8_version") == "150.4.0", "V8 version mismatch")
    require(manifest.get("icu_revision") == "ee5f27adc28bd3f15b2c293f726d14d2e336cbd5", "ICU revision mismatch")
    require(manifest.get("v8_heap_sandbox") is True and manifest.get("pointer_compression") is True,
            "native sandbox and pointer compression are required")
    args = manifest.get("gn_args", "").split()
    for option in ("v8_drumbrake_bounds_checks=true", "v8_enable_sandbox=true", "v8_enable_pointer_compression=true"):
        require(option in args, "required engine option missing: " + option)
        name = option.split("=", 1)[0]
        require(sum(value.startswith(name + "=") for value in args) == 1, "conflicting engine option: " + name)
    for field, name in PATCHES.items():
        patch = root / "patches/v8" / (name + ".patch")
        metadata = json.loads(patch.with_suffix(".json").read_text())
        actual = digest(patch)
        require(manifest.get(field) == actual == metadata["patch_sha256"], "engine patch mismatch: " + name)
        for prerequisite, expected in metadata.get("required_patch_sha256", {}).items():
            require(digest(root / "patches/v8" / (prerequisite + ".patch")) == expected,
                    "engine prerequisite mismatch: " + prerequisite)
    regressions = root / "patches/v8/tests/manifest.json"
    require(manifest.get("v8_regression_manifest_sha256") == digest(regressions),
            "engine regression manifest mismatch")
    regression_metadata = json.loads(regressions.read_text())
    for name, expected in regression_metadata.get("required_patch_sha256", {}).items():
        require(digest(root / "patches/v8" / (name + ".patch")) == expected,
                "engine regression prerequisite mismatch: " + name)
    for name, expected in regression_metadata["sources"].items():
        require(digest(regressions.parent / name) == expected, "engine regression source mismatch: " + name)
    for name, expected in regression_metadata.get("test_only_compatibility", {}).items():
        metadata_path = regressions.parent / (name + ".json")
        require(digest(metadata_path) == expected, "test compatibility metadata mismatch: " + name)
        metadata = json.loads(metadata_path.read_text())
        require(digest(regressions.parent / (name + ".patch")) == metadata["patch_sha256"],
                "test compatibility patch mismatch: " + name)
    for name in ARTIFACTS:
        value = manifest.get(name + "_path")
        require(isinstance(value, str) and Path(value).is_absolute(), "absolute artifact path required: " + name)
        path = Path(value)
        require(path.is_file(), "artifact missing: " + name)
        require(path.stat().st_size == manifest.get(name + "_size_bytes"), "artifact size mismatch: " + name)
        require(digest(path) == manifest.get(name + "_sha256"), "artifact digest mismatch: " + name)
    spec = importlib.util.spec_from_file_location("rusty_v8_sources", root / "scripts/rusty_v8_source_provenance.py")
    sources = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sources)
    selected = sources.validate(root)
    for field, expected in selected.items():
        # The artifact loop above pins the retained manifest bytes. Every
        # non-path field still joins to sources.validate(root), including that
        # manifest's size/hash and the current Cargo selection, tree and patch.
        if field == "rusty_v8_source_manifest_path" and allow_relocated_source_manifest:
            continue
        require(manifest.get(field) == expected, "Rust V8 selected source mismatch: " + field)
    compatibility = manifest.get("rusty_v8_archive_compatibility")
    require(compatibility == dict(cpp_changes=False, crate_version="150.4.0",
                                 archive_sha256=manifest["archive_sha256"],
                                 binding_sha256=manifest["binding_sha256"]),
            "Rust V8 archive/binding compatibility mismatch")
    bridge = json.loads(Path(manifest["bridge_abi_path"]).read_text())
    require(bridge.get("schema_version") == 1, "bridge ABI schema mismatch")
    require(bridge.get("target") == manifest["target"], "bridge ABI target mismatch")
    require(bridge.get("libcxx_namespace") == "__Cr" and bridge.get("libcxx_abi_version") == 2,
            "bridge Chromium libc++ ABI mismatch")
    require(bridge.get("stdlib_linkage") == "included-in-rusty-v8", "bridge C++ runtime mismatch")
    require("-nostdinc++" in bridge.get("flags", []), "bridge system C++ headers forbidden")
    for key in ("archive", "binding"):
        require(bridge[key]["sha256"] == manifest[key + "_sha256"] and
                bridge[key]["size_bytes"] == manifest[key + "_size_bytes"],
                "bridge selected artifact mismatch: " + key)
    inputs = bridge.get("inputs", [])
    require(inputs and any(item["path"] == bridge.get("compiler") for item in inputs),
            "bridge compiler provenance missing")
    for item in inputs:
        path = Path(item["path"])
        require(path.is_absolute() and path.is_file(), "bridge retained input missing")
        require(path.stat().st_size == item["size_bytes"] and digest(path) == item["sha256"],
                "bridge retained input drift: " + str(path))
    return manifest

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--profile", choices=("core-full", "core-lite"))
    parser.add_argument("--target")
    parser.add_argument("--allow-relocated-source-manifest", action="store_true",
                        help="allow only byte-identical retained Rust source manifests at a different path")
    args = parser.parse_args()
    try:
        validate(json.loads(args.manifest.read_text()), Path(__file__).resolve().parents[1], args.profile, args.target,
                 allow_relocated_source_manifest=args.allow_relocated_source_manifest)
    except (ValueError, KeyError, OSError, TypeError) as error:
        parser.error(str(error))
    print("Current engine patches and runtime artifact inputs verified; qualification remains required.")

if __name__ == "__main__":
    main()

"""Profile rejection regression sources; run after the global generation gate."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("v8_profile", ROOT / "scripts/validate-v8-runtime-profile.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

class ProfileValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.profile = {
            "schema_version": 6, "profile": "core-full", "target": "aarch64-apple-darwin",
            "v8_version": "150.4.0", "v8_revision": "ac1e23989121713ca642f6650b34deff7b686896",
            "rusty_v8_revision": "5c15a6995c9bb4bacd3e341b59fff32c909c80bf",
            "icu_revision": "ee5f27adc28bd3f15b2c293f726d14d2e336cbd5",
            "v8_heap_sandbox": True, "pointer_compression": True,
            "gn_args": "v8_drumbrake_bounds_checks=true v8_enable_sandbox=true v8_enable_pointer_compression=true",
        }
        self.profile["v8_regression_manifest_sha256"] = MODULE.digest(ROOT / "patches/v8/tests/manifest.json")
        for field, name in MODULE.PATCHES.items():
            self.profile[field] = MODULE.digest(ROOT / "patches/v8" / (name + ".patch"))
        for name in MODULE.ARTIFACTS:
            path = Path(self.temp.name) / name
            path.write_bytes(name.encode())
            self.profile[name + "_path"] = str(path)
            self.profile[name + "_size_bytes"] = path.stat().st_size
            self.profile[name + "_sha256"] = MODULE.digest(path)

        import rusty_v8_source_provenance
        self.profile.update(rusty_v8_source_provenance.validate(ROOT))
        self.profile["rusty_v8_archive_compatibility"] = dict(cpp_changes=False, crate_version="150.4.0",
            archive_sha256=self.profile["archive_sha256"], binding_sha256=self.profile["binding_sha256"])

        compiler = Path(self.temp.name) / "compiler"
        compiler.write_bytes(b"pinned compiler")
        self.bridge = {"schema_version": 1, "target": self.profile["target"],
            "libcxx_namespace": "__Cr", "libcxx_abi_version": 2,
            "stdlib_linkage": "included-in-rusty-v8", "flags": ["-nostdinc++"],
            "compiler": str(compiler), "inputs": [{"path": str(compiler),
                "sha256": MODULE.digest(compiler), "size_bytes": compiler.stat().st_size}]}
        for key in ("archive", "binding"):
            self.bridge[key] = {"sha256": self.profile[key + "_sha256"],
                                "size_bytes": self.profile[key + "_size_bytes"]}
        self.write_bridge()

    def write_bridge(self):
        path = Path(self.profile["bridge_abi_path"])
        path.write_text(json.dumps(self.bridge))
        self.profile["bridge_abi_size_bytes"] = path.stat().st_size
        self.profile["bridge_abi_sha256"] = MODULE.digest(path)

    def test_bridge_selected_archive_and_retained_compiler_rejected_on_drift(self):
        self.bridge["archive"]["sha256"] = "0" * 64
        self.write_bridge()
        with self.assertRaisesRegex(ValueError, "bridge selected artifact mismatch"):
            MODULE.validate(self.profile, ROOT)
        self.bridge["archive"]["sha256"] = self.profile["archive_sha256"]
        self.write_bridge()
        Path(self.bridge["compiler"]).write_bytes(b"changed compiler")
        with self.assertRaisesRegex(ValueError, "bridge retained input drift"):
            MODULE.validate(self.profile, ROOT)

    def test_current_inputs_and_exact_target(self):
        MODULE.validate(self.profile, ROOT, "core-full", "aarch64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "target mismatch"):
            MODULE.validate(self.profile, ROOT, "core-full", "x86_64-unknown-linux-gnu")

    def test_each_missing_or_stale_patch_is_rejected(self):
        for field in MODULE.PATCHES:
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.profile)
                candidate[field] = "0" * 64
                with self.assertRaisesRegex(ValueError, "patch mismatch"):
                    MODULE.validate(candidate, ROOT)

    def test_missing_reattachment_and_regression_provenance_rejected(self):
        for field in ("frame_context_reattachment_patch_sha256", "html_constructor_deferred_prototype_patch_sha256", "v8_regression_manifest_sha256"):
            candidate = copy.deepcopy(self.profile)
            del candidate[field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                MODULE.validate(candidate, ROOT)

    def test_rust_source_and_generated_binding_compatibility_are_mandatory(self):
        for field in ("rusty_v8_source_tree_sha256", "rusty_v8_source_manifest_sha256",
                      "rusty_v8_weak_teardown_patch_sha256", "rusty_v8_patch_metadata_sha256"):
            candidate = copy.deepcopy(self.profile)
            candidate[field] = "0" * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                MODULE.validate(candidate, ROOT)
        candidate = copy.deepcopy(self.profile)
        candidate["rusty_v8_archive_compatibility"]["binding_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "compatibility mismatch"):
            MODULE.validate(candidate, ROOT)

    def test_modified_archive_rejected_even_if_length_unchanged(self):
        path = Path(self.profile["archive_path"])
        path.write_bytes(b"x" * path.stat().st_size)
        with self.assertRaisesRegex(ValueError, "digest mismatch: archive"):
            MODULE.validate(self.profile, ROOT)

    def test_old_schema_and_conflicting_bounds_options_rejected(self):
        candidate = copy.deepcopy(self.profile)
        candidate["schema_version"] = 1
        with self.assertRaisesRegex(ValueError, "schema 6"):
            MODULE.validate(candidate, ROOT)
        candidate = copy.deepcopy(self.profile)
        candidate["gn_args"] += " v8_drumbrake_bounds_checks=false"
        with self.assertRaisesRegex(ValueError, "conflicting engine option"):
            MODULE.validate(candidate, ROOT)

if __name__ == "__main__":
    unittest.main()

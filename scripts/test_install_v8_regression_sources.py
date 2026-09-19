"""Source-installation rejection cases; execute after the global generation gate."""
import hashlib
import difflib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

def digest(data):
    return hashlib.sha256(data).hexdigest()

class RegressionSourceInstallationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        self.script = self.root / "scripts/install-v8-regression-sources.py"
        shutil.copyfile(ROOT / "scripts/install-v8-regression-sources.py", self.script)
        self.fixtures = self.root / "patches/v8/tests"
        self.fixtures.mkdir(parents=True)
        self.source = self.root / "source"
        self.target = self.source / "test/cctest"
        self.target.mkdir(parents=True)
        self.base = b'sources = [\n    "test-api-array-buffer.cc",\n]\n'
        self.previous = self.base.replace(b']\n', b'    "test-oryn-frame-context-reattachment.cc",\n    "test-oryn-shared-memory-growth.cc",\n]\n')
        self.sources = {"test-oryn-frame-context-reattachment.cc": b"// frame\n",
                        "test-oryn-shared-memory-growth.cc": b"// memory\n",
                        "test-oryn-html-constructor.cc": b"// constructor\n"}
        additions = b"".join(('    "' + name + '",\n').encode() for name in sorted(self.sources))
        self.final = self.base.replace(b']\n', additions + b']\n')
        for name, data in self.sources.items():
            (self.fixtures / name).write_bytes(data)
        (self.fixtures / "manifest.json").write_text(json.dumps({
            "cctest_build_base_sha256": digest(self.base),
            "previous_cctest_build_sha256": digest(self.previous),
            "previous_sources": ["test-oryn-frame-context-reattachment.cc", "test-oryn-shared-memory-growth.cc"],
            "cctest_build_patched_sha256": digest(self.final),
            "sources": {name: digest(data) for name, data in self.sources.items()},
        }))
        (self.target / "BUILD.gn").write_bytes(self.base)

    def run_installer(self):
        return subprocess.run([sys.executable, str(self.script), str(self.source)], capture_output=True)

    def test_base_previous_and_current_installations_are_exact_and_idempotent(self):
        for original in (self.base, self.previous, self.final):
            with self.subTest(original=original):
                (self.target / "BUILD.gn").write_bytes(original)
                self.assertEqual(self.run_installer().returncode, 0)
                self.assertEqual((self.target / "BUILD.gn").read_bytes(), self.final)
                for name, data in self.sources.items():
                    self.assertEqual((self.target / name).read_bytes(), data)

    def test_complete_prerequisite_chain_uses_latest_overlapping_file(self):
        manifests = self.fixtures.parent
        original_patch, final_patch = b"original patch", b"final patch"
        (manifests / "original.patch").write_bytes(original_patch)
        (manifests / "final.patch").write_bytes(final_patch)
        (manifests / "original.json").write_text(json.dumps({
            "patch_sha256": digest(original_patch),
            "patched_files": {"engine.cc": digest(b"old"), "retained.h": digest(b"retained")},
        }))
        (manifests / "final.json").write_text(json.dumps({
            "patch_sha256": digest(final_patch),
            "required_patch_sha256": {"original": digest(original_patch)},
            "patched_files": {"engine.cc": digest(b"new")},
        }))
        manifest = json.loads((self.fixtures / "manifest.json").read_text())
        manifest["required_patch_sha256"] = {"final": digest(final_patch)}
        (self.fixtures / "manifest.json").write_text(json.dumps(manifest))
        (self.source / "engine.cc").write_bytes(b"new")
        (self.source / "retained.h").write_bytes(b"retained")
        self.assertEqual(self.run_installer().returncode, 0)
        (self.target / "BUILD.gn").write_bytes(self.base)
        (self.source / "retained.h").write_bytes(b"changed")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual((self.target / "BUILD.gn").read_bytes(), self.base)
        (self.source / "retained.h").write_bytes(b"retained")
        (manifests / "original.patch").write_bytes(b"tampered")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual((self.target / "BUILD.gn").read_bytes(), self.base)

    def test_divergent_existing_fixture_is_not_overwritten(self):
        path = self.target / "test-oryn-frame-context-reattachment.cc"
        path.write_bytes(b"// unrelated local changes\n")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual(path.read_bytes(), b"// unrelated local changes\n")
        self.assertEqual((self.target / "BUILD.gn").read_bytes(), self.base)

    def test_exact_previous_fixture_can_upgrade_without_accepting_other_edits(self):
        name = "test-oryn-frame-context-reattachment.cc"
        old = b"// exact previous fixture\n"
        manifest = json.loads((self.fixtures / "manifest.json").read_text())
        manifest["previous_source_sha256"] = {name: digest(old)}
        (self.fixtures / "manifest.json").write_text(json.dumps(manifest))
        (self.target / name).write_bytes(old)
        self.assertEqual(self.run_installer().returncode, 0)
        self.assertEqual((self.target / name).read_bytes(), self.sources[name])
        (self.target / name).write_bytes(old + b"// divergent\n")
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_zero_capacity_compatibility_is_exact_idempotent_and_rejects_drift(self):
        before = "uint8_t buffer[0];\nspan storage(buffer);\n"
        after = "uint8_t buffer[0];\nspan storage(buffer, sizeof(buffer));\n"
        relative = "test/cctest/test-api-array-buffer.cc"
        patch = "".join(difflib.unified_diff(before.splitlines(True), after.splitlines(True),
                                            fromfile="a/" + relative, tofile="b/" + relative))
        name = "zero-capacity"
        (self.fixtures / (name + ".patch")).write_text(patch)
        metadata = {"patch_sha256": digest(patch.encode()),
                    "base_files": {relative: digest(before.encode())},
                    "patched_files": {relative: digest(after.encode())}}
        metadata_path = self.fixtures / (name + ".json")
        metadata_path.write_text(json.dumps(metadata))
        manifest = json.loads((self.fixtures / "manifest.json").read_text())
        manifest["test_only_compatibility"] = {name: digest(metadata_path.read_bytes())}
        (self.fixtures / "manifest.json").write_text(json.dumps(manifest))
        source = self.source / relative
        source.write_text(before)
        self.assertEqual(self.run_installer().returncode, 0)
        self.assertEqual(source.read_text(), after)
        self.assertEqual(self.run_installer().returncode, 0)
        source.write_text(before + "// unrelated change\n")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual(source.read_text(), before + "// unrelated change\n")
        source.write_text(before)
        (self.fixtures / (name + ".patch")).write_text(patch + "tampered")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertEqual(source.read_text(), before)

    def test_changed_fixture_and_target_fail_before_publication(self):
        (self.fixtures / "test-oryn-frame-context-reattachment.cc").write_bytes(b"// altered\n")
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse((self.target / "test-oryn-shared-memory-growth.cc").exists())
        (self.target / "BUILD.gn").write_bytes(self.base + b"# authored change\n")
        self.assertNotEqual(self.run_installer().returncode, 0)

if __name__ == "__main__":
    unittest.main()

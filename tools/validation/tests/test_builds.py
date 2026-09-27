"""Changed build contracts; all fixture builds and signatures are simulated."""
import hashlib
import copy
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from oryn_validation import builds


class BuildContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def configuration(self):
        return dict(toolchain="rustc 1.89.0", host="aarch64-apple-darwin",
                    target="aarch64-apple-darwin", profile="release",
                    features=["core-full", "native"], flags=[], cargo_lock_sha256="a" * 64)

    def repository(self):
        root = self.root / "checkout"
        root.mkdir()
        for name in ("Cargo.toml", "Cargo.lock", "engine.rs"):
            (root / name).write_text("original\n")
        (root / ".gitignore").write_text("ignored.rs\ncache.dat\n")
        def git(*args):
            return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE)
        git("init", "-q")
        git("add", ".")
        git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c",
            "core.hooksPath=/dev/null", "commit", "--no-gpg-sign", "-qm", "fixture")
        return root

    def admit(self, root, **kwargs):
        return builds.audit_source(root, effective_inputs=["Cargo.toml", "Cargo.lock", "engine.rs"], **kwargs)

    def test_clean_commit_and_dirty_source(self):
        root = self.repository()
        result = self.admit(root)
        self.assertFalse(result["dirty"])
        (root / "engine.rs").write_text("changed!\n")
        with self.assertRaisesRegex(ValueError, "clean committed"):
            self.admit(root)
        self.assertTrue(self.admit(root, official=False)["development_only"])

    def test_assume_unchanged_cannot_hide_modified_committed_source(self):
        root = self.repository()
        subprocess.run(["git", "-C", str(root), "update-index", "--assume-unchanged", "engine.rs"], check=True)
        (root / "engine.rs").write_text("hidden changes\n")
        with self.assertRaisesRegex(ValueError, "committed Git blob"):
            self.admit(root)

    def test_ignored_effective_source_requires_explicit_pin(self):
        root = self.repository()
        (root / "ignored.rs").write_text("changes native behavior\n")
        effective = ["Cargo.toml", "Cargo.lock", "engine.rs", "ignored.rs"]
        with self.assertRaisesRegex(ValueError, "not explicitly pinned"):
            builds.audit_source(root, effective_inputs=effective)
        external = [{"path": "ignored.rs", "sha256": hashlib.sha256((root / "ignored.rs").read_bytes()).hexdigest()}]
        self.assertIn("ignored.rs", self.admit(root, external_inputs=external)["external_inputs"])
        (root / "ignored.rs").write_text("drift")
        with self.assertRaisesRegex(ValueError, "digest mismatch"):
            self.admit(root, external_inputs=external)

    def test_cache_diagnostic_is_not_input_exemption(self):
        root = self.repository()
        (root / "ignored.rs").write_text("source")
        result = self.admit(root)
        self.assertIn("ignored.rs", result["ignored_entries_diagnostic"])
        self.assertNotIn("ignored.rs", result["tracked"])
        with self.assertRaisesRegex(ValueError, "not explicitly pinned"):
            builds.audit_source(root, effective_inputs=["Cargo.toml", "Cargo.lock", "ignored.rs"])

    def test_cache_does_not_grow_per_commit_or_lockfile(self):
        config = self.configuration()
        key = builds.cargo_cache_key(config)
        config.update(source_commit="b" * 40, cargo_lock_sha256="c" * 64)
        self.assertEqual(key, builds.cargo_cache_key(config))
        config["features"].reverse()
        self.assertEqual(key, builds.cargo_cache_key(config))
        config["flags"] = ["-Ctarget-cpu=native"]
        self.assertNotEqual(key, builds.cargo_cache_key(config))

    def test_native_digest_binds_all_but_exact_runner_paths(self):
        source = {"tracked": {"Cargo.lock": {"sha256": "a" * 64}, "engine.rs": {"sha256": "a"}, "tools/validation/run.py": {"sha256": "b"}},
                  "external_inputs": {}}
        config = self.configuration()
        exclusions = ["tools/validation/run.py"]
        before = builds.native_inputs_digest(source, config, runner_only_paths=exclusions)
        source["tracked"][exclusions[0]]["sha256"] = "c"
        self.assertEqual(before, builds.native_inputs_digest(source, config, runner_only_paths=exclusions))
        source["tracked"]["engine.rs"]["sha256"] = "d"
        self.assertNotEqual(before, builds.native_inputs_digest(source, config, runner_only_paths=exclusions))
        with self.assertRaises(ValueError):
            builds.native_inputs_digest(source, config, runner_only_paths=["engine.rs"])
        config["cargo_lock_sha256"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "lockfile"):
            builds.native_inputs_digest(source, config, runner_only_paths=exclusions)

    def test_reuse_preserves_build_provenance_and_rejects_changed_native_identity(self):
        original = {"schema_version": 1, "kind": "build", "build_id": "original",
                    "source": {"commit": "a" * 40, "tree_sha256": "a" * 64, "dirty": False},
                    "build_inputs_sha256": "c" * 64, "configuration": self.configuration(),
                    "commands": [["/bin/bash", "scripts/build-sandboxed-worker.sh"]],
                    "build_inputs": [], "immutable_inputs": [], "artifacts": [{"scope": "bundle", "path": "bin/worker",
                    "kind": "file", "sha256": "d" * 64, "size": 7, "executable": True}],
                    "signatures": self.signature(None), "development_only": False}
        current = copy.deepcopy(original)
        current["source"]["commit"] = "b" * 40
        proof = builds.native_reuse_proof(original, current)
        self.assertEqual(proof["original_source_commit"], "a" * 40)
        self.assertEqual(proof["runner_source_commit"], "b" * 40)
        for change in (lambda doc: doc.update(build_inputs_sha256="e" * 64),
                       lambda doc: doc["configuration"].update(toolchain="other"),
                       lambda doc: doc.update(development_only=True),
                       lambda doc: doc["artifacts"][0].update(sha256="f" * 64),
                       lambda doc: doc["build_inputs"].append(dict(doc["artifacts"][0], scope="store")),
                       lambda doc: doc["immutable_inputs"].append(dict(doc["artifacts"][0], scope="store"))):
            invalid = copy.deepcopy(current)
            change(invalid)
            with self.assertRaises(ValueError):
                builds.native_reuse_proof(original, invalid)

    def signature(self, root):
        # Unit-test seam only; production uses the fresh codesign adapter.
        return [{"path": "bin/worker", "verifier": "test-fixture", "identity": "fixture", "valid": True}]

    def seal(self):
        source = self.root / "worker"
        source.write_bytes(b"runtime")
        source.chmod(0o755)
        destination = self.root / "bundle"
        result = builds.seal_bundle(self.root, destination, {"bin/worker": source}, signature_verifier=self.signature)
        self.addCleanup(self.unseal, destination)
        return source, destination, result

    @staticmethod
    def unseal(path):
        if path.exists():
            path.chmod(0o755)
            for item in path.rglob("*"):
                item.chmod(0o755 if item.is_dir() else 0o644)

    def test_copy_is_independent_and_relocation_preserves_identity(self):
        source, destination, receipt = self.seal()
        self.assertNotEqual(source.stat().st_ino, (destination / "bin/worker").stat().st_ino)
        source.write_bytes(b"modified cache")
        relocated = self.root / "other-bundle"
        shutil.copytree(destination, relocated)
        self.addCleanup(self.unseal, relocated)
        self.assertTrue(builds.verify_sealed_bundle(relocated, receipt, signature_verifier=self.signature)["valid"])
        self.assertNotIn(str(self.root), json.dumps(receipt))

    def test_same_size_change_and_empty_directory_drift_rejected(self):
        _, destination, receipt = self.seal()
        self.unseal(destination)
        (destination / "new-empty").mkdir()
        with self.assertRaisesRegex(ValueError, "membership"):
            builds.verify_sealed_bundle(destination, receipt, signature_verifier=self.signature)
        (destination / "new-empty").rmdir()
        (destination / "bin/worker").write_bytes(b"RUNTIME")
        with self.assertRaisesRegex(ValueError, "membership"):
            builds.verify_sealed_bundle(destination, receipt, signature_verifier=self.signature)

    def test_symlink_source_and_missing_signature_rejected(self):
        actual = self.root / "actual"
        actual.write_bytes(b"bytes")
        link = self.root / "link"
        link.symlink_to(actual)
        with self.assertRaisesRegex(ValueError, "symlink"):
            builds.seal_bundle(self.root, self.root / "bad-link", {"bin/worker": link}, signature_verifier=self.signature)
        with self.assertRaisesRegex(ValueError, "signature"):
            builds.seal_bundle(self.root, self.root / "unsigned", {"bin/worker": actual}, signature_verifier=lambda root: [])

    def test_signature_drift_rejected_at_admission(self):
        _, destination, receipt = self.seal()
        with self.assertRaisesRegex(ValueError, "signature"):
            builds.verify_sealed_bundle(destination, receipt, signature_verifier=lambda root: [])


if __name__ == "__main__":
    unittest.main()

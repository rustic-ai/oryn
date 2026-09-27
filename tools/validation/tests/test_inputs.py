"""Input custody tests; no external dependencies, tools, or legacy candidates."""
import json
import os
import tempfile
import unittest
from pathlib import Path

from oryn_validation.inputs import InputError, InputLease, snapshot_input, verify_input, verify_inputs
from oryn_validation.manifests import ManifestError


class InputTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        # macOS /var may be an alias: configured roots must be canonical.
        self.root = Path(self.tmp.name).resolve()
        (self.root / "dataset" / "empty").mkdir(parents=True)
        (self.root / "dataset" / "test.js").write_bytes(b"first")
        self.record = snapshot_input(self.root, "dataset", kind="directory")

    def tearDown(self):
        self.tmp.cleanup()

    def test_same_size_mutation_with_restored_mtime_is_rejected(self):
        target = self.root / "dataset" / "test.js"
        before = target.stat()
        target.write_bytes(b"other")
        os.utime(target, ns=(before.st_atime_ns, before.st_mtime_ns))
        with self.assertRaises(InputError):
            verify_input(self.record, store_root=self.root)

    def test_undeclared_file_and_empty_directory_are_rejected(self):
        for name, directory in (("extra.js", False), ("extra", True)):
            target = self.root / "dataset" / name
            target.mkdir() if directory else target.write_bytes(b"extra")
            with self.assertRaises(InputError):
                verify_input(self.record, store_root=self.root)
            target.rmdir() if directory else target.unlink()

    def test_missing_membership_is_rejected(self):
        (self.root / "dataset" / "empty").rmdir()
        with self.assertRaises(InputError):
            verify_input(self.record, store_root=self.root)

    def test_symlink_leaf_and_parent_are_rejected(self):
        (self.root / "alias").symlink_to(self.root / "dataset", target_is_directory=True)
        with self.assertRaises(InputError):
            snapshot_input(self.root, "alias/test.js")
        (self.root / "dataset" / "link").symlink_to(self.root / "dataset" / "test.js")
        with self.assertRaises(InputError):
            snapshot_input(self.root, "dataset", kind="directory")

    def test_escape_and_noncanonical_paths_are_rejected(self):
        for path in ("../dataset", "/dataset", "dataset/../dataset", "dataset//test.js", "dataset/./test.js", "C:/foo"):
            with self.subTest(path=path), self.assertRaises(ManifestError):
                snapshot_input(self.root, path)

    def test_special_file_is_rejected_without_opening_it(self):
        os.mkfifo(self.root / "dataset" / "fifo")
        with self.assertRaises(InputError):
            snapshot_input(self.root, "dataset", kind="directory")

    def test_status_and_historical_json_do_not_expand_inputs(self):
        (self.root / "history.json").write_text(json.dumps({"artifact_path": "/missing/old/candidate"}))
        with InputLease([self.record], store_root=self.root) as lease:
            (self.root / "status.json").write_text('{"running": true}')
            (self.root / "history.json").write_text('{"artifact_path": "/other/missing"}')
            lease.checkpoint()
        self.assertEqual(lease.finalization["input_count"], 1)

    def test_declared_json_is_hashed_but_its_paths_not_followed(self):
        (self.root / "manifest.json").write_text('{"runtime_path": "/missing"}')
        record = snapshot_input(self.root, "manifest.json")
        self.assertTrue(verify_inputs([record], store_root=self.root)["valid"])

    def test_relocation_preserves_bundle_identity(self):
        import shutil
        with tempfile.TemporaryDirectory() as destination:
            relocated = Path(destination).resolve()
            shutil.copytree(self.root / "dataset", relocated / "dataset")
            self.assertTrue(verify_inputs([self.record], store_root=relocated)["valid"])

    def test_context_finalization_detects_drift_and_preserves_primary_error(self):
        with self.assertRaisesRegex(RuntimeError, "primary"):
            with InputLease([self.record], store_root=self.root) as lease:
                (self.root / "dataset" / "test.js").write_bytes(b"other")
                raise RuntimeError("primary")
        self.assertIsInstance(lease.final_error, InputError)
        self.assertIsNone(lease.finalization)

    def test_duplicate_input_and_missing_bundle_root_rejected(self):
        with self.assertRaises(InputError):
            verify_inputs([self.record, self.record], store_root=self.root)
        record = dict(self.record, scope="bundle")
        with self.assertRaises(InputError):
            verify_input(record, store_root=self.root)

    def test_context_exit_rechecks_after_explicit_final_check(self):
        with self.assertRaises(InputError):
            with InputLease([self.record], store_root=self.root) as lease:
                lease.final_check()
                (self.root / "dataset" / "test.js").write_bytes(b"other")

    def test_final_verification_detects_file_and_member_executable_drift(self):
        target = self.root / "dataset" / "test.js"
        original_mode = target.stat().st_mode
        file_record = snapshot_input(self.root, "dataset/test.js")
        for record in (file_record, self.record):
            with self.subTest(kind=record["kind"]), self.assertRaises(InputError):
                with InputLease([record], store_root=self.root):
                    target.chmod(original_mode ^ 0o100)
            target.chmod(original_mode)

    def test_member_digest_cannot_be_replaced_without_membership_digest(self):
        import copy
        record = copy.deepcopy(self.record)
        record["members"][0]["sha256"] = "0" * 64
        with self.assertRaises(ManifestError):
            verify_input(record, store_root=self.root)


if __name__ == "__main__":
    unittest.main()

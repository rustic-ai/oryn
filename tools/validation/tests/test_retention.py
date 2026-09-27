"""Retention tests operate only on disposable temporary fixture caches."""
import json
from pathlib import Path
import tempfile
import unittest

from oryn_validation.retention import execute_cleanup, preview_cleanup, restore_cache_registration


class RetentionContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.index = {"schema_version": 1, "objects": {}, "leases": []}

    def object(self, key, *, kind="cargo-cache", pinned=False, dependencies=()):
        relative = "cache/cargo/" + key if kind == "cargo-cache" else kind + "/" + key
        path = self.root / relative
        path.mkdir(parents=True)
        (path / "data").write_bytes(b"data")
        self.index["objects"][key] = {"kind": kind, "path": relative, "pinned": pinned,
                                       "dependencies": list(dependencies), "last_used_ns": 0}
        return path

    def preview(self):
        return preview_cleanup(self.root, self.index, cache_budget_bytes=0)

    def test_pin_and_active_or_unsettled_lease_transitive_closure(self):
        for state in ("active", "unsettled"):
            with self.subTest(state=state):
                if not self.index["objects"]:
                    self.object("cache")
                    self.object("runtime", kind="build", dependencies=["cache"])
                    self.object("other")
                self.index["leases"] = [{"lease_id": "owned", "state": state, "objects": ["runtime"]}]
                preview = self.preview()
                self.assertEqual([row["object_id"] for row in preview["removals"]], ["other"])
                self.assertFalse(preview["budget_satisfied"])
        self.index["leases"] = []
        self.index["objects"]["runtime"]["pinned"] = True
        self.assertIn("cache", self.preview()["protected_objects"])

    def test_evidence_is_never_deleted(self):
        self.object("failed-original", kind="run")
        self.object("old-build", kind="build")
        self.assertEqual(self.preview()["removals"], [])

    def test_path_escape_overlap_and_symlink_fail_closed(self):
        cache = self.object("cache")
        (cache / "link").symlink_to(self.root)
        with self.assertRaises(ValueError):
            self.preview()
        (cache / "link").unlink()
        self.index["objects"]["cache"]["path"] = "../escape"
        with self.assertRaises(ValueError):
            self.preview()
        self.index["objects"]["cache"]["path"] = "cache/cargo/cache"
        self.index["objects"]["overlap"] = dict(self.index["objects"]["cache"])
        with self.assertRaisesRegex(ValueError, "overlap"):
            self.preview()

    def execute(self, preview, callback=lambda objects: True):
        index_path = self.root / "index.json"
        index_path.write_text(json.dumps(self.index))
        return execute_cleanup(self.root, index_path, preview,
                               expected_preview_sha256=preview["preview_sha256"],
                               receipt_path=self.root / "cleanup.json", confirm_no_active_writers=callback)

    def test_new_lease_invalidates_preview(self):
        cache = self.object("cache")
        preview = self.preview()
        self.index["leases"] = [{"lease_id": "new", "state": "active", "objects": ["cache"]}]
        with self.assertRaisesRegex(ValueError, "changed"):
            self.execute(preview)
        self.assertTrue(cache.exists())

    def test_cache_drift_and_unconfirmed_process_owner_refuse_delete(self):
        cache = self.object("cache")
        preview = self.preview()
        (cache / "data").write_bytes(b"drift")
        with self.assertRaisesRegex(ValueError, "changed"):
            self.execute(preview)
        with self.assertRaisesRegex(ValueError, "confirmation"):
            self.execute(self.preview(), callback=lambda objects: False)
        self.assertTrue(cache.exists())

    def test_exact_removal_with_actual_space_receipt(self):
        cache = self.object("cache")
        evidence = self.object("original", kind="run")
        result = self.execute(self.preview())
        self.assertTrue(result["complete"])
        self.assertFalse(cache.exists())
        self.assertTrue(evidence.exists())
        self.assertEqual(json.loads((self.root / "cleanup.json").read_text()), result)
        self.assertIsInstance(result["actual_free_bytes_change"], int)
        updated = json.loads((self.root / "index.json").read_text())
        self.assertEqual(preview_cleanup(self.root, updated, cache_budget_bytes=0)["removals"], [])
        cache.mkdir()
        with self.assertRaisesRegex(ValueError, "re-registration"):
            preview_cleanup(self.root, updated, cache_budget_bytes=0)
        restored = restore_cache_registration(self.root, self.root / "index.json", "cache")
        self.assertEqual(restored["registered"], "cache")
        self.assertNotIn("removed_ns", json.loads((self.root / "index.json").read_text())["objects"]["cache"])


if __name__ == "__main__":
    unittest.main()

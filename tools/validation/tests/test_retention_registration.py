import json
from pathlib import Path
import tempfile
import unittest

from oryn_validation.retention import register_object


class RegistrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        (self.root / "builds/one/runtime").mkdir(parents=True)
        (self.root / "environments/python").mkdir(parents=True)

    def test_explicit_dependency_pin_and_identity_replacement_refusal(self):
        register_object(self.root, "python", path="environments/python", kind="environment")
        value = register_object(self.root, "build:one", path="builds/one", kind="build",
                                dependencies=["python"], pinned=True)
        self.assertEqual(value["dependencies"], ["python"])
        self.assertTrue(value["pinned"])
        with self.assertRaisesRegex(ValueError, "already registered"):
            register_object(self.root, "build:one", path="builds/one", kind="build")

    def test_overlapping_roots_and_unknown_dependencies_rejected_without_index_mutation(self):
        register_object(self.root, "build:one", path="builds/one", kind="build")
        before = (self.root / "retention.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "overlap"):
            register_object(self.root, "nested", path="builds/one/runtime", kind="build")
        with self.assertRaisesRegex(ValueError, "dependency"):
            register_object(self.root, "python", path="environments/python", kind="environment",
                            dependencies=["missing"])
        self.assertEqual((self.root / "retention.json").read_bytes(), before)

    def test_symlink_and_invalid_retention_class_rejected(self):
        (self.root / "alias").symlink_to(self.root / "environments/python", target_is_directory=True)
        with self.assertRaises(ValueError):
            register_object(self.root, "alias", path="alias", kind="environment")
        with self.assertRaises(ValueError):
            register_object(self.root, "bad", path="environments/python", kind="unknown")
        self.assertFalse((self.root / "retention.json").exists())


if __name__ == "__main__":
    unittest.main()

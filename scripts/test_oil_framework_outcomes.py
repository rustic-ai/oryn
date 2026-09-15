"""Framework evidence must demonstrate state change and preserve diagnostics."""

import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "framework_runner", ROOT / "scripts/run-oil-framework-outcomes.py"
)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class FrameworkEvidenceTests(unittest.TestCase):
    def frames(self):
        return [
            dict(kind="observation", observation={}),
            dict(kind="action", result=dict(action_id=1, execution_domain="native")),
            dict(kind="structured", value="Items: 1"),
            dict(kind="trace", events=[]),
        ]

    def test_framework_state_change_passes(self):
        self.assertEqual(runner.validate("vue", 'click "Add item"', self.frames()), [])

    def test_ssr_only_errors_missing_trace_and_wrong_action_fail(self):
        for corruption in ("ssr", "diagnostic", "trace", "action", "empty"):
            frames = self.frames()
            if corruption == "ssr":
                frames[2]["value"] = "Items: 0"
            elif corruption == "diagnostic":
                frames[-1]["events"] = [dict(kind=dict(kind="script_diagnostic", detail="failed"))]
            elif corruption == "trace":
                frames.pop()
            elif corruption == "action":
                frames[1]["result"]["execution_domain"] = "chromium"
            else:
                frames = []
            with self.subTest(corruption=corruption):
                self.assertTrue(runner.validate("vue", 'click "Add item"', frames))

    def test_react_requires_full_declared_surface(self):
        self.assertTrue(runner.validate("react", 'click "Add item"', self.frames()))
        frames = self.frames()
        frames.insert(
            3,
            dict(
                kind="structured",
                value="Surface: oryn|tools|2026-08-27|2|1|fixture.txt:7|loaded|closed",
            ),
        )
        self.assertEqual(runner.validate("react", 'click "Add item"', frames), [])
        corrupted = copy.deepcopy(frames)
        corrupted[3]["value"] = corrupted[3]["value"].replace("loaded", "idle")
        self.assertTrue(runner.validate("react", 'click "Add item"', corrupted))

    def test_artifact_inventory_binds_paths_bytes_and_entry_point(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                runner.build.inventory(root)
            (root / "index.html").write_text("<main>real output</main>")
            (root / "app.js").write_text("export const value=1")
            first = runner.build.inventory(root)
            (root / "app.js").write_text("export const value=2")
            self.assertNotEqual(first["sha256"], runner.build.inventory(root)["sha256"])
            (root / "app.js").rename(root / "other.js")
            self.assertNotEqual(first["sha256"], runner.build.inventory(root)["sha256"])
            (root / "linked.js").symlink_to(root / "other.js")
            with self.assertRaises(ValueError):
                runner.build.inventory(root)


if __name__ == "__main__":
    unittest.main()

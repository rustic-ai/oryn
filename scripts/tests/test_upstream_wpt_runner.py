from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "run_upstream_wpt", ROOT / "scripts/run-upstream-wpt.py"
)
assert SPEC is not None and SPEC.loader is not None
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)
INVENTORY_SPEC = importlib.util.spec_from_file_location(
    "build_runtime_conformance_inventory",
    ROOT / "scripts/build-runtime-conformance-inventory.py",
)
assert INVENTORY_SPEC is not None and INVENTORY_SPEC.loader is not None
INVENTORY = importlib.util.module_from_spec(INVENTORY_SPEC)
INVENTORY_SPEC.loader.exec_module(INVENTORY)


class UpstreamWptRunnerTests(unittest.TestCase):
    def test_meta_scripts_are_resolved_in_declared_order(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "IndexedDB/case.any.js"
            local = root / "IndexedDB/resources/support.js"
            common = root / "common/dispatcher/dispatcher.js"
            local.parent.mkdir(parents=True)
            common.parent.mkdir(parents=True)
            source.write_text("// META: script=resources/support.js\n// META: script=/common/dispatcher/dispatcher.js\n")
            local.write_text("local")
            common.write_text("common")
            self.assertEqual(
                RUNNER.meta_script_paths(root, source, source.read_text()),
                [
                    "/IndexedDB/resources/support.js",
                    "/common/dispatcher/dispatcher.js",
                ],
            )

    def test_meta_script_cannot_escape_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "case.any.js"
            source.write_text("// META: script=../outside.js\n")
            with self.assertRaisesRegex(ValueError, "invalid WPT META script"):
                RUNNER.meta_script_paths(root, source, source.read_text())

    def test_partial_checkpoint_is_never_reported_as_passed(self) -> None:
        assertions = [{"id": "one", "status": "passed"}]
        self.assertEqual(RUNNER.outcome(assertions, 2, False)["status"], "running")
        self.assertEqual(RUNNER.outcome(assertions, 2, True)["status"], "failed")
        self.assertEqual(RUNNER.outcome(assertions, 1, True)["status"], "passed")

    def test_html_variants_are_all_preserved(self) -> None:
        source = """
        <meta name=variant content=?1-1000>
        <meta content='?1001-last' name='variant'>
        """
        self.assertEqual(
            INVENTORY.html_variants(source), ["?1-1000", "?1001-last"]
        )
        self.assertEqual(INVENTORY.html_variants("<title>no variants</title>"), [""])


if __name__ == "__main__":
    unittest.main()

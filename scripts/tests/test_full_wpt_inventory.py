import importlib.util
import json
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("full_inventory", Path(__file__).resolve().parents[1] / "prepare-full-wpt-baseline.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FullInventoryTests(unittest.TestCase):
    def test_priority_is_independent_of_runner_and_environment(self):
        policy = json.loads((MODULE.ROOT / "benchmarks/conformance/wpt-capability-tiers-v2.json").read_text())
        for path in ["html/semantics/embedded-content/the-iframe-element/a.html", "FileAPI/a.any.js", "streams/a.any.js", "websockets/a.html", "trusted-types/a.html"]:
            self.assertEqual(MODULE.tiers.classify(path, policy), ("core", False))
        self.assertEqual(MODULE.tiers.classify("webauthn/a.html", policy), ("necessary", False))
        self.assertEqual(MODULE.tiers.classify("unreviewed/a.html", policy), ("extended", True))
        self.assertNotIn("platform_dependent", policy["tiers"])

    def test_official_generated_url_preserves_variant(self):
        self.assertEqual(MODULE.runner_mapping("testharness", "dom/a.any.js", "/dom/a.any.worker.html?x=1", {}), (("dedicatedworker", "?x=1"), None))

    def test_module_and_service_workers_are_visible_unsupported(self):
        for suffix in ["serviceworker", "worker-module"]:
            mapping, reason = MODULE.runner_mapping("testharness", "a.any.js", f"/a.any.{suffix}.html", {})
            self.assertIsNone(mapping)
            self.assertIn("generated_global", reason)

    def test_reference_test_is_not_an_html_execution(self):
        self.assertEqual(MODULE.runner_mapping("reftest", "a.html", "/a.html", {})[0], None)

    def test_manifest_trie_keeps_source_path_and_items(self):
        value = ["hash", [None, {}], ["/a?variant", {}]]
        self.assertEqual(list(MODULE.leaves({"dom": {"a.html": value}})), [("dom/a.html", value)])


if __name__ == "__main__":
    unittest.main()

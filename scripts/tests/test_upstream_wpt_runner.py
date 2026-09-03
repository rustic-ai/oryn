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

    def test_wptserve_tokens_are_substituted_for_html_and_scripts(self) -> None:
        source = (
            "http://{{host}}:{{ports[http][0]}}/same "
            "http://{{hosts[alt][]}}:{{ports[http][1]}}/cross"
        )
        self.assertEqual(
            RUNNER.substitute_wpt_tokens(source, "localhost", 8443),
            "http://localhost:8443/same http://127.0.0.1:8443/cross",
        )

    def test_fs_path_tokens_resolve_only_inside_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "IndexedDB/file_support.any.js"
            asset = root / "IndexedDB/resources/file_to_save.txt"
            asset.parent.mkdir(parents=True)
            asset.write_text("upload body")
            source.write_text("{{fs_path(resources/file_to_save.txt)}}")
            self.assertEqual(
                RUNNER.substitute_wpt_tokens(
                    source.read_text(), "localhost", 8443, root, source
                ),
                str(asset),
            )
            self.assertEqual(RUNNER.fs_path_assets(root, source), [asset])

    def test_fs_path_token_cannot_escape_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "case.any.js"
            source.write_text("{{fs_path(../outside.txt)}}")
            with self.assertRaisesRegex(ValueError, "invalid WPT fs_path"):
                RUNNER.fs_path_assets(root, source)

    def test_html_variants_are_all_preserved(self) -> None:
        source = """
        <meta name=variant content=?1-1000>
        <meta content='?1001-last' name='variant'>
        """
        self.assertEqual(
            INVENTORY.html_variants(source), ["?1-1000", "?1001-last"]
        )
        self.assertEqual(INVENTORY.html_variants("<title>no variants</title>"), [""])

    def test_runner_expands_declared_html_variants(self) -> None:
        source = """
        <meta name=variant content=?editor=input>
        <meta content='?editor=textarea' name='variant'>
        """
        self.assertEqual(
            RUNNER.declared_html_variants(source),
            ["?editor=input", "?editor=textarea"],
        )

    def test_discovered_html_tests_expand_declared_variants(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            test = root / "editing/variant.html"
            test.parent.mkdir(parents=True)
            test.write_text(
                '<meta name="variant" content="?editor=input">\n'
                '<meta name="variant" content="?editor=textarea">\n'
                '<script src="/resources/testharness.js"></script>'
            )
            selected = [
                (path.as_posix(), variant)
                for path, _classification, _global, variant in RUNNER.applicable_tests(root)
            ]
            self.assertEqual(
                selected,
                [
                    ("editing/variant.html", "?editor=input"),
                    ("editing/variant.html", "?editor=textarea"),
                ],
            )

    def test_completion_channel_is_outside_hidden_testharness_body(self) -> None:
        self.assertIn(
            "document.documentElement.appendChild(output)", RUNNER.COMPLETION
        )
        self.assertIn("display:block!important", RUNNER.COMPLETION)

    def test_testdriver_click_wrapper_remains_constructable(self) -> None:
        # Some pinned editing tests use the historically accepted
        # `new test_driver.click(element)` spelling. An arrow-function wrapper
        # rejects that call before it can reach the native automation bridge.
        source = (ROOT / "scripts/run-upstream-wpt.py").read_text()
        self.assertIn("window.test_driver.click = function(element)", source)
        self.assertNotIn("window.test_driver.click = element =>", source)
        self.assertIn(
            "testdriver:globalThis.__oryn_testdriver_log || []", RUNNER.COMPLETION
        )

    def test_html_applicability_excludes_crash_and_helper_pages(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            test = root / "editing/event.html"
            crash = root / "editing/crashtests/crash.html"
            helper = root / "editing/resources/helper.html"
            for path in (test, crash, helper):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('<script src="/resources/testharness.js"></script>')
            plain = root / "editing/plain.html"
            plain.write_text("<title>fixture only</title>")
            self.assertTrue(RUNNER.is_testharness_html(test))
            self.assertFalse(RUNNER.is_testharness_html(crash))
            self.assertFalse(RUNNER.is_testharness_html(helper))
            self.assertFalse(RUNNER.is_testharness_html(plain))


if __name__ == "__main__":
    unittest.main()

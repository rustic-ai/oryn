from __future__ import annotations

import importlib.util
import inspect
import json
import hashlib
import tempfile
import unittest
from unittest import mock
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
    def test_untracked_source_content_and_names_affect_fingerprint(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "new.rs"
            source.write_text("first")
            with mock.patch.object(RUNNER.subprocess, "check_output", return_value=b"new.rs\0"):
                first, count = RUNNER.untracked_source_digest(root)
                self.assertEqual(count, 1)
                source.write_text("second")
                second, _ = RUNNER.untracked_source_digest(root)
                self.assertNotEqual(first, second)
            source.rename(root / "renamed.rs")
            with mock.patch.object(RUNNER.subprocess, "check_output", return_value=b"renamed.rs\0"):
                renamed, _ = RUNNER.untracked_source_digest(root)
                self.assertNotEqual(second, renamed)

    def test_locked_inventory_is_authoritative_for_runner_and_validator(self) -> None:
        validator_spec = importlib.util.spec_from_file_location(
            "validate_runtime_result",
            ROOT / "scripts/validate-runtime-api-style-semantics-result.py",
        )
        validator = importlib.util.module_from_spec(validator_spec)
        validator_spec.loader.exec_module(validator)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "case.html"
            source.write_text('<meta name="variant" content="?discovered">')
            inventory = {
                "upstream_revision": json.loads(RUNNER.EXPANDED_MANIFEST.read_text())["upstream"]["revision"],
                "tests": [{
                    "path": "case.html",
                    "sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                    "globals": ["window", "dedicatedworker", "sharedworker", "serviceworker"],
                    "variants": [""],
                    "workstream": "generated_webidl",
                }],
            }
            path = root / "inventory.json"
            path.write_text(json.dumps(inventory))
            actual = {f"{path}|{global_name}|{variant}" for path, _, global_name, variant
                      in RUNNER.inventoried_tests(root, path)}
            self.assertEqual(actual, validator.expected_assertions(inventory))
            self.assertEqual(len(actual), 4)
            self.assertTrue(all(identifier.endswith("|") for identifier in actual))
            inventory["tests"][0]["variants"] = ["?one", "?two"]
            path.write_text(json.dumps(inventory))
            actual = {f"{path}|{global_name}|{variant}" for path, _, global_name, variant
                      in RUNNER.inventoried_tests(root, path)}
            self.assertEqual(actual, validator.expected_assertions(inventory))
            self.assertEqual(len(actual), 8)
            source.write_text("changed upstream fixture")
            with self.assertRaisesRegex(SystemExit, "source digest mismatch"):
                RUNNER.inventoried_tests(root, path)

    def test_release_validator_requires_real_harness_completion(self) -> None:
        spec = importlib.util.spec_from_file_location(
            "validate_runtime_result", ROOT / "scripts/validate-runtime-api-style-semantics-result.py"
        )
        validator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(validator)
        valid = {"status": "passed", "result": {
            "complete": True, "harness": 0, "tests": [{"name": "one", "status": 0}]
        }}
        validator.validate_passed_assertion(valid, "test")
        for updates in [{"complete": False}, {"harness": 1}, {"tests": []},
                        {"tests": [{"status": 1}]}]:
            invalid = {**valid, "result": {**valid["result"], **updates}}
            with self.subTest(updates=updates), self.assertRaises(SystemExit):
                validator.validate_passed_assertion(invalid, "test")
        with self.assertRaises(SystemExit):
            validator.validate_passed_assertion(
                {**valid, "result_transport": "isolated_dom_probe"}, "test"
            )
        validator.validate_passed_assertion({"status": "failed"}, "test")

    def test_open_ended_editing_range_runs_beyond_one_thousand_assertions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "editing/run/example.html"
            source.parent.mkdir(parents=True)
            source.write_text("<html></html>")
            original = RUNNER.run_case
            def partition(*args):
                return {"variant": args[6], "status": "passed", "result": {
                    "complete": True, "harness": 0, "range_total": 1150,
                    "tests": [{"name": args[6], "status": 0}],
                }}
            with mock.patch.object(RUNNER, "run_case", side_effect=partition) as run:
                result = original(Path("unused"), root, 8000,
                                  Path("editing/run/example.html"), "generated_webidl",
                                  "window", "?1-last", 30, False, None)
            self.assertEqual(run.call_count, 12)
            self.assertEqual(result["partitions"][-1], "?1101-1150")
            self.assertEqual(result["status"], "passed")

    def test_open_ended_editing_range_retains_failed_empty_partition(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "editing/run/example.html"
            source.parent.mkdir(parents=True)
            source.write_text("<html></html>")
            original = RUNNER.run_case
            parts = [
                {"variant": "?1-100", "status": "passed", "result": {
                    "complete": True, "harness": 0, "range_total": 150,
                    "tests": [{"name": "first", "status": 0}],
                }},
                {"variant": "?101-150", "status": "failed", "failure_domain": "timeout"},
            ]
            with mock.patch.object(RUNNER, "run_case", side_effect=parts):
                result = original(Path("unused"), root, 8000,
                                  Path("editing/run/example.html"), "generated_webidl",
                                  "window", "?1-last", 30, False, None)
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["failure_domain"], "timeout")
            self.assertFalse(result["result"]["complete"])
            self.assertEqual(len(result["partitions"]), 2)

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

    def test_meta_scripts_apply_wpt_server_rewrites(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / "IndexedDB/idlharness.any.js"
            parser = root / "resources/webidl2/lib/webidl2.js"
            source.parent.mkdir(parents=True)
            parser.parent.mkdir(parents=True)
            source.write_text("// META: script=/resources/WebIDLParser.js\n")
            parser.write_text("globalThis.WebIDL2 = {};\n")
            self.assertEqual(
                RUNNER.meta_script_paths(root, source, source.read_text()),
                ["/resources/webidl2/lib/webidl2.js"],
            )
            self.assertEqual(
                RUNNER.wpt_server_path("/resources/WebIDLParser.js"),
                "/resources/webidl2/lib/webidl2.js",
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
            "http://{{hosts[alt][]}}:{{ports[http][1]}}/cross "
            "ws://{{domains[www1]}}:{{ports[ws][0]}}/socket"
        )
        self.assertEqual(
            RUNNER.substitute_wpt_tokens(source, "localhost", 8443),
            "http://localhost:8443/same http://127.0.0.1:8443/cross "
            "ws://127.0.0.1:8443/socket",
        )

    def test_wptserve_uses_distinct_http_and_https_ports(self) -> None:
        source = (
            "http://{{host}}:{{ports[http][0]}}/plain "
            "https://{{host}}:{{ports[https][0]}}/secure"
        )
        self.assertEqual(
            RUNNER.substitute_wpt_tokens(source, "localhost", 8000, https_port=8443),
            "http://localhost:8000/plain https://localhost:8443/secure",
        )

    def test_proxied_redirects_preserve_public_tls_and_alt_origin(self) -> None:
        self.assertEqual(
            RUNNER.rewrite_wpt_location("http://localhost:8443/resource", True),
            "https://localhost:8443/resource",
        )
        self.assertEqual(
            RUNNER.rewrite_wpt_location("http://www1.localhost:8000/resource", False),
            "http://127.0.0.1:8000/resource",
        )
        self.assertEqual(
            RUNNER.rewrite_wpt_location("http://www1.127.0.0.1:8000/resource", False),
            "http://127.0.0.1:8000/resource",
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
        self.assertIn("output.setAttribute('id', 'oryn-wpt-result')", RUNNER.COMPLETION)
        result_callback = RUNNER.COMPLETION.split("add_result_callback", 1)[1].split(
            "add_completion_callback", 1
        )[0]
        self.assertNotIn("__oryn_publish_wpt", result_callback)

    def test_worker_result_element_is_created_only_after_completion(self) -> None:
        source = inspect.getsource(RUNNER.Handler.do_GET)
        worker_runner = source.split(
            'if parsed.path == "/__oryn_worker_runner.html":', 1
        )[1].split('if parsed.path == "/__oryn_worker_bootstrap.js":', 1)[0]
        self.assertIn("document.createElement('pre')", worker_runner)
        self.assertNotIn('<pre id="oryn-wpt-result"></pre>', worker_runner)

    def test_worker_bootstrap_signals_end_of_imported_test_source(self) -> None:
        source = inspect.getsource(RUNNER.Handler.do_GET)
        worker_bootstrap = source.split(
            'if parsed.path == "/__oryn_worker_bootstrap.js":', 1
        )[1].split('if path.name.endswith(".sub.js")', 1)[0]
        self.assertIn('+ "\\n;done();\\n"', worker_bootstrap)

    def test_editing_prefilter_shares_the_final_data_dependent_script(self) -> None:
        source = (
            '<script src="../data/justifycenter.js"></script>'
            '<script>"use strict"; browserTests.forEach(runConformanceTest);</script>'
        )
        instrumented = RUNNER.inject_editing_subset_bootstrap(
            source, "editing/run/justifycenter.html"
        )
        self.assertEqual(instrumented.count("<script>"), 1)
        self.assertLess(
            instrumented.index("Preselect entries"),
            instrumented.index('"use strict"'),
        )
        self.assertEqual(
            RUNNER.inject_editing_subset_bootstrap(source, "dom/ranges.html"),
            source,
        )

    def test_dom_probe_recovers_partial_assertions_without_claiming_completion(self) -> None:
        stdout = (
            'function|function|results=[{"name":"one","status":0,"message":""}]'
            '|messages=[]|driver=[]|document=<html></html>'
        )
        result = RUNNER.probe_wpt_result(stdout)
        self.assertIsNotNone(result)
        self.assertFalse(result["complete"])
        self.assertIsNone(result["harness"])
        self.assertEqual(result["tests"][0]["name"], "one")
        self.assertIsNone(RUNNER.probe_wpt_result("results=[]|messages=[]"))

    def test_server_path_decodes_percent_encoded_wpt_directories(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            handler = object.__new__(RUNNER.Handler)
            handler.root = Path(directory).resolve()
            self.assertEqual(
                Path(handler.translate_path("/referrer-policy/4K%2B1/case.html")),
                handler.root / "referrer-policy/4K+1/case.html",
            )

    def test_instrumented_static_files_retain_wpt_sidecar_headers(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            test = Path(directory) / "case.html"
            test.write_text("<title>case</title>")
            Path(str(test) + ".headers").write_text(
                "Access-Control-Allow-Origin: *\n"
                "Referrer-Policy: no-referrer\n"
                "Content-Length: 999\n"
            )
            self.assertEqual(
                RUNNER.Handler._sidecar_headers(test),
                [
                    ("Access-Control-Allow-Origin", "*"),
                    ("Referrer-Policy", "no-referrer"),
                ],
            )

    def test_nested_security_helper_does_not_report_as_an_independent_test(self) -> None:
        body = (
            b'<script src="/resources/testharness.js"></script>'
            b'<script src="http://localhost:8000/resources/testharnessreport.js"></script>'
            b'<script>parent.postMessage({sourceContextUrl:location.href}, "*")</script>'
        )
        stripped = RUNNER.strip_nested_helper_harness(
            "/common/security-features/scope/document.py?policyDeliveries=[]",
            "text/html; charset=utf-8",
            body,
        )
        self.assertNotIn(b"testharness.js", stripped)
        self.assertNotIn(b"testharnessreport.js", stripped)
        self.assertIn(b"sourceContextUrl", stripped)
        self.assertEqual(
            RUNNER.strip_nested_helper_harness("/ordinary.html", "text/html", body),
            body,
        )

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
        self.assertIn("test_driver_internal.get_computed_label", source)
        self.assertIn("__oryn_computed_accessibility(element).name", source)
        self.assertIn("test_driver_internal.get_computed_role", source)
        self.assertIn(
            "test_driver_internal.get_accessibility_properties_for_element", source
        )
        self.assertIn(
            "test_driver_internal.get_accessibility_properties_for_accessibility_node",
            source,
        )
        self.assertIn("process_timeout = timeout * 4", source)

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

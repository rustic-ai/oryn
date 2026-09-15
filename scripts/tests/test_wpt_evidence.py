import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("evidence", Path(__file__).resolve().parents[1] / "wpt_evidence.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class EvidenceTests(unittest.TestCase):
    def test_deferred_or_failed_fetch_is_not_a_confirmed_template_failure(self):
        for check in ({'deferred_to_runtime': True}, {'status': None, 'preflight_error': 'timeout'}, {}):
            self.assertFalse(module.confirmed_dependency_failure(check))
        self.assertTrue(module.confirmed_dependency_failure({'template_failure': True}))
    def test_literal_module_edges_are_discovered(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'module.js').write_text("import './one.mjs'; export {x} from './two.mjs'; import('./three.mjs');")
            self.assertEqual(module.script_dependencies(root, 'module.js'), ['/one.mjs', '/three.mjs', '/two.mjs'])
    def test_html_dependencies_obey_base_and_ignore_comments(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'a.html').write_text('<!-- <script src="ignored.js"></script> -->'
                '<base href=/assets/><script src=helper.js?x=1&amp;y=2></script>')
            self.assertEqual(module.script_dependencies(root, 'a.html'), ['/assets/helper.js?x=1&y=2'])
    def test_timeout_layers_and_crashes_are_distinct(self):
        for row, expected in [
            ({"failure_domain": "timeout"}, "outer_timeout"),
            ({"result": {"harness": 2}}, "harness_timeout"),
            ({"diagnostics": ['WorkerLimitExceeded { resource: "wall time" }']}, "worker_wall_limit"),
            ({"diagnostics": ['WorkerLimitExceeded { resource: "wall time (phase=ipc_response, command=load_html, message_id=2)" }']}, "worker_wall_limit"),
            ({"diagnostics": ['WorkerLimitExceeded { resource: "wall time (phase=worker_execution, command=oil, message_id=3)" }']}, "worker_wall_limit"),
            ({"diagnostics": ['WorkerLimitExceeded: generated platform operation wall time']}, "generated_operation_wall_limit"),
            ({"diagnostics": ['WorkerCrashed: closed stream']}, "reported_worker_crash"),
            ({"result": {"harnessMessage": "click is not implemented by testdriver-vendor.js"}}, "testdriver_operation_not_implemented"),
        ]:
            self.assertEqual(module.diagnostics(row), [expected])

    def test_expected_errors_are_not_template_failures(self):
        for status, body in [(404, b"missing"), (500, b"intentional error"), (200, b"{{ports[h2][0]}}"), (200, b"malformed script :")]:
            self.assertFalse(module.template_failure(status, body))
        self.assertTrue(module.template_failure(500, b"Traceback config_replacement IndexError"))

    def test_recursive_dependencies_preserve_queries_and_break_cycles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "a.html").write_text('<script src="b.js?pipe=sub"></script>')
            (root / "b.js").write_text('importScripts("c.sub.js");')
            (root / "c.sub.js").write_text('// META: script=b.js?pipe=sub\n')
            self.assertEqual(module.script_dependencies(root, "a.html"), ["/b.js?pipe=sub", "/c.sub.js"])

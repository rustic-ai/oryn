from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest

from oryn_validation.cli import main


class CLITests(unittest.TestCase):
    def test_snapshot_rejects_escape(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()) as error:
            code = main(["input-snapshot", "--root", temporary, "--path", "../outside"])
        self.assertEqual(code, 2)
        self.assertIn("error", json.loads(error.getvalue()))

    def test_reconciliation_preserves_failures_and_separate_assessments(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "include").write_text("/test.html\n")
            events = [{"action": "suite_start"}, {"action": "test_start", "test": "/test.html"},
                      {"action": "test_end", "test": "/test.html", "status": "FAIL"},
                      {"action": "suite_end"}]
            (root / "events").write_text("\n".join(map(json.dumps, events)))
            (root / "execution").write_text(json.dumps({"returncode": 1, "timed_out": False,
                "cleanup_complete": True, "cleanup_errors": [], "error": None}))
            with redirect_stdout(io.StringIO()) as output:
                code = main(["wpt-reconcile", "--include", str(root / "include"),
                             "--events", str(root / "events"), "--execution", str(root / "execution")])
        value = json.loads(output.getvalue())
        self.assertEqual(code, 0)
        self.assertEqual(value["outcomes"], {"FAIL": 1})
        self.assertEqual(value["assessment_total"], 752)
        self.assertEqual(value["qualification"], "not-evaluated")

    def test_cleanup_refuses_index_other_than_resource_owner_index(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stderr(io.StringIO()) as error:
            code = main(["cleanup-preview", "--store-root", temporary,
                         "--index", str(Path(temporary) / "unrelated.json")])
        self.assertEqual(code, 2)
        self.assertIn("same retention.json", error.getvalue())


if __name__ == "__main__":
    unittest.main()

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation.cli import main


class CLITests(unittest.TestCase):
    def test_reference_stage_forwards_publisher_archive_without_interpreting_it(self):
        with patch('oryn_validation.reference_environment.stage_reference_environment',
                   return_value={'prepared': True}) as stage, redirect_stdout(io.StringIO()) as output:
            code = main(['reference-environment-stage', '--source-app', '/input/Chrome.app',
                '--source-driver', '/input/chromedriver', '--source-archive', '/input/publisher.zip',
                '--store-root', '/data', '--output-directory', 'environments/reference'])
        stage.assert_called_once_with(Path('/input/Chrome.app'), Path('/input/chromedriver'),
            Path('/data'), 'environments/reference', source_archive=Path('/input/publisher.zip'))
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output.getvalue()), {'prepared': True})

    def test_reference_stage_preserves_native_signature_authentication_option(self):
        with patch('oryn_validation.reference_environment.stage_reference_environment',
                   return_value={}) as stage, redirect_stdout(io.StringIO()):
            code = main(['reference-environment-stage', '--source-app', '/input/Chrome.app',
                '--source-driver', '/input/chromedriver', '--store-root', '/data',
                '--output-directory', 'environments/reference'])
        self.assertEqual(code, 0)
        self.assertIsNone(stage.call_args.kwargs['source_archive'])

    def test_relay_command_only_dispatches_identity_preparation_and_reports_failure(self):
        with patch('oryn_validation.fixture_relay.prepare_relay_identity',
                   side_effect=ValueError('relay payload changed')) as prepare, redirect_stderr(io.StringIO()) as error:
            code = main(['fixture-relay-identity', '--store-root', '/data',
                '--executable-path', 'environments/relay/payload/bin/socat',
                '--identity-path', 'environments/relay/identity.json'])
        prepare.assert_called_once_with(Path('/data'),
            executable_path='environments/relay/payload/bin/socat', identity_path='environments/relay/identity.json')
        self.assertEqual(code, 2)
        self.assertEqual(json.loads(error.getvalue()), {'command': 'fixture-relay-identity',
                                                       'error': 'relay payload changed'})

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

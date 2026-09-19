"""Framework evidence must demonstrate state change and preserve diagnostics."""

import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

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

    def test_process_custody_rejects_missing_forced_or_surviving_groups(self):
        process = mock.Mock(pid=23456)
        process.poll.return_value = 0
        with mock.patch.object(runner.os, 'killpg', side_effect=ProcessLookupError):
            clean = runner.finish_case_process(process)
        record = dict(returncode=0, timed_out=False, process_cleanup=clean)
        self.assertEqual(runner.validate_process_record(record), [])
        process.wait.assert_called_once_with(timeout=10)
        with mock.patch.object(runner.os, 'killpg') as kill:
            leaked = runner.finish_case_process(process)
        self.assertFalse(leaked['descendants_clean'])
        self.assertTrue(leaked['forced'])
        kill.assert_any_call(process.pid, runner.signal.SIGKILL)
        self.assertTrue(runner.validate_process_record(dict(record, process_cleanup=leaked)))
        for bad in (None, {}, dict(clean, parent_reaped=False), dict(clean, errors=['reap failed'])):
            self.assertTrue(runner.validate_process_record(dict(record, process_cleanup=bad)))
        self.assertTrue(runner.validate_process_record(dict(record, returncode=False)))

    def test_case_cleanup_failure_is_retained_after_parent_exit(self):
        process = mock.Mock(pid=23456)
        process.poll.return_value = None
        process.wait.side_effect = runner.subprocess.TimeoutExpired('owned', 10)
        with mock.patch.object(runner.os, 'killpg'):
            result = runner.finish_case_process(process)
        self.assertFalse(result['parent_reaped'])
        self.assertTrue(result['forced'])
        self.assertTrue(result['errors'])


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


class FrameworkGenerationCustodyTests(unittest.TestCase):
    def test_official_case_gate_drift_preserves_failed_record_without_launch(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            gate = root / 'gate.json'; gate.write_text('{}')
            args = SimpleNamespace(output=root, binary=root/'unused', worker=root/'unused',
                                   generation_gate=gate,
                                   generation_identity=dict(path=str(gate), sha256=runner.build.digest(gate)))
            gate.write_text('{"replacement":true}')
            with mock.patch.object(runner, 'generation_gate'), mock.patch.object(runner.subprocess, 'Popen') as launch:
                record = runner.run_case('vue', 1, 'http://unused', args)
            launch.assert_not_called()
            self.assertFalse(record['passed'])
            self.assertTrue(record['errors'])
            self.assertTrue((root/'vue-01'/'result.json').is_file())

    def test_incomplete_gate_precedes_output_and_fixture_side_effects(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)/'never-created'
            argv = ['runner', '--generation-gate', 'incomplete', '--build', 'unused', '--output', str(output)]
            with mock.patch.object(runner.sys, 'argv', argv), \
                 mock.patch.object(runner, 'generation_gate', side_effect=ValueError('incomplete')), \
                 mock.patch.object(runner, 'ThreadingHTTPServer') as server, \
                 mock.patch.object(runner.subprocess, 'run') as launch:
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    runner.main()
            server.assert_not_called(); launch.assert_not_called()
            self.assertFalse(output.exists())


    def test_transient_gate_failure_cancels_later_children_after_restoration(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as directory:
            gate = Path(directory)/'gate.json'; gate.write_text('{}')
            args = SimpleNamespace(generation_gate=gate, generation_failure=runner.threading.Event(),
                                   generation_identity=dict(path=str(gate), sha256=runner.build.digest(gate)))
            with mock.patch.object(runner, 'generation_gate'):
                runner.check_generation(args)
                gate.write_text('{"changed":true}')
                with self.assertRaises(ValueError):
                    runner.check_generation(args)
                gate.write_text('{}')
                with self.assertRaisesRegex(ValueError, 'later launches cancelled'):
                    runner.check_generation(args)

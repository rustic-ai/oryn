import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from oryn_validation import processes


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()

    def test_child_failure_preserved_and_resources_reusable(self):
        output = self.root / "attempt"
        result = processes.run_owned([sys.executable, "-c", "raise SystemExit(7)"],
                                    cwd=self.root, output=output, timeout_seconds=10,
                                    store_root=self.root, resource_keys=["fixture"])
        self.assertEqual(result["returncode"], 7)
        self.assertTrue(result["cleanup_complete"])
        self.assertEqual(json.loads((output / "process-result.json").read_text()), result)
        with processes.ResourceLease(self.root, ["fixture"]) as lease:
            lease.mark_settled(result)

    def test_timeout_records_terminal_failure(self):
        result = processes.run_owned([sys.executable, "-c", "import time; time.sleep(30)"],
                                    cwd=self.root, output=self.root / "timeout", timeout_seconds=.3,
                                    grace_seconds=.1)
        self.assertTrue(result["timed_out"])
        self.assertTrue(result["cleanup_complete"])
        self.assertIn("timed out", result["error"])

    def test_unsettled_resource_blocks_future_launch(self):
        with processes.ResourceLease(self.root, ["fixture"]):
            pass
        with self.assertRaisesRegex(RuntimeError, "unsettled"):
            with processes.ResourceLease(self.root, ["fixture"]):
                self.fail("unsettled resource admitted")
        with self.assertRaises(RuntimeError):
            processes.assert_no_active_writers(self.root)

    def test_live_resource_lock_conflict(self):
        with processes.ResourceLease(self.root, ["fixture"]) as owner:
            with self.assertRaisesRegex(RuntimeError, "busy"):
                with processes.ResourceLease(self.root, ["fixture"]):
                    self.fail("duplicate lease admitted")
            owner.mark_settled({"cleanup_complete": True, "cleanup_errors": []})

    def test_output_symlink_parent_has_no_side_effect(self):
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / "alias").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            processes.run_owned([sys.executable, "-c", "pass"], cwd=self.root,
                                output=self.root / "alias/attempt", timeout_seconds=1)
        self.assertEqual(list(outside.iterdir()), [])

    def test_resource_symlink_parent_has_no_side_effect(self):
        outside = self.root / "outside"
        outside.mkdir()
        (self.root / "status").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(ValueError):
            with processes.ResourceLease(self.root, ["fixture"]):
                self.fail("symlink resource root admitted")
        self.assertEqual(list(outside.iterdir()), [])

    def test_metadata_failure_does_not_replace_primary_exception(self):
        primary = RuntimeError("original cancellation")
        lease = processes.ResourceLease(self.root, ["fixture"])
        lease.__enter__()
        with patch.object(processes, "_write", side_effect=OSError("disk error")):
            lease.__exit__(type(primary), primary, None)
        self.assertIn("disk error", primary.__notes__[0])
        self.assertEqual(str(primary), "original cancellation")
        self.assertEqual(lease.handles, [])

    def test_refuse_overwriting_prior_attempt(self):
        output = self.root / "existing"
        output.mkdir()
        (output / "marker").write_text("original")
        with self.assertRaisesRegex(ValueError, "already exists"):
            processes.run_owned([sys.executable, "-c", "pass"], cwd=self.root,
                                output=output, timeout_seconds=1)
        self.assertEqual((output / "marker").read_text(), "original")

    def test_supervisor_failure_after_root_exit_never_settles_unknown_descendants(self):
        def broken(process, *_args, **_kwargs):
            process.wait(timeout=5)
            raise RuntimeError("custody observer failed after child exit")
        with patch.object(processes.supervisor(), "supervise", side_effect=broken):
            result = processes.run_owned([sys.executable, "-c", "pass"], cwd=self.root,
                                        output=self.root / "lost-custody", timeout_seconds=5,
                                        store_root=self.root, resource_keys=["fixture"])
        self.assertFalse(result["cleanup_complete"])
        self.assertIn("custody observer failed", result["error"])
        with self.assertRaises(RuntimeError):
            processes.assert_no_active_writers(self.root)

    def test_service_supervised_through_explicit_shutdown(self):
        with processes.OwnedService([sys.executable, "-c", "import time; time.sleep(30)"],
                                    cwd=self.root, output=self.root / "service", timeout_seconds=10) as service:
            time.sleep(.4)
            service.check()
        self.assertTrue(service.result["service_stop_requested"])
        self.assertTrue(service.result["cleanup_complete"])
        self.assertEqual(service.result["cleanup_errors"], [])
        self.assertIsNotNone(service.result["error"])  # Intentional shutdown remains recorded.
        self.assertTrue((self.root / "service/service-result.json").is_file())

    def test_service_early_exit_cannot_be_called_successful_teardown(self):
        with self.assertRaisesRegex(RuntimeError, "service"):
            with processes.OwnedService([sys.executable, "-c", "pass"], cwd=self.root,
                                        output=self.root / "early", timeout_seconds=10) as service:
                service.thread.join(timeout=5)
                service.check()
        self.assertFalse(service.result["service_stop_requested"])


if __name__ == "__main__":
    unittest.main()

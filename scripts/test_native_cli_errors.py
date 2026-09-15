"""Signed CLI regression; opt in with ORYN_TEST_NATIVE_BIN and ORYN_PAGE_WORKER."""

import os
from pathlib import Path
import subprocess
import unittest


@unittest.skipUnless(os.environ.get("ORYN_TEST_NATIVE_BIN"), "requires explicit built native CLI")
class NativeCliErrorTests(unittest.TestCase):
    def run_cli(self, *flags, source="invalid-command-for-regression\nobserve\nexit\n"):
        return subprocess.run(
            [str(Path(os.environ["ORYN_TEST_NATIVE_BIN"]).resolve()), "native", *flags],
            input=source,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def test_piped_error_is_nonzero_and_stops_before_next_command(self):
        result = self.run_cli()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid-command-for-regression", result.stderr)
        self.assertNotIn('"kind":"observation"', result.stdout)

    def test_sdk_repl_can_recover_after_an_error(self):
        result = self.run_cli("--sdk-repl")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("invalid-command-for-regression", result.stderr)
        self.assertIn('"kind":"observation"', result.stdout)

    def test_successful_piped_commands_remain_successful(self):
        result = self.run_cli(source="observe\nexit\n")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"kind":"observation"', result.stdout)
        self.assertEqual(result.stderr, "")


if __name__ == "__main__":
    unittest.main()

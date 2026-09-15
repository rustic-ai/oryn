"""Auth evidence rejects swallowed actions without assuming global action IDs."""

import copy
import json
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]


class AuthEvidenceTests(unittest.TestCase):
    def frames(self):
        return [
            dict(kind="observation", observation={}),
            dict(kind="action", result=dict(page_id=1, action_id=1, execution_domain="native")),
            dict(kind="action", result=dict(page_id=2, action_id=1, execution_domain="native")),
            dict(kind="structured", value="Authenticated with PKCE"),
            dict(kind="trace", events=[]),
        ]

    def accepted(self, frames):
        return (
            subprocess.run(
                [
                    "jq",
                    "-s",
                    "-e",
                    "--arg",
                    "expected",
                    "Authenticated with PKCE",
                    "--argjson",
                    "actions",
                    "2",
                    "-f",
                    str(ROOT / "scripts/validate_auth_flow.jq"),
                ],
                input="\n".join(json.dumps(frame) for frame in frames),
                capture_output=True,
                text=True,
                timeout=5,
                check=False,
            ).returncode
            == 0
        )

    def test_page_local_action_ids_are_valid(self):
        self.assertTrue(self.accepted(self.frames()))

    def test_missing_action_cannot_be_hidden_by_successful_output(self):
        frames = self.frames()
        del frames[2]
        self.assertFalse(self.accepted(frames))

    def test_missing_trace_error_wrong_output_or_duplicate_action_fails(self):
        for corruption in ("trace", "diagnostic", "output", "duplicate", "domain", "error"):
            frames = copy.deepcopy(self.frames())
            if corruption == "trace":
                frames.pop()
            elif corruption == "diagnostic":
                frames[-1]["events"] = [dict(kind=dict(kind="script_diagnostic"))]
            elif corruption == "output":
                frames[-2]["value"] = "Signed out"
            elif corruption == "duplicate":
                frames[2]["result"]["page_id"] = 1
            elif corruption == "domain":
                frames[2]["result"]["execution_domain"] = "chromium"
            else:
                frames.append(dict(kind="error"))
            with self.subTest(corruption=corruption):
                self.assertFalse(self.accepted(frames))


if __name__ == "__main__":
    unittest.main()

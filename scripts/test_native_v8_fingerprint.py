#!/usr/bin/env python3
"""Keep native worker engine identity aligned with the schema-6 patch contract."""
import re
import runpy
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class NativeEngineFingerprintTests(unittest.TestCase):
    def test_native_fingerprint_binds_every_profile_patch_in_canonical_order(self):
        contract = runpy.run_path(str(ROOT / "scripts/validate-v8-runtime-profile.py"))
        expected = list(contract["PATCHES"].values())
        source = (ROOT / "crates/oryn-native/build.rs").read_text()
        block = source.split("let mut engine_chain = Sha256::new();", 1)[1]
        patch_list = re.search(r"for name in \[(.*?)\] \{", block, re.S)
        self.assertIsNotNone(patch_list)
        actual = re.findall(r'"([a-z-]+)"', patch_list.group(1))
        self.assertEqual(actual, expected)
        # Duplicate entries would also make an incompatible worker identity.
        self.assertEqual(len(actual), len(set(actual)))


if __name__ == "__main__":
    unittest.main()

import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('merge', Path(__file__).resolve().parents[1] / 'merge-wpt-qualification-ledger.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class MergeTests(unittest.TestCase):
    def setUp(self):
        self.entries = [
            {'id': 'testharness:/a', 'runner_id': 'a|window|', 'status': 'failed',
             'qualification': 'review_candidate', 'evidence': {'sha256': 'raw'}},
            {'id': 'reftest:/b', 'status': 'unsupported', 'qualification': 'unsupported'}]
        self.overlay = [{'id': 'a|window|', 'status': 'failed', 'qualification': 'qualified',
                         'qualification_reasons': [], 'original': {'sha256': 'raw'}, 'audit': {'reviewed': True}}]

    def test_retains_every_manifest_entry_and_raw_failure(self):
        rows = module.merge(self.entries, self.overlay)
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]['status'], 'failed')
        self.assertEqual(rows[0]['qualification'], 'qualified')
        self.assertEqual(rows[1], self.entries[1])
        self.assertEqual(self.entries[0]['qualification'], 'review_candidate')

    def test_missing_extra_and_duplicate_ids_are_rejected(self):
        for overlay in ([], self.overlay * 2, [dict(self.overlay[0], id='wrong')]):
            with self.assertRaises(ValueError):
                module.merge(self.entries, overlay)

    def test_changed_outcome_and_provenance_are_rejected(self):
        for patch in ({'status': 'passed'}, {'original': {'sha256': 'other'}}):
            overlay = copy.deepcopy(self.overlay)
            overlay[0].update(patch)
            with self.assertRaises(ValueError):
                module.merge(self.entries, overlay)

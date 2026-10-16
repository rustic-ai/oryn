import unittest
from oryn_validation.rust_source_rebind_marker import pending_rebind


class RebindMarkerTests(unittest.TestCase):
    def test_absent_marker_is_ordinary_profile(self):
        self.assertIsNone(pending_rebind({}))

    def test_malformed_marker_presence_fails_closed(self):
        for value in ({}, None, False, [], {'schema_version': True}):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'malformed'):
                pending_rebind({'rust_source_rebind_intermediate': value})

    def test_valid_marker_returns_independent_copy(self):
        value = {'schema_version': 1, 'old_profile_sha256': '0' * 64,
                 'old_rust_tree': '1' * 64, 'new_rust_tree': '2' * 64,
                 'fresh_snapshot_required': True, 'fresh_worker_required': True,
                 'qualification_complete': False}
        result = pending_rebind({'rust_source_rebind_intermediate': value})
        result['fresh_snapshot_required'] = False
        self.assertTrue(value['fresh_snapshot_required'])

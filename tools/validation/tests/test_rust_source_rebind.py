"""Coordinator source-rebind controls; mocked validators isolate exact transition."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from oryn_validation.rust_source_rebind import rebind_archive_watch


class ArchiveWatchRebindTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.old = self.root / 'old'
        self.new = self.root / 'new'
        self.profile = self.root / 'old.json'
        self.profile.write_text(json.dumps({'worker_sha256': 'old-worker', 'archive_sha256': 'archive'}))
        before = b'  println!("cargo:rerun-if-changed=.gn");\n'
        after = b'  // Archive-only subset has no upstream .gn; watching it forces perpetual rebuilds.\n'
        for root, content, digest in ((self.old, before, 'before'), (self.new, after, 'after')):
            vendor = root / 'vendor/rusty-v8'
            vendor.mkdir(parents=True)
            (vendor / 'build.rs').write_bytes(content)
            (vendor / 'oryn-source-manifest.json').write_text(json.dumps({'files': {'build.rs': digest, 'src/api.rs': 'same'}, 'base_files': {'build.rs': 'upstream'}}))

    def run_rebind(self):
        with patch('oryn_validation.rust_source_rebind._sources', side_effect=[{'rusty_v8_source_tree_sha256': 'before'}, {'rusty_v8_source_tree_sha256': 'after'}]), patch('oryn_validation.rust_source_rebind._legacy_validator') as validator:
            result = rebind_archive_watch(self.old, self.new, self.profile, self.root / 'new.json')
            self.assertEqual(validator.return_value.validate.call_count, 2)
            return result

    def test_exact_change_retains_old_worker_as_nonproduct_intermediate(self):
        old_bytes = self.profile.read_bytes()
        self.assertTrue(self.run_rebind()['intermediate_only'])
        output = json.loads((self.root / 'new.json').read_text())
        self.assertEqual(output['worker_sha256'], 'old-worker')
        self.assertTrue(output['rust_source_rebind_intermediate']['fresh_worker_required'])
        self.assertEqual(self.profile.read_bytes(), old_bytes)

    def test_extra_abi_source_change_rejects_without_output(self):
        path = self.new / 'vendor/rusty-v8/oryn-source-manifest.json'
        value = json.loads(path.read_text())
        value['files']['src/api.rs'] = 'changed'
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'only reviewed'):
            self.run_rebind()
        self.assertFalse((self.root / 'new.json').exists())

    def test_old_profile_validation_failure_never_publishes(self):
        with patch('oryn_validation.rust_source_rebind._legacy_validator') as validator:
            validator.return_value.validate.side_effect = ValueError('old closure drift')
            with self.assertRaisesRegex(ValueError, 'old closure drift'):
                rebind_archive_watch(self.old, self.new, self.profile, self.root / 'new.json')
        self.assertFalse((self.root / 'new.json').exists())

    def test_current_cpp_closure_failure_never_publishes(self):
        with patch('oryn_validation.rust_source_rebind._sources', side_effect=[{'rusty_v8_source_tree_sha256': 'before'}, {'rusty_v8_source_tree_sha256': 'after'}]), patch('oryn_validation.rust_source_rebind._legacy_validator') as validator:
            validator.return_value.validate.side_effect = [None, ValueError('current ABI drift')]
            with self.assertRaisesRegex(ValueError, 'current ABI drift'):
                rebind_archive_watch(self.old, self.new, self.profile, self.root / 'new.json')
        self.assertFalse((self.root / 'new.json').exists())

    def test_output_cannot_overwrite_historical_profile(self):
        original = self.profile.read_bytes()
        with self.assertRaisesRegex(ValueError, 'cannot be rewritten'):
            rebind_archive_watch(self.old, self.new, self.profile, self.profile)
        self.assertEqual(self.profile.read_bytes(), original)

    def test_extra_build_behavior_rejects_without_output(self):
        path = self.new / 'vendor/rusty-v8/build.rs'
        path.write_bytes(path.read_bytes() + b'other behavior')
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            self.run_rebind()
        self.assertFalse((self.root / 'new.json').exists())

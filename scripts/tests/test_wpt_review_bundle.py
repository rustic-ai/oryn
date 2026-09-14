import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
spec = importlib.util.spec_from_file_location('bundle', Path(__file__).resolve().parents[1] / 'prepare-wpt-review-bundle.py')
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


class ReviewBundleTests(unittest.TestCase):
    def test_generated_bundle_never_approves_its_observations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'test.html').write_text('<!doctype html>')
            data = {'schema_version':4, 'assertions':[{'id':'test', 'path':'test.html',
                'status':'passed', 'result':{'tests':[{'name':'one','status':0}],
                'testdriver':[{'operation':'click'}]}}]}
            review = bundle.prepare(data, 'a'*64, root)['test']
            self.assertEqual(review['fixture_audit'], '')
            self.assertEqual(review['verified_resources'], {})
            self.assertEqual(review['verified_automation_operations'], [])
            self.assertFalse(review['tls_validation_independent'])
            self.assertEqual(review['candidate_observations']['automation_operations'], ['click'])
            self.assertEqual(len(review['source_files']['test.html']), 64)

    def test_source_escape_and_historical_schema_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            for data in ({'schema_version':3}, {'schema_version':4,
                'assertions':[{'id':'bad','path':'../outside','status':'failed'}]}):
                with self.assertRaises(ValueError):
                    bundle.prepare(data, 'a'*64, Path(directory))

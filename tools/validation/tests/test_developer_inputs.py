"""Developer startup stays small; expensive audits remain deliberate."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import developer_inputs as inputs, developer_wpt as runner
from oryn_validation import wpt


class DeveloperInputTests(unittest.TestCase):
    def template(self, root):
        python = root / 'env/bin/python'
        python.parent.mkdir(parents=True)
        python.write_text('test executable'); python.chmod(0o700)
        file = inputs.snapshot_input(root, 'env/bin/python', scope='bundle')
        assets = dict.fromkeys(('runtime', 'driver', 'python', 'fixture_relay', 'worker'), file)
        assets.update(python_environment=dict(scope='store', path='env', kind='directory', sha256='a' * 64,
                                             members=['not needed for developer startup']),
                      wpt_checkout=dict(scope='store', path='env', kind='directory', sha256='b' * 64))
        run = dict(kind='run', schema_version=1, run_id='test', profile='standard-block',
                   build={'build_id': 'test', 'manifest_sha256': 'a' * 64}, runner={},
                   original_run_id='test', attempt=1, suite={'name': 'wpt-core'}, workers=1,
                   assets=assets, immutable_inputs=[file])
        path = root / 'run.json'; path.write_text(json.dumps(run))
        return path

    def test_local_startup_does_not_hash_dataset_or_requalify_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = self.template(root)
            with patch.object(inputs, '_resolve_inputs', side_effect=AssertionError('full schema audit')), \
                 patch.object(inputs, 'verify_inputs', side_effect=AssertionError('recursive audit')), \
                 patch('oryn_validation.production.validate_production', side_effect=AssertionError('build audit')):
                result = inputs.load_developer_inputs(path, store_root=root, bundle_root=root)
            self.assertEqual(result['assets']['fixture_tls_module'].parent, inputs.SOURCE_ROOT / 'tools/validation/oryn_validation')
            self.assertEqual(result['run']['assets']['fixture_tls_module']['sha256'],
                             inputs._digest(result['assets']['fixture_tls_module']))
            self.assertNotIn('members', result['run']['assets']['python_environment'])

    def test_compact_configuration_is_reusable_without_original_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = self.template(root)
            bound = inputs.load_developer_inputs(path, store_root=root, bundle_root=root)
            compact = root / 'developer-config.json'
            compact.write_text(json.dumps(dict(kind='developer-wpt', qualification='not-evaluated',
                                               inputs=inputs.serialize_inputs(bound))))
            path.unlink()
            result = inputs.load_developer_inputs(compact, store_root=root, bundle_root=root)
            self.assertEqual(result['run'], bound['run'])
            child, _ = inputs.configured_inputs(compact, inputs._digest(compact))
            self.assertEqual(child['path'], compact)
            with self.assertRaisesRegex(ValueError, 'configuration changed'):
                inputs.configured_inputs(compact, '0' * 64)

    def test_missing_executable_fails_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            path = self.template(root)
            (root / 'env/bin/python').chmod(0o600)
            with self.assertRaisesRegex(ValueError, 'non-executable.*runtime'):
                inputs.load_developer_inputs(path, store_root=root, bundle_root=root)

    def test_explicit_audit_verifies_original_records_once(self):
        original = {'records': ['original'], 'store_root': Path('/store'), 'bundle_root': Path('/bundle')}
        bound = dict(original, path=Path('/run'))
        with patch.object(inputs, '_resolve_inputs', return_value=original), \
             patch.object(inputs, 'verify_inputs') as verify, \
             patch('oryn_validation.production.validate_production') as production:
            inputs.audit_inputs(bound)
        verify.assert_called_once_with(['original'], store_root=Path('/store'), bundle_root=Path('/bundle'))
        production.assert_called_once_with(original)

    def test_qualification_rejects_developer_bootstrap_before_admission(self):
        with patch.dict(os.environ, ORYN_WPT_DEVELOPER_CONFIG='/developer.json'), \
             patch.object(wpt, 'inspect_admission') as inspect:
            with self.assertRaisesRegex(ValueError, 'cannot be used for qualification'):
                wpt.official_admission({})
        inspect.assert_not_called()

    def test_population_must_be_explicit_and_unique(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'include.txt'
            for content in ('', '/a\n/a\n', 'a\n', '//host/test\n'):
                path.write_text(content)
                with self.assertRaises(ValueError):
                    runner.urls(path)
            path.write_text('/a?variant=1\n/a?variant=2\n')
            self.assertEqual(runner.urls(path), ['/a?variant=1', '/a?variant=2'])

    def test_failure_results_are_not_discarded_for_exit_one(self):
        events = [{'action': 'suite_start'}, {'action': 'test_start', 'test': '/a'},
                  {'action': 'test_status', 'test': '/a', 'status': 'FAIL'},
                  {'action': 'test_end', 'test': '/a', 'status': 'ERROR'}, {'action': 'suite_end'}]
        execution = {'returncode': 1, 'timed_out': False, 'forced_cleanup': False,
                     'cleanup_complete': True, 'cleanup_errors': [], 'error': None, 'cancellation_signal': None}
        result = runner.reconcile_wpt(['/a'], events, execution=execution, assessment_total=0)
        self.assertTrue(result['compatibility_complete'])
        self.assertIn('ERROR', str(result))
        self.assertIn('FAIL', str(result))
        incomplete = runner.reconcile_wpt(['/a', '/b'], events, execution=execution, assessment_total=0)
        self.assertFalse(incomplete['compatibility_complete'])


if __name__ == '__main__':
    unittest.main()

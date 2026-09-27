"""Pure developer harness guards; these tests never start its native child."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import developer_protocol_smoke as smoke


class DeveloperSmokeGuards(unittest.TestCase):
    def test_ordinary_run_output_cannot_be_used_for_developer_smoke(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, 'never official runs'):
                    smoke.execute(root / 'run.json', store_root=root, bundle_root=root / 'build',
                                  output=root / 'runs/fake-original')
                load.assert_not_called()

    def test_parent_traversal_cannot_escape_into_official_run_namespace(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            output = root / 'migration/verification/../../runs/fake-original'
            with patch.object(smoke, 'load_inputs') as load:
                with self.assertRaisesRegex(ValueError, 'parent directories'):
                    smoke.execute(root / 'run.json', store_root=root, bundle_root=root / 'build', output=output)
                load.assert_not_called()
                self.assertFalse((root / 'runs').exists())

    def test_child_refuses_unbound_configuration_before_protocol_import_or_launch(self):
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / 'config.json'; config.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'configuration changed'):
                smoke.child(config, 'f' * 64)
            with self.assertRaisesRegex(ValueError, 'non-qualification'):
                smoke.child(config, smoke._digest(config))

    def test_cleanup_failure_cannot_hide_primary_initialization_error(self):
        primary = ValueError('actual initialization failed')
        secondary = RuntimeError('actual delete failed')
        result = {'errors': []}
        def fail(): raise secondary
        smoke._cleanup('protocol teardown', fail, result, primary)
        self.assertIn('actual delete failed', primary.__notes__[0])
        self.assertIn('actual delete failed', result['errors'][0])
        with self.assertRaises(RuntimeError) as caught:
            smoke._cleanup('protocol teardown', fail, result, None)
        self.assertIs(caught.exception, secondary)

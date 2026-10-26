import unittest
from pathlib import Path
from unittest.mock import patch
from oryn_validation import developer_wpt


class DeveloperCompletionEnvironmentTests(unittest.TestCase):
    def environment(self, flag):
        bound = {'store_root': '/store', 'bundle_root': '/bundle', 'path': '/contract',
                 'sha256': 'a' * 64, 'assets': {'fixture_tls_policy': '/tls', 'wpt_checkout': '/wpt'},
                 'run': {'build': {'manifest_sha256': 'b' * 64}}}
        inherited = {'ORYN_UNCHECKED_AUTHORITY': 'must-not-pass'}
        if flag is not None:
            inherited['ORYN_WPT_COMPLETION_DIAGNOSTICS'] = flag
        with patch.dict('os.environ', inherited, clear=True), \
             patch.object(developer_wpt, '_digest', return_value='c' * 64), \
             patch.object(developer_wpt, 'python_environment', return_value={}), \
             patch.object(developer_wpt, 'worker_environment', return_value={'ORYN_PAGE_WORKER': '/sealed-worker'}), \
             patch.object(developer_wpt, 'assert_worker_environment'):
            return developer_wpt.runtime_environment(bound, Path('/output'), Path('/config'))

    def test_explicit_one_enables_only_python_observer(self):
        env = self.environment('1')
        self.assertEqual(env['ORYN_WPT_COMPLETION_DIAGNOSTICS'], '1')
        self.assertEqual(env['ORYN_PAGE_WORKER'], '/sealed-worker')
        self.assertNotIn('ORYN_UNCHECKED_AUTHORITY', env)

    def test_default_and_other_values_remain_scrubbed(self):
        for flag in (None, '0', 'true'):
            env = self.environment(flag)
            self.assertNotIn('ORYN_WPT_COMPLETION_DIAGNOSTICS', env)
            self.assertNotIn('ORYN_UNCHECKED_AUTHORITY', env)

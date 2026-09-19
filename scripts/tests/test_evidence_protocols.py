"""Current candidates require captured identities; history never gains them."""
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import unittest
from unittest.mock import patch
from evidence_protocols import validate_protocols, validate_protocol_pair


class EvidenceProtocols(unittest.TestCase):
    def setUp(self):
        self.runtime = dict(worker_protocol_version=16, context_service_protocol_version=49, agent_ipc_protocol_version=18)
        self.worker = dict(protocol_version=16, context_service_protocol_version=49, agent_ipc_protocol_version=18)
        self.constants = patch('native_protocol_versions.versions', return_value=dict(worker=16, context=49, agent=18))
        self.constants.start()
        self.addCleanup(self.constants.stop)

    def test_current_and_compact_only_require_all_captured_protocols(self):
        validate_protocol_pair(self.runtime, self.worker, Path('.'))
        for field in self.runtime:
            for value in (None, True, str(self.runtime[field]), self.runtime[field]+1):
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    validate_protocols(dict(self.runtime, **{field:value}), Path('.'))
        for field in self.worker:
            missing=dict(self.worker);missing.pop(field)
            with self.assertRaises(ValueError):
                validate_protocols(missing, Path('.'), fingerprint=True)

    def test_history_keeps_absent_values_and_requires_captured_pair_consistency(self):
        runtime={'worker_protocol_version':3};worker={'protocol_version':3}
        validate_protocol_pair(runtime, worker, Path('.'), historical=True)
        self.assertEqual(runtime, {'worker_protocol_version':3})
        self.assertEqual(worker, {'protocol_version':3})
        with self.assertRaises(ValueError):validate_protocol_pair(runtime, worker, Path('.'))
        with self.assertRaises(ValueError):
            validate_protocol_pair(dict(runtime, agent_ipc_protocol_version=8), worker, Path('.'), historical=True)
        with self.assertRaises(ValueError):validate_protocols({'worker_protocol_version':True}, Path('.'), historical=True)


class WptProtocolProfile(unittest.TestCase):
    def test_current_qualification_rejects_legacy_absence(self):
        import importlib.util
        spec=importlib.util.spec_from_file_location('wpt_protocol_profile', Path(__file__).resolve().parents[1] / 'run-upstream-wpt.py')
        runner=importlib.util.module_from_spec(spec);spec.loader.exec_module(runner)
        with patch('native_protocol_versions.versions', return_value=dict(worker=16,context=49,agent=18)):
            legacy={'worker_protocol_version':None}
            runner.validate_runtime_protocol_profile('legacy', legacy)
            with self.assertRaises(ValueError):runner.validate_runtime_protocol_profile('upstream', legacy)
            runner.validate_runtime_protocol_profile('upstream',dict(worker_protocol_version=16,context_service_protocol_version=49,agent_ipc_protocol_version=18))

if __name__ == '__main__':
    unittest.main()

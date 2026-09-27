"""Migration completion cannot substitute for any actual production/run evidence."""
import unittest
from unittest.mock import patch

from oryn_validation import admission, controls, prerequisites, production, wpt


class RealEvidenceGates(unittest.TestCase):
    def empty_bound(self):
        return {'assets': {}, 'records': [], 'run': {'assets': {}, 'immutable_inputs': []}}

    def test_static_source_port_status_does_not_admit_missing_production_assets(self):
        self.assertEqual(admission.UNMIGRATED_GATES, ())
        with patch.object(wpt, 'inspect_admission', return_value={'collection': 'core'}), \
                patch.object(prerequisites, 'validate_prerequisites') as later:
            with self.assertRaisesRegex(ValueError, 'production admission lacks typed assets'):
                wpt.official_admission(self.empty_bound())
            later.assert_not_called()

    def test_missing_runtime_evidence_still_stops_official_admission(self):
        with patch.object(wpt, 'inspect_admission', return_value={'collection': 'core'}), \
                patch.object(production, 'validate_production', return_value={}), \
                patch.object(controls, 'verify_controls') as later:
            with self.assertRaisesRegex(ValueError, 'current prerequisite evidence assets missing: runtime_readiness'):
                wpt.official_admission(self.empty_bound())
            later.assert_not_called()

    def test_runtime_readiness_does_not_replace_native_original(self):
        with patch.object(prerequisites, 'validate_runtime_readiness', return_value={}):
            with self.assertRaisesRegex(ValueError, 'current prerequisite evidence assets missing: native_prerequisites'):
                prerequisites.validate_prerequisites(self.empty_bound())

    def test_native_original_does_not_replace_topology_original(self):
        with patch.object(prerequisites, 'validate_native_prerequisites', return_value={}):
            with self.assertRaisesRegex(ValueError, 'current prerequisite evidence assets missing: fixture_topology'):
                prerequisites.validate_prerequisites(self.empty_bound())

    def test_prerequisite_results_do_not_replace_control_originals(self):
        with patch.object(wpt, 'inspect_admission', return_value={'collection': 'core'}), \
                patch.object(production, 'validate_production', return_value={}), \
                patch.object(prerequisites, 'validate_prerequisites', return_value={}):
            with self.assertRaisesRegex(ValueError, 'control evidence missing typed assets'):
                wpt.official_admission(self.empty_bound())
        for collection in ('canaries', 'placement'):
            with self.subTest(collection=collection), self.assertRaisesRegex(ValueError, 'control evidence missing typed assets'):
                controls.verify_control(self.empty_bound(), collection)


if __name__ == '__main__':
    unittest.main()

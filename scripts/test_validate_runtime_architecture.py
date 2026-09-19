"""Candidate-bound cutover entry rejects unsigned invocation and identity drift."""
import contextlib
import io
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import validate_runtime_architecture as validator


class ArchitectureEntryTests(unittest.TestCase):
    def arguments(self):
        return ['validate_runtime_architecture.py', '--require-cutover', '--candidate', '/candidate.json',
                '--candidate-sha256', 'a' * 64, '--output', '/original-run/raw']

    def test_legacy_unbound_cutover_command_cannot_execute(self):
        with patch('sys.argv', ['validate_runtime_architecture.py', '--require-cutover']), \
                contextlib.redirect_stderr(io.StringIO()), patch.object(validator, 'execute') as execute:
            with self.assertRaises(SystemExit): validator.main()
            execute.assert_not_called()

    def test_generation_failure_precedes_candidate_load_and_execution(self):
        with patch('sys.argv', self.arguments()), \
                patch.object(validator, 'inherited_generation_lease', side_effect=ValueError('generation open')), \
                patch.object(validator, 'load_candidate') as load, patch.object(validator, 'execute') as execute:
            with self.assertRaisesRegex(ValueError, 'generation'): validator.main()
            load.assert_not_called(); execute.assert_not_called()

    def test_wrong_candidate_digest_prevents_native_execution(self):
        with patch('sys.argv', self.arguments()), \
                patch.object(validator, 'inherited_generation_lease', return_value=SimpleNamespace(check=lambda: None)), \
                patch.object(validator, 'load_candidate', return_value={'sha256': 'b' * 64}), \
                patch.object(validator, 'execute') as execute:
            with self.assertRaisesRegex(ValueError, 'digest mismatch'): validator.main()
            execute.assert_not_called()

    def test_candidate_drift_after_child_execution_cannot_publish_success(self):
        with patch('sys.argv', self.arguments()), \
                patch.object(validator, 'inherited_generation_lease', return_value=SimpleNamespace(check=lambda: None)), \
                patch.object(validator, 'load_candidate', side_effect=[{'sha256': 'a' * 64}, {'sha256': 'b' * 64}]), \
                patch.object(validator, 'execute', return_value={'cutover_ready': True}) as execute, \
                contextlib.redirect_stdout(io.StringIO()) as output:
            with self.assertRaisesRegex(ValueError, 'changed during execution'): validator.main()
            execute.assert_called_once(); self.assertEqual(output.getvalue(), '')


if __name__ == '__main__': unittest.main()

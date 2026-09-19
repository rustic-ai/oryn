"""Real byte custody and negative helper admission; native launches are mocked."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import generation_gate as gates
import workflow_native_adapter as native


class GenerationLeaseTests(unittest.TestCase):
    def test_valid_replacement_during_validation_does_not_rebind(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'gate.json'; path.write_text('original')
            def replace(_): path.write_text('different but otherwise valid')
            with self.assertRaisesRegex(ValueError, 'declaration changed'):
                gates.GenerationLease(path, validator=replace)

    def test_revocation_survives_restoration_of_original_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'gate.json'; path.write_text('original')
            lease = gates.GenerationLease(path, validator=lambda _: None)
            path.write_text('changed')
            with self.assertRaises(ValueError): lease.check()
            path.write_text('original')
            with self.assertRaisesRegex(ValueError, 'revoked'): lease.environment({})

    def test_child_requires_exact_original_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'gate.json'; path.write_text('original')
            lease = gates.GenerationLease(path, validator=lambda _: None)
            environment = lease.environment({})
            path.write_text('replacement')
            with patch.dict('os.environ', environment, clear=True), patch.object(gates, 'generation_gate'):
                with self.assertRaisesRegex(ValueError, 'declaration changed'):
                    gates.inherited_generation_lease()

    def test_failed_initial_nested_admission_cannot_be_resurrected(self):
        with patch.object(gates, '_execution_lease', None), patch.object(gates, '_execution_failure', None), \
             patch.object(gates, 'inherited_generation_lease', side_effect=ValueError('changed')) as create:
            with self.assertRaises(ValueError): gates.check_execution_generation()
            with self.assertRaisesRegex(ValueError, 'revoked'): gates.check_execution_generation()
            create.assert_called_once()

    def test_adapter_denial_precedes_output_and_native_process(self):
        import threading
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(native, 'check_execution_generation', side_effect=ValueError('incomplete')), \
                 patch.object(native.subprocess, 'Popen') as launch:
                with self.assertRaisesRegex(ValueError, 'incomplete'):
                    native.execute_native(Path('/native'), Path('/worker'), {}, root, threading.Event())
                launch.assert_not_called()
                self.assertEqual(list(root.iterdir()), [])


class ChildSupervisorAdmissionTests(unittest.TestCase):
    def test_direct_supervisor_helper_requires_frozen_declaration_before_output(self):
        import completion_execution as execution
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(execution.subprocess, 'Popen') as launch:
                with self.assertRaisesRegex(ValueError, 'frozen generation admission'):
                    execution.run_owned(['/native'], root, environment={})
                launch.assert_not_called()
                self.assertEqual(list(root.iterdir()), [])

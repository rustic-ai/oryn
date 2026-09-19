"""Producer supervision regressions; native commands are replaced with fixtures."""
import json
import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import completion_execution as execution


class CompletionExecutionTests(unittest.TestCase):
    def test_architecture_command_binds_candidate_and_raw_output_without_cargo(self):
        candidate = {'path': Path('/frozen/candidate.json'), 'sha256': 'a' * 64, 'artifacts': {}}
        command = execution.command('architecture', candidate, '/gate.json', '/original/architecture')
        self.assertEqual(command[2:], ['--require-cutover', '--candidate', '/frozen/candidate.json',
            '--candidate-sha256', 'a' * 64, '--output', '/original/architecture/raw'])
        self.assertNotIn('cargo', command)

    def test_booking_expected_failure_requires_real_clean_original_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve();candidate=root/'candidate.json';candidate.write_text('{}')
            gate=root/'gate.json';gate.write_text('{}')
            bound=dict(path=candidate,sha256=execution.digest(candidate),record=dict(source_tree_sha256='a'*64),artifacts=dict(worker=root/'worker'))
            argv=[str(root/'regression'),'--exact','booking_mechanism','--nocapture']
            def run(command,target,**kwargs):
                (target/'stdout.log').write_text('test booking_mechanism ... FAILED\ntest result: FAILED. 0 passed; 1 failed;')
                (target/'stderr.log').write_text('')
                return dict(argv=command,returncode=101,timed_out=False,error=None,cleanup_complete=True)
            with patch.object(execution,'generation_gate'),patch.object(execution,'load_candidate',return_value=bound),patch.object(execution,'source_hash',return_value='a'*64),patch.object(execution,'command',return_value=argv),patch.object(execution,'run_owned',side_effect=run):
                receipt=execution.produce('booking-regression',candidate,gate,root/'run')
                path=root/'run/receipt.json'
                execution.verify_receipt(path,'booking-regression',bound,expected_regression_failure=True)
                with self.assertRaisesRegex(ValueError,'cleanly'):execution.verify_receipt(path,'booking-regression',bound)
                for mutate in (lambda value:value['execution'].update(returncode=0),
                               lambda value:value['execution'].update(returncode=-9),
                               lambda value:value['execution'].update(returncode=True),
                               lambda value:value['execution'].update(timed_out=True),
                               lambda value:value['execution'].update(cleanup_complete=False),
                               lambda value:value['errors'].append('candidate drift')):
                    changed=copy.deepcopy(receipt);mutate(changed);path.write_text(json.dumps(changed))
                    with self.assertRaisesRegex(ValueError,'cleanly'):
                        execution.verify_receipt(path,'booking-regression',bound,expected_regression_failure=True)

    def test_preflight_gate_replacement_prevents_child_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate = root / 'candidate.json'; candidate.write_text('{}')
            gate = root / 'gate.json'; gate.write_text('{"declaration":"original"}')
            bound = dict(path=candidate, sha256=execution.digest(candidate), record=dict(source_tree_sha256='a'*64),
                         artifacts={name: root/name for name in ('runtime', 'worker', 'framework_build')})
            original_persist = execution.persist
            def persist(path, value):
                original_persist(path, value)
                if path.name == 'preflight.json':
                    gate.write_text('{"declaration":"replacement"}')
            with patch.object(execution, 'generation_gate'), patch.object(execution, 'load_candidate', return_value=bound), \
                 patch.object(execution, 'source_hash', return_value='a'*64), patch.object(execution, 'persist', side_effect=persist), \
                 patch.object(execution, 'run_owned') as launch:
                with self.assertRaises(ValueError):
                    execution.produce('frameworks', candidate, gate, root / 'run')
                launch.assert_not_called()
            self.assertTrue((root / 'run/preflight.json').is_file())
            self.assertFalse((root / 'run/receipt.json').exists())

    def test_global_gate_precedes_candidate_read_or_process_launch(self):
        with patch.object(execution, 'generation_gate', side_effect=ValueError('incomplete')), patch.object(execution, 'load_candidate') as load, patch.object(execution, 'run_owned') as run:
            with self.assertRaisesRegex(ValueError, 'incomplete'):
                execution.produce('frameworks', 'candidate', 'gate', 'output')
            load.assert_not_called()
            run.assert_not_called()

    def test_preflight_candidate_or_source_drift_prevents_child_launch(self):
        for changed in ('candidate', 'source'):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                candidate = root / 'candidate.json'; candidate.write_text('{}')
                gate = root / 'gate.json'; gate.write_text('{}')
                bound = dict(path=candidate, sha256=execution.digest(candidate), record=dict(source_tree_sha256='a'*64),
                             artifacts={name: root/name for name in ('runtime', 'worker', 'framework_build')})
                replacement = dict(bound, sha256='b'*64)
                candidates = [bound, replacement if changed == 'candidate' else bound]
                hashes = ['a'*64, 'b'*64] if changed == 'source' else ['a'*64]
                with patch.object(execution, 'generation_gate'), patch.object(execution, 'load_candidate', side_effect=candidates), \
                     patch.object(execution, 'source_hash', side_effect=hashes), patch.object(execution, 'run_owned') as launch:
                    with self.assertRaisesRegex(ValueError, 'candidate drift before producer launch'):
                        execution.produce('frameworks', candidate, gate, root / 'run')
                    launch.assert_not_called()
                self.assertTrue((root / 'run/preflight.json').is_file())
                self.assertFalse((root / 'run/receipt.json').exists())

    def test_original_preflight_failure_retention_and_receipt_integrity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate = root / 'candidate.json'; candidate.write_text('{}')
            gate = root / 'gate.json'; gate.write_text('{}')
            bound = dict(path=candidate, sha256=execution.digest(candidate), record=dict(source_tree_sha256='a'*64),
                         artifacts={name: root/name for name in ('runtime', 'worker', 'framework_build')})
            output = root / 'run'
            def run(argv, target, **kwargs):
                self.assertTrue((target / 'preflight.json').is_file())
                (target / 'stdout.log').write_text('original failed native run')
                (target / 'stderr.log').write_text('failure retained')
                return dict(argv=argv, returncode=1, timed_out=False, error=None, cleanup_complete=True)
            with patch.object(execution, 'generation_gate'), patch.object(execution, 'load_candidate', return_value=bound), patch.object(execution, 'source_hash', return_value='a'*64), patch.object(execution, 'run_owned', side_effect=run):
                receipt = execution.produce('frameworks', candidate, gate, output)
            self.assertFalse(receipt['producer_completed'])
            self.assertFalse(receipt['qualified'])
            self.assertTrue(receipt['artifacts'])
            with self.assertRaisesRegex(ValueError, 'cleanly'):
                execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)

    def test_receipt_cannot_change_command_or_drop_raw_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate = root / 'candidate.json'; candidate.write_text('{}')
            gate = root / 'gate.json'; gate.write_text('{}')
            bound = dict(path=candidate, sha256=execution.digest(candidate), record=dict(source_tree_sha256='a'*64),
                         artifacts={name: root/name for name in ('runtime', 'worker', 'framework_build')})
            output = root / 'run'
            def run(argv, target, **kwargs):
                (target / 'stdout.log').write_text('retained original output')
                (target / 'stderr.log').write_text('')
                return dict(argv=argv, returncode=0, timed_out=False, error=None, cleanup_complete=True)
            with patch.object(execution, 'generation_gate'), patch.object(execution, 'load_candidate', return_value=bound), patch.object(execution, 'source_hash', return_value='a'*64), patch.object(execution, 'run_owned', side_effect=run):
                receipt = execution.produce('frameworks', candidate, gate, output)
            execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)
            for change in (
                    lambda row: row['execution'].update(returncode=False),
                    lambda row: row['execution'].update(returncode=0.0),
                    lambda row: row.update(errors=None),
                    lambda row: row['execution'].pop('error'),
                    lambda row: row.pop('errors'),
                    lambda row: row.update(execution=None),
                    lambda row: row.update(artifacts=None),
                    lambda row: row.update(schema_version=True)):
                changed = copy.deepcopy(receipt); change(changed)
                (output / 'receipt.json').write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)
            for malformed in (None, [], 1):
                (output / 'receipt.json').write_text(json.dumps(malformed))
                with self.assertRaises(ValueError):
                    execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)
            (output / 'receipt.json').write_text(json.dumps(receipt))
            (output / 'extra-raw.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'inventory'):
                execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)
            (output / 'extra-raw.json').unlink()
            receipt['execution']['argv'] = ['true']
            (output / 'receipt.json').write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError, 'substituted'):
                execution.verify_receipt(output / 'receipt.json', 'frameworks', bound)


    def test_valid_replacement_gate_still_fails_original_run_custody(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            candidate = root / 'candidate.json'; candidate.write_text('{}')
            gate = root / 'gate.json'; gate.write_text('{"declaration":"original"}')
            original = execution.reference(gate)
            bound = dict(path=candidate, sha256=execution.digest(candidate),
                         record=dict(source_tree_sha256='a'*64),
                         artifacts={name: root/name for name in ('runtime', 'worker', 'framework_build')})
            output = root / 'run'
            def run(argv, target, **kwargs):
                gate.write_text('{"declaration":"replacement also structurally valid"}')
                (target / 'stdout.log').write_text('retain successful child output despite custody failure')
                (target / 'stderr.log').write_text('')
                return dict(argv=argv, returncode=0, timed_out=False, error=None, cleanup_complete=True)
            with patch.object(execution, 'generation_gate') as validate, \
                 patch.object(execution, 'load_candidate', return_value=bound), \
                 patch.object(execution, 'source_hash', return_value='a'*64), \
                 patch.object(execution, 'run_owned', side_effect=run):
                receipt = execution.produce('frameworks', candidate, gate, output)
            # Lease admission adds checks around each immutable boundary.
            self.assertGreaterEqual(validate.call_count, 3)
            self.assertFalse(receipt['producer_completed'])
            self.assertTrue(receipt['errors'])
            self.assertEqual(json.loads((output / 'preflight.json').read_text())['generation_gate'], original)
            self.assertEqual(receipt['execution']['returncode'], 0)
            self.assertTrue(any(item['path'].endswith('stdout.log') for item in receipt['artifacts']))

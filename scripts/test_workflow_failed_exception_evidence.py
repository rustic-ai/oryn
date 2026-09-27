"""Failed adapter projections retain failure independently of terminal custody."""
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from workflow_contract import DEFAULT_CONTRACT, planned_attempts
from workflow_evidence import reconcile


class FailedExceptionEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.contract = json.loads(DEFAULT_CONTRACT.read_text())
        self.attempt = planned_attempts(self.contract)[0]
        self.candidate = dict(parent_sha256='a'*64, worker_sha256='b'*64,
                              source_tree_sha256='c'*64)
        self.row = dict(self.attempt, **self.candidate, passed=False,
                        timed_out=False, interrupted=False, disposition='failed',
                        errors=['adapter failure: cardinality mismatch'],
                        primary_error='cardinality mismatch', cleanup_errors=[],
                        cleanup_uncertain=False, terminal_evidence_errors=[])
        self.waves = [self.wave(100, 'cardinality mismatch')]

    def wave(self, pid, error=None):
        return {
            'native-execution-terminal.json': dict(parent_pid=pid, returncode=0,
                primary_error=None, timed_out=False, interrupted=False),
            'native-process-cleanup.json': dict(parent_pid=pid, process_group=pid,
                returncode=0, group_settled=True, discovery_sealed=True,
                errors=[], unresolved_candidates=[]),
            'fixture-cleanup.json': dict(primary_error=error, errors=[], custody_lost=False,
                receipts=[dict(returncode=0, forced=False)]),
        }

    def publish(self):
        prefix = Path('artifacts')/hashlib.sha256(self.row['attempt_id'].encode()).hexdigest()
        self.row['terminal_evidence'] = []
        for index, wave in enumerate(self.waves):
            for name, value in wave.items():
                path = prefix/f'mixed-{index}'/name
                target = self.root/path
                target.parent.mkdir(parents=True, exist_ok=True)
                data = json.dumps(value).encode()
                target.write_bytes(data)
                self.row['terminal_evidence'].append(dict(path=str(path),
                    sha256=hashlib.sha256(data).hexdigest()))

    def result(self):
        return reconcile(self.contract, self.candidate, [self.row], self.root, {},
                         attempt_ids=[self.row['attempt_id']])

    def diagnostic(self, result):
        return result['terminal_diagnostics'][self.row['attempt_id']]

    def test_failed_projection_with_clean_waves_retains_failure_and_denominator(self):
        self.waves.insert(0, self.wave(99))
        self.publish()
        original = copy.deepcopy(self.row)
        result = self.result()
        self.assertEqual(self.diagnostic(result)['cleanup_status'], 'settled')
        self.assertEqual(result['planned'], 1)
        self.assertEqual(result['missing_attempts'], [])
        self.assertEqual(result['validated'], [])
        self.assertFalse(result['structurally_valid'])
        self.assertTrue(any('failed attempt retained' in error for error in result['errors']))
        self.assertFalse(any('fixture cleanup failed' in error or 'missing artifact reference' in error
                             for error in result['errors']))
        self.assertEqual(self.row, original)
        self.assertNotIn('functional_passed', self.row)
        self.assertNotIn('actions', self.row)

    def test_failed_exception_missing_or_unpinned_terminal_is_unverified(self):
        for mode in ('empty', 'digest', 'missing'):
            with self.subTest(mode=mode):
                self.publish()
                if mode == 'empty': self.row['terminal_evidence'] = []
                elif mode == 'digest': self.row['terminal_evidence'][0]['sha256'] = '0'*64
                else: (self.root/self.row['terminal_evidence'][0]['path']).unlink()
                self.assertEqual(self.diagnostic(self.result())['cleanup_status'], 'unverified')

    def test_other_wave_or_other_error_cannot_supply_initiating_failure(self):
        self.waves = [self.wave(99), self.wave(100, 'different failure')]
        self.publish()
        diagnostic = self.diagnostic(self.result())
        self.assertEqual(diagnostic['cleanup_status'], 'unverified')
        self.assertIn('initiating execution terminal unavailable', diagnostic['errors'])

    def test_unsettled_or_failed_cleanup_is_not_promoted(self):
        cases = [('native-process-cleanup.json', 'group_settled', False, 'uncertain'),
                 ('native-process-cleanup.json', 'discovery_sealed', False, 'uncertain'),
                 ('native-process-cleanup.json', 'errors', ['unreaped'], 'failed'),
                 ('fixture-cleanup.json', 'custody_lost', True, 'uncertain'),
                 ('fixture-cleanup.json', 'receipts', [dict(returncode=-9, forced=True)], 'failed')]
        for name, key, value, expected in cases:
            with self.subTest(name=name, key=key):
                self.waves = [self.wave(100, self.row['primary_error'])]
                self.waves[0][name][key] = value
                self.publish()
                self.assertEqual(self.diagnostic(self.result())['cleanup_status'], expected)

    def test_native_result_or_process_mismatch_remains_unverified(self):
        for key, value in [('parent_pid', 999), ('returncode', -9)]:
            with self.subTest(key=key):
                self.waves = [self.wave(100, self.row['primary_error'])]
                self.waves[0]['native-process-cleanup.json'][key] = value
                self.publish()
                self.assertEqual(self.diagnostic(self.result())['cleanup_status'], 'unverified')

    def test_fixture_initiator_does_not_relax_timeout_or_interruption(self):
        for disposition in ('timed_out', 'interrupted'):
            with self.subTest(disposition=disposition):
                self.row.update(disposition=disposition, timed_out=disposition=='timed_out',
                                interrupted=disposition=='interrupted')
                self.publish()
                self.assertEqual(self.diagnostic(self.result())['cleanup_status'], 'unverified')

    def test_success_shaped_failed_result_keeps_normal_evidence_validation(self):
        self.publish()
        del self.row['terminal_evidence']
        result = self.result()
        self.assertEqual(result['terminal_diagnostics'], {})
        self.assertTrue(any('fixture cleanup failed' in error for error in result['errors']))
        self.assertTrue(any('missing artifact reference' in error for error in result['errors']))

    def test_failed_exception_cannot_claim_pass_or_foreign_candidate(self):
        self.publish()
        self.row['passed'] = True
        self.row['worker_sha256'] = 'd'*64
        result = self.result()
        self.assertEqual(result['validated'], [])
        self.assertTrue(any('execution disposition contradicts flags' in error for error in result['errors']))
        self.assertTrue(any('candidate identity mismatch' in error for error in result['errors']))


if __name__ == '__main__':
    unittest.main()

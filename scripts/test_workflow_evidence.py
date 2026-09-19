"""Negative controls for exact matrix accounting and independent workflow evidence."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from workflow_contract import DEFAULT_CONTRACT, DIMENSIONS, planned_attempts
from workflow_evidence import artifact, reconcile


class WorkflowEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.contract = json.loads(DEFAULT_CONTRACT.read_text())
        self.candidate = dict(parent_sha256='a'*64, worker_sha256='b'*64, source_tree_sha256='c'*64)
        self.oracles = {cell['id']: {'outcome': cell['id']} for family in self.contract['families'] for cell in family['cells']}
        actions = self.write('actions.json', [dict(action_id=1, execution_domain='native')])
        self.records = []
        for i, attempt in enumerate(planned_attempts(self.contract)):
            count = attempt['context_count']
            row = dict(attempt, **self.candidate, passed=True, errors=[], timed_out=False,
                       cleanup=dict(returncode=0, forced=False), actions=actions,
                       oracle=self.write('oracle-'+attempt['cell']+'.json', self.oracles[attempt['cell']]),
                       context_activity=self.write(f'activity-{count}.json', [dict(context_id=str(n), start_ns=10+n, end_ns=100+n) for n in range(count)]),
                       frontend_dimensions=sorted(DIMENSIONS))
            if attempt['fault']:
                row['fault_evidence'] = self.write('fault-'+attempt['fault']+'.json', dict(fault=attempt['fault'], injected=True))
            if attempt['cell'] in ('forms.confinement', 'saas.isolation'):
                contexts = [dict(id=f'owner-{n}', context_id=str(n)) for n in range(count)]
                contexts.append(dict(id='probe', context_id='probe', adversarial_for='owner-0'))
                row['context_topology'] = self.write(f'topology-{count}.json', [dict(
                    cohort_count=count, probe_count=1, total_live_contexts=count+1, contexts=contexts)])
                row['adversarial_evidence'] = self.write('probe.json', [dict(passed=True)])
            self.records.append(row)

    def tearDown(self):
        self.temporary.cleanup()

    def write(self, name, value):
        data = json.dumps(value).encode()
        (self.root/name).write_bytes(data)
        return dict(path=name, sha256=hashlib.sha256(data).hexdigest())

    def result(self, records=None):
        return reconcile(self.contract, self.candidate, self.records if records is None else records,
                         self.root, self.oracles)

    def test_structurally_complete_evidence_is_reconciled(self):
        self.assertTrue(self.result()['structurally_valid'])

    def test_resource_wave_cannot_hide_failed_accounting_or_foreign_cleanup(self):
        baseline=dict(accounting=dict(complete=True,errors=[]),
                      samples=dict(parent_pid=123,overflow=False,errors=[],samples=[{}]),
                      process_cleanup=dict(parent_pid=123,returncode=0,errors=[]))
        def attach(values):
            resource={key:self.write('resource-'+key+'.json',value) for key,value in values.items()}
            self.records[0]['resource_evidence']=self.write('resources.json',[resource])
        attach(baseline)
        self.assertTrue(self.result()['structurally_valid'])
        for component,field,value in [
            ('accounting','complete',False),('accounting','errors',['unowned worker']),
            ('samples','overflow',True),('samples','samples',[]),
            ('process_cleanup','parent_pid',999),('process_cleanup','returncode',False),
            ('process_cleanup','errors',['survivor']),
        ]:
            with self.subTest(component=component,field=field):
                changed=copy.deepcopy(baseline);changed[component][field]=value
                attach(changed)
                self.assertFalse(self.result()['structurally_valid'])
        changed=copy.deepcopy(baseline);changed.pop('process_cleanup');attach(changed)
        self.assertFalse(self.result()['structurally_valid'])

    def test_manifest_bound_run_cannot_accept_legacy_or_relabelled_attempt(self):
        self.candidate['candidate_manifest_sha256'] = 'd' * 64
        self.assertFalse(self.result()['structurally_valid'])
        for row in self.records:
            row['candidate_manifest_sha256'] = 'd' * 64
        # Identity control isolates the separately tested native frontend witnesses.
        with patch('workflow_evidence.validate_frontend', return_value=DIMENSIONS), \
                patch('workflow_raw_evidence.validate'):
            self.assertTrue(self.result()['structurally_valid'])
        self.records[0]['candidate_manifest_sha256'] = 'e' * 64
        self.assertFalse(self.result()['structurally_valid'])

    def test_missing_attempt_cannot_be_replaced_by_duplicate_or_retry(self):
        rows = copy.deepcopy(self.records)
        missing = rows.pop()
        rows.append(copy.deepcopy(rows[0]))
        self.assertFalse(self.result(rows)['structurally_valid'])
        rows[-1] = dict(missing, attempt_id='retry-1', retry_of=missing['attempt_id'])
        self.assertFalse(self.result(rows)['structurally_valid'])

    def test_successful_retry_preserves_failed_original(self):
        rows = copy.deepcopy(self.records)
        rows.append(dict(rows[0], attempt_id='retry-1', retry_of=rows[0]['attempt_id']))
        rows[0]['passed'] = False
        self.assertFalse(self.result(rows)['structurally_valid'])

    def test_sequential_contexts_do_not_satisfy_mixed_concurrency(self):
        row = next(row for row in self.records if row['context_count'] == 5)
        row['context_activity'] = self.write('sequential.json', [dict(context_id=str(n), start_ns=n*10, end_ns=n*10+5) for n in range(5)])
        self.assertTrue(any('simultaneously' in error for error in self.result()['errors']))

    def test_oracle_candidate_cleanup_and_fault_controls(self):
        original = copy.deepcopy(self.records)
        mutations = [
            lambda rows: rows[0].update(worker_sha256='d'*64),
            lambda rows: rows[0].update(cleanup=dict(returncode=0, forced=True)),
            lambda rows: rows[0].update(oracle=self.write('wrong-oracle.json', {'outcome': 'page says success'})),
            lambda rows: next(row for row in rows if row['fault']).pop('fault_evidence'),
            lambda rows: rows[0].update(actions=self.write('empty-actions.json', [])),
        ]
        for mutation in mutations:
            rows = copy.deepcopy(original)
            mutation(rows)
            self.assertFalse(self.result(rows)['structurally_valid'])

    def test_artifact_digest_and_root_confinement(self):
        ref = self.write('local.json', {})
        (self.root/'local.json').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'digest'):
            artifact(self.root, ref)
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            artifact(self.root, dict(path='../escape.json', sha256='0'*64))

    def test_missing_or_false_like_outcome_and_cleanup_fields_are_not_success(self):
        for field, value in [('errors', None), ('errors', ''), ('timed_out', None),
                             ('timed_out', 0), ('cleanup', dict(returncode=False, forced=False)),
                             ('cleanup', dict(returncode=0, forced=0))]:
            with self.subTest(field=field, value=value):
                rows = copy.deepcopy(self.records)
                rows[0][field] = value
                result = self.result(rows)
                self.assertFalse(result['structurally_valid'])
                self.assertNotIn(rows[0]['attempt_id'], result['validated'])
        for field in ('errors', 'timed_out', 'cleanup'):
            rows = copy.deepcopy(self.records)
            del rows[0][field]
            self.assertFalse(self.result(rows)['structurally_valid'])

    def test_artifact_read_checks_open_file_and_enforces_actual_byte_limit(self):
        data = b'x' * 33
        path = self.root/'bounded.json'
        path.write_bytes(data)
        reference = dict(path=path.name, sha256=hashlib.sha256(data).hexdigest())
        with patch('workflow_evidence.MAX_ARTIFACT_BYTES', 32):
            with self.assertRaisesRegex(ValueError, 'oversized'):
                artifact(self.root, reference)
            # Simulate a file growing after descriptor metadata was observed.
            metadata = path.stat()
            fields = list(metadata)
            fields[6] = 0
            with patch('workflow_evidence.os.fstat', return_value=os.stat_result(fields)):
                with self.assertRaisesRegex(ValueError, 'oversized'):
                    artifact(self.root, reference)

    @unittest.skipUnless(hasattr(os, 'mkfifo'), 'requires native FIFO support')
    def test_artifact_fifo_is_rejected_without_waiting_for_a_writer(self):
        path = self.root/'pipe.json'
        os.mkfifo(path)
        with self.assertRaisesRegex(ValueError, 'missing or oversized'):
            artifact(self.root, dict(path=path.name, sha256='0' * 64))

    def test_no_records_is_missing_evidence_not_vacuous_success(self):
        self.assertFalse(self.result([])['structurally_valid'])

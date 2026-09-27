"""Pinned focused status receipts, candidate isolation and legacy dispatch."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('current_status', Path(__file__).with_name('render-current-status.py'))
status = importlib.util.module_from_spec(spec)
spec.loader.exec_module(status)


class FocusedStatusTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.old_root, self.old_summary = status.ROOT, status.SUMMARY
        status.ROOT, status.SUMMARY = self.root, self.root/'docs/CURRENT_STATUS.md'
        self.addCleanup(self.restore)
        self.source = 'a'*64

    def restore(self):
        status.ROOT, status.SUMMARY = self.old_root, self.old_summary

    def pin(self, name, value):
        path = self.root/name
        path.parent.mkdir(parents=True, exist_ok=True)
        data = value.encode() if isinstance(value, str) else json.dumps(value).encode()
        path.write_bytes(data)
        return dict(path=name, sha256=hashlib.sha256(data).hexdigest())

    def record(self, gates=None):
        return dict(schema_version='focused-repair-v1', as_of='2026-09-22',
                    current_candidate=dict(id='next', source_tree_sha256=None, judgments={}, gates={}),
                    previous_baseline=dict(id='old', source_tree_sha256=self.source, judgments={}, gates=gates or {}),
                    external_proposals={}, remaining=[])

    def gate(self, kind='rust', rc=0, source=None, text=None):
        receipt = self.pin('run/result.json', dict(source_tree_sha256=source or self.source,
            returncode=rc, passed=rc == 0, errors=[], steps=[dict(returncode=rc, cleanup_complete=True)]))
        log = self.pin('run/process-0/stdout.log', text or 'test result: ok. 64 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1s\n')
        return dict(kind=kind, receipt=receipt, logs=[log])

    def test_baseline_passes_cannot_populate_unexecuted_successor(self):
        record = self.record({'native': self.gate()})
        record['current_candidate']['gates']['native'] = dict(kind='rust', status='passed', counts={'passed':999})
        result = status.derive(record)
        self.assertEqual(result['previous_baseline']['gates']['native']['counts']['passed'], 64)
        current = result['current_candidate']['gates']['native']
        self.assertEqual(current['status'], 'not_run')
        self.assertNotIn('counts', current)
        self.assertFalse(result['qualified'])
        self.assertIn('Previous executed baseline', status.render(result))
        self.assertEqual(result, status.derive(result))

    def test_failed_rust_population_still_has_terminal_counts(self):
        gate = self.gate(rc=101, text='test result: FAILED. 64 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1s\n')
        derived = status.derive(self.record({'native':gate}))['previous_baseline']['gates']['native']
        self.assertEqual(derived['status'], 'failed')
        self.assertEqual(derived['counts']['total'], 65)

    def test_changed_pinned_log_rejected(self):
        record = self.record({'native': self.gate()})
        (self.root/'run/process-0/stdout.log').write_text('forged')
        with self.assertRaisesRegex(ValueError, 'Evidence changed'):
            status.derive(record)

    def test_foreign_source_or_foreign_log_rejected(self):
        with self.assertRaisesRegex(ValueError, 'source mismatch'):
            status.derive(self.record({'native':self.gate(source='b'*64)}))
        gate = self.gate()
        gate['logs'] = [self.pin('other/process-0/stdout.log', 'test result: ok. 1 passed; 0 failed; 0 ignored;\n')]
        with self.assertRaisesRegex(ValueError, 'selected receipt'):
            status.derive(self.record({'native':gate}))

    def test_python_counts_failures_errors_and_skips(self):
        text = 'some tests\nRan 10 tests in 0.100s\n\nFAILED (failures=2, errors=1, skipped=1)\n'
        gate = self.gate(kind='python', rc=1, text=text)
        result = status.derive(self.record({'python':gate}))['previous_baseline']['gates']['python']
        self.assertEqual(result['counts'], dict(passed=6, failed=2, errors=1, skipped=1, total=10))

    def test_pass_cannot_hide_failing_harness(self):
        gate = self.gate(text='test result: FAILED. 2 passed; 1 failed; 0 ignored;\n')
        with self.assertRaisesRegex(ValueError, 'contradicts failing'):
            status.derive(self.record({'native':gate}))

    def test_matrix_input_source_and_incomplete_counts(self):
        receipt = self.pin('matrix/result.json', dict(matrix_passed=False, errors=[],
            counts=dict(serial=dict(passed=23, failed=9, interrupted=1, not_executed=54), mixed_family=dict(not_executed=45))))
        lock = self.pin('matrix/workflow-input-lock.json', dict(source_tree_sha256=self.source))
        gate = dict(kind='matrix', receipt=receipt, input_lock=lock)
        result = status.derive(self.record({'matrix':gate}))['previous_baseline']['gates']['matrix']
        self.assertEqual(result['counts']['serial']['interrupted'], 1)
        self.assertEqual(result['status'], 'failed')
        gate['input_lock'] = self.pin('matrix/workflow-input-lock.json', dict(source_tree_sha256='c'*64))
        with self.assertRaisesRegex(ValueError, 'Matrix source mismatch'):
            status.derive(self.record({'matrix':gate}))

    def test_passing_matrix_requires_exact_focused_denominator(self):
        lock = self.pin('matrix/workflow-input-lock.json', dict(source_tree_sha256=self.source))
        for counts in (dict(serial=dict(passed=1), mixed_family=dict(passed=45)),
                       dict(serial=dict(passed=87), mixed_family=dict(passed=44)),
                       dict(serial=dict(passed=87), mixed_family=dict(passed=45), extra=dict(passed=1))):
            with self.subTest(counts=counts):
                receipt = self.pin('matrix/result.json', dict(matrix_passed=True, errors=[], counts=counts))
                with self.assertRaisesRegex(ValueError, 'exactly 87 serial and 45 mixed'):
                    status.derive(self.record({'matrix':dict(kind='matrix', receipt=receipt, input_lock=lock)}))
        receipt = self.pin('matrix/result.json', dict(matrix_passed=True, errors=[],
            counts=dict(serial=dict(passed=87), mixed_family=dict(passed=45))))
        derived = status.derive(self.record({'matrix':dict(kind='matrix', receipt=receipt, input_lock=lock)}))
        self.assertEqual(derived['previous_baseline']['gates']['matrix']['status'], 'passed')

    def test_judgment_notes_follow_all_table_rows(self):
        record = self.record()
        record['current_candidate']['judgments'] = dict(
            generation=dict(state='in_progress', note='Generation note', evidence=[]),
            review=dict(state='pending', note='Review note', evidence=[]))
        rendered = status.render(status.derive(record))
        self.assertIn('| generation | in_progress | — |\n| review | pending | — |', rendered)
        self.assertLess(rendered.index('| review | pending |'), rendered.index('Generation note'))
        self.assertLess(rendered.index('Review note'), rendered.index('| Gate |'))

    def test_external_proposals_do_not_inherit_passes(self):
        record = self.record()
        record['external_proposals']['repair'] = dict(patch=self.pin('repair.patch','patch'),
            review=self.pin('review.json',dict(verdict='accepted_source_review')), integrated=True)
        with self.assertRaisesRegex(ValueError, 'cannot inherit'):
            status.derive(record)

    def test_pointer_selects_only_artifact_outputs_and_preserves_source_pointer(self):
        self.pin('artifacts/status/current-status.json', self.record())
        pointer = dict(schema_version='focused-repair-pointer-v1',
                       index='artifacts/status/current-status.json',
                       summary='artifacts/status/CURRENT_STATUS.md')
        saved = copy.deepcopy(pointer)
        value, index, summary = status.resolve_status_input(pointer)
        self.assertEqual(value, self.record())
        self.assertEqual(index, self.root/'artifacts/status/current-status.json')
        self.assertEqual(summary, self.root/'artifacts/status/CURRENT_STATUS.md')
        self.assertEqual(pointer, saved)
        pointer['summary'] = 'docs/CURRENT_STATUS.md'
        with self.assertRaisesRegex(ValueError, 'inside repository artifacts'):
            status.resolve_status_input(pointer)

    def test_pointer_rejects_artifact_symlink_escape(self):
        (self.root/'artifacts').mkdir()
        (self.root/'outside').mkdir()
        (self.root/'artifacts/escape').symlink_to(self.root/'outside', target_is_directory=True)
        pointer = dict(schema_version='focused-repair-pointer-v1',
                       index='artifacts/escape/index.json', summary='artifacts/status.md')
        with self.assertRaisesRegex(ValueError, 'inside repository artifacts'):
            status.resolve_status_input(pointer)

    def test_legacy_empty_candidate_index_still_dispatches(self):
        record = dict(candidates={}, external_proposals={}, legacy_marker=True)
        self.assertEqual(status.derive(record), record)


if __name__ == '__main__':
    unittest.main()

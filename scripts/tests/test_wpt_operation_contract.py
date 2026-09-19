"""Every saved operation needs a route or exact pinned conditional exclusion."""
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_operation_contract as operations


class OperationCoverageTests(unittest.TestCase):
    def test_unknown_required_caller_and_missing_native_source_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in {name for paths in operations.OPERATIONS.values() for name in paths}:
                path = root / name; path.parent.mkdir(parents=True, exist_ok=True); path.write_text('source fixture')
            records = [{'operation': name, 'direct_core_source': True, 'source': 'core.html'} for name in operations.OPERATIONS]
            records += [{'operation': name, 'direct_core_source': False, 'source': 'resources/web-bluetooth-bidi-test.js'} for name in operations.CONDITIONAL_BLUETOOTH]
            self.assertEqual(len(records), 38)
            operations.validate(records, root)
            records.append({'operation': 'new_required_operation'})
            with self.assertRaisesRegex(ValueError, 'coverage changed'): operations.validate(records, root)
            records.pop()
            records[-1]['direct_core_source'] = True
            with self.assertRaisesRegex(ValueError, 'required Core caller'): operations.validate(records, root)
            records[-1]['direct_core_source'] = False
            (root / operations.OPERATIONS['send_keys'][0]).unlink()
            with self.assertRaisesRegex(ValueError, 'source missing'): operations.validate(records, root)


class DynamicHelperCoverageTests(unittest.TestCase):
    def test_generated_code_is_not_reclassified_as_a_missing_script_url(self):
        import copy
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'native.rs').write_text('native source fixture')
            rows=[]
            for source,(count,kind) in operations.DYNAMIC_HELPERS.items():
                for _ in range(count):
                    rows.append({'source':source,'classification':'dynamic_loader_parameter','occurrence':len(rows),
                        'review_reason':'explicit caller source','dynamic_review':{'kind':kind,'consumers':['native.rs'],
                            'upstream_dependencies':[{'path':'helper.js','sha256':'a'*64}]}})
            operations.validate_dynamic_helpers(rows,root)
            bad=copy.deepcopy(rows);bad[-1]['dynamic_review']['kind']='required_script_loader'
            with self.assertRaisesRegex(ValueError,'exact caller'):operations.validate_dynamic_helpers(bad,root)
            with self.assertRaisesRegex(ValueError,'inventory changed'):operations.validate_dynamic_helpers(rows[:-1],root)
            bad=copy.deepcopy(rows);bad[-1]['occurrence']=bad[0]['occurrence']
            with self.assertRaisesRegex(ValueError,'duplicate'):operations.validate_dynamic_helpers(bad,root)
            (root/'native.rs').unlink()
            with self.assertRaisesRegex(ValueError,'consumer missing'):operations.validate_dynamic_helpers(rows,root)


class PinnedOccurrenceCustodyTests(unittest.TestCase):
    @staticmethod
    def records():
        import json
        root = Path(__file__).resolve().parents[2] / 'docs/oryn2/wpt-procedures'
        return [json.loads(line) for name in ('core-driver-operation-occurrences-2026-09-16.jsonl',
                                             'core-helper-occurrences-2026-09-16.jsonl')
                for line in (root / name).read_text().splitlines()]

    def test_every_repeated_caller_is_required_even_when_operation_names_survive(self):
        rows = self.records()
        operations.validate_occurrence_identities(rows, require_complete=True)
        removed = next(index for index, row in enumerate(rows) if row.get('operation') == 'Actions')
        reduced = rows[:removed] + rows[removed + 1:]
        self.assertEqual({row['operation'] for row in rows if 'operation' in row},
                         {row['operation'] for row in reduced if 'operation' in row})
        with self.assertRaisesRegex(ValueError, 'operations occurrence count'):
            operations.validate_occurrence_identities(reduced, require_complete=True)

    def test_duplicate_and_reclassified_helper_cannot_preserve_counts(self):
        import copy
        rows = self.records()
        positions = [index for index, row in enumerate(rows) if 'occurrence' in row]
        duplicate = copy.deepcopy(rows)
        duplicate[positions[1]] = copy.deepcopy(duplicate[positions[0]])
        with self.assertRaisesRegex(ValueError, 'helpers occurrence identity'):
            operations.validate_occurrence_identities(duplicate, require_complete=True)
        changed = copy.deepcopy(rows)
        changed[positions[0]]['classification'] = 'inline_resource'
        with self.assertRaisesRegex(ValueError, 'helpers occurrence identity'):
            operations.validate_occurrence_identities(changed, require_complete=True)

    def test_source_scope_is_immutable_but_review_order_and_prose_are_not(self):
        import copy
        rows = list(reversed(self.records()))
        rows[0]['review_reason'] = 'Additional independent source review'
        operations.validate_occurrence_identities(rows, require_complete=True)
        bad = copy.deepcopy(rows)
        row = next(row for row in bad if row.get('operation', '').startswith('bidi.bluetooth.'))
        row['direct_core_source'] = True
        with self.assertRaisesRegex(ValueError, 'operations occurrence identity'):
            operations.validate_occurrence_identities(bad, require_complete=True)
        with self.assertRaisesRegex(ValueError, 'operations occurrence count'):
            operations.validate_occurrence_identities([], require_complete=True)

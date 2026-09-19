"""Frontend witness joins; fixture algorithms retain their separate regression suites."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from completion_provenance import digest
from workflow_contract import DIMENSIONS
from workflow_evidence import artifact
import workflow_frontend_evidence as frontend
import workflow_native_adapter as adapter


class FrontendWitnessTests(unittest.TestCase):
    def test_concrete_witnesses_cover_exact_frozen_frontend_dimensions(self):
        self.assertEqual(set().union(*map(set, frontend.WITNESSES.values())), DIMENSIONS)
        self.assertNotIn(('shopping', 1, 'persisted_checkout'), frontend.WITNESSES)

    def test_native_plan_result_and_sealed_outcome_are_all_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def saved(name, value):
                path = root / name; path.write_text(json.dumps(value))
                return {'path': name, 'sha256': digest(path)}
            recipe = ('research', 2, 'hydrated_pagination')
            plan = {'attempt_id': 'a', 'sessions': [{'id': 'context-0', 'fault': None,
                'steps': [{'oil': 'goto "http://fixture/search"'}, {'oil': 'observe'}]}]}
            session = {'id': 'context-0', 'errors': [], 'cleanup_errors': [], 'workers_retired': True,
                       'steps': [{'index': i, 'error': None, 'timed_out': False, 'outputs': []} for i in range(2)]}
            evidence = {'schema_version': 1, 'attempt_id': 'a', 'infrastructure_errors': [],
                        'runtime': {'worker_binary_sha256': 'b' * 64}, 'sessions': [session]}
            claim = {'recipe': list(recipe), 'session': 'context-0', 'base': 'http://fixture/search',
                     'sources': frontend.source_records(recipe), 'native_plan': saved('plan.json', plan),
                     'native_evidence': saved('native.json', evidence), 'sealed_oracle': saved('oracle.json', {'visits': []})}
            attempt = {'attempt_id': 'a', 'fault': None}; candidate = {'worker_sha256': 'b' * 64}
            def check():
                return frontend.validate_claim(root, claim, attempt, candidate, artifact)
            with patch.object(adapter, 'read_case_contract', return_value={'cases': [{'id': recipe[2]}]}), patch.object(adapter, 'family_lines', return_value=['goto "http://fixture/search"', 'observe']), patch.object(adapter.research, 'validate', return_value=[]) as oracle:
                self.assertEqual(check(), set(frontend.WITNESSES[recipe]))
                oracle.assert_called_once()
                oracle.return_value = ['wrong persisted fact']
                with self.assertRaisesRegex(ValueError, 'independent outcome'):
                    check()
                oracle.return_value = []
                session['steps'].pop()
                claim['native_evidence'] = saved('native.json', evidence)
                with self.assertRaisesRegex(ValueError, 'truncated'):
                    check()
                session['steps'].append({'index': 1, 'error': None, 'timed_out': False, 'outputs': []})
                claim['native_evidence'] = saved('native.json', evidence)
                plan['sessions'][0]['steps'][0]['oil'] = 'observe'
                claim['native_plan'] = saved('plan.json', plan)
                with self.assertRaisesRegex(ValueError, 'task differs'):
                    check()

    def test_empty_evidence_cannot_claim_a_frontend_dimension(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); path = root / 'empty.json'; path.write_text('[]')
            self.assertEqual(frontend.validate(root, {'path': path.name, 'sha256': digest(path)},
                                              {'attempt_id': 'a'}, {}, artifact), set())

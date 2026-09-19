import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_manual_procedures as procedures


class ProcedureTests(unittest.TestCase):
    def test_all_pinned_manual_visual_ids_have_distinct_source_procedures(self):
        root = Path(__file__).resolve().parents[2]
        coverage = [json.loads(line) for line in (root / 'artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl').read_text().splitlines()]
        expected = {row['id']: row for row in coverage if row['tier'] == 'core' and row['test_type'] in ('manual', 'visual')}
        records = [json.loads(line) for line in procedures.DEFAULT_CORPUS.read_text().splitlines()]
        self.assertEqual(len(records), 752)
        self.assertEqual({record['id'] for record in records}, set(expected))
        for record in records:
            self.assertEqual(record['source_sha256'], expected[record['id']]['sha256'])
            self.assertTrue(record['procedure'])
            self.assertTrue(record['expected_observations'])
            self.assertIsNone(record['observations'])
            self.assertIsNone(record['qualification'])
        self.assertEqual(sum('atta' in record for record in records), 392)

    def test_changed_steps_and_unresolved_fixture_cannot_be_reviewed_into_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = {'id': 'manual:/a', 'path': 'a', 'url': '/a', 'source_sha256': 'a'*64, 'inventory_source_sha256': 'a'*64,
                      'upstream_revision': procedures.REVISION, 'procedure': ['press native button'], 'expected_observations': ['dialog'], 'required_platforms': []}
            original = root / 'original.jsonl'
            original.write_text(json.dumps(record))
            copy_path = root / 'copy.jsonl'
            altered = copy.deepcopy(record)
            altered['procedure'] = ['skip the action']
            copy_path.write_text(json.dumps(altered))
            row = {'id': record['id'], 'path': 'a', 'url': '/a', 'sha256': 'a'*64}
            assessment = {'procedure_source': {'path': str(copy_path), 'sha256': procedures.digest(copy_path)}}
            with patch.object(procedures, 'DEFAULT_CORPUS', original):
                self.assertIn('procedure_differs_from_generated_source', procedures.assessment_errors(row, assessment, root))
                record['source_issues'] = [{'issue': 'missing actual helper'}]
                original.write_text(json.dumps(record))
                assessment['procedure_source'] = {'path': str(original), 'sha256': procedures.digest(original)}
                self.assertIn('procedure_has_unresolved_source_prerequisites', procedures.assessment_errors(row, assessment, root))

    def test_native_platform_selects_only_its_exact_atta_assertions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = {'id': 'manual:/a', 'path': 'a', 'url': '/a', 'source_sha256': 'a'*64,
                      'inventory_source_sha256': 'a'*64, 'upstream_revision': procedures.REVISION,
                      'procedure': ['inspect native role'], 'expected_observations': ['source assertions'],
                      'required_platforms': [{'platform': 'macOS', 'api': 'AXAPI'}, {'platform': 'Linux AT-SPI/ATK', 'api': 'ATK'}],
                      'atta': {'steps': [{'element': 'target', 'title': 'role', 'test': {'AXAPI': [['property', 'AXRole', 'is', 'AXButton']], 'ATK': [['property', 'role', 'is', 'push button']]}}]}}
            corpus = root / 'procedures.jsonl'
            corpus.write_text(json.dumps(record))
            artifact = {'path': 'native-observations', 'sha256': 'b'*64}
            assessment = {'platform': 'macOS', 'platform_selection': {'platform': 'macOS', 'api': 'AXAPI'},
                          'procedure_source': {'path': str(corpus), 'sha256': procedures.digest(corpus)},
                          'artifacts': [artifact], 'platform_observations': [{
                              'requirement': record['required_platforms'][0], 'observations': 'actual AXRole',
                              'artifacts': [artifact], 'assertions': [{'element': 'target', 'title': 'role',
                                  'assertions': record['atta']['steps'][0]['test']['AXAPI']}]}]}
            row = {'id': record['id'], 'path': 'a', 'url': '/a', 'sha256': 'a'*64}
            # This unit isolates platform verdicts; packet source integrity has its own tests.
            with patch.object(procedures, 'DEFAULT_CORPUS', corpus), patch('wpt_assessment_packet.validate', return_value={}):
                self.assertEqual(procedures.assessment_errors(row, assessment, root), [])
                assessment['platform_selection']['api'] = 'ATK'
                self.assertIn('procedure_platform_api_mismatch', procedures.assessment_errors(row, assessment, root))
                assessment['platform_selection']['api'] = 'AXAPI'
                record['required_platforms'].pop(0)
                corpus.write_text(json.dumps(record))
                assessment['procedure_source']['sha256'] = procedures.digest(corpus)
                self.assertIn('procedure_native_api_assertions_missing', procedures.assessment_errors(row, assessment, root))


class ProfileRepairObservationTests(unittest.TestCase):
    def test_fixture_repair_observation_cannot_be_reused_for_another_profile(self):
        import wpt_assessment_packet
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            manifest=Path(__file__).resolve().parents[2]/'tools/wptrunner-oryn/oryn_wptrunner/fixture-repairs.json'
            repair=json.loads(manifest.read_text())['repairs'][0]
            import wpt_manual_procedures as procedures
            row=dict(id='manual:/'+repair['source_path'],path=repair['source_path'],url='/'+repair['source_path'],sha256=repair['source_sha256'])
            record=dict(id=row['id'],path=row['path'],url=row['url'],source_sha256=row['sha256'],
                inventory_source_sha256=row['sha256'],upstream_revision=procedures.REVISION,
                source_issues=[{'requested_path':repair['request_path'].lstrip('/')}],procedure=['Observe delivery'],
                expected_observations=['Correct resource'],required_platforms=[{'platform':'macOS'}])
            corpus=root/'procedures.jsonl';corpus.write_text(json.dumps(record))
            identity=dict(candidate_sha256='a'*64,registry_sha256='b'*64,profile_id='baseline',profile_sha256='c'*64,
                envelope_sha256='d'*64,run_id='original',collection_id='core',attempt_id='attempt-1',original_run_id='original',platform='macOS')
            observed=dict(candidate_sha256='a'*64,fixture_repairs_sha256=procedures.digest(manifest),profile_identity=identity,
                requests=[dict(method='GET',request_path=repair['request_path'],status=200,target_sha256=repair['target_sha256'])])
            receipt=root/'observation.json';receipt.write_text(json.dumps(observed))
            binding={'path':str(receipt),'sha256':procedures.digest(receipt)}
            assessment=dict(candidate_sha256='a'*64,profile_identity=identity,
                procedure_source={'path':str(corpus),'sha256':procedures.digest(corpus)},
                fixture_repairs_sha256=procedures.digest(manifest),fixture_repair_evidence=binding,artifacts=[binding],
                platform='macOS',platform_selection={'platform':'macOS','api':None},
                platform_observations=[dict(requirement={'platform':'macOS'},observations='Native delivery',artifacts=[binding])])
            with patch.object(procedures,'DEFAULT_CORPUS',corpus),patch.object(wpt_assessment_packet,'validate',return_value={}):
                self.assertEqual(procedures.assessment_errors(row,assessment,root),[])
                for value in (None,{**identity,'profile_id':'other'},{**identity,'envelope_sha256':'f'*64}):
                    observed['profile_identity']=value;receipt.write_text(json.dumps(observed))
                    binding['sha256']=procedures.digest(receipt)
                    self.assertIn('procedure_repair_observation_profile_mismatch',procedures.assessment_errors(row,assessment,root))

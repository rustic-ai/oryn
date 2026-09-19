"""Source controls for v2 research assets, persisted routes and false evidence."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module

runner = load('research_v2_runner', ROOT / 'scripts/run-oil-research-outcomes.py')
backend = load('research_v2_fixture', ROOT / 'test-harness/workflows/research_fixture_v2.py')


class ResearchFrontendTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = Path(self.directory.name) / 'research.sqlite3'
        self.fixture = backend.Fixture(self.database)
        self.case_id = 'a' * 32
        self.base = 'http://127.0.0.1:3100/research/' + self.case_id
        self.fixture.command(dict(command='create', case_id=self.case_id))
        self.contract = json.loads((ROOT / 'test-harness/workflows/research-cases-v2.json').read_text())

    def tearDown(self):
        self.fixture.db.close()
        self.directory.cleanup()

    def test_shell_hash_is_the_delivered_representation_and_ssr_facts_exist(self):
        status, body, _ = self.fixture.get(self.case_id, 'search', {'q': ['energy']})
        self.assertEqual(status, 200)
        self.assertIn('Alpha report', body)
        self.assertIn('data-state="hydrating"', body)
        self.assertEqual(backend.snapshot(self.database, self.case_id)['visits'][-1]['body_sha256'],
                         hashlib.sha256(body.encode()).hexdigest())

    def test_assets_require_an_existing_case_and_have_module_mime(self):
        self.assertEqual(self.fixture.get('b' * 32, 'research-client.mjs', {})[0], 404)
        for asset in backend.ASSETS:
            status, body, headers = self.fixture.get(self.case_id, asset, {})
            self.assertEqual(status, 200)
            self.assertTrue(body)
            self.assertIn(('Content-Type', 'text/javascript; charset=utf-8'), headers)

    def test_exact_persisted_routes_and_extractions_reject_missing_or_extra_evidence(self):
        for case in self.contract['cases']:
            with self.subTest(case=case['id']):
                self.fixture.db.execute('DELETE FROM visits')
                self.fixture.db.commit()
                for visit in case['visits']:
                    self.fixture.get(self.case_id, visit['path'], visit['query'])
                data = backend.snapshot(self.database, self.case_id)
                lines = runner.task_lines(self.contract, case, self.base)
                count = sum(line.split()[0] in ('click', 'type', 'select') for line in lines)
                frames = [dict(kind='action', result=dict(action_id=i+1, execution_domain='native')) for i in range(count)]
                values = json.loads(json.dumps(case['expected']).replace('{base}', '/research/' + self.case_id))
                frames += [dict(kind='structured', value=value) for value in values]
                frames += [dict(kind='observation', observation=dict(page=dict(url=self.base+'/'+case['final'])))]
                self.assertEqual(runner.validate(case, lines, self.base, frames, data), [])
                changed = copy.deepcopy(data)
                changed['visits'].pop()
                self.assertTrue(runner.validate(case, lines, self.base, frames, changed))
                changed = copy.deepcopy(frames)
                changed.insert(0, dict(kind='structured', value='foreign document secret'))
                self.assertTrue(runner.validate(case, lines, self.base, changed, data))

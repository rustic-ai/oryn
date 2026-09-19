import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_source_matrix as matrix


class MatrixTests(unittest.TestCase):
    def test_changed_reference_cannot_inherit_original_source_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'test.html'
            target = root / 'ref.html'
            source.write_text('<link rel=match href=ref.html>')
            target.write_text('original reference')
            digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
            corpus = root / 'matrix.jsonl'
            record = {'path': 'test.html', 'source_sha256': digest(source),
                      'upstream_revision': matrix.REVISION, 'qualification': None,
                      'references': [{'path': 'ref.html', 'sha256': digest(target)}]}
            corpus.write_text(json.dumps(record))
            self.assertEqual(len(matrix.validate(root, [corpus])), 1)
            target.write_text('substituted easy reference')
            with self.assertRaisesRegex(ValueError, 'identity changed'):
                matrix.validate(root, [corpus])
            record['qualification'] = 'PASS'
            corpus.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, 'inferred qualification'):
                matrix.validate(root, [corpus])

    def test_reference_inventory_preserves_entire_core_denominator(self):
        coverage = [json.loads(line) for line in (matrix.ROOT / 'artifacts/wpt-full-baseline/inventory-v3/coverage.jsonl').read_text().splitlines()]
        expected = {row['id']: row for row in coverage if row['tier'] == 'core' and row['test_type'] in ('reftest', 'print-reftest')}
        records = [json.loads(line) for line in next(path for path in matrix.CORPORA if path.name.startswith('core-reference-matrix-')).read_text().splitlines()]
        self.assertEqual(len(records), 1015)
        self.assertEqual({row['id'] for row in records}, set(expected))
        self.assertEqual(sum(len(row['references']) for row in records), 1016)
        for row in records:
            self.assertEqual(row['source_sha256'], expected[row['id']]['sha256'])
            self.assertTrue(row['references'])
            self.assertTrue(all(reference['sha256'] for reference in row['references']))
            self.assertIsNone(row['qualification'])


    def test_effect_assertion_line_is_bound_even_when_file_hash_is_correct(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'effect.html'
            source.write_text('filter: url(#native);\nassert_true(event.isTrusted);\n')
            record = {'path': source.name, 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                      'upstream_revision': matrix.REVISION, 'qualification': None,
                      'evidence': [{'line': 2, 'text': 'assert_true(event.isTrusted);'}]}
            corpus = root / 'effects.jsonl'
            corpus.write_text(json.dumps(record))
            self.assertEqual(len(matrix.validate(root, [corpus])), 1)
            record['evidence'][0]['line'] = 1
            corpus.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, 'assertion occurrence'):
                matrix.validate(root, [corpus])

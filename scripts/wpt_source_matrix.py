"""Validate source identities of the saved G occurrence/reference review.

This is a generation artifact validator, not a conformance executor or a pass
oracle. A classified missing resource does not erase its eventual raw outcome.
"""
import hashlib
import json
import wpt_operation_contract
from pathlib import Path

REVISION = '719d5e38fdd0903a18ed9007aba816c98cc491e0'
ROOT = Path(__file__).resolve().parents[1]
CORPORA = [ROOT / 'docs/oryn2/wpt-procedures' / name for name in (
    'core-manual-visual-2026-09-16.jsonl',
    'core-helper-occurrences-2026-09-16.jsonl',
    'core-driver-operation-occurrences-2026-09-16.jsonl',
    'core-reference-matrix-2026-09-16.jsonl',
    'core-effect-source-evidence-2026-09-16.jsonl')]


def validate(checkout, corpora=None):
    root = Path(checkout).resolve()
    records = []
    for corpus in CORPORA if corpora is None else corpora:
        for line in Path(corpus).read_text().splitlines():
            record = json.loads(line)
            if record.get('upstream_revision') != REVISION or record.get('qualification') is not None:
                raise ValueError('source matrix cannot carry inferred qualification')
            identities = [(record.get('source', record.get('path')), record['source_sha256'])]
            identities.extend((helper['path'], helper['sha256']) for helper in record.get('instruction_helpers', []))
            identities.extend((reference['path'], reference['sha256']) for reference in record.get('references', []))
            identities.extend((dependency['path'], dependency['sha256']) for dependency in record.get('dynamic_review', {}).get('upstream_dependencies', []))
            if record.get('target_sha256'):
                identities.append((record['target'], record['target_sha256']))
            for path, expected in identities:
                source = (root / path).resolve()
                if root not in source.parents or not source.is_file() or hashlib.sha256(source.read_bytes()).hexdigest() != expected:
                    raise ValueError('source matrix identity changed: ' + path)
            if 'operation' in record:
                prefix = 'test_driver.' + record['operation']
                lines = (root / identities[0][0]).read_text().splitlines()
                number = record.get('line')
                if (type(number) is not int or not 1 <= number <= len(lines)
                        or not record.get('literal', '').startswith(prefix) or prefix not in lines[number - 1]):
                    raise ValueError('operation occurrence differs from its pinned source')
            if 'evidence' in record:
                lines = (root / identities[0][0]).read_text().splitlines()
                for occurrence in record['evidence']:
                    number = occurrence.get('line')
                    if (type(number) is not int or not 1 <= number <= len(lines)
                            or occurrence.get('text') != lines[number - 1]):
                        raise ValueError('source matrix assertion occurrence changed')
            records.append(record)
    wpt_operation_contract.validate_occurrence_identities(records, require_complete=corpora is None)
    wpt_operation_contract.validate(records, ROOT)
    wpt_operation_contract.validate_dynamic_helpers(records, ROOT)
    return records

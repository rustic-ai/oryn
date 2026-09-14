#!/usr/bin/env python3
"""Prepare exact-execution review candidates; never automatically qualify evidence."""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit
from wpt_evidence import current_harness_errors


def prepare(data, evidence_sha256, checkout):
    if data.get('schema_version') != 4:
        raise ValueError('review bundles require schema-v4 evidence')
    root = checkout.resolve()
    reviews = {}
    for row in data['assertions']:
        if row.get('status') == 'environment_blocked':
            continue
        files = {}
        for name in [row['path'], *row.get('dependencies', [])]:
            relative = urlsplit(name).path.lstrip('/')
            path = (root / relative).resolve()
            if root not in path.parents:
                raise ValueError('source escapes pinned checkout')
            if path.is_file():
                files[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
        resources = {str(item['sequence']): {field:item.get(field) for field in
            ('url','query_sha256','status','mime','response_sha256','terminal')}
            for item in row.get('resources', [])}
        operations = sorted({event['operation'] for event in (row.get('result') or {}).get('testdriver', [])})
        reviews[row['id']] = {
            'evidence_sha256': evidence_sha256, 'source_files':files,
            'expected_subtests':len((row.get('result') or {}).get('tests', [])),
            'fixture_audit':'', 'automation_audit':'', 'tls_validation_independent':False,
            'verified_dependencies':[], 'verified_resources':{}, 'verified_automation_operations':[],
            'verified_websockets':[], 'expected_resource_errors':{}, 'expected_websocket_errors':{},
            'unobserved_dependencies':{},
            'candidate_observations':{'resources':resources, 'automation_operations':operations,
                'websockets':(row.get('resource_audit') or {}).get('websockets', []),
                'harness_errors':current_harness_errors(row)},
        }
    return reviews


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    payload = args.evidence.read_bytes()
    result = prepare(json.loads(payload), hashlib.sha256(payload).hexdigest(), args.checkout)
    # Exclusive creation preserves existing manual reviews.
    with args.output.open('x') as output:
        json.dump(result, output, indent=2, sort_keys=True)
        output.write('\n')


if __name__ == '__main__':
    main()

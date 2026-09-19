#!/usr/bin/env python3
"""Compare complete standard/legacy Core runs without merging per-test outcomes."""
import argparse
import json
from pathlib import Path
from completion_execution import reference
from wpt_execution_evidence import verify, feature_profile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--standard', type=Path, required=True, help='Standard-profile execution.json')
    parser.add_argument('--legacy', type=Path, required=True, help='Legacy-profile execution.json')
    args = parser.parse_args()
    records, inputs = [], {}
    for label, path in (('standard', args.standard), ('legacy', args.legacy)):
        record = json.loads(path.read_text())
        verified = verify(path.parent / 'wptreport.json', path, record['candidate_sha256'], 'core')
        if verified['feature_profile']['web_features']['usermedia_element'] != label:
            raise ValueError('native feature profile differs from requested report label')
        records.append(verified)
        inputs[label] = reference(path)
    result = feature_profile.comparison(*records)
    result['executions'] = inputs
    result['qualification'] = 'configuration comparison only; retain each full raw report and profile attribution'
    print(json.dumps(result, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()

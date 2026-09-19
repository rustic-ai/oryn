#!/usr/bin/env python3
"""Write a NEW full Core ledger from pinned inventory and official reports."""
import argparse
import json
from pathlib import Path
from wpt_full_qualification import reconcile, validate_inventory, bind_inventory, sha256
import wpt_source_matrix
from wpt_execution_evidence import verify as verify_execution
from completion_provenance import load_candidate, read_record


def merge_inputs(target, additions):
    for path, expected in additions.items():
        if path in target and target[path] != expected:
            raise ValueError('qualification input identity changed: ' + path)
        target[path] = expected


def capture_input(inputs, destinations, path, expected=None):
    original = Path(path).absolute()
    resolved = original.resolve()
    observed = sha256(original)
    if expected is not None and observed != expected:
        raise ValueError('qualification input changed: ' + str(original))
    if str(original) in destinations and destinations[str(original)] != str(resolved):
        raise ValueError('qualification input destination changed: ' + str(original))
    destinations[str(original)] = str(resolved)
    merge_inputs(inputs, {str(original): observed, str(resolved): observed})


def evidence_inputs(value, checkout, destinations=None):
    """Retain nested prerequisite/assessment bytes, not only their parent JSON."""
    inputs = {}
    if destinations is None:
        destinations = {}
    if isinstance(value, dict):
        if set(value) == {'path', 'sha256'}:
            read_record(value)
            capture_input(inputs, destinations, value['path'], value['sha256'])
        for name, expected in (value.get('source_files') or {}).items():
            path = Path(checkout).resolve() / name
            if Path(checkout).resolve() not in path.resolve().parents:
                raise ValueError('reviewed source escapes or changed')
            capture_input(inputs, destinations, path, expected)
        for child in value.values():
            merge_inputs(inputs, evidence_inputs(child, checkout, destinations))
    elif isinstance(value, list):
        for child in value:
            merge_inputs(inputs, evidence_inputs(child, checkout, destinations))
    return inputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--inventory', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--policy', type=Path, default=Path(__file__).resolve().parents[1] / 'benchmarks/conformance/wpt-capability-tiers-v2.json')
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--report', type=Path, action='append', default=[])
    parser.add_argument('--reviews', type=Path, required=True)
    parser.add_argument('--assessments', type=Path)
    parser.add_argument('--attribution', type=Path)
    parser.add_argument('--candidate-sha256', required=True)
    parser.add_argument('--candidate', type=Path)
    parser.add_argument('--allow-historical', action='store_true', help='Explicit historical single-profile reproduction only')
    parser.add_argument('--profile-id')
    parser.add_argument('--disposition-policy', type=Path)
    parser.add_argument('--disposition-applications', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    plan_path = args.inventory / 'plan.json'
    coverage_path = args.inventory / 'coverage.jsonl'
    input_paths = [plan_path, coverage_path, args.manifest, args.policy, args.reviews, *args.report]
    input_paths += [path for path in (args.assessments, args.attribution, args.candidate, args.disposition_policy, args.disposition_applications) if path is not None]
    input_paths += [path.parent / name for path in args.report
                    for name in ('execution.json', 'preflight.json', 'protocol-custody.jsonl')]
    inputs, input_destinations = {}, {}
    for path in input_paths:
        capture_input(inputs, input_destinations, path)
    coverage = [json.loads(line) for line in coverage_path.read_text().splitlines()]
    core = validate_inventory(coverage, json.loads(plan_path.read_text()))
    profile_context = None
    disposition_policy = None
    selected = (args.candidate, args.profile_id, args.disposition_policy, args.disposition_applications)
    if any(value is not None for value in selected):
        if args.allow_historical:
            raise ValueError('required-profile qualification cannot use historical mode')
        if not all(value is not None for value in selected):
            raise ValueError('profile qualification requires candidate, profile, frozen policy and applications')
        from core_profile_contract import load_profile_context
        from wpt_reviewed_dispositions import load_policy
        profile_context = load_profile_context(args.candidate, args.profile_id, args.report)
        if profile_context['candidate_sha256'] != args.candidate_sha256 or profile_context['coverage_sha256'] != sha256(coverage_path):
            raise ValueError('selected profile candidate/coverage differs from reconciliation')
        candidate = load_candidate(args.candidate)
        candidate['core_profiles'] = profile_context['profiles']
        disposition_policy = load_policy(args.disposition_policy, candidate=candidate, coverage=coverage, checkout=args.checkout, profile_context=profile_context)
    elif not args.allow_historical:
        raise ValueError('single-profile reproduction requires explicit --allow-historical')
    bind_inventory(core, args.manifest, args.policy, args.checkout)
    source_review_inputs = {str(path.resolve()): sha256(path) for path in wpt_source_matrix.CORPORA}
    source_review_inputs[str(Path(wpt_source_matrix.__file__).resolve())] = sha256(wpt_source_matrix.__file__)
    source_review_inputs[str(Path(wpt_source_matrix.wpt_operation_contract.__file__).resolve())] = sha256(wpt_source_matrix.wpt_operation_contract.__file__)
    source_records = wpt_source_matrix.validate(args.checkout)
    executions = [verify_execution(path, path.parent / 'execution.json', args.candidate_sha256, 'core',
                                   **({'profile_id': args.profile_id} if profile_context else {}))
                  for path in args.report]
    reports = [(path, json.loads(path.read_text())) for path in args.report]
    reviews = json.loads(args.reviews.read_text())
    attribution = json.loads(args.attribution.read_text()) if args.attribution else {}
    assessments = json.loads(args.assessments.read_text()) if args.assessments else {}
    merge_inputs(inputs, evidence_inputs(reviews, args.checkout, input_destinations))
    merge_inputs(inputs, evidence_inputs(assessments, args.checkout, input_destinations))
    rows, summary = reconcile(core, reports, reviews, args.candidate_sha256, args.checkout, attribution,
                              assessments, profile_context=profile_context, disposition_policy=disposition_policy,
                              disposition_applications=args.disposition_applications)
    summary['raw_executor_nonzero'] = [str(path) for path, execution in zip(args.report, executions) if execution['returncode'] != 0]
    disposed_reports = {attempt['report_sha256'] for row in rows for attempt in row['attempts']
                        if attempt['reviewed_dispositions']}
    summary['executor_failures'] = [str(path) for path, execution in zip(args.report, executions)
        if execution['returncode'] != 0 and not (profile_context and execution['returncode'] == 1
                                               and summary['complete'] and sha256(path) in disposed_reports)]
    if summary['executor_failures']:
        summary['complete'] = False
    merge_inputs(summary['inputs'], inputs)
    if any(sha256(path) != expected for path, expected in source_review_inputs.items()):
        raise ValueError('source review changed during qualification')
    merge_inputs(summary['inputs'], source_review_inputs)
    summary['source_review'] = {'records': len(source_records), 'qualification': None,
                              'scope': 'Pinned source identity and occurrence validation only'}
    if any(str(Path(path).resolve()) != destination for path, destination in input_destinations.items()):
        raise ValueError('qualification input destination changed during reconciliation')
    if any(not Path(path).is_file() or sha256(path) != expected for path, expected in summary['inputs'].items()):
        raise ValueError('qualification input changed during reconciliation')
    args.output.mkdir(parents=True, exist_ok=False)
    ledger = args.output / 'ledger.jsonl'
    ledger.write_text(''.join(json.dumps(row, sort_keys=True) + '\n' for row in rows))
    summary['ledger_sha256'] = sha256(ledger)
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2, sort_keys=True) + '\n')


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Refresh the current status index from pinned, existing receipts; never execute tests."""
import argparse
import copy
import hashlib
import json
import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
RECORD = ROOT / 'docs/oryn2/current-status.json'
SUMMARY = ROOT / 'docs/oryn2/CURRENT_STATUS.md'


def read_ref(ref):
    path = ROOT / ref['path']
    data = path.read_bytes()
    if hashlib.sha256(data).hexdigest() != ref['sha256']:
        raise ValueError(f'Evidence changed: {ref["path"]}')
    return json.loads(data) if path.suffix == '.json' else data


def verify_refs(value):
    if isinstance(value, dict):
        if 'path' in value and 'sha256' in value:
            read_ref(value)
        else:
            for child in value.values():
                verify_refs(child)
    elif isinstance(value, list):
        for child in value:
            verify_refs(child)


def derive(record):
    if record.get("schema_version") == FOCUSED_SCHEMA:
        return derive_focused(record)
    result = copy.deepcopy(record)
    verify_refs(result)
    for candidate_id, candidate in result['candidates'].items():
        declaration = read_ref(candidate['declaration'])
        requirements = declaration['requirements']
        candidate['generated_requirements'] = {
            'total': len(requirements),
            'implementation': sum(v['implementation_generated'] is True for v in requirements.values()),
            'tests': sum(v['tests_generated'] is True for v in requirements.values()),
        }
        for gate in candidate['gates'].values():
            if 'receipt' not in gate:
                continue
            receipt = read_ref(gate['receipt'])
            if receipt.get('candidate', candidate_id) != candidate_id:
                raise ValueError('Receipt candidate mismatch')
            rc = receipt.get('returncode')
            if type(rc) is not int:
                raise ValueError('Missing terminal return code')
            gate['returncode'] = rc
            invalid = any(receipt.get(k) for k in (
                'execution_error', 'postcheck_error', 'postcheck_errors',
                'cleanup_errors', 'changed_inputs', 'dispatch_blocked'))
            invalid |= receipt.get('group_settled') is False
            gate['status'] = ('invalid_evidence' if invalid else
                              'passed' if rc == 0 and receipt.get('passed', True) is True
                              else 'failed')
            if 'terminal_summary' not in gate:
                continue
            summary = read_ref(gate['terminal_summary'])
            if summary['candidate'] != candidate_id:
                raise ValueError('Summary candidate mismatch')
            binding = summary.get('result', summary.get('launch_result'))
            if binding is None or binding['sha256'] != gate['receipt']['sha256']:
                raise ValueError('Summary does not bind selected result')
            if Path(binding['path']).resolve() != (ROOT / gate['receipt']['path']).resolve():
                raise ValueError('Summary result path mismatch')
            counts = summary['counts']
            if 'tests' in counts:
                gate['counts'] = {'passed': summary['passed'], 'failed': counts['failures'],
                                  'errors': counts['errors'], 'skipped': counts['skipped'],
                                  'total': counts['tests']}
                if sum(gate['counts'][k] for k in ('passed', 'failed', 'errors', 'skipped')) != counts['tests']:
                    raise ValueError('Python population does not reconcile')
            else:
                gate['counts'] = counts
            gate['failures'] = summary.get('failures', [summary['failure']] if 'failure' in summary else
                                          [s['test'] for s in summary.get('steps', []) if not s['passed']])
            if gate['status'] == 'passed' and (gate['counts'].get('failed', 0) or gate['counts'].get('errors', 0)):
                raise ValueError('Passing receipt contradicts failing counts')
        gates = candidate['gates']
        candidate['stages']['compiled'] = 'passed' if all(
            gates[k]['status'] == 'passed' for k in ('runtime-build', 'default-compile', 'workspace-compile')) else 'not_passed'
        candidate['stages']['runtime_passed'] = 'passed' if all(
            gates[k]['status'] == 'passed' for k in ('workspace', 'default', 'python', 'extra-ignored', 'media')) else 'not_passed'
        # This index records qualification; it never substitutes for the acceptance validator.
        candidate['stages']['qualified'] = 'not_qualified'
    for proposal in result['external_proposals'].values():
        review = read_ref(proposal['review'])
        if review.get('integrated', review.get('integrated_into_frozen_source', False)):
            raise ValueError('External proposal has integrated evidence; reconcile its classification')
        if proposal['integrated'] or proposal['compiled'] or proposal['runtime_passed'] or proposal['qualified']:
            raise ValueError('External proposal cannot inherit candidate execution')
        proposal['generated'] = True
        proposal['reviewed'] = 'accepted' if review.get('accepted') is True else 'not_accepted'
    return result


def link(ref, label):
    return f'[{label}]({os.path.relpath(ROOT / ref["path"], SUMMARY.parent)})'


def render(record):
    if record.get("schema_version") == FOCUSED_SCHEMA:
        return render_focused(record)
    name = record['active_candidate']
    candidate = record['candidates'][name]
    lines = ['# Current Core and OIL status', '',
             '<!-- Generated by scripts/render-current-status.py; edit current-status.json inputs. -->', '',
             f'As of **{record["as_of"]}** · candidate **{name}** · source batch **{candidate["source_batch"]}** · implementation **{record["implementation_lifecycle"]}**.', '',
             f'Checkpoint commit: `{record["checkpoint_commit"][:8]}`. '+record['scope_note'], '',
             '## Implementation stages', '', '| Stage | State |', '| --- | --- |']
    for key, value in candidate['stages'].items():
        lines.append(f'| {key.replace("_", " ")} | {value.replace("_", " ")} |')
    generated = candidate['generated_requirements']
    lines += ['', f'Declared generation: {generated["implementation"]}/{generated["total"]} implementation requirements and {generated["tests"]}/{generated["total"]} test requirements. '+link(candidate['declaration'], 'Declaration')+'; '+link(candidate['source_assessment'], 'integration/review assessment')+'.', '',
              '## Candidate gates', '', '| Gate | Result | Counts / scope | Evidence |', '| --- | --- | --- | --- |']
    for key, gate in candidate['gates'].items():
        detail = '; '.join(f'{v:,} {k}' for k, v in gate.get('counts', {}).items()) or gate.get('note', '—')
        evidence = link(gate['receipt'], 'receipt') if 'receipt' in gate else link(gate['basis'], 'assessment')
        if 'terminal_summary' in gate:
            evidence += ' · '+link(gate['terminal_summary'], 'counts/failures')
        lines.append(f'| {key} | {gate["status"].replace("_", " ")} | {detail} | {evidence} |')
    lines += ['', candidate.get('execution_note', 'See candidate gate receipts for execution coverage; complete qualification remains pending.'), '',
              '## Accepted external proposals', '', '| Proposal | Generated | Reviewed | Integrated | Compiled | Runtime passed | Qualified |', '| --- | --- | --- | --- | --- | --- | --- |']
    for name, proposal in record['external_proposals'].items():
        lines.append(f'| {link(proposal["patch"], name)} | yes | {link(proposal["review"], proposal["reviewed"])} | no | no | no | no |')
    lines += ['', 'Wrapper-graph work is an optional performance proposal; the original capacity test passed.', '', '## Remaining preparation', '']
    for name, description in record['remaining_preparation'].items():
        lines.append(f'- **{name}:** {description}')
    lines += ['', '## Governing references and history', '']
    for name, ref in {**record['references'], **record['history']}.items():
        lines.append('- '+link(ref, name))
    lines += ['', '## Updating this status', '',
              'The single editable index is [current-status.json](current-status.json). Select explicit candidate/receipt references and record source-review judgments there; receipt-derived results are regenerated. Evidence hashes are verified before either output is written. Missing or changed evidence fails the refresh rather than producing a passing status.', '',
              '```sh', record['maintenance']['command'], record['maintenance']['check'], '```', '',
              'Archived reports are immutable. This is an evidence index, not a qualification validator or a fresh test run. Receipt paths refer to retained local artifacts; keep them with the checkout.']
    return '\n'.join(lines)+'\n'


FOCUSED_SCHEMA = 'focused-repair-v1'


def terminal_counts(kind, text):
    """Read terminal harness summaries, not progress lines or inferred test names."""
    if kind == 'rust':
        matches = re.findall(r'^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', text, re.M)
        if len(matches) != 1:
            raise ValueError('Expected exactly one Rust terminal summary per process')
        passed, failed, skipped = map(int, matches[0])
        return dict(passed=passed, failed=failed, errors=0, skipped=skipped,
                    total=passed + failed + skipped)
    matches = re.findall(r'^Ran (\d+) tests? in [^\n]+\n\s*\n(OK(?: \([^\n]*\))?|FAILED \([^\n]*\))\s*$', text, re.M)
    if len(matches) != 1:
        raise ValueError('Expected exactly one Python terminal summary per process')
    total, terminal = matches[0]
    fields = dict(re.findall(r'(failures|errors|skipped|expected failures|unexpected successes)=(\d+)', terminal))
    counts = {key: int(fields.get(key, 0)) for key in ('failures', 'errors', 'skipped')}
    # Expected failures are not passing tests; unexpected successes fail unittest.
    skipped = counts['skipped'] + int(fields.get('expected failures', 0))
    failed = counts['failures'] + int(fields.get('unexpected successes', 0))
    passed = int(total) - failed - counts['errors'] - skipped
    if passed < 0 or (terminal.startswith('OK') and (failed or counts['errors'])):
        raise ValueError('Python terminal population does not reconcile')
    return dict(passed=passed, failed=failed, errors=counts['errors'], skipped=skipped, total=int(total))


def focused_gate(gate, candidate):
    # Discard editable derived fields: they are never evidence.
    for key in ('status', 'counts', 'returncode'):
        gate.pop(key, None)
    if 'receipt' not in gate:
        gate['status'] = 'not_run'
        return
    receipt = read_ref(gate['receipt'])
    if not isinstance(receipt, dict):
        raise ValueError('Terminal receipt must be JSON')
    source = candidate.get('source_tree_sha256')
    if not isinstance(source, str) or not re.fullmatch(r'[0-9a-f]{64}', source):
        raise ValueError('Executed candidate requires frozen source hash')
    if receipt.get('candidate', candidate['id']) != candidate['id']:
        raise ValueError('Receipt candidate mismatch')
    receipt_path = (ROOT / gate['receipt']['path']).resolve()
    kind = gate['kind']
    if kind == 'matrix':
        lock_ref = gate['input_lock']
        if (ROOT / lock_ref['path']).resolve() != receipt_path.parent / 'workflow-input-lock.json':
            raise ValueError('Matrix input lock must belong to selected receipt directory')
        lock = read_ref(lock_ref)
        if lock.get('source_tree_sha256') != source:
            raise ValueError('Matrix source mismatch')
        if receipt.get('source_tree_sha256', source) != source:
            raise ValueError('Receipt source mismatch')
        counts = receipt['counts']
        if not isinstance(counts, dict) or not counts:
            raise ValueError('Missing matrix populations')
        allowed = {'passed', 'failed', 'interrupted', 'not_executed', 'invalid'}
        for group in counts.values():
            if not isinstance(group, dict) or not group or set(group) - allowed:
                raise ValueError('Unknown matrix count population')
            if any(type(n) is not int or n < 0 for n in group.values()):
                raise ValueError('Invalid matrix count')
        passed = receipt.get('matrix_passed') is True
        if passed and (set(counts) != {'serial', 'mixed_family'}
                       or sum(counts['serial'].values()) != 87
                       or sum(counts['mixed_family'].values()) != 45):
            raise ValueError('Passing focused matrix must contain exactly 87 serial and 45 mixed attempts')
        incomplete = any(any(n for k, n in group.items() if k != 'passed') for group in counts.values())
        if passed and (incomplete or receipt.get('errors') or not sum(sum(g.values()) for g in counts.values())):
            raise ValueError('Passing matrix contradicts populations/errors')
        gate.update(status='passed' if passed else 'failed', counts=counts)
        return
    if kind not in ('rust', 'python', 'build'):
        raise ValueError('Unknown focused gate kind')
    if receipt.get('source_tree_sha256') != source:
        raise ValueError('Receipt source mismatch')
    rc = receipt.get('returncode')
    if type(rc) is not int:
        raise ValueError('Missing terminal return code')
    steps = receipt.get('steps', [])
    invalid = any(receipt.get(k) for k in ('cleanup_errors', 'changed_inputs', 'postcheck_errors'))
    for step in steps:
        invalid |= step.get('cleanup_complete') is False or step.get('group_settled') is False
        invalid |= bool(step.get('cleanup_errors'))
    success = rc == 0 and receipt.get('passed') is True and not receipt.get('errors')
    if success and any(step.get('returncode') != 0 or step.get('error') or step.get('timed_out') for step in steps):
        raise ValueError('Passing wrapper contradicts process receipt')
    gate.update(returncode=rc, status='invalid_evidence' if invalid else 'passed' if success else 'failed')
    if kind == 'build':
        return
    logs = gate.get('logs', [])
    if len(logs) != len(steps) or not steps:
        raise ValueError('One pinned terminal log is required per process')
    total = dict(passed=0, failed=0, errors=0, skipped=0, total=0)
    for index, ref in enumerate(logs):
        path = (ROOT / ref['path']).resolve()
        if path.parent != receipt_path.parent / f'process-{index}' or path.name not in ('stdout.log', 'stderr.log'):
            raise ValueError('Harness log is not from selected receipt process')
        data = read_ref(ref)
        if not isinstance(data, bytes):
            raise ValueError('Harness log must be text bytes')
        counts = terminal_counts(kind, data.decode('utf-8', errors='replace'))
        for key, value in counts.items():
            total[key] += value
    if success and (total['failed'] or total['errors']):
        raise ValueError('Passing receipt contradicts failing tests')
    gate['counts'] = total


def derive_focused(record):
    result = copy.deepcopy(record)
    # Inspect only explicitly pinned references, never an artifact-directory scan.
    verify_refs(result)
    current = result['current_candidate']
    baseline = result.get('previous_baseline')
    if baseline and current['id'] == baseline['id']:
        raise ValueError('Current and previous candidate identities must differ')
    for candidate in [current] + ([baseline] if baseline else []):
        for judgment in candidate.get('judgments', {}).values():
            if judgment['state'] not in ('pending', 'in_progress', 'complete', 'accepted', 'rejected'):
                raise ValueError('Unknown source judgment')
            if judgment['state'] in ('complete', 'accepted', 'rejected') and not judgment.get('evidence'):
                raise ValueError('Completed source judgment needs pinned evidence')
        for gate in candidate.get('gates', {}).values():
            focused_gate(gate, candidate)
        candidate['qualified'] = False
    for proposal in result.get('external_proposals', {}).values():
        review = read_ref(proposal['review'])
        for key in ('integrated', 'compiled', 'runtime_passed', 'qualified'):
            if proposal.get(key) is True:
                raise ValueError('External proposal cannot inherit integration/execution')
            proposal[key] = False
        if review.get('integrated') or review.get('integrated_into_frozen_source'):
            raise ValueError('Integrated review is misclassified as external')
        # External source review has no implication about runtime qualification.
        review_text = json.dumps(review, sort_keys=True)
        if proposal['patch']['sha256'] not in review_text:
            raise ValueError('External review does not bind selected patch')
        if 'manifest' in proposal and proposal['manifest']['sha256'] not in review_text:
            raise ValueError('External review does not bind selected manifest')
        proposal['reviewed'] = 'accepted' if review.get('verdict') in ('accepted', 'accepted_source_review') or review.get('accepted') is True else 'pending'
    result['qualified'] = False
    return result


def render_focused(record):
    lines = ['# Current Core and OIL status', '',
             '<!-- Generated from pinned focused-repair receipts. -->', '',
             f'As of **{record["as_of"]}**. Qualification remains incomplete.', '']
    for title, candidate in [('Current candidate', record['current_candidate']),
                             ('Previous executed baseline', record.get('previous_baseline'))]:
        if not candidate:
            continue
        lines += [f'## {title}: {candidate["id"]}', '',
                  f'Source: `{candidate.get("source_tree_sha256") or "not frozen"}`.', '',
                  '| Source judgment | State | Evidence |', '| --- | --- | --- |']
        judgment_notes = []
        for name, judgment in candidate.get('judgments', {}).items():
            refs = ' · '.join(link(ref, 'assessment') for ref in judgment.get('evidence', [])) or '—'
            lines.append(f'| {name} | {judgment["state"]} | {refs} |')
            if judgment.get('note'):
                judgment_notes.append(f'**{name}:** {judgment["note"]}')
        for note in judgment_notes:
            lines += ['', note]
        lines += ['', '| Gate | Result | Counts | Evidence |', '| --- | --- | --- | --- |']
        for name, gate in candidate.get('gates', {}).items():
            counts = json.dumps(gate['counts'], sort_keys=True) if 'counts' in gate else '—'
            evidence = link(gate['receipt'], 'terminal receipt') if 'receipt' in gate else 'No terminal receipt'
            evidence += ''.join(' · '+link(ref, 'harness log') for ref in gate.get('logs', []))
            lines.append(f'| {name} | {gate["status"]} | {counts} | {evidence} |')
        lines += ['', 'Results belong only to this source; overlapping suites are not summed.', '']
    lines += ['## External proposals', '']
    for name, proposal in record.get('external_proposals', {}).items():
        lines.append(f'- {link(proposal["patch"], name)}: review {proposal["reviewed"]}; not integrated or executed. '+link(proposal['review'], 'review'))
    lines += ['', '## Remaining work', ''] + ['- '+item for item in record.get('remaining', [])]
    lines += ['', '## Pinned references', '']
    lines += ['- '+link(ref, name) for name, ref in record.get('references', {}).items()]
    lines += ['', 'This is a status index, not an acceptance validator. Previous baseline results do not count as current candidate passes.', '']
    return '\n'.join(lines)


def resolve_status_input(record):
    """A stable repository pointer keeps mutable reporting out of source identity."""
    if record.get('schema_version') != 'focused-repair-pointer-v1':
        return record, RECORD, SUMMARY
    paths = []
    artifact_root = (ROOT / 'artifacts').resolve()
    for key in ('index', 'summary'):
        relative = Path(record[key])
        path = (ROOT / relative).resolve()
        if relative.is_absolute() or not path.is_relative_to(artifact_root) or path == artifact_root:
            raise ValueError('Focused pointer must remain inside repository artifacts')
        paths.append(path)
    index, summary = paths
    if index == summary or index.suffix != '.json' or summary.suffix != '.md':
        raise ValueError('Focused pointer needs distinct JSON and Markdown outputs')
    value = json.loads(index.read_text())
    if value.get('schema_version') != FOCUSED_SCHEMA:
        raise ValueError('Focused pointer must select a focused-repair index')
    return value, index, summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Verify receipts and output freshness without writing')
    args = parser.parse_args()
    global SUMMARY
    inputs, index_path, summary_path = resolve_status_input(json.loads(RECORD.read_text()))
    SUMMARY = summary_path
    record = derive(inputs)
    outputs = {index_path: json.dumps(record, indent=2)+'\n', summary_path: render(record)}
    if args.check:
        for path, expected in outputs.items():
            if not path.exists() or path.read_text() != expected:
                raise SystemExit(f'Stale output: {path}')
        print('Current status is fresh; all pinned evidence hashes verified.')
    else:
        for path, text in outputs.items():
            path.write_text(text)
        print('Refreshed selected status index and summary from existing receipts.')


if __name__ == '__main__':
    main()

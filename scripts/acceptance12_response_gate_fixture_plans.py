"""Export real adapter-generated cancellation plans without launching fixtures.

Run once before the new native admission regression. This is plan generation,
not runtime qualification: no HTTP server, browser, worker, or SQLite is started.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import sys

FAILED_IDS = (
    'forms.validation:fault_injection:1:cancellation:contexts=1',
    'saas.mfa:fault_injection:1:cancellation:contexts=1',
    'saas.session:fault_injection:1:cancellation:contexts=1',
    'booking.stale_slot:fault_injection:1:cancellation:contexts=1',
    'research.stale:fault_injection:1:cancellation:contexts=1',
    'forms.reentrancy:fault_injection:1:cancellation:contexts=1',
    'shopping.stale_quote:fault_injection:1:cancellation:contexts=1',
    'booking.cancel_retry:fault_injection:1:cancellation:contexts=1',
    'shopping.duplicate:fault_injection:1:cancellation:contexts=1',
    'saas.save_recovery:fault_injection:1:cancellation:contexts=1',
)


class ArmRecorder:
    def __init__(self):
        self.requests = []

    def request(self, command, case_id, **fields):
        if command != 'arm_response_fault':
            raise AssertionError('export attempted an operation other than recording fault configuration')
        self.requests.append(copy.deepcopy(dict(command=command, case_id=case_id, **fields)))
        return dict(armed=True)


def export(root, directory):
    root = root.resolve(strict=True)
    summary_path = root / 'artifacts/acceptance11-failed-only-2026-09-24/SUMMARY.json'
    summary_bytes = summary_path.read_bytes()
    summary = json.loads(summary_bytes)
    if summary.get('candidate') != 'acceptance11' or summary.get('failed_ids') != list(FAILED_IDS):
        raise ValueError('exact acceptance11 failure population changed')
    if summary.get('family', {}).get('groups', {}).get('cancellation') != dict(passed=0, failed=10, total=10):
        raise ValueError('expected ten terminal cancellation failures')
    sys.path.insert(0, str(root / 'scripts'))
    import workflow_native_adapter as adapter
    if Path(adapter.__file__).resolve() != root / 'scripts/workflow_native_adapter.py':
        raise ValueError('adapter import came from a different source tree')
    directory = directory.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    plans, arms = [], []
    for ordinal, attempt_id in enumerate(FAILED_IDS):
        cell = attempt_id.split(':', 1)[0]
        # Same recipe inventory and preferred-fault selection as MixedCellAdapter.
        recipes = adapter.MixedCellAdapter(cell, '/unused-worker', '/unused-worker').recipes()
        matches = [recipe for recipe in recipes if recipe[2] == adapter.PREFERRED.get(cell)]
        if len(matches) != 1:
            raise ValueError('fault recipe is missing or ambiguous: ' + cell)
        recipe = matches[0]
        prepared = adapter.PreparedCase.__new__(adapter.PreparedCase)
        prepared.family, prepared.version, prepared.name = recipe
        prepared.directory = directory / f'case-{ordinal}'
        prepared.directory.mkdir()
        prepared.contract = adapter.read_case_contract(*recipe)
        cases = [case for case in prepared.contract['cases'] if case['id'] == prepared.name]
        if len(cases) != 1:
            raise ValueError('concrete fixture case is missing or ambiguous')
        prepared.case = copy.deepcopy(cases[0])
        prepared.runner = (adapter.research if prepared.family == 'research' else
                           adapter.load('fixture_plan_' + prepared.family,
                                        root / f'scripts/run-oil-{prepared.family}-outcomes.py'))
        prepared.identity = hashlib.sha256(attempt_id.encode()).hexdigest()[:32]
        prepared.origin = 'http://127.0.0.1:3100'
        prefix = ('advanced' if prepared.family == 'forms' and prepared.case.get('extended') else
                  dict(research='research', forms='form', shopping='shop', saas='saas', booking='booking')[prepared.family])
        prepared.base = f'{prepared.origin}/{prefix}/{prepared.identity}'
        prepared.lines = adapter.family_lines(prepared.family, prepared.runner,
                                             prepared.contract, prepared.case, prepared.base)
        prepared.oracle = ArmRecorder()
        prepared.fault = None
        prepared.recovery_steps = []
        prepared.response_gate = None
        # This calls the production decorate_fault and emits the actual arm route,
        # expected status, skipped read count, delay and per-step expected error.
        prepared.configure_fault('cancellation')
        session = prepared.session('context-0')
        if len(prepared.oracle.requests) != 1:
            raise AssertionError('expected exactly one real configure_fault arm request')
        request = prepared.oracle.requests[0]
        if session['response_gate'] != request['response_gate'] or request['route'] != session['response_gate']['route']:
            raise AssertionError('producer session and producer arm request disagree')
        if Path(session['response_gate']['marker']).exists():
            raise AssertionError('response marker must remain absent before native regression')
        plans.append(dict(schema_version=1, attempt_id=attempt_id, sessions=[session]))
        arms.append(dict(attempt_id=attempt_id, recipe=list(recipe), request=request))
    output = directory / 'plans.json'
    with output.open('x', encoding='utf-8') as stream:
        json.dump(dict(schema_version=1, plans=plans, arms=arms,
                       summary_sha256=hashlib.sha256(summary_bytes).hexdigest(),
                       runtime_executed=False, qualified=False), stream, indent=2)
        stream.write('\n')
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--output-directory', type=Path, required=True,
                        help='fresh directory; exporter refuses to overwrite prior plans')
    args = parser.parse_args()
    print(export(args.root, args.output_directory))


if __name__ == '__main__':
    main()

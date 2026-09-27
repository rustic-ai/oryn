"""Recompute combined acceptance through actual domain validators and raw data."""
import importlib.util
import json
from pathlib import Path
import sys

from completion_execution import ROOT, persist, reference, run_owned, verify_receipt, source_hash
from completion_provenance import digest, load_candidate, read_record
from generation_gate import generation_gate, GenerationLease
from workflow_contract import planned_attempts
from workflow_evidence import reconcile as reconcile_workflows

# Preserved required_blocks/sample 1 failure, not the later passing recheck.
# Copies of the original bytes are valid evidence; another attempt is not.
BOOKING_ORIGINAL_FAILURE_SHA256 = '0843d5c10df4e893c3ff76d40408446043f32b29c19eb0e180d4e35db775da70'
BOOKING_ORIGINAL_TRACE = {
    'task.oil': 'c1a5216ae56fcd764f55a7b481c2a3a643dea4e349a5d0ea52d8ac7db5e87101',
    'stdout.jsonl': 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    'stderr.txt': 'ca63215b73d4dbbef7feb2d5607401b0dcec3d559df23ff8b62e8e3bc59d61d1',
    'final-database.json': '01e2fa81754c4f9179044320aacd6eddc4fc33796052b6a6a8b80fcd8483f125',
    'oracle.json': '01e2fa81754c4f9179044320aacd6eddc4fc33796052b6a6a8b80fcd8483f125',
    'preliminary-result.json': '0ab471145c52c41125d54900fa734a341f3dc7726dcc7949070f1477da62f484',
}

def module(name):
    spec = importlib.util.spec_from_file_location('completion_' + name.replace('-', '_'), ROOT / 'scripts' / (name + '.py'))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def read(path):
    value = json.loads(Path(path).read_text())
    if not isinstance(value, dict):
        raise ValueError('qualification evidence must be an object')
    return value


def workflow(path, candidate):
    from workflow_native_adapter import MixedCellAdapter, RESEARCH_RECIPES
    from workflow_family_recipes import RECIPES
    path = Path(path)
    plan = read(path / 'plan.json')
    expected_identity = dict(parent_sha256=candidate['record']['artifacts']['workflow_executor']['sha256'],
                             worker_sha256=candidate['record']['artifacts']['worker']['sha256'],
                             source_tree_sha256=candidate['record']['source_tree_sha256'],
                             candidate_manifest_sha256=candidate['sha256'])
    if plan.get('candidate') != expected_identity:
        raise ValueError('workflow plan belongs to another candidate')
    from workflow_input_lock import for_candidate
    contract = for_candidate(candidate)
    attempts = planned_attempts(contract)
    if plan.get('attempts') != attempts:
        raise ValueError('workflow sampling plan changed')
    adapters = {cell: MixedCellAdapter(cell, candidate['artifacts']['workflow_executor'], candidate['artifacts']['worker']) for cell in [*RESEARCH_RECIPES, *RECIPES]}
    expected = {attempt['attempt_id']: adapters[attempt['cell']].expected(attempt['context_count'], attempt['mode'], attempt['fault']) for attempt in attempts}
    if read(path / 'expected-oracles.json') != expected:
        raise ValueError('frozen oracle differs from independently recomputed contract')
    rows = [json.loads(line) for line in (path / 'attempts.jsonl').read_text().splitlines() if line]
    result = reconcile_workflows(contract, expected_identity, rows, path, expected)
    if not result['structurally_valid'] or result['planned'] != len(result['validated']):
        raise ValueError('complete workflow denominator did not pass')
    return result


def framework(raw, candidate):
    validator = module('run-oil-integrated-outcomes')
    value = read(Path(raw) / 'summary.json')
    identity = dict(parent_sha256=candidate['record']['artifacts']['runtime']['sha256'],
                    worker_sha256=candidate['record']['artifacts']['worker']['sha256'],
                    source_tree_sha256=candidate['record']['source_tree_sha256'],
                    framework_build_sha256=candidate['record']['artifacts']['framework_build']['sha256'])
    errors = validator.validate_summary(value, identity, validator.FRAMEWORKS, 3, 'framework')
    if value.get('build_manifest_sha256') != candidate['record']['artifacts']['framework_build']['sha256']:
        errors.append('framework build differs from candidate')
    if errors:
        raise ValueError('framework qualification failed: ' + '; '.join(errors))
    # Recompute each actual OIL response through the original family validator,
    # rather than accepting the stored record's passed flag.
    runner = module('run-oil-framework-outcomes')
    for record in value.get('records', []):
        directory = Path(raw) / (record['framework'] + '-' + format(record['sample'], '02d'))
        frames = [json.loads(line) for line in (directory / 'stdout.jsonl').read_text().splitlines() if line.strip()]
        task = 'framework-react-surface.oil' if record['framework'] == 'react' else 'framework-action-smoke.oil'
        original = runner.build.CORPUS / task
        if record.get('task_sha256') != digest(original) or (directory / 'task.oil').read_text() != original.read_text():
            errors.append('framework task changed')
        errors.extend(runner.validate(record['framework'], original.read_text(), frames))
        errors.extend(runner.validate_process_record(record))
    if errors:
        raise ValueError('framework qualification failed: ' + '; '.join(errors))
    return value


def trailing_object(path):
    text = Path(path).read_text()
    decoder = json.JSONDecoder()
    for index, char in enumerate(text):
        if char != '{':
            continue
        try:
            value, length = decoder.raw_decode(text[index:])
        except ValueError:
            continue
        if not text[index + length:].strip() and isinstance(value, dict):
            return value
    raise ValueError('producer has no terminal structured result')


def architecture(receipt):
    value = trailing_object(Path(receipt).parent / 'stdout.log')
    if value.get('cutover_ready') is not True or value.get('pending') or value.get('legacy_production_markers') or value.get('pending_release_attestations'):
        raise ValueError('architecture cutover or release attestations remain open')
    manifest = read(ROOT / 'benchmarks/conformance/runtime-architecture-v1.json')
    if value.get('cases') != len(manifest['cases']):
        raise ValueError('architecture denominator changed')
    return value


def booking_resolution(record, candidate):
    """Keep the historical unexplained timeout as a completion blocker.

    Causality requires an independent source-bound diagnosis, plus the same named
    executable regression failing before the fix and passing on this candidate.
    A clean repetition of unrelated workflow cases is not a mechanism closure.
    """
    value = read(read_record(record))
    if type(value.get('schema_version')) is not int or value.get('schema_version') != 1 or value.get('candidate_manifest_sha256') != candidate['sha256']:
        raise ValueError('booking diagnosis candidate mismatch')
    for field in ('confirmed_cause', 'diagnosis', 'author', 'reviewer', 'regression_name'):
        if not isinstance(value.get(field), str) or not value[field].strip():
            raise ValueError('booking timeout needs confirmed diagnosis and independent review')
    if value['author'] == value['reviewer']:
        raise ValueError('booking diagnosis review is not independent')
    original = read_record(value.get('original_failure'))
    if digest(original) != BOOKING_ORIGINAL_FAILURE_SHA256:
        raise ValueError('booking diagnosis does not bind the preserved original failed attempt')
    sources = value.get('changed_sources', [])
    if not sources:
        raise ValueError('booking fix has no source-bound implementation')
    for source in sources:
        path = read_record(source)
        if ROOT not in path.parents:
            raise ValueError('booking fix source outside candidate tree')
    trace = value.get('original_trace')
    if not isinstance(trace, dict) or set(trace) != set(BOOKING_ORIGINAL_TRACE):
        raise ValueError('booking original diagnostic trace incomplete')
    for name, expected in BOOKING_ORIGINAL_TRACE.items():
        if digest(read_record(trace[name])) != expected:
            raise ValueError('booking original diagnostic trace substituted: ' + name)
    before_path = read_record(value.get('regression_before'))
    before_receipt_path = read_record(value.get('regression_before_receipt'))
    # The before run necessarily belongs to older source. Validate its preserved
    # snapshot and executable inventory, not the current checkout's input lock.
    from completion_provenance import REQUIRED_ARTIFACTS, is_digest, validate_source_snapshot
    before_candidate_path = read_record(value.get('regression_before_candidate'))
    before_record = read(before_candidate_path)
    before_records = before_record.get('artifacts')
    if (type(before_record.get('schema_version')) is not int or before_record['schema_version'] != 1
            or not is_digest(before_record.get('source_tree_sha256'))
            or not isinstance(before_records, dict) or not REQUIRED_ARTIFACTS <= before_records.keys()):
        raise ValueError('booking before candidate inventory incomplete')
    before_artifacts = {name: read_record(record) for name, record in before_records.items()}
    before_index = validate_source_snapshot(before_artifacts['source_snapshot'], before_record['source_tree_sha256'])
    after_index = validate_source_snapshot(candidate['artifacts']['source_snapshot'], candidate['record']['source_tree_sha256'])
    before_sources = {item['path']: item['sha256'] for item in before_index['files']}
    after_sources = {item['path']: item['sha256'] for item in after_index['files']}
    for source in sources:
        name = read_record(source).relative_to(ROOT).as_posix()
        if after_sources.get(name) != source['sha256'] or before_sources.get(name) == source['sha256']:
            raise ValueError('booking changed source is not a before/after candidate difference')
    before_candidate = dict(path=before_candidate_path, sha256=digest(before_candidate_path),
                            record=before_record, artifacts=before_artifacts)
    if (before_candidate['sha256'] == candidate['sha256']
            or before_record['source_tree_sha256'] == candidate['record']['source_tree_sha256']):
        raise ValueError('booking before evidence must precede the source fix')
    before_receipt = verify_receipt(before_receipt_path, 'booking-regression', before_candidate,
                                    expected_regression_failure=True)
    if (before_path != before_receipt_path.parent / 'stdout.log'
            or value['regression_before'] not in before_receipt['artifacts']
            or before_receipt['execution']['argv'][2] != value['regression_name']):
        raise ValueError('booking before regression is not original failing-run output')
    before = before_path.read_text()
    regression_receipt = read_record(value.get('regression_receipt'))
    receipt = verify_receipt(regression_receipt, 'booking-regression', candidate)
    after_path = read_record(value.get('regression_after'))
    if after_path != regression_receipt.parent / 'stdout.log' or value['regression_after'] not in receipt['artifacts']:
        raise ValueError('booking regression not from original candidate run')
    after = after_path.read_text()
    name = value['regression_name']
    if receipt['execution']['argv'][2] != name:
        raise ValueError('booking diagnosis and executed regression differ')
    if ('test ' + name + ' ... FAILED') not in before or 'test result: FAILED. 0 passed; 1 failed;' not in before or ('test ' + name + ' ... ok') not in after:
        raise ValueError('booking mechanism regression lacks before/after evidence')
    if 'test result: FAILED' in after or 'test result: ok. 1 passed; 0 failed;' not in after or value.get('after_candidate_manifest_sha256') != candidate['sha256']:
        raise ValueError('booking regression did not pass on final candidate')
    return value


def validate_command(argv, directory, admission):
    admission.check()
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=False)
    result = run_owned(argv, directory, environment=admission.environment())
    persist(directory / 'execution.json', result)
    if result['returncode'] != 0 or result['timed_out'] or result['error'] or not result['cleanup_complete']:
        raise ValueError('actual validator failed: ' + str(argv[1]))
    return result


def native_media(receipt_path, candidate):
    from native_media_qualification import validate_result
    return validate_result(Path(receipt_path).parent / 'raw', candidate)


def native_prerequisites(receipt_path, candidate):
    from wpt_native_controls import validate_results
    path = Path(receipt_path).parent / 'raw/result.json'
    record = read(path)
    if record.get('candidate_manifest_sha256') != candidate['sha256']:
        raise ValueError('native prerequisite candidate mismatch')
    for artifact in record.get('artifacts', []):
        read_record(artifact)
    preflight = read(path.parent / 'preflight.json')
    if read_record(preflight['candidate']) != candidate['path']:
        raise ValueError('prerequisite preflight candidate differs')
    for item in preflight['control_inputs']: read_record(item)
    errors = validate_results(record)
    if record.get('prerequisites_passed') is not True or record.get('errors') or errors:
        raise ValueError('native/reference prerequisites failed: ' + '; '.join(errors))
    for name in ('native', 'reference'):
        browser = record[name]
        for key, value in browser['controls'].items():
            if isinstance(value, dict) and 'response' in value and value not in browser.get('commands', []):
                raise ValueError('native prerequisite result differs from raw command transcript')
    return record



def fixture_topology(receipt_path, candidate):
    from wpt_fixture_contract import validate
    from wpt_topology_probe import validate_observations
    result_path = Path(receipt_path).resolve().parent / 'raw/result.json'
    record = read(result_path)
    if (record.get('candidate_manifest_sha256') != candidate['sha256']
            or record.get('topology_passed') is not True or record.get('errors')):
        raise ValueError('fixture topology failed or foreign candidate')
    preflight = read(result_path.parent / 'preflight.json')
    for key, value in preflight.items():
        for item in value if key == 'sources' else [value]: read_record(item)
    if read_record(preflight['candidate']) != candidate['path']:
        raise ValueError('topology preflight candidate differs')
    fixture = candidate['artifacts']['fixture_contract']
    if read_record(preflight['fixture']) != fixture: raise ValueError('topology fixture differs')
    plan = read(candidate['artifacts']['qualification_plan'])
    negative = Path(plan['receipts']['native-prerequisites']).resolve()
    if read_record(record['negative_receipt']) != negative or preflight['negative_receipt'] != record['negative_receipt']:
        raise ValueError('topology negative control receipt differs')
    verify_receipt(negative, 'native-prerequisites', candidate)
    native_prerequisites(negative, candidate)
    controls = read(candidate["artifacts"]["native_controls_contract"])
    checkout = Path(controls["upstream_checkout"]).resolve()
    expected_sources = [checkout / "common/square.png", checkout / "websockets/handlers/echo_wsh.py"]
    if [read_record(item) for item in preflight["sources"]] != expected_sources:
        raise ValueError("topology helper sources substituted")
    validate_observations(record, preflight["sources"][0]["sha256"])
    validate(read(fixture), fixture.parent, record)
    return record


def booking_evidence_record(plan, frozen_inputs=None):
    """Freeze an output path before execution without a candidate hash cycle."""
    name = plan.get('booking_resolution')
    if not isinstance(name, str) or not name or not Path(name).is_absolute():
        raise ValueError('frozen booking diagnosis must be an absolute output path')
    path = Path(name)
    if path.is_symlink() or path.resolve() != path:
        raise ValueError('booking diagnosis path is not stable')
    if frozen_inputs is None:
        return reference(path)
    matches = [record for record in frozen_inputs if record.get('path') == str(path)]
    if len(matches) != 1:
        raise ValueError('booking diagnosis absent from original input inventory')
    read_record(matches[0])
    return matches[0]


def profile_prerequisites(candidate):
    from core_profile_contract import validate_plan, validate_prerequisite_equivalence
    plan = read(candidate['artifacts']['qualification_plan'])
    validate_plan(plan, candidate)
    equivalence = validate_prerequisite_equivalence(candidate)
    for kind, validator in (('native-prerequisites', native_prerequisites), ('fixture-topology', fixture_topology)):
        path = plan['receipts'][kind]
        verify_receipt(path, kind, candidate)
        validator(path, candidate)
    return equivalence


def profile_controls(profile, context, candidate):
    """Recompute the original canaries and placement; saved pass flags do not count."""
    from wpt_execution_evidence import verify
    from wpt_canary_qualification import qualify as qualify_canaries
    from wpt_shared_placement import qualify as qualify_placement
    from core_profile_contract import resolve_envelope, identity, check_identity
    envelope = resolve_envelope(profile['run_contract'], candidate)
    expected = identity(envelope, 'core')
    check_identity(context, expected)
    if Path(profile['prepared']) != Path(envelope['envelope']['prepared']):
        raise ValueError('profile controls use a different prepared workflow')
    results = {}
    for kind in ('canaries', 'placement'):
        root = Path(profile[kind]['directory'])
        execution = verify(root/'wptreport.json', root/'execution.json', candidate['sha256'], kind,
                           profile_id=context['profile_id'])
        check_identity(execution['profile_context'], identity(envelope, kind))
        if kind == 'canaries':
            result = qualify_canaries(root/'wptreport.json', root/'execution.json', profile[kind]['audit'], candidate['sha256'])
            audit = read(profile[kind]['audit'])
            check_identity(audit, identity(envelope, kind))
            if result.get('controls_rejected') is not True:
                raise ValueError('profile canary controls did not reject every expected failure')
        else:
            if execution['returncode'] != 0: raise ValueError('profile placement executor failed')
            placement_context = read(root/'placement-context.json')
            if placement_context.get('run_contract_sha256') != context['envelope_sha256']:
                raise ValueError('profile placement belongs to another envelope')
            result = qualify_placement(read(root/'placement.json'), read(root/'wptreport.json'),
                                       placement_context, candidate['record']['artifacts']['worker']['sha256'])
            if result.get('qualification_accepted') is not True:
                raise ValueError('profile native shared placement incomplete')
        results[kind] = result
    return results


def profile_original_controls(profile, envelope, candidate):
    """A retry retains the original profile controls and original-run identity."""
    from core_profile_contract import resolve_envelope, identity
    original = resolve_envelope(profile['run_contract'], candidate)
    for key in ('candidate_sha256', 'registry_sha256', 'profile_id', 'profile_sha256', 'platform'):
        if envelope[key] != original[key]:
            raise ValueError('Core attempt substitutes the original profile controls')
    if (original['run_id'] != original['original_run_id']
            or envelope['original_run_id'] != original['run_id']
            or (envelope['run_id'] == original['run_id'] and envelope['envelope_sha256'] != original['envelope_sha256'])):
        raise ValueError('Core attempt does not retain its planned original run')
    return profile_controls(profile, identity(original, 'core'), candidate)


def qualify_core_profiles(plan, candidate, output, checked_command):
    from core_profile_contract import (validate_plan, validate_prerequisite_equivalence,
        load_profile_context, resolve_envelope, identity, check_identity, validate_profile_ledger, validate_core_aggregate)
    from wpt_full_qualification import validate_inventory
    registry = validate_plan(plan, candidate)
    equivalence = validate_prerequisite_equivalence(candidate)
    core = plan['core']; folder = Path(core['inventory'])
    core_ids = validate_inventory([json.loads(line) for line in (folder/'coverage.jsonl').read_text().splitlines()],
                                 read(folder/'plan.json'))
    profiles, errors = {}, []
    for name in registry['value']['required_profile_ids']:
        profile = core['profiles'][name]
        try:
            context = load_profile_context(candidate['path'], name, profile['reports'])
            original = resolve_envelope(profile['run_contract'], candidate)
            check_identity(context, identity(original, 'core'))
            controls = profile_controls(profile, context, candidate)
            directory = output/'core-ledgers'/name
            argv = [sys.executable, str(ROOT/'scripts/reconcile-full-wpt-qualification.py'),
                    '--candidate', str(candidate['path']), '--candidate-sha256', candidate['sha256'],
                    '--profile-id', name, '--output', str(directory),
                    '--disposition-policy', str(candidate['artifacts']['core_disposition_policy']),
                    '--disposition-applications', profile['disposition_applications']]
            for key in ('inventory', 'manifest', 'policy', 'checkout'): argv += ['--'+key, core[key]]
            for key in ('reviews', 'assessments', 'attribution'): argv += ['--'+key, profile[key]]
            for report in profile['reports']: argv += ['--report', report]
            checked_command(argv, output/'core-validation'/name)
            summary = read(directory/'summary.json')
            ledger = directory/'ledger.jsonl'
            rows = [json.loads(line) for line in ledger.read_text().splitlines()]
            if summary.get('ledger_sha256') != digest(ledger):
                raise ValueError('profile ledger differs from original reconciler summary')
            observed = validate_profile_ledger(summary, rows, core_ids, context)
            profiles[name] = {'summary': reference(directory/'summary.json'), 'ledger': reference(ledger),
                              **observed, 'controls': controls,
                              'profile_sha256': context['profile_sha256'],
                              'original_report_sha256': context['original_report_sha256']}
        except Exception as error:
            errors.append(name + ': ' + str(error))
    primary = registry['value']['primary_profile_id']
    raw_primary = (primary in profiles and profiles[primary]['raw_pass_count'] == 15985
                   and profiles[primary]['reviewed_disposition_count'] == 0)
    aggregate = {'schema_version': 2, 'aggregation': 'independent_required_profiles',
        'candidate_sha256': candidate['sha256'], 'registry_sha256': registry['sha256'],
        'primary_profile_id': primary, 'required_profile_ids': registry['value']['required_profile_ids'],
        'profile_count': len(profiles), 'core_total_per_profile': 16737,
        'profiles': profiles, 'complete': not errors, 'errors': errors,
        'single_profile_conformance': raw_primary, 'prerequisite_equivalence': equivalence,
        'non_core_scope': plan['non_core_scope']}
    persist(output/'core-profiles-summary.json', aggregate)
    generated = [reference(output/'core-profiles-summary.json')]
    generated += [member[key] for member in profiles.values() for key in ('summary', 'ledger')]
    if errors: raise ValueError('required Core profiles incomplete: ' + '; '.join(errors))
    validate_core_aggregate(aggregate, candidate)
    for record in generated: read_record(record)
    aggregate['generated_artifacts'] = generated
    return aggregate


def input_inventory(plan, candidate=None):
    """Pin original evidence before validators read it, including all raw files."""
    roots = [Path(plan['workflows']), Path(plan['profiles'])]
    roots += [Path(path).parent for path in plan['receipts'].values()]
    paths, asserted = set(), {}
    def keep(record):
        path = read_record(record)
        if path in asserted and asserted[path]['sha256'] != record['sha256']:
            raise ValueError('original input has conflicting asserted hashes')
        asserted[path] = dict(record); paths.add(path)
        return path
    for root in roots:
        if not root.is_dir():
            raise ValueError('original evidence directory missing: ' + str(root))
        for path in root.rglob('*'):
            if path.is_symlink():
                raise ValueError('evidence tree contains a mutable symlink')
            if path.is_file():
                paths.add(path.resolve())
    core = plan['core']
    if plan.get('schema_version') == 2:
        if candidate is None: raise ValueError('profile input inventory needs the frozen candidate')
        from core_profile_contract import validate_plan, record_inputs
        registry = validate_plan(plan, candidate)
        for record in registry['inputs']: keep(record)
        for record in record_inputs(candidate['record']['artifacts']['core_disposition_policy'], checkout=core['checkout']): keep(record)
        paths.update(Path(core[key]).resolve() for key in ('manifest', 'policy'))
        paths.update((Path(core['inventory'])/name).resolve() for name in ('plan.json', 'coverage.jsonl'))
        for profile in core['profiles'].values():
            files = [profile[key] for key in ('run_contract', 'reviews', 'assessments', 'attribution', 'disposition_applications')]
            files.append(profile['canaries']['audit'])
            for name in files:
                path = Path(name).resolve(); keep(reference(path))
                for record in record_inputs(read(path), checkout=core['checkout']): keep(record)
            evidence_roots = [Path(profile['prepared']), Path(profile['canaries']['directory']),
                              Path(profile['placement']['directory']), *[Path(p).parent for p in profile['reports']]]
            for root in evidence_roots:
                if not root.is_dir(): raise ValueError('profile original evidence directory missing: ' + str(root))
                for path in root.rglob('*'):
                    if path.is_symlink(): raise ValueError('profile evidence tree contains a mutable symlink')
                    if path.is_file():
                        keep(reference(path))
                        # Every recorded original input is kept, including retry
                        # envelopes and generated lock dependencies outside raw.
                        if path.name in ('preflight.json', 'workflow-lock.json'):
                            record = read(path)
                            for item in record_inputs(record, checkout=core['checkout']): keep(item)
                            if path.name == 'workflow-lock.json':
                                for name, expected in record.get('inputs', {}).items(): keep({'path': name, 'sha256': expected})
    else:
        for key in ('manifest', 'policy', 'reviews', 'assessments', 'attribution'):
            if core.get(key): paths.add(Path(core[key]).resolve())
        for name in ('plan.json', 'coverage.jsonl'):
            paths.add((Path(core['inventory']) / name).resolve())
        paths.update(Path(path).resolve() for path in core['reports'])
        for report in core['reports']:
            for name in ('execution.json', 'preflight.json', 'protocol-custody.jsonl'):
                paths.add((Path(report).parent / name).resolve())
    diagnosis_path = read_record(booking_evidence_record(plan))
    paths.add(diagnosis_path)
    diagnosis = read(diagnosis_path)
    for key in ('original_failure', 'regression_before', 'regression_before_receipt', 'regression_before_candidate', 'regression_after', 'regression_receipt'):
        paths.add(read_record(diagnosis[key]))
    before_candidate = read(read_record(diagnosis['regression_before_candidate']))
    for record in before_candidate.get('artifacts', {}).values():
        paths.add(read_record(record))
    before_receipt = read(read_record(diagnosis['regression_before_receipt']))
    for record in before_receipt.get('artifacts', []):
        paths.add(read_record(record))
    preflight_path = read_record(before_receipt['preflight'])
    paths.add(preflight_path)
    preflight = read(preflight_path)
    for key in ('candidate', 'generation_gate', 'producer_source'):
        paths.add(read_record(preflight[key]))
    for record in diagnosis.get('changed_sources', []):
        paths.add(read_record(record))
    for record in diagnosis.get('original_trace', {}).values():
        paths.add(read_record(record))
    result = [asserted.get(path) or reference(path) for path in sorted(paths)]
    for record in result: read_record(record)
    return result


def qualify(candidate_path, gate, plan_path, output, *, allow_historical=False):
    admission = GenerationLease(gate, validator=generation_gate)
    frozen_gate = reference(gate)
    gate_invalid = False
    candidate_invalid = False

    def check_gate_identity():
        nonlocal gate_invalid
        try:
            gate_invalid = gate_invalid or reference(gate) != frozen_gate
            if not gate_invalid: admission.check()
        except Exception:
            gate_invalid = True
        if gate_invalid:
            raise ValueError('generation gate changed during qualification')

    def check_candidate_identity():
        nonlocal candidate_invalid
        try:
            candidate_invalid = candidate_invalid or (
                load_candidate(candidate_path)['sha256'] != candidate['sha256']
                or source_hash() != candidate['record']['source_tree_sha256'])
        except Exception:
            candidate_invalid = True
        if candidate_invalid:
            raise ValueError('candidate changed during qualification')

    def check_integrity():
        check_gate_identity()
        check_candidate_identity()

    def checked_action(action):
        check_integrity()
        try:
            return action()
        finally:
            check_integrity()

    def checked_command(argv, directory):
        return checked_action(lambda: validate_command(argv, directory, admission))

    candidate = load_candidate(candidate_path)
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('qualification source differs from frozen candidate')
    record = candidate['record']['artifacts'].get('qualification_plan')
    if record is None or read_record(record) != Path(plan_path).resolve():
        raise ValueError('final qualification plan must be frozen in candidate')
    plan = read(plan_path)
    if plan.get('schema_version') not in (1, 2) or set(plan.get('receipts', {})) != {'frameworks', 'security', 'architecture', 'reliability', 'native-prerequisites', 'fixture-topology', 'native-media'}:
        raise ValueError('complete final qualification plan required')
    if plan['schema_version'] == 1:
        if not allow_historical or 'core_profile_registry' in candidate['artifacts']:
            raise ValueError('current completion requires schema2 required-profile qualification; historical opt-in cannot downgrade a registry')
    else:
        from core_profile_contract import validate_plan
        validate_plan(plan, candidate)
    output = Path(output).resolve()
    check_integrity()
    output.mkdir(parents=True, exist_ok=False)
    persist(output / 'preflight.json', dict(candidate_manifest_sha256=candidate['sha256'], plan=reference(plan_path), generation_gate=frozen_gate))
    results = {}
    errors = []
    frozen_inputs = []
    try:
        frozen_inputs = input_inventory(plan, candidate)
    except Exception as error:
        errors.append('input inventory: ' + str(error))
    receipts = {}
    for kind, path in plan['receipts'].items():
        try:
            receipts[kind] = checked_action(lambda: verify_receipt(path, kind, candidate))
        except Exception as error:
            errors.append(kind + ': ' + str(error))
    checks = {
        'fixture_topology': lambda: fixture_topology(plan['receipts']['fixture-topology'], candidate) if 'fixture-topology' in receipts else (_ for _ in ()).throw(ValueError('receipt missing')),
        'native_media': lambda: native_media(plan['receipts']['native-media'], candidate) if 'native-media' in receipts else (_ for _ in ()).throw(ValueError('receipt missing')),
        'native_prerequisites': lambda: native_prerequisites(plan['receipts']['native-prerequisites'], candidate) if 'native-prerequisites' in receipts else (_ for _ in ()).throw(ValueError('receipt missing')),
        'booking_resolution': lambda: booking_resolution(booking_evidence_record(plan, frozen_inputs), candidate),
        'workflows': lambda: workflow(plan['workflows'], candidate),
        'frameworks': lambda: framework(Path(plan['receipts']['frameworks']).parent / 'raw', candidate) if 'frameworks' in receipts else (_ for _ in ()).throw(ValueError('receipt missing')),
        'reliability': lambda: workflow(Path(plan['receipts']['reliability']).parent / 'raw', candidate) if 'reliability' in receipts else (_ for _ in ()).throw(ValueError('receipt missing')),
    }
    for kind, check in checks.items():
        try:
            results[kind] = checked_action(check)
        except Exception as error:
            errors.append(kind + ': ' + str(error))
    try:
        if plan['schema_version'] == 2:
            results['core'] = checked_action(lambda: qualify_core_profiles(plan, candidate, output, checked_command))
        else:
            core = plan['core']
            argv = [sys.executable, str(ROOT / 'scripts/reconcile-full-wpt-qualification.py'), '--candidate-sha256', candidate['sha256'], '--allow-historical', '--output', str(output / 'core-ledger')]
            for key in ('inventory', 'manifest', 'policy', 'checkout', 'reviews', 'assessments', 'attribution'):
                if core.get(key):
                    argv += ['--' + key, core[key]]
            for report in core['reports']:
                argv += ['--report', report]
            checked_command(argv, output / 'core-validation')
            result = read(output / 'core-ledger/summary.json')
            if result.get('complete') is not True or result.get('core_total') != 16737 or result.get('counts') != {'qualified_pass': 16737}:
                raise ValueError('full Core including manual assessments incomplete')
            results['core'] = result
    except Exception as error:
        errors.append('core: ' + str(error))
    try:
        checked_command([sys.executable, str(ROOT / 'scripts/qualify-workflow-q.py'), '--workflows', plan['workflows'], '--profiles', plan['profiles'], '--candidate', str(candidate['path']), '--output', str(output / 'q.json')], output / 'q-validation')
        result = read(output / 'q.json')
        if result.get('candidate_manifest_sha256') != candidate['sha256'] or result.get('q_component_checks_passed') is not True:
            raise ValueError('Q result failed or candidate differs')
        results['q'] = result
    except Exception as error:
        errors.append('q: ' + str(error))
    try:
        if 'security' not in receipts:
            raise ValueError('original security receipt missing')
        raw = Path(plan['receipts']['security']).parent / 'raw'
        checked_command([sys.executable, str(ROOT / 'scripts/validate_security_result.py'), str(raw / 'security-result-v3.json'), '--repo-root', str(ROOT), '--artifact-root', str(raw), '--require-raw'], output / 'security-validation')
        result = read(raw / 'security-result-v3.json')
        if (result['fingerprint']['worker']['sha256'] != candidate['record']['artifacts']['worker']['sha256']
                or result['fingerprint']['parent']['sha256'] != candidate['record']['artifacts']['runtime']['sha256']
                or result['fingerprint']['source_tree_sha256'] != candidate['record']['source_tree_sha256']):
            raise ValueError('security native fingerprint differs')
        results['security'] = result
    except Exception as error:
        errors.append('security: ' + str(error))
    try:
        if 'architecture' not in receipts:
            raise ValueError('original architecture receipt missing')
        if plan['schema_version'] == 2:
            if 'security' not in results:
                raise ValueError('architecture requires successfully validated same-candidate security')
            from architecture_prebuilt import qualify_architecture
            results['architecture'] = checked_action(lambda: qualify_architecture(
                plan['receipts']['architecture'], candidate, results['security'], plan['receipts']['security']))
        else:
            results['architecture'] = checked_action(lambda: architecture(plan['receipts']['architecture']))
    except Exception as error:
        errors.append('architecture: ' + str(error))
    try:
        generation_gate(gate)
        check_integrity()
    except Exception as error:
        errors.append('final integrity: ' + str(error))
    try:
        if input_inventory(plan, candidate) != frozen_inputs:
            raise ValueError('original qualification evidence changed during validation')
    except Exception as error:
        errors.append('input integrity: ' + str(error))
    if plan['schema_version'] == 2 and 'core' in results:
        try:
            generated = results['core']['generated_artifacts']
            for record in generated: read_record(record)
            expected = {read_record(member[key]) for member in results['core']['profiles'].values()
                        for key in ('summary', 'ledger')}
            actual = {path.resolve() for path in (output/'core-ledgers').rglob('*') if path.is_file()}
            if actual != expected or any(path.is_symlink() for path in (output/'core-ledgers').rglob('*')):
                raise ValueError('generated Core ledger inventory changed after validation')
        except Exception as error:
            errors.append('generated Core integrity: ' + str(error))
    required = {'core', 'workflows', 'frameworks', 'q', 'security', 'architecture', 'reliability', 'booking_resolution', 'native_prerequisites', 'fixture_topology', 'native_media'}
    if set(results) != required:
        errors.append('required final component missing')
    result = dict(schema_version=2 if plan['schema_version'] == 2 else 1,
                  qualification_scope='current_required_profiles' if plan['schema_version'] == 2 else 'historical_single_profile',
                  current_plan_complete=plan['schema_version'] == 2 and not errors,
                  candidate_manifest_sha256=candidate['sha256'],
                  plan=reference(plan_path), generation_gate=frozen_gate, components=results, inputs=frozen_inputs, errors=errors, qualified=not errors,
                  evidence=[reference(path) for path in sorted(output.rglob('*')) if path.is_file()])
    persist(output / 'qualification.json', result)
    return result

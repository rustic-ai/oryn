"""Replay all frozen family cells from pinned native and sealed SQLite evidence.

No runtime/fixture launch occurs here. SQLite is opened from a private copy of
verified bytes so a mutable evidence pathname cannot change the parsed database.
The candidate's source/input lock is checked by the surrounding final supervisor.
"""
import json
import hashlib
from functools import wraps
import re
from contextlib import contextmanager
import sqlite3
from pathlib import Path
import tempfile
from types import SimpleNamespace
from urllib.parse import urlsplit


def object_value(value, label):
    if not isinstance(value, dict):
        raise ValueError(label + ' must be an object')
    return value


def object_rows(value, label):
    if not isinstance(value, list) or any(not isinstance(row, dict) for row in value):
        raise ValueError(label + ' must be a list of objects')
    return value


def decode(encoded):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate raw evidence JSON field')
            result[key] = value
        return result
    def constant(_value):
        raise ValueError('nonfinite raw evidence JSON number')
    return json.loads(encoded, object_pairs_hook=pairs, parse_constant=constant)


def malformed_input(function):
    """Domain validators may reject malformed row contents by field access.

    Keep those failures typed at every public replay boundary, without including
    potentially private payloads or converting a failed read into success.
    """
    @wraps(function)
    def checked(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except (TypeError, KeyError, AttributeError, IndexError, OverflowError, RecursionError) as error:
            raise ValueError('malformed raw evidence at ' + function.__name__) from error
    return checked


def equal(left, right):
    # JSON bool/int distinctions are evidence, not Python numeric equivalence.
    return json.dumps(left, sort_keys=True) == json.dumps(right, sort_keys=True)


@contextmanager
def database_bytes(encoded):
    with tempfile.TemporaryDirectory(prefix='oryn-oracle-replay-') as directory:
        path = Path(directory) / 'sealed.sqlite3'
        path.write_bytes(encoded)
        try:
            yield path
        except sqlite3.Error as error:
            raise ValueError("raw SQLite evidence is invalid") from error


def snapshot_bytes(receiver, encoded, identity):
    with database_bytes(encoded) as path:
        return receiver.snapshot(path, identity)


@malformed_input
def prepared_recipe(recipe, base, fault):
    import workflow_native_adapter as adapter
    if (not isinstance(recipe, (list, tuple)) or len(recipe) != 3
            or recipe[0] not in ('research', 'forms', 'shopping', 'saas', 'booking')
            or type(recipe[1]) is not int or recipe[1] <= 0 or not isinstance(recipe[2], str)
            or not isinstance(base, str)):
        raise ValueError('malformed raw recipe or fixture URL')
    family, version, name = recipe
    url = urlsplit(base)
    identity = url.path.rsplit('/', 1)[-1]
    if (url.scheme != 'http' or url.hostname != '127.0.0.1' or not url.port
            or url.username or url.password or url.query or url.fragment
            or len(identity) != 32 or any(c not in '0123456789abcdef' for c in identity)):
        raise ValueError('raw family fixture identity is invalid')
    contract = adapter.read_case_contract(family, version, name)
    matches = [row for row in contract['cases'] if row['id'] == name]
    if len(matches) != 1:
        raise ValueError('raw concrete recipe missing or duplicated')
    case = matches[0]
    prefix = 'advanced' if family == 'forms' and case.get('extended') else {
        'research': 'research', 'forms': 'form', 'shopping': 'shop', 'saas': 'saas', 'booking': 'booking'}[family]
    if url.path != '/' + prefix + '/' + identity:
        raise ValueError('raw family fixture route differs')
    runner = adapter.research if family == 'research' else adapter.load('raw_' + family, adapter.ROOT / f'scripts/run-oil-{family}-outcomes.py')
    suffix = '' if version == 1 else f'_v{version}'
    receiver = adapter.research.receiver if family == 'research' else adapter.load('raw_receiver_' + family + str(version), adapter.FIXTURES / f'{family}_fixture{suffix}.py')
    lines = adapter.family_lines(family, runner, contract, case, base)
    recovery = []; native_fault = None
    if fault:
        decoration = adapter.decorate_fault(family, case, lines, fault, base)
        case, lines = decoration['case'], decoration['lines']
        recovery, native_fault = decoration['recovery_steps'], decoration['fault']
    return SimpleNamespace(family=family, version=version, name=name, case=case,
        contract=contract, runner=runner, receiver=receiver, base=base, identity=identity,
        origin=url.scheme + '://' + url.netloc, lines=lines, recovery_steps=recovery, fault=native_fault)


@malformed_input
def check_steps(planned, session):
    object_value(session, 'native session')
    object_rows(planned, 'native planned steps')
    steps = object_rows(session.get('steps'), 'native result steps')
    if not isinstance(steps, list) or len(steps) != len(planned):
        raise ValueError('raw native task truncated')
    for index, (source, result) in enumerate(zip(planned, steps)):
        expected = source.get('expected_error')
        error = result.get('error')
        if ('error' not in result or type(result.get('index')) is not int or result['index'] != index
                or result.get('expected_error_matched') is not True
                or not isinstance(result.get('outputs'), list)
                or any(not isinstance(frame, dict) for frame in result['outputs'])
                or type(result.get('timed_out')) is not bool
                or (expected is None and (error is not None or result['timed_out']))
                or (expected is not None and (not isinstance(error, str) or expected not in error))):
            raise ValueError('raw native step result differs from expected operation')


@malformed_input
def check_run(plan, native, attempt_id, candidate):
    object_value(plan, 'native plan'); object_value(native, 'native evidence'); object_value(candidate, 'candidate')
    runtime = object_value(native.get('runtime'), 'native runtime')
    if (type(plan.get('schema_version')) is not int or type(native.get('schema_version')) is not int
            or plan.get('schema_version') != 1 or plan.get('attempt_id') != attempt_id
            or native.get('schema_version') != 1 or native.get('attempt_id') != attempt_id
            or native.get('infrastructure_errors') != []
            or runtime.get('sandboxed') is not True
            or runtime.get('worker_binary_sha256') != candidate['worker_sha256']):
        raise ValueError('raw native candidate/attempt identity differs')
    inputs = object_rows(plan.get('sessions'), 'planned sessions'); results = object_rows(native.get('sessions'), 'native sessions')
    if not isinstance(inputs, list) or not isinstance(results, list):
        raise ValueError('raw native sessions missing')
    ids = [row.get('id') for row in inputs]; actual = [row.get('id') for row in results]
    if (any(not isinstance(identity, str) for identity in ids + actual)
            or len(set(ids)) != len(ids) or len(set(actual)) != len(actual) or set(ids) != set(actual)):
        raise ValueError('raw native session inventory differs')
    if any(type(row.get('context_id')) is not int or row['context_id'] <= 0 for row in results):
        raise ValueError('raw native context identity malformed')
    if len({row['context_id'] for row in results}) != len(results):
        raise ValueError('raw native context identity reused')
    for row in results:
        if row.get('errors') != [] or row.get('cleanup_errors') != [] or row.get('workers_retired') is not True:
            raise ValueError('raw native session failed or cleanup incomplete')
    return {row['id']: row for row in inputs}, {row['id']: row for row in results}


@malformed_input
def validate(root, reference, attempt, candidate, artifact, observed, actions):
    import workflow_native_adapter as adapter
    object_value(attempt, 'attempt'); object_value(candidate, 'candidate')
    manifest = object_value(decode(artifact(root, reference)), 'raw manifest')
    if type(manifest.get('schema_version')) is not int or manifest.get('schema_version') != 1 or manifest.get('attempt_id') != attempt['attempt_id']:
        raise ValueError('raw family manifest identity differs')
    primary_recipes = adapter.MixedCellAdapter(attempt['cell'], Path('/unused'), Path('/unused')).recipes()
    waves = object_rows(manifest.get('waves'), 'raw waves')
    if not isinstance(waves, list) or len(waves) != len(primary_recipes):
        raise ValueError('raw family wave denominator differs')
    outcomes = []; all_actions = []; used = set(); fault_records = []; topologies = []; resources = []; probes = []; activities = []
    for ordinal, (wave, primary) in enumerate(zip(waves, primary_recipes)):
        if not equal(wave.get('primary'), list(primary)):
            raise ValueError('raw family primary recipe differs')
        recipes = adapter.mixed_recipes(primary, attempt['context_count']) if attempt['mode'] == 'mixed_family' else [primary] * attempt['context_count']
        claims = object_rows(wave.get('cases'), 'raw case claims')
        if not isinstance(claims, list) or len(claims) != len(recipes):
            raise ValueError('raw family context denominator differs')
        plan = decode(artifact(root, wave.get('native_plan')))
        native = decode(artifact(root, wave.get('native_evidence')))
        inputs, sessions = check_run(plan, native, attempt['attempt_id'], candidate)
        check_resources(root, wave, native, artifact)
        topologies.append(adapter.context_topology(native, attempt['context_count']))
        activities.append(adapter.native_activity(native, attempt['context_count']))
        resources.append(wave['native_evidence'])
        cohort = [row['id'] for row in plan['sessions'] if row.get('adversarial_for') is None]
        if cohort != [f'context-{index}' for index in range(len(recipes))]:
            raise ValueError('raw family cohort differs')
        wave_outcomes = []
        for index, (claim, recipe) in enumerate(zip(claims, recipes)):
            session_id = f'context-{index}'
            if not equal(claim.get('fixture_cleanup'), dict(returncode=0, forced=False)):
                raise ValueError('raw fixture cleanup incomplete')
            if not equal(claim.get('recipe'), list(recipe)) or claim.get('session') != session_id:
                raise ValueError('raw concrete family recipe differs')
            custody = (wave['native_evidence']['path'], session_id)
            if custody in used:
                raise ValueError('raw native session reused across waves')
            used.add(custody)
            fault = attempt['fault'] if recipe[2] == adapter.PREFERRED.get(attempt['cell']) else None
            case = prepared_recipe(recipe, claim['base'], fault)
            expected_steps = [dict(oil=line, **case.case.get('step_options', {}).get(str(i), {})) for i, line in enumerate(case.lines)]
            planned, session = inputs[session_id], sessions[session_id]
            if (not equal(planned.get('steps'), expected_steps)
                    or not equal(planned.get('uploads'), adapter.authorized_uploads(case.family, case.case))
                    or not equal(planned.get('fault'), case.fault)
                    or not equal(planned.get('recovery_steps', []), case.recovery_steps)
                    or planned.get('commit_recovery') is not None or planned.get('adversarial_for') is not None):
                raise ValueError('raw native input differs from frozen family recipe')
            check_steps(expected_steps, session)
            recovery = object_rows(session.get('recovery'), 'native recovery')
            if (len(recovery) != len(case.recovery_steps) or any(row.get('index') != i
                    or 'error' not in row or row.get('error') is not None or not isinstance(row.get('outputs'), list)
                    for i, row in enumerate(recovery))):
                raise ValueError('raw native recovery task differs')
            with database_bytes(artifact(root, claim.get('database'))) as path:
                data = case.receiver.snapshot(path, case.identity)
                probe = replay_probe(case, claim.get('probe'), index, inputs, sessions, path)
                if probe is not None:
                    probes.append(probe)
                if case.fault:
                    case.database = path
                    fault_records.append(adapter.read_fault_evidence(case, session))
            if not equal(data, decode(artifact(root, claim.get('sealed_oracle')))):
                raise ValueError('raw SQLite and sealed projection differ')
            outcome, case_actions, errors = adapter.replay_case_outcome(case, session, data)
            if errors:
                raise ValueError('raw independent family outcome rejected: ' + '; '.join(errors))
            for action in case_actions:
                action['recipe'] = f'{ordinal}:{index}:' + action['recipe']
            all_actions.extend(case_actions); wave_outcomes.append(outcome)
        expected_probes = {claim['probe']['id'] for claim in claims if claim.get('probe')}
        actual_probes = {row['id'] for row in plan['sessions'] if row.get('adversarial_for') is not None}
        if expected_probes != actual_probes:
            raise ValueError('raw adversarial session inventory differs')
        outcomes.append(dict(case=primary[2], contexts=wave_outcomes))
    if not equal(activities[0], decode(artifact(root, attempt.get('context_activity')))):
        raise ValueError('raw native activity differs from stored projection')
    if not equal(probes, decode(artifact(root, attempt.get('adversarial_evidence')))):
        raise ValueError('raw adversarial replay differs from stored projection')
    supplements = validate_supplements(root, manifest.get('supplements'), attempt, candidate, artifact)
    fault_records.extend(supplements)
    stored_faults = decode(artifact(root, attempt.get('fault_evidence')))
    expected_faults = dict(fault=attempt['fault'], injected=bool(fault_records) and all(record.get('injected') is True for record in fault_records), records=fault_records)
    if not equal(stored_faults, expected_faults):
        raise ValueError('raw fault/commit evidence differs from stored projection')
    if not equal(topologies, decode(artifact(root, attempt.get('context_topology')))):
        raise ValueError('raw topology differs from stored projection')
    resource_manifest = decode(artifact(root, attempt.get('resource_evidence')))
    if not isinstance(resource_manifest, list) or len(resource_manifest) != len(resources) or any(
            row.get('wave') != index or not equal(row.get('native_evidence'), resources[index])
            for index, row in enumerate(resource_manifest)):
        raise ValueError('raw resource wave inventory differs')
    if not equal(outcomes, observed) or not equal(all_actions, actions):
        raise ValueError('raw family replay differs from stored outcome/action projection')
    return outcomes


@malformed_input
def validate_supplements(root, waves, attempt, candidate, artifact):
    import workflow_native_adapter as adapter
    required = attempt['fault'] in ('worker_failure', 'cancellation') and not attempt['cell'].startswith('research.')
    phases = ('before_commit', 'after_commit') if required else ()
    waves = object_rows(waves, 'commit waves')
    if not isinstance(waves, list) or len(waves) != len(phases):
        raise ValueError('raw commit supplement denominator differs')
    if not phases:
        return []
    recipes = adapter.MixedCellAdapter(attempt['cell'], Path('/unused'), Path('/unused')).recipes()
    recipe = next(recipe for recipe in recipes if recipe[2] == adapter.PREFERRED[attempt['cell']])
    records = []; used = set()
    for wave, phase in zip(waves, phases):
        object_rows(wave.get('cases'), 'commit case claims')
        if wave.get('phase') != phase or len(wave.get('cases', [])) != attempt['context_count']:
            raise ValueError('raw commit phase/context denominator differs')
        plan = decode(artifact(root, wave.get('native_plan')))
        native = decode(artifact(root, wave.get('native_evidence')))
        inputs, sessions = check_run(plan, native, attempt['attempt_id'] + ':' + phase, candidate)
        check_resources(root, wave, native, artifact)
        if set(inputs) != {f'context-{i}' for i in range(attempt['context_count'])}:
            raise ValueError('raw commit session inventory differs')
        outcomes = []
        for index, claim in enumerate(wave['cases']):
            session_id = f'context-{index}'
            if not equal(claim.get('fixture_cleanup'), dict(returncode=0, forced=False)):
                raise ValueError('raw fixture cleanup incomplete')
            if claim.get('session') != session_id or not equal(claim.get('recipe'), list(recipe)):
                raise ValueError('raw commit recipe differs')
            if claim['database']['path'] in used:
                raise ValueError('raw commit database reused')
            used.add(claim['database']['path'])
            case = prepared_recipe(recipe, claim['base'], None)
            planned, session = inputs[session_id], sessions[session_id]
            boundary = adapter.commit_recipe(case)
            fault = object_value(planned.get('fault'), 'commit fault'); gate = object_value(fault.get('commit_gate'), 'commit gate')
            token = gate.get('token', '')
            if (not isinstance(token, str) or len(token) != 32 or any(c not in '0123456789abcdef' for c in token)
                    or gate.get('phase') != phase or set(gate) != {'token', 'phase', 'marker', 'release'}
                    or not isinstance(gate.get('marker'), str) or not isinstance(gate.get('release'), str)
                    or Path(gate['marker']).name != 'gate.json' or Path(gate['release']).name != 'release.json'
                    or Path(gate['marker']).parent != Path(gate['release']).parent):
                raise ValueError('raw commit gate identity differs')
            expected_steps = [dict(oil=line, **case.case.get('step_options', {}).get(str(i), {})) for i, line in enumerate(case.lines)]
            if (not equal(planned.get('steps'), expected_steps)
                    or not equal(planned.get('uploads'), adapter.authorized_uploads(case.family, case.case))
                    or not equal(fault, dict(kind=attempt['fault'], before_step=boundary['before_step'], delay_ms=300, commit_gate=gate))
                    or not equal(planned.get('commit_recovery'), dict(outcome_url=case.base + '/workflow-outcome?token=' + token, resume_after=boundary['resume_after']))
                    or planned.get('recovery_steps') != [] or planned.get('adversarial_for') is not None):
                raise ValueError('raw commit input differs from frozen boundary')
            native_fault = object_value(session.get('fault_evidence'), 'native commit fault')
            object_value(native_fault.get('native'), 'native commit retirement')
            if (native_fault.get('injected') is not True or native_fault.get('native', {}).get('interrupted_inflight') is not True
                    or native_fault.get('native', {}).get('reaped') is not True
                    or not equal(native_fault.get('commit_gate'), gate)):
                raise ValueError('raw commit lacks native inflight retirement')
            # The interrupted step is retained, not relabelled as a success. All
            # original commands still have one result with their original index.
            steps = object_rows(session.get('steps'), 'commit result steps')
            if len(steps) != len(expected_steps):
                raise ValueError('raw commit task truncated')
            interrupted = []
            for i, step in enumerate(steps):
                if (type(step.get('index')) is not int or step['index'] != i
                        or step.get('expected_error_matched') is not True
                        or type(step.get('timed_out')) is not bool or not isinstance(step.get('outputs'), list)):
                    raise ValueError('raw commit step identity/error differs')
                if step['timed_out']:
                    if i < boundary['before_step'] or step.get('error') != 'native operation deadline exceeded':
                        raise ValueError('raw commit interruption precedes the armed boundary')
                    interrupted.append(i)
                else:
                    expected_error = expected_steps[i].get('expected_error')
                    if ((expected_error is None and step.get('error') is not None)
                            or (expected_error is not None and (not isinstance(step.get('error'), str) or expected_error not in step['error']))):
                        raise ValueError('raw commit unrelated step failed')
            if len(interrupted) != 1:
                raise ValueError('raw commit requires exactly one retained interruption')
            recovery = object_rows(session.get('recovery'), 'native recovery')
            state = 'not_committed' if phase == 'before_commit' else 'committed'
            if ([step.get('stage') for step in recovery] != ['inspect_commit', 'inspect_commit', 'resolve_commit']
                    or any('error' not in step or step.get('error') is not None or not isinstance(step.get('outputs'), list) for step in recovery)
                    or [frame.get('value') for frame in recovery[1]['outputs'] if frame.get('kind') == 'structured'] != [state]):
                raise ValueError('raw commit inspection/recovery sequence differs')
            with database_bytes(artifact(root, claim.get('database'))) as path:
                data = case.receiver.snapshot(path, case.identity)
                with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as db:
                    db.row_factory = sqlite3.Row
                    ledger = [dict(row) for row in db.execute('SELECT token,phase,body_sha256,started_ns,released_ns,terminal,response_status FROM workflow_commit_gates WHERE case_id=?', (case.identity,))]
                    reads = [dict(row) for row in db.execute('SELECT token,state FROM workflow_recovery_views WHERE case_id=?', (case.identity,))]
                    retries = [dict(row) for row in db.execute('SELECT token,body_sha256,matched FROM workflow_recovery_retries WHERE case_id=?', (case.identity,))]
            if len(ledger) != 1 or ledger[0]['token'] != token:
                raise ValueError('raw commit ledger belongs to another gate')
            outcome, problems = adapter.replay_commit_outcome(case, session, data, ledger, reads, retries, phase)
            if problems:
                raise ValueError('raw commit durable replay rejected: ' + '; '.join(problems))
            outcomes.append(outcome)
        records.append(dict(phase=phase, fault=attempt['fault'], injected=True, errors=[],
            context_topology=adapter.context_topology(native, attempt['context_count']),
            native_activity=adapter.native_activity(native, attempt['context_count']), outcomes=outcomes, raw_wave=wave))
    return records


@malformed_input
def replay_probe(case, probe, index, inputs, sessions, database):
    import workflow_native_adapter as adapter
    required = ('forms' if case.family == 'forms' and case.case.get('scenario') == 'confinement'
                else 'saas' if case.family == 'saas' and case.case.get('isolation_probe') else None)
    owner = f'context-{index}'
    if required is None:
        if probe is not None or inputs[owner].get('secrets'):
            raise ValueError('unexpected raw probe or secret authority')
        return None
    identity = f'adversary-{index}'
    if not isinstance(probe, dict) or (probe.get('kind'), probe.get('owner'), probe.get('id')) != (required, owner, identity):
        raise ValueError('raw required probe identity differs')
    planned = inputs.get(identity, {}); session = sessions.get(identity, {})
    if planned.get('adversarial_for') != owner or planned.get('uploads') != [] or planned.get('fault') is not None or planned.get('secrets'):
        raise ValueError('raw probe has foreign authority')
    if required == 'saas':
        steps = [dict(oil=f'goto "{case.base}/records"'), dict(oil='extract text --selector "h1"'),
                 dict(oil=f'goto "{case.base}/probe"'), dict(oil='click "Attempt unauthorized save"'),
                 dict(oil='extract text --selector "h1"'), dict(oil='observe')]
        if inputs[owner].get('secrets'):
            raise ValueError('SaaS probe owner has unexpected injected secret')
    else:
        foreign = probe.get('case_id', '')
        secret_hash = probe.get('secret_sha256', '')
        if (not isinstance(foreign, str) or len(foreign) != 32 or any(c not in '0123456789abcdef' for c in foreign)
                or foreign == case.identity or not isinstance(secret_hash, str) or len(secret_hash) != 64
                or any(c not in '0123456789abcdef' for c in secret_hash)):
            raise ValueError('raw confinement identity or secret digest malformed')
        secrets = inputs[owner].get('secrets')
        if (not isinstance(secrets, list) or len(secrets) != 1 or secrets[0].get('label') != 'confined'
                or secrets[0].get('origin') != case.origin or not isinstance(secrets[0].get('path'), str)
                or Path(secrets[0]['path']).name != f'synthetic-secret-{index}'):
            raise ValueError('raw confinement owner authority differs')
        steps = [dict(oil=f'goto "{case.base}/receipt"'), dict(oil='extract text --selector "#denied"'),
                 dict(oil=f'goto "{case.origin}/advanced/{foreign}/start"'),
                 dict(oil='upload "Authorized document" $file:authorized', expected_error='upload label is unknown'),
                 dict(oil='type "Full name" $secret:confined', expected_error='secret label is unknown'), dict(oil='observe')]
    if not equal(planned.get('steps'), steps) or planned.get('commit_recovery') is not None or planned.get('recovery_steps'):
        raise ValueError('raw adversarial task differs')
    check_steps(steps, session)
    if session.get('recovery'):
        raise ValueError('unexpected raw adversarial recovery')
    if required == 'saas':
        result, errors = adapter.validate_saas_probe(probe, sessions)
        if errors:
            raise ValueError('raw SaaS isolation rejected: ' + '; '.join(errors))
        return result
    frames = [frame for step in session['steps'] for frame in step['outputs']]
    if ([frame['value'] for frame in frames if frame.get('kind') == 'structured'] != ['Context access denied']
            or session.get('secret_labels') != [] or sessions[owner].get('secret_labels') != ['confined']):
        raise ValueError('raw confinement denial/secret ownership differs')
    # The synthetic secret is 64 lowercase hex characters. Check every candidate
    # window without retaining the original secret in the qualification manifest.
    encoded = json.dumps(sessions)
    if any(hashlib.sha256(match.group(1).encode()).hexdigest() == secret_hash
           for match in re.finditer(r'(?=([0-9a-f]{64}))', encoded)):
        raise ValueError('raw native evidence disclosed the synthetic secret')
    data = case.receiver.snapshot(database, foreign)
    if data['requests'] or data['receipts']:
        raise ValueError('raw confinement probe caused a durable mutation')
    return dict(kind='forms_confinement', owner=owner, context_id=str(session['context_id']),
                steps=session['steps'], requests=data['requests'], receipts=data['receipts'], passed=True)


@malformed_input
def check_resources(root, wave, native, artifact):
    from workflow_resources import reconcile_resources
    object_value(wave, 'resource wave')
    samples = object_value(decode(artifact(root, wave.get('samples'))), 'resource samples')
    cleanup = object_value(decode(artifact(root, wave.get('process_cleanup'))), 'resource cleanup')
    accounting = object_value(decode(artifact(root, wave.get('accounting'))), 'resource accounting')
    if (type(cleanup.get('returncode')) is not int or cleanup['returncode'] != 0
            or cleanup.get('errors') != [] or type(cleanup.get('parent_pid')) is not int
            or cleanup['parent_pid'] <= 0 or cleanup['parent_pid'] != samples.get('parent_pid')):
        raise ValueError('raw native process cleanup incomplete or foreign')
    recomputed = reconcile_resources(samples, native)
    if recomputed.get('complete') is not True or recomputed.get('errors') != [] or not equal(accounting, recomputed):
        raise ValueError('raw native resource accounting differs or failed')

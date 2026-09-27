"""Reconcile full-contract attempts without promoting the legacy regression batch.

This validates evidence structure/provenance and exact independent oracle values.
Runtime signature/security qualification remains a separate final-candidate gate.
"""
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import stat

from workflow_contract import DIMENSIONS, planned_attempts, select_attempts
from workflow_frontend_evidence import validate as validate_frontend

CANDIDATE_FIELDS = ('parent_sha256', 'worker_sha256', 'source_tree_sha256')
MAX_ARTIFACT_BYTES = 16 * 1024 * 1024


def artifact(root, reference, *, confined_to=None):
    """Read a bounded, repository-produced artifact with a pinned digest."""
    if not isinstance(reference, dict):
        raise ValueError('missing artifact reference')
    name, digest = reference.get('path'), reference.get('sha256')
    if not isinstance(name, str) or not name or Path(name).is_absolute() or '..' in Path(name).parts:
        raise ValueError('unsafe artifact path')
    path = (Path(root) / name).resolve()
    if not path.is_relative_to(Path(root).resolve()):
        raise ValueError('artifact escapes evidence root')
    if confined_to is not None and not path.is_relative_to(Path(root).resolve()/confined_to):
        raise ValueError('terminal receipt resolves outside exact attempt')
    # Validate the opened descriptor, then bound the actual read. A separate
    # path.stat()/read_bytes() pair can read an arbitrarily grown/replaced file.
    flags = os.O_RDONLY | getattr(os, 'O_NONBLOCK', 0) | getattr(os, 'O_NOFOLLOW', 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ValueError('missing or unreadable artifact') from error
    with os.fdopen(descriptor, 'rb') as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_ARTIFACT_BYTES:
            raise ValueError('missing or oversized artifact')
        data = stream.read(MAX_ARTIFACT_BYTES + 1)
    if len(data) > MAX_ARTIFACT_BYTES:
        raise ValueError('oversized artifact')
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError('artifact digest mismatch')
    return data


def exception_cleanup_diagnostic(root, row):
    """Classify terminal cleanup independently from success qualification."""
    if row.get('disposition') == 'not_executed':
        return dict(cleanup_status='not_applicable', errors=[])
    errors=list(row.get('terminal_evidence_errors') or [])
    uncertain=row.get('cleanup_uncertain') is True
    failed=bool(row.get('cleanup_errors'))
    waves={}
    references=row.get('terminal_evidence')
    prefix=Path('artifacts')/hashlib.sha256(row['attempt_id'].encode()).hexdigest()
    if not isinstance(references,list) or not references:
        return dict(cleanup_status='uncertain' if uncertain else 'failed' if failed else 'unverified',
                    errors=errors+['terminal cleanup evidence unavailable'])
    if len(references)>768:return dict(cleanup_status='unverified',errors=['terminal receipt bound exceeded'])
    for ref in references:
        try:
            path=Path(ref['path'])
            if not path.is_relative_to(prefix) or path.name not in ('native-process-cleanup.json','native-execution-terminal.json','fixture-cleanup.json'):
                raise ValueError('terminal receipt outside attempt or wrong kind')
            wave=waves.setdefault(str(path.parent),{})
            if path.name in wave:raise ValueError('duplicate terminal receipt')
            value=json.loads(artifact(root,ref,confined_to=prefix))
            if not isinstance(value,dict):raise ValueError('malformed terminal receipt')
            wave[path.name]=value
        except (OSError,ValueError,TypeError,KeyError,AttributeError) as error:
            errors.append(str(error))
    initiating_terminal = False
    for wave in waves.values():
        cleanup=wave.get('native-process-cleanup.json');terminal=wave.get('native-execution-terminal.json');fixture=wave.get('fixture-cleanup.json')
        if cleanup is None or terminal is None or fixture is None:
            errors.append('terminal cleanup evidence incomplete');continue
        exact_initiator=(terminal.get('primary_error') == row.get('primary_error')
                and isinstance(row.get('primary_error'),str)
                and terminal.get('timed_out') is (row.get('disposition')=='timed_out')
                and terminal.get('interrupted') is (row.get('disposition')=='interrupted'))
        cleanup_matches_initiator=(exact_initiator
                and all(type(cleanup.get(key)) is type(terminal.get(key))
                        and cleanup.get(key)==terminal.get(key)
                        for key in ('parent_pid','primary_error','timed_out','interrupted')))
        # TimeoutExpired writes its initiating terminal before killing/waiting.
        # A null returncode is not settlement evidence; only the independently
        # pinned final cleanup can establish the final owned process result.
        pending_initiator=('returncode' in terminal and terminal['returncode'] is None and cleanup_matches_initiator
                and cleanup.get('discovery_sealed') is True
                and cleanup.get('group_settled') is True and cleanup.get('errors')==[]
                and cleanup.get('unresolved_candidates')==[])
        final_result_matches=(type(terminal.get('returncode')) is int
                and terminal.get('returncode')==cleanup.get('returncode'))
        if (type(cleanup.get('parent_pid')) is not int or cleanup['parent_pid']<=0
            or type(cleanup.get('process_group')) is not int
            or cleanup.get('process_group')!=cleanup['parent_pid']
            or type(cleanup.get('returncode')) is not int
            or type(terminal.get('parent_pid')) is not int
            or terminal.get('parent_pid')!=cleanup['parent_pid']
            or not (pending_initiator or final_result_matches)):
            errors.append('terminal process identity/result mismatch')
        # Adapter projection can fail after native execution has settled. Its
        # initiating error belongs to the pinned fixture-finally receipt, not
        # the earlier successful native terminal. This only diagnoses custody;
        # it never supplies functional, raw-evidence or oracle qualification.
        adapter_initiator=(row.get('disposition') == 'failed'
                and isinstance(row.get('primary_error'),str) and bool(row['primary_error'])
                and fixture.get('primary_error') == row['primary_error']
                and terminal.get('primary_error') is None
                and terminal.get('timed_out') is False
                and terminal.get('interrupted') is False
                and final_result_matches)
        if exact_initiator or adapter_initiator:
            initiating_terminal=True
        if cleanup.get('discovery_sealed') is not True:
            errors.append('terminal ownership discovery not sealed')
        if cleanup.get('group_settled') is not True or cleanup.get('unresolved_candidates'):
            uncertain=True
        if cleanup.get('discovery_sealed') is False:uncertain=True
        if cleanup.get('errors')!=[]:failed=True
        if fixture.get('custody_lost') is not False:uncertain=True
        if fixture.get('errors')!=[]:failed=True
        receipts=fixture.get('receipts')
        if not isinstance(receipts,list):errors.append('malformed terminal fixture receipts')
        elif any(not isinstance(receipt,dict) or type(receipt.get('returncode')) is not int
                 or type(receipt.get('forced')) is not bool
                 or receipt!={'returncode':0,'forced':False} for receipt in receipts):failed=True
    if not initiating_terminal:errors.append('initiating execution terminal unavailable')
    return dict(cleanup_status='uncertain' if uncertain else 'failed' if failed else 'unverified' if errors else 'settled',errors=errors)


def reconcile(contract, candidate, records, root, oracles, *, attempt_ids=None):
    """Oracles are frozen caller inputs, never runner-provided expected values.

    Every required attempt remains present, including failed initial attempts.
    Retries use retry_of plus a distinct ID and cannot replace an original failure.
    A mixed-family attempt must include measured simultaneous context activity;
    configured worker counts and sequential context creation are insufficient.
    """
    planned = {item['attempt_id']: item for item in
               select_attempts(planned_attempts(contract), attempt_ids)}
    errors = []
    for field in CANDIDATE_FIELDS:
        value = candidate.get(field)
        if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
            errors.append('invalid candidate digest: ' + field)
    candidate_fields = CANDIDATE_FIELDS
    if 'candidate_manifest_sha256' in candidate:
        candidate_fields = (*candidate_fields, 'candidate_manifest_sha256')
        value = candidate['candidate_manifest_sha256']
        if not isinstance(value, str) or len(value) != 64 or any(c not in '0123456789abcdef' for c in value):
            errors.append('invalid combined candidate identity')
    validated = []
    terminal_diagnostics = {}
    dimensions = set()
    if not isinstance(records, list) or any(not isinstance(row, dict) for row in records):
        return dict(structurally_valid=False, errors=['malformed records'], planned=len(planned), validated=[])
    if any(not isinstance(row.get('attempt_id'), str) for row in records):
        return dict(structurally_valid=False, errors=['malformed attempt identity'], planned=len(planned), validated=[])
    counts = Counter(row['attempt_id'] for row in records)
    if any(count != 1 for count in counts.values()):
        errors.append('duplicate attempt identities')
    originals = {row['attempt_id']: row for row in records if row.get('retry_of') is None}
    if set(originals) != set(planned):
        errors.append('missing or unexpected required attempts')
    for row in records:
        identity = row['attempt_id']
        if row.get('retry_of') is not None and (not isinstance(row['retry_of'], str) or not row['retry_of']):
            errors.append(identity + ': malformed retry identity')
            continue
        expected_id = row.get('retry_of') or identity
        expected = planned.get(expected_id)
        problems = []
        if expected is None or (row.get('retry_of') is not None and identity in planned):
            errors.append(identity + ': invalid retry or unplanned attempt')
            continue
        if any(type(row.get(key)) is not type(value) or row.get(key) != value
               for key, value in expected.items() if key != 'attempt_id'):
            problems.append('attempt specification mismatch')
        if any(row.get(key) != candidate.get(key) or not candidate.get(key) for key in candidate_fields):
            problems.append('candidate identity mismatch')
        if (row.get('passed') is not True or row.get('errors') != []
                or row.get('timed_out') is not False):
            problems.append('failed attempt retained')
        disposition=row.get('disposition')
        failed_exception=(disposition == 'failed' and 'terminal_evidence' in row)
        if disposition in ('not_executed','interrupted','timed_out') or failed_exception:
            flags=(row.get('passed'),row.get('timed_out'),row.get('interrupted'))
            expected_flags=(False,disposition=='timed_out',disposition=='interrupted')
            if any(type(value) is not bool for value in flags) or flags!=expected_flags:
                problems.append('execution disposition contradicts flags')
            if disposition=='not_executed' and (any(key in row for key in
                ('actions','oracle','cleanup','terminal_evidence'))
                or row.get('functional_passed') is True or row.get('cleanup_uncertain') is True):
                problems.append('unexecuted original claims execution evidence')
            diagnostic=exception_cleanup_diagnostic(root,row)
            terminal_diagnostics[identity]=dict(disposition=disposition,**diagnostic)
            # Nonpassing originals retain the denominator and fail qualification;
            # absence of a success payload is not a runtime/cleanup failure.
            if diagnostic['cleanup_status'] in ('uncertain','failed','unverified'):
                problems.append('terminal cleanup '+diagnostic['cleanup_status'])
            problems.extend(diagnostic['errors'])
            errors.extend(identity + ': ' + problem for problem in problems)
            continue
        cleanup = row.get('cleanup')
        if (not isinstance(cleanup, dict) or type(cleanup.get('returncode')) is not int
                or cleanup != {'returncode': 0, 'forced': False}
                or cleanup.get('forced') is not False):
            problems.append('fixture cleanup failed')
        try:
            # These are separately pinned artifacts rather than success booleans.
            actions = json.loads(artifact(root, row.get('actions')))
            observed = json.loads(artifact(root, row.get('oracle')))
            oracle_key=expected_id if expected_id in oracles else expected['cell']
            if (oracle_key not in oracles or json.dumps(observed, sort_keys=True)
                    != json.dumps(oracles[oracle_key], sort_keys=True)):
                problems.append('independent oracle mismatch')
            if not isinstance(actions, list) or not actions:
                problems.append('missing genuine OIL actions')
            elif any(not isinstance(action, dict) or action.get('execution_domain') != 'native'
                     or type(action.get('action_id')) is not int or action['action_id'] < 1
                     or (action.get('page_id') is not None and type(action['page_id']) is not int)
                     for action in actions):
                problems.append('invalid native OIL action identity')
            elif len({(action.get('recipe'),action.get('context_id'),action.get('page_id'), action['action_id']) for action in actions}) != len(actions):
                problems.append('duplicate native OIL action identity')
            if 'candidate_manifest_sha256' in candidate:
                from workflow_raw_evidence import validate as validate_raw
                validate_raw(root, row.get('raw_evidence'), row, candidate, artifact, observed, actions)
            # Authority history and records are fields of the frozen oracle;
            # receipt claims in the page are never substituted for that oracle.
            activity = json.loads(artifact(root, row.get('context_activity')))
            if not isinstance(activity, list) or len(activity) != expected['context_count']:
                problems.append('context activity count mismatch')
            elif any(not isinstance(item, dict) or not isinstance(item.get('context_id'), str)
                     or type(item.get('start_ns')) is not int or type(item.get('end_ns')) is not int
                     or item['start_ns'] >= item['end_ns'] for item in activity):
                problems.append('invalid context activity interval')
            elif (len({item['context_id'] for item in activity}) != len(activity)
                  or max(item['start_ns'] for item in activity) >= min(item['end_ns'] for item in activity)):
                problems.append('required contexts were not simultaneously active')
            if row.get('resource_evidence') is not None:
                resources=json.loads(artifact(root,row['resource_evidence']))
                if not isinstance(resources,list) or not resources:
                    raise ValueError('missing native resource waves')
                for resource in resources:
                    accounting=json.loads(artifact(root,resource['accounting']))
                    samples=json.loads(artifact(root,resource['samples']))
                    cleanup=json.loads(artifact(root,resource['process_cleanup']))
                    if accounting.get('complete') is not True or accounting.get('errors')!=[]:
                        problems.append('native resource accounting failed')
                    if samples.get('errors')!=[] or samples.get('overflow') is not False or not samples.get('samples'):
                        problems.append('native resource samples incomplete')
                    if (cleanup.get('group_settled') is not True or type(cleanup.get('process_group')) is not int
                            or cleanup.get('process_group') != cleanup.get('parent_pid')
                            or type(cleanup.get('returncode')) is not int or cleanup['returncode']!=0
                            or cleanup.get('errors')!=[] or type(cleanup.get('parent_pid')) is not int
                            or cleanup['parent_pid']<=0 or cleanup['parent_pid']!=samples.get('parent_pid')):
                        problems.append('native process cleanup failed or belongs to another parent')
            if row.get('context_topology') is not None:
                topology=json.loads(artifact(root,row['context_topology']))
                probe_count=0
                if not isinstance(topology,list) or not topology:problems.append('missing native context topology')
                else:
                    for wave in topology:
                        contexts=wave.get('contexts',[])
                        cohort=[item for item in contexts if item.get('adversarial_for') is None]
                        probes=[item for item in contexts if item.get('adversarial_for') is not None]
                        owners={item.get('id') for item in cohort}
                        if (wave.get('cohort_count')!=expected['context_count'] or len(cohort)!=expected['context_count']
                                or wave.get('probe_count')!=len(probes) or wave.get('total_live_contexts')!=len(contexts)
                                or len({item.get('context_id') for item in contexts})!=len(contexts)
                                or any(item['adversarial_for'] not in owners for item in probes)):
                            problems.append('native cohort/probe topology mismatch')
                        probe_count+=len(probes)
                if expected['cell'] in ('forms.confinement','saas.isolation') and probe_count==0:
                    problems.append('isolation cell lacks an actual adversarial context')
                if probe_count:
                    probes=json.loads(artifact(root,row.get('adversarial_evidence')))
                    if not isinstance(probes,list) or len(probes)!=probe_count or any(probe.get('passed') is not True for probe in probes):
                        problems.append('missing successful native adversarial evidence')
            elif expected['cell'] in ('forms.confinement','saas.isolation'):
                problems.append('isolation cell lacks explicit workload/probe topology')
            if expected['fault']:
                fault = json.loads(artifact(root, row.get('fault_evidence')))
                if fault.get('fault') != expected['fault'] or fault.get('injected') is not True:
                    problems.append('missing actual fault injection')
        except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
            problems.append(str(error))
        frontend = row.get('frontend_dimensions', [])
        if not isinstance(frontend, list) or any(not isinstance(item, str) or item not in DIMENSIONS for item in frontend):
            problems.append('invalid frontend coverage')
        if 'candidate_manifest_sha256' in candidate:
            try:
                witnessed = validate_frontend(root, row.get('frontend_evidence'), row, candidate, artifact)
                if set(frontend) != witnessed:
                    problems.append('frontend labels differ from independently validated witnesses')
            except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
                problems.append('frontend witness: ' + str(error))
        if problems:
            errors.extend(identity + ': ' + problem for problem in problems)
        else:
            validated.append(identity)
            dimensions.update(frontend)
    if dimensions != DIMENSIONS:
        errors.append('frontend covering matrix incomplete')
    return dict(structurally_valid=not errors, errors=errors, planned=len(planned), validated=validated,
                terminal_diagnostics=terminal_diagnostics,
                retries=sum(row.get('retry_of') is not None for row in records),
                missing_attempts=sorted(set(planned)-set(originals)),
                missing_frontend_dimensions=sorted(DIMENSIONS-dimensions))

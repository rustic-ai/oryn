"""Observed failure attribution, without inventing a root cause from correlation."""
import re

_FOLLOWUP_PREFIX = 'Action completed; follow-up observation unavailable: '
_IPC_DEADLINE = re.compile(
    r'^Action completed; follow-up observation unavailable: '
    r'sandboxed native worker exceeded wall time '
    r'\(phase=(?P<phase>[a-z_]+), command=(?P<command>[a-z_]+), '
    r'message_id=(?P<message_id>[0-9]+)\) limit\. Cached aliases were cleared\.$')


def attribute_native_failure(native):
    failures = []
    observations = []
    first_observations = []
    for message in native.get('infrastructure_errors', []):
        failures.append(dict(stage='native_infrastructure', message=message, confirmed_cause=None))
    for session_index, session in enumerate(native.get('sessions', [])):
        identity = dict(context_id=str(session.get('context_id')),
                        page_id=str(session.get('page_id')), session=session.get('id'))
        previous_followup = None
        first_followup = None
        for step_index, step in enumerate(session.get('steps', [])):
            current_followup = None
            for output_index, output in enumerate(step.get('outputs', [])):
                if output.get('kind') != 'action' or step.get('error') is not None:
                    continue
                action = output.get('result', {})
                if type(action.get('action_id')) is not int:
                    continue
                for diagnostic_index, diagnostic in enumerate(action.get('diagnostics', [])):
                    if not isinstance(diagnostic, str) or not diagnostic.startswith(_FOLLOWUP_PREFIX):
                        continue
                    match = _IPC_DEADLINE.fullmatch(diagnostic)
                    row = dict(**identity, stage='native_followup_observation',
                               step=step.get('index'), action_id=action['action_id'],
                               action_completed=True, message=diagnostic,
                               observation_timed_out=match is not None,
                               phase=match['phase'] if match else None,
                               command=match['command'] if match else None,
                               message_id=int(match['message_id']) if match else None,
                               timeout_limit_ms=None, confirmed_cause=None,
                               source_pointer=f'/sessions/{session_index}/steps/{step_index}/outputs/{output_index}/result/diagnostics/{diagnostic_index}')
                    # This describes the failed observation, never a failed or
                    # replayable action. The original native receipt is untouched.
                    observations.append(row)
                    current_followup = row
                    if first_followup is None:
                        first_followup = row
                        first_observations.append(row)
            if step.get('error') and step.get('expected_error_matched') is not True:
                row = dict(**identity, stage='native_operation', step=step.get('index'),
                           timed_out=step.get('timed_out', False), message=step['error'],
                           confirmed_cause=None)
                if (previous_followup is not None
                        and previous_followup['observation_timed_out']
                        and previous_followup['command'] == 'observe'
                        and previous_followup['phase'] == 'ipc_response'
                        and step['error'] == 'invalid page command: browser context has no active page'):
                    row['preceding_followup_observation'] = dict(
                        step=previous_followup['step'], action_id=previous_followup['action_id'],
                        message_id=previous_followup['message_id'],
                        source_pointer=previous_followup['source_pointer'],
                        relationship='observed_predecessor_not_proven_root_cause')
                failures.append(row)
            # Associate only an immediately preceding step. A later unrelated
            # missing-page failure must not inherit an old session diagnostic.
            previous_followup = current_followup
        for message in session.get('errors', []):
            failures.append(dict(**identity, stage='native_session', message=message, confirmed_cause=None))
        for message in session.get('cleanup_errors', []):
            failures.append(dict(**identity, stage='native_teardown', message=message, confirmed_cause=None))
        if session.get('workers_retired') is not True:
            failures.append(dict(**identity, stage='native_teardown', message='workers not retired', confirmed_cause=None))
        if session.get('fault_evidence') and session['fault_evidence'].get('injected') is not True:
            failures.append(dict(**identity, stage='fault_injection', message='requested actual fault lacks injection evidence', confirmed_cause=None))
    return dict(schema_version=1, failures=failures, root_causes_confirmed=False,
                followup_observations=observations,
                first_followup_observation_per_session=first_observations)

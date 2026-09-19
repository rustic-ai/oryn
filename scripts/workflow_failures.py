"""Observed failure attribution, without inventing a root cause from correlation."""


def attribute_native_failure(native):
    failures=[]
    for message in native.get('infrastructure_errors',[]):failures.append(dict(stage='native_infrastructure',message=message,confirmed_cause=None))
    for session in native.get('sessions',[]):
        identity=dict(context_id=str(session.get('context_id')),page_id=str(session.get('page_id')),session=session.get('id'))
        for step in session.get('steps',[]):
            if step.get('error') and step.get('expected_error_matched') is not True:
                failures.append(dict(**identity,stage='native_operation',step=step.get('index'),timed_out=step.get('timed_out',False),message=step['error'],confirmed_cause=None))
        for message in session.get('errors',[]):failures.append(dict(**identity,stage='native_session',message=message,confirmed_cause=None))
        for message in session.get('cleanup_errors',[]):failures.append(dict(**identity,stage='native_teardown',message=message,confirmed_cause=None))
        if session.get('workers_retired') is not True:failures.append(dict(**identity,stage='native_teardown',message='workers not retired',confirmed_cause=None))
        if session.get('fault_evidence') and session['fault_evidence'].get('injected') is not True:
            failures.append(dict(**identity,stage='fault_injection',message='requested actual fault lacks injection evidence',confirmed_cause=None))
    return dict(schema_version=1,failures=failures,root_causes_confirmed=False)

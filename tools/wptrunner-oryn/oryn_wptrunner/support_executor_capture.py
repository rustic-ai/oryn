"""Capture after original upstream conversion, before test-window teardown.

Authentication is the existing Oryn native driver session transport. Descriptor
issuance is native fixture configuration, never this result-consumer function.
"""
from .support_observation_join import join, PARENT

def capture(executor, test, converted, descriptor, candidate):
    if test.url != PARENT:
        return converted
    result, subtests = converted
    # A crashed parent cannot be promoted by stale child output or even trigger
    # a fresh native read to reconstruct a missing original observation.
    if result.status != 'OK':
        observation = join(descriptor, None, result.status, [],
                           candidate=candidate, process_complete=False)
    else:
        try:
            native = executor.protocol.webdriver.send_session_command(
                'GET', 'oryn/diagnostics/support-resource')
            rows = [{'name': row.name, 'status': row.status} for row in subtests]
            # Process settlement is recorded later by the canonical supervisor.
            # Capture only establishes live native/original-observation closure.
            observation = join(descriptor, native, result.status, rows,
                               candidate=candidate, process_complete=True)
            observation['native_observation_complete'] = observation['complete']
            observation['complete'] = False
            observation['process_settlement_required'] = True
        except Exception as error:
            observation = {'schema': 1, 'kind': 'parent-derived-support-observation',
                           'parent': PARENT, 'complete': False,
                           'collection_error': type(error).__name__,
                           'qualification': 'not-evaluated'}
    result.extra = dict(result.extra or {})
    result.extra['oryn_support_resource'] = observation
    return converted

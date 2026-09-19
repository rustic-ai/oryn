"""Occurrence-level route coverage for the pinned, selected Core inventory.

A mapped production route is source evidence only. Its algorithms, fault controls
and original-run results still require separate review and qualification.
"""
from pathlib import Path

NATIVE = 'crates/oryn/src/webdriver/'
OPERATIONS = {
    'Actions': [NATIVE + 'server.rs', 'crates/oryn-native/src/webdriver_actions.rs', 'crates/oryn-native/src/webdriver_action_execution.rs'],
    'click': [NATIVE + 'server.rs', NATIVE + 'element_input.rs'],
    'bless': [NATIVE + 'server.rs', NATIVE + 'element_input.rs'],
    'send_keys': [NATIVE + 'element_input.rs', 'crates/oryn-native/src/webdriver_actions.rs'],
    'get_computed_label': [NATIVE + 'server.rs', 'crates/oryn-native/src/webdriver_script_v8.rs'],
    'get_computed_role': [NATIVE + 'server.rs', 'crates/oryn-native/src/webdriver_script_v8.rs'],
    'get_accessibility_properties_for_element': [NATIVE + 'server.rs', 'tools/wptrunner-oryn/oryn_wptrunner/protocol.py'],
    'get_accessibility_properties_for_accessibility_node': [NATIVE + 'server.rs', 'tools/wptrunner-oryn/oryn_wptrunner/protocol.py'],
    'get_all_cookies': [NATIVE + 'server.rs', NATIVE + 'session.rs'],
    'get_named_cookie': [NATIVE + 'server.rs', NATIVE + 'session.rs'],
    'delete_all_cookies': [NATIVE + 'server.rs', NATIVE + 'session.rs'],
    'set_permission': [NATIVE + 'server.rs', 'crates/oryn-native/src/platform_control.rs'],
    'bidi.permissions.set_permission': ['tools/wptrunner-oryn/oryn_wptrunner/protocol.py', NATIVE + 'server.rs', NATIVE + 'session.rs', 'crates/oryn-native/src/runtime.rs', 'crates/oryn-native/src/platform_control.rs'],
    'bidi.emulation.set_geolocation_override': ['tools/wptrunner-oryn/oryn_wptrunner/protocol.py', NATIVE + 'server.rs', NATIVE + 'session.rs', 'crates/oryn-native/src/runtime.rs', 'crates/oryn-native/src/platform_control.rs'],
    'set_window_rect': [NATIVE + 'window_rect.rs'],
    'get_window_rect': [NATIVE + 'window_rect.rs'],
    'minimize_window': [NATIVE + 'server.rs', 'crates/oryn-native/src/platform_control.rs'],
    'set_rph_registration_mode': [NATIVE + 'server.rs', 'crates/oryn-native/src/platform_control.rs'],
    'set_test_context': ['tools/wptrunner-oryn/oryn_wptrunner/product.py', 'tools/wptrunner-oryn/oryn_wptrunner/protocol.py', NATIVE + 'session.rs', 'crates/oryn-native/src/webdriver_script_v8.rs'],
}
CONDITIONAL_BLUETOOTH = {'bidi.bluetooth.' + name for name in (
    'simulate_service', 'simulate_gatt_connection_response', 'simulate_characteristic', 'simulate_descriptor',
    'characteristic_event_generated.on', 'descriptor_event_generated.on', 'request_device_prompt_updated.subscribe',
    'gatt_connection_attempted.subscribe', 'characteristic_event_generated.subscribe', 'descriptor_event_generated.subscribe',
    'disable_simulation', 'request_device_prompt_updated.once', 'handle_request_device_prompt', 'simulate_adapter',
    'simulate_preconnected_peripheral', 'gatt_connection_attempted.on', 'simulate_gatt_disconnection',
    'simulate_characteristic_response', 'simulate_descriptor_response')}
CONDITIONAL_SOURCES = {'bluetooth/resources/bluetooth-test.js', 'resources/web-bluetooth-bidi-test.js'}


def validate(records, root):
    rows = [record for record in records if 'operation' in record]
    if not rows: return
    if {row['operation'] for row in rows} != set(OPERATIONS) | CONDITIONAL_BLUETOOTH:
        raise ValueError('pinned operation source coverage changed; review every added/removed operation')
    for row in rows:
        operation = row['operation']
        if operation in CONDITIONAL_BLUETOOTH:
            if row.get('direct_core_source') is not False or row['source'] not in CONDITIONAL_SOURCES:
                raise ValueError('Bluetooth conditional exclusion cannot cover a required Core caller')
        else:
            for path in OPERATIONS[operation]:
                if not (Path(root) / path).is_file():
                    raise ValueError('required native operation source missing: ' + path)


# Saved occurrence counts are an audit boundary, never a replacement test denominator.
DYNAMIC_HELPERS = {
    'bluetooth/resources/bluetooth-test.js': (3, 'conditional_script_loader'),
    'content-security-policy/reporting/post-redirect-stacktrace.https.html': (1, 'required_script_loader'),
    'cors/script-304.html': (3, 'required_script_loader'),
    'fetch/cross-origin-resource-policy/script-loads.html': (5, 'required_script_loader'),
    'fetch/range/resources/utils.js': (1, 'required_script_loader'),
    'fetch/stale-while-revalidate/stale-script.html': (3, 'required_script_loader'),
    'html/anonymous-iframe/cache-storage.tentative.https.window.js': (4, 'required_generated_dispatcher_code'),
}


def validate_dynamic_helpers(records, root):
    from collections import Counter
    rows = [row for row in records if row.get('classification') == 'dynamic_loader_parameter']
    if not rows:
        return
    if Counter(row.get('source') for row in rows) != Counter({key: value[0] for key, value in DYNAMIC_HELPERS.items()}):
        raise ValueError('dynamic helper occurrence inventory changed')
    if len({row.get('occurrence') for row in rows}) != len(rows):
        raise ValueError('duplicate dynamic helper occurrence')
    for row in rows:
        review = row.get('dynamic_review')
        if not isinstance(review, dict) or review.get('kind') != DYNAMIC_HELPERS[row['source']][1]:
            raise ValueError('dynamic helper requires exact caller classification')
        if not row.get('review_reason') or not review.get('upstream_dependencies') or not review.get('consumers'):
            raise ValueError('dynamic helper requires upstream and native caller evidence')
        for dependency in review['upstream_dependencies']:
            if not isinstance(dependency, dict) or not dependency.get('path') or len(dependency.get('sha256', '')) != 64:
                raise ValueError('dynamic helper dependency identity missing')
        for consumer in review['consumers']:
            path = (Path(root) / consumer).resolve()
            if Path(root).resolve() not in path.parents or not path.is_file():
                raise ValueError('dynamic helper native consumer missing: ' + consumer)

# Immutable occurrence identity, separate from editable review prose. Names alone
# cannot detect a deleted caller when another occurrence uses the same operation.
PINNED_OCCURRENCES = {
    'operations': (1516, 'da233f8fbbe8cf30b1b3782a17dcd13a172b537e4dbd12cd3159e786a3b12850',
                   ('upstream_revision', 'source', 'source_sha256', 'line', 'literal', 'operation', 'direct_core_source')),
    'helpers': (1199, '22daebc5e413d7333a5067cbf82e67f6854f908389d90b93fcde3827a4b33814',
                ('upstream_revision', 'occurrence', 'source', 'source_sha256', 'line', 'literal', 'target', 'classification')),
}


def validate_occurrence_identities(records, require_complete=False):
    """Bind every saved caller/helper, including multiplicity and conditional scope.

    Sorting permits review-file reordering. Review prose and later qualification
    remain separate; changing the immutable source/classification requires a new
    reviewed inventory, never a silently reduced caller count.
    """
    import hashlib
    import json
    groups = {
        'operations': [row for row in records if 'operation' in row],
        'helpers': [row for row in records if 'occurrence' in row and 'classification' in row],
    }
    for kind, rows in groups.items():
        if not rows and not require_complete:
            continue
        count, expected, fields = PINNED_OCCURRENCES[kind]
        if len(rows) != count:
            raise ValueError('pinned ' + kind + ' occurrence count changed')
        encoded = sorted(json.dumps({key: row.get(key) for key in fields},
                                    sort_keys=True, separators=(',', ':'), ensure_ascii=True)
                         for row in rows)
        actual = hashlib.sha256(('\n'.join(encoded) + '\n').encode()).hexdigest()
        if actual != expected:
            raise ValueError('pinned ' + kind + ' occurrence identity changed')

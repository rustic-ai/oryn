"""Prerequisite verdict controls; no browser, TLS server or hardware execution."""
import copy
from pathlib import Path
import signal
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from wpt_native_controls import validate_results


def response(value, status=200): return {'status': status, 'response': {'value': value}}


def accepted_fixture():
    browser = {'cleanup': {'complete': True}, 'errors': [], 'controls': {
        'tls_trusted': response(None),
        'tls_untrusted': response({'error': 'insecure certificate'}, 500),
        'tls_wrong_hostname': response({'error': 'insecure certificate'}, 500),
        'initial_focus': True, 'restored_focus': True, 'denied_focus': False, 'focus_samples': [True, False]}}
    native = copy.deepcopy(browser)
    native['controls'].update({'unsupported': response({'error': 'unknown command'}, 404),
        'missing_completion': response({'error': 'script timeout'}, 500), 'duplicate_completion': response(1),
        'worker_crash': response({'reaped': True, 'signal': signal.SIGKILL, 'worker_pid': 123}),
        'after_crash': response({'error': 'no such window'}, 404)})
    return {'focus_isolation': 'host_serialized', 'tls_probes': {'trusted': {'verified': True},
        'untrusted': {'verified': False, 'verify_code': 20}, 'wrong_hostname': {'verified': False, 'verify_code': 62}},
        'native': native, 'reference': browser, 'protocol_faults': {'successful': True, 'tests_run': 4,
        'tests': ['test_malformed_command_response_reaches_real_upstream_decoder',
                  'test_failed_native_teardown_is_logged_and_propagated', 'test_custody_cannot_be_redirected_to_a_symlink',
                  'test_checked_session_requires_native_receipt_for_already_retired_session'],
        'failures': [], 'errors': [], 'skipped': []}}


class NativeControlTests(unittest.TestCase):
    def test_complete_independent_controls_are_required(self):
        record = accepted_fixture()
        self.assertEqual(validate_results(record), [])
        for mutate in (
            lambda value: value['tls_probes']['wrong_hostname'].update(verify_code=20),
            lambda value: value['tls_probes']['untrusted'].update(verified=True),
            lambda value: value['reference']['controls'].update(focus_samples=[True], denied_focus=True),
            lambda value: value['native']['controls']['worker_crash']['response']['value'].update(reaped=False),
            lambda value: value['native']['controls'].update(duplicate_completion=response(2)),
            lambda value: value['native'].update(cleanup=None),
            lambda value: value['protocol_faults'].update(tests_run=2),
        ):
            changed = copy.deepcopy(record); mutate(changed)
            self.assertTrue(validate_results(changed))

    def test_network_timeout_is_not_a_certificate_negative_control(self):
        record = accepted_fixture()
        record['reference']['controls']['tls_untrusted'] = response({'error': 'timeout', 'message': 'network timeout'}, 500)
        self.assertTrue(any('certificate rejection' in error for error in validate_results(record)))

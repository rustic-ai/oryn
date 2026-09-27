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
        'native': native, 'reference': browser, 'protocol_faults': {'successful': True, 'tests_run': 6,
        'tests': ['test_malformed_command_response_reaches_real_upstream_decoder',
                  'test_failed_native_teardown_is_logged_and_propagated', 'test_custody_cannot_be_redirected_to_a_symlink',
                  'test_checked_session_requires_native_receipt_for_already_retired_session',
                'test_checked_session_preserves_delete_error_when_transport_close_fails',
                'test_checked_session_failure_reaches_exact_protocol_terminal_receipt'],
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


class CertificateObservationTests(unittest.TestCase):
    def chrome_interstitial(self, record, kind):
        from wpt_native_controls import CERTIFICATE_DOCUMENT_SCRIPT
        controls = record['reference']['controls']
        controls.update(session_id='owned', capabilities={'browserName': 'chrome', 'acceptInsecureCerts': False},
                        tls_urls={'untrusted': 'https://web-platform.test:8444/native-prerequisite',
                                  'wrong_hostname': 'https://web-platform.test:9000/native-prerequisite'})
        controls['tls_' + kind] = dict(response(None), method='POST', path='/session/owned/url',
                                     body={'url': controls['tls_urls'][kind]})
        code = {'untrusted': 'net::ERR_CERT_AUTHORITY_INVALID',
                'wrong_hostname': 'net::ERR_CERT_COMMON_NAME_INVALID'}[kind]
        controls['tls_' + kind + '_document'] = dict(response({'title': 'Privacy error', 'code': code,
            'fixture_marker_present': False}), method='POST', path='/session/owned/execute/sync',
            body={'script': CERTIFICATE_DOCUMENT_SCRIPT, 'args': []})
        return controls

    def test_reference_null_navigation_requires_exact_observed_certificate_interstitial(self):
        record = accepted_fixture()
        for kind in ('untrusted', 'wrong_hostname'):
            self.chrome_interstitial(record, kind)
        self.assertEqual(validate_results(record), [])
        for kind in ('untrusted', 'wrong_hostname'):
            bad = copy.deepcopy(record)
            del bad['reference']['controls']['tls_' + kind + '_document']
            self.assertTrue(any('certificate rejection' in e for e in validate_results(bad)))

    def test_interstitial_wrong_code_marker_authority_or_session_cannot_pass(self):
        record = accepted_fixture(); self.chrome_interstitial(record, 'untrusted')
        mutations = [
            lambda c: c['tls_untrusted_document']['response']['value'].update(code='net::ERR_CERT_COMMON_NAME_INVALID'),
            lambda c: c['tls_untrusted_document']['response']['value'].update(code='net::ERR_CONNECTION_REFUSED'),
            lambda c: c['tls_untrusted_document']['response']['value'].update(title='Other page'),
            lambda c: c['tls_untrusted_document']['response']['value'].update(fixture_marker_present=True),
            lambda c: c['tls_untrusted_document'].update(path='/session/foreign/execute/sync'),
            lambda c: c['tls_untrusted'].update(body={'url':'https://foreign.test/'}),
            lambda c: c['tls_untrusted_document']['body'].update(script='return {}'),
            lambda c: c['capabilities'].update(acceptInsecureCerts=True),
            lambda c: c['capabilities'].update(browserName='other'),
            lambda c: c['tls_untrusted_document'].update(status=500),
            lambda c: c['tls_untrusted']['response'].clear(),
        ]
        for mutate in mutations:
            bad = copy.deepcopy(record); mutate(bad['reference']['controls'])
            self.assertTrue(any('certificate rejection' in e for e in validate_results(bad)))

    def test_null_scalar_protocol_payloads_fail_without_crashing_or_native_fallback(self):
        for value in (None, [], 'certificate', 1, True):
            for name in ('native', 'reference'):
                bad = accepted_fixture(); bad[name]['controls']['tls_untrusted'] = response(value, 500)
                self.assertTrue(any('certificate rejection' in e for e in validate_results(bad)))
            for command in ('unsupported','missing_completion','after_crash','worker_crash'):
                bad = accepted_fixture(); bad['native']['controls'][command] = response(value)
                self.assertTrue(validate_results(bad))
        bad=accepted_fixture(); controls=self.chrome_interstitial(bad,'untrusted')
        bad['native']['controls'].update(copy.deepcopy(controls))
        self.assertTrue(any(e.startswith('native: untrusted') for e in validate_results(bad)))

    def test_interstitial_does_not_waive_tls_probe_focus_or_cleanup(self):
        record=accepted_fixture(); self.chrome_interstitial(record,'untrusted')
        for mutate in (
            lambda r:r['tls_probes']['untrusted'].update(verified=True),
            lambda r:r['reference']['controls'].update(focus_samples=[True],denied_focus=True),
            lambda r:r['reference']['cleanup'].update(complete=False),
        ):
            bad=copy.deepcopy(record);mutate(bad);self.assertTrue(validate_results(bad))

    def test_reference_capabilities_are_visible_and_keep_certificate_verification(self):
        from wpt_native_controls import reference_capabilities
        self.assertEqual(reference_capabilities(Path('/pinned/chrome')), {
            'browserName':'chrome','acceptInsecureCerts':False,
            'goog:chromeOptions':{'binary':'/pinned/chrome','args':[]}})

    def test_collector_captures_negative_document_before_next_navigation(self):
        from wpt_native_controls import browser_controls, CERTIFICATE_DOCUMENT_SCRIPT, MARKER
        class FakeWire:
            session=None
            def __init__(self):self.commands=[];self.url=None
            def scoped(self,path):return '/session/'+self.session+'/'+path
            def command(self,method,path,body=None):
                self.commands.append((method,path,body))
                if path.endswith('/url'):self.url=body['url'];value=None
                else:
                    self_test.assertEqual(body,{'script':CERTIFICATE_DOCUMENT_SCRIPT,'args':[]})
                    code='net::ERR_CERT_AUTHORITY_INVALID' if ':8444/' in self.url else 'net::ERR_CERT_COMMON_NAME_INVALID'
                    value={'title':'Privacy error','code':code,'fixture_marker_present':False}
                return dict(response(value),method=method,path=path,body=body)
            def value(self,method,path,body=None):
                if path=='/session':return {'sessionId':'owned','capabilities':{'browserName':'chrome','acceptInsecureCerts':False}}
                if path.endswith('/url'):return self.command(method,path,body)['response']['value']
                if path.endswith('/window/new'):return {'handle':'second'}
                if path.endswith('/window'):return 'first'
                return None
            def script(self,script):
                if script=='return document.title':return MARKER
                if script=='return document.hasFocus()':return True
                from wpt_native_controls import REFERENCE_FOCUS_STOP
                if script == REFERENCE_FOCUS_STOP:
                    return {'document_url':self.url,'overflow':0,'rows':[
                        {'phase':'installed','at_ms':0,'focused':True},
                        {'phase':'blur','at_ms':1,'focused':False}]}
                if script.startswith('clearInterval'):return [True,False]
                return None
        self_test=self;wire=FakeWire()
        contract={'tls':{kind:{'url':f'https://web-platform.test:{port}/native-prerequisite'}
                         for kind,port in [('trusted',8443),('untrusted',8444),('wrong_hostname',9000)]}}
        from unittest.mock import patch
        def activated(wire, results, phase):
            handle='second' if phase=='other' else 'first'
            results.setdefault('focus_activation',[]).append({'phase':phase,'handle':handle,
                'target_after':{'targetId':handle},'window_after':{'windowId':7}})
        with patch('wpt_native_controls.activate_reference_window', side_effect=activated) as activate:
            result=browser_controls(wire,contract,{},False)
        self.assertEqual([call.args[2] for call in activate.call_args_list], ['initial','other','restored'])
        operations=[(path.rsplit('/',1)[-1],body) for _,path,body in wire.commands]
        self.assertEqual([x[0] for x in operations],['url','url','sync','url','sync','url'])
        self.assertEqual(result['tls_untrusted_document']['body']['script'],CERTIFICATE_DOCUMENT_SCRIPT)
        self.assertFalse(result['denied_focus'])

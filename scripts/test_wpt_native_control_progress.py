"""Source-generated controls regressions: no real browser or server."""
import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch, Mock

sys.path.insert(0, str(Path(__file__).resolve().parent/'tests'))
from test_wpt_native_controls import accepted_fixture
from wpt_native_controls import (MARKER, TLS_KINDS, activate_reference_window,
    browser_controls, new_controls, validate_results)


class FocusWire:
    session = 'session'
    def __init__(self, samples=(False, True), mismatch=False, retire=False, handle='CDwindow-owned'):
        self.calls=[];self.samples=iter(samples);self.mismatch=mismatch;self.retire=retire;self.targets=0;self.handle=handle
    def scoped(self,path):return '/session/session/'+path
    def value(self,method,path,body=None,*,deadline=None):
        self.calls.append((method,path,body,deadline))
        if path.endswith('/window'):return self.handle
        cmd=body['cmd']
        if cmd=='Target.getTargetInfo':
            self.targets+=1
            return {'targetInfo':dict(type='page',targetId='foreign' if self.mismatch or (self.retire and self.targets>1) else 'owned')}
        if cmd=='Browser.getWindowForTarget':return dict(windowId=7,bounds=dict(windowState='normal'))
        return {}
    def script(self,script,*,deadline=None):
        self.calls.append(('script',script,None,deadline))
        return next(self.samples,False)


class NativeFailureWire:
    session=None
    def __init__(self):self.calls=[]
    def scoped(self,path):return '/session/owned/'+path
    def value(self,method,path,body=None,**kwargs):
        self.calls.append(path)
        if path=='/session':return dict(sessionId='owned',capabilities=dict(browserName='oryn',acceptInsecureCerts=False))
        if path.endswith('/window'):return 'owned-window'
        return None
    def command(self,method,path,body=None):
        self.calls.append(path)
        return dict(status=200 if body['url']=='https://test/trusted' else 500,
                    response={'value':None if body['url']=='https://test/trusted' else {'error':'insecure certificate'}})
    def script(self,script):
        if script=='return document.title':return MARKER
        if script=='return document.hasFocus()':raise ValueError('document.hasFocus unavailable')
        return None


def mock_connection():
    return patch('wpt_native_controls.http.client.HTTPConnection')


class ControlProgressTests(unittest.TestCase):
    def test_expired_wire_deadline_opens_no_connection(self):
        from wpt_native_controls import Wire
        with mock_connection() as connection, patch('wpt_native_controls.time.monotonic', return_value=10):
            with self.assertRaises(TimeoutError): Wire(1).command('GET','/status',deadline=9)
        connection.assert_not_called()

    def test_wire_uses_remaining_budget_and_closes_on_read_timeout(self):
        from wpt_native_controls import Wire
        with mock_connection() as connection, patch('wpt_native_controls.time.monotonic', return_value=10):
            connection.return_value.getresponse.return_value.read1.side_effect=TimeoutError('read deadline')
            with self.assertRaises(TimeoutError): Wire(1).command('GET','/status',deadline=12)
        self.assertEqual(connection.call_args.kwargs['timeout'],2)
        connection.return_value.sock.settimeout.assert_called_with(2)
        connection.return_value.close.assert_called_once()
        connection.return_value.getresponse.return_value.close.assert_called_once()

    def test_close_delimited_response_keeps_deadline_on_retained_socket(self):
        from wpt_native_controls import Wire
        transport=Mock();response=Mock(status=200)
        response.read1.side_effect=[b'{"value":null}',b'']
        response.isclosed.return_value=False
        with mock_connection() as factory, patch('wpt_native_controls.time.monotonic', return_value=10):
            connection=factory.return_value;connection.sock=transport
            def headers():
                connection.sock=None  # HTTPConnection detaches a close-delimited response.
                return response
            connection.getresponse.side_effect=headers
            self.assertEqual(Wire(1).command('GET','/status',deadline=12)['response'],{'value':None})
        self.assertGreaterEqual(transport.settimeout.call_count,3)
        transport.settimeout.assert_called_with(2)
        connection.close.assert_called_once()

    def test_known_length_closed_response_does_not_reuse_closed_transport(self):
        from wpt_native_controls import Wire
        transport=Mock();response=Mock(status=200);closed=[False]
        def timeout(value):
            if closed[0]: raise OSError('bad file descriptor')
        transport.settimeout.side_effect=timeout
        def body(limit):
            closed[0]=True
            return b'{"value":null}'
        response.read1.side_effect=body
        response.isclosed.return_value=True
        with mock_connection() as factory, patch('wpt_native_controls.time.monotonic', return_value=10):
            connection=factory.return_value;connection.sock=transport
            def headers():
                connection.sock=None
                return response
            connection.getresponse.side_effect=headers
            self.assertEqual(Wire(1).command('GET','/status',deadline=12)['response'],{'value':None})
        response.read1.assert_called_once()
        response.close.assert_called_once()
        connection.close.assert_called_once()

    def test_focus_exception_retains_prior_tls_and_marks_faults_not_run(self):
        wire=NativeFailureWire();results=new_controls(True)
        contract={'tls':{kind:{'url':'https://test/'+('trusted' if kind=='trusted' else 'bad-'+kind)} for kind in TLS_KINDS}}
        with self.assertRaisesRegex(ValueError,'hasFocus'):
            browser_controls(wire,contract,{},True,results=results)
        self.assertEqual(results['trusted_title'],MARKER)
        self.assertEqual(results['control_execution']['tls_trusted']['status'],'completed')
        self.assertEqual(results['control_execution']['focus']['status'],'failed')
        self.assertEqual(results['control_execution']['worker_crash']['status'],'not_run')
        record=accepted_fixture();record['native']['controls']=results;record['native']['errors']=['hasFocus unavailable']
        errors=validate_results(record)
        self.assertFalse(any('trusted navigation failed' in e for e in errors))
        self.assertTrue(any('worker_crash not run' in e for e in errors))
        self.assertFalse(any('was not rejected' in e for e in errors))

    def test_unstarted_inventory_cannot_qualify(self):
        record=accepted_fixture();record['reference']['controls']=new_controls(False)
        errors=validate_results(record)
        self.assertTrue(any('session not run' in e for e in errors))
        self.assertTrue(any('tls_trusted not run' in e for e in errors))

    def test_completed_focus_false_remains_actual_failure(self):
        record=accepted_fixture();record['reference']['controls'].update(initial_focus=False,restored_focus=False)
        self.assertTrue(any('foreground/background' in e for e in validate_results(record)))

    def test_generic_certificate_message_cannot_qualify_rejection(self):
        record=accepted_fixture()
        record['native']['controls']['tls_untrusted']['response']['value']={
            'error':'unknown error','message':'certificate command timed out'}
        self.assertTrue(any('certificate rejection' in e for e in validate_results(record)))

    def test_activation_matches_native_window_and_observes_real_focus(self):
        wire=FocusWire((False,False,True));results={}
        with patch('wpt_native_controls.time.sleep'):
            activate_reference_window(wire,results,'initial')
        commands=[c[2]['cmd'] for c in wire.calls if isinstance(c[2],dict)]
        self.assertLess(commands.index('Target.activateTarget'),commands.index('Page.bringToFront'))
        self.assertEqual(results['focus_activation'][0]['samples'],[False,True])
        self.assertTrue(all(c[3] is not None for c in wire.calls))

    def test_native_activation_accepts_exact_raw_and_legacy_handles(self):
        for handle in ('owned', 'CDwindow-owned'):
            with self.subTest(handle=handle):
                wire=FocusWire((False,True),handle=handle);results={}
                activate_reference_window(wire,results,'initial')
                observation=results['focus_activation'][0]
                self.assertEqual(observation['handle'],handle)
                self.assertEqual(observation['target_after']['targetId'],'owned')
                self.assertEqual(observation['samples'],[True])
                activation=[c[2] for c in wire.calls if isinstance(c[2],dict) and c[2]['cmd']=='Target.activateTarget']
                self.assertEqual(activation,[{'cmd':'Target.activateTarget','params':{'targetId':'owned'}}])

    def test_native_activation_rejects_nonmatching_handle_formats(self):
        for handle in ('foreign','CDwindow-foreign','CDwindow-CDwindow-owned','',None,17):
            with self.subTest(handle=handle):
                wire=FocusWire(handle=handle)
                with self.assertRaisesRegex(ValueError,'selected window'):
                    activate_reference_window(wire,{},'initial')
                self.assertFalse(any(isinstance(c[2],dict) and c[2]['cmd']=='Target.activateTarget' for c in wire.calls))

    def test_foreign_target_is_not_activated(self):
        wire=FocusWire(mismatch=True)
        with self.assertRaisesRegex(ValueError,'selected window'):
            activate_reference_window(wire,{},'initial')
        self.assertFalse(any(isinstance(c[2],dict) and c[2]['cmd']=='Target.activateTarget' for c in wire.calls))

    def test_retired_target_after_observation_is_rejected(self):
        wire=FocusWire((False,True),retire=True)
        with self.assertRaisesRegex(ValueError,'changed during activation'):
            activate_reference_window(wire,{},'initial')

    def test_never_focused_remains_bounded_failure_with_partial_samples(self):
        wire=FocusWire((False,False));results={}
        with patch('wpt_native_controls.time.sleep'),patch('wpt_native_controls.time.monotonic',side_effect=[0,0,3]):
            with self.assertRaises(TimeoutError):activate_reference_window(wire,results,'initial')
        self.assertEqual(results['focus_activation'][0]['samples'],[False])
        self.assertNotIn('restored_focus',results)


if __name__=='__main__':unittest.main()

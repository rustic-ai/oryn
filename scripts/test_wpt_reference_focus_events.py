"""Reference collector regression sources; real browser validation is separate."""
import copy
import unittest
from unittest.mock import patch
from wpt_native_controls import (REFERENCE_FOCUS_START, REFERENCE_FOCUS_STOP,
    reference_focus_samples, reference_focus_transition, validate_results, assert_reference_tab_topology)
from test_wpt_native_control_progress import accepted_fixture

URL = 'https://web-platform.test:8443/native-prerequisite'

def activation(phase, handle):
    return dict(phase=phase, handle=handle, target_after={'targetId':handle}, window_after={'windowId':7})

def activate(wire, results, phase):
    results.setdefault('focus_activation', []).append(activation(phase, 'other' if phase=='other' else 'first'))


def observation(blur=False):
    return dict(document_url=URL, overflow=0, rows=[
        dict(phase='installed', at_ms=0, focused=True),
        dict(phase='blur', at_ms=1, focused=blur),
        dict(phase='focus', at_ms=2, focused=True)])

class Wire:
    def __init__(self, value, fail=False, cleanup_fail=False):
        self.observation=value; self.calls=[]; self.fail=fail; self.cleanup_fail=cleanup_fail
    def scoped(self,path): return '/session/owned/'+path
    def value(self,method,path,body=None,**kwargs):
        self.calls.append((path,body,kwargs))
        if path.endswith('/window/new'):return {'handle':'other'}
        if path.endswith('/execute/async') and self.fail:raise ValueError('primary observation failure')
        if path.endswith('/window') and body=={'handle':'first'} and self.cleanup_fail:raise OSError('cleanup switch failed')
    def script(self,script,**kwargs):
        self.calls.append(('script',script,kwargs))
        if script==REFERENCE_FOCUS_STOP:return self.observation
        if script=='return document.hasFocus()':return True

class ReferenceFocusEventTests(unittest.TestCase):
    def results(self):return dict(tls_urls={'trusted':URL}, initial_focus=True, focus_activation=[activation('initial','first')])
    def test_blur_event_real_false_counts_when_timer_has_no_background_sample(self):
        wire=Wire(observation());results=self.results()
        with patch('wpt_native_controls.activate_reference_window', side_effect=activate) as native_activate:
            reference_focus_transition(wire,results,'first')
        self.assertEqual(results['focus_samples'],[True,False,True])
        self.assertFalse(results['denied_focus'])
        self.assertEqual([c[1] for c in wire.calls if c[0].endswith('/window/new')],[{'type':'tab'}])
        self.assertEqual(results['focus_topology']['native_window_id'],7)
        self.assertEqual([c.args[2] for c in native_activate.call_args_list],['other','restored'])
        self.assertEqual([c[1] for c in wire.calls if c[0].endswith('/execute/async')],
            [{'script':'const done=arguments[arguments.length-1];setTimeout(()=>done(null),200)','args':[]}])
        self.assertEqual(sum(c[1]==REFERENCE_FOCUS_STOP for c in wire.calls),1)
    def test_blur_label_never_manufactures_false_focus(self):
        results=self.results()
        with patch('wpt_native_controls.activate_reference_window', side_effect=activate):
            reference_focus_transition(Wire(observation(blur=True)),results,'first')
        self.assertTrue(results['denied_focus'])
        record=accepted_fixture();record['reference']['controls'].update(results)
        self.assertTrue(any('foreground/background' in e for e in validate_results(record)))
    def test_malformed_overflow_or_reordered_samples_are_rejected(self):
        values=[]
        for mutate in (lambda v:v.update(overflow=1),lambda v:v['rows'][1].update(focused=0),
                       lambda v:v['rows'][1].update(at_ms=-1),lambda v:v['rows'][1].update(at_ms=float('nan')),
                       lambda v:v['rows'][0].update(phase='blur')):
            value=observation();mutate(value);values.append(value)
        for value in values:
            with self.subTest(value=value),self.assertRaises(ValueError):reference_focus_samples(value)
    def test_primary_failure_cleans_original_observer_with_bounded_requests(self):
        wire=Wire(observation(),fail=True);results=self.results()
        with patch('wpt_native_controls.activate_reference_window', side_effect=activate),self.assertRaisesRegex(ValueError,'primary observation'):
            reference_focus_transition(wire,results,'first')
        self.assertTrue(results['reference_focus_cleanup']['complete'])
        self.assertEqual(results['reference_focus_observation'],observation())
        self.assertEqual(wire.calls[-2][1],{'handle':'first'})
        self.assertIn('deadline',wire.calls[-2][2]);self.assertIn('deadline',wire.calls[-1][2])
    def test_cleanup_failure_does_not_replace_primary_or_claim_success(self):
        wire=Wire(observation(),fail=True,cleanup_fail=True);results=self.results()
        with patch('wpt_native_controls.activate_reference_window', side_effect=activate),self.assertRaisesRegex(ValueError,'primary observation'):
            reference_focus_transition(wire,results,'first')
        self.assertFalse(results['reference_focus_cleanup']['complete'])
        self.assertIn('cleanup switch failed',results['reference_focus_cleanup']['error'])
        self.assertNotIn('focus_samples',results)
    def test_changed_document_cannot_supply_focus_evidence(self):
        value=observation();value['document_url']='https://foreign.invalid/'
        with patch('wpt_native_controls.activate_reference_window', side_effect=activate),self.assertRaisesRegex(ValueError,'document changed'):
            reference_focus_transition(Wire(value),self.results(),'first')

    def test_tab_topology_requires_distinct_target_ids_in_same_native_window(self):
        results=self.results();results['focus_windows']={'first':'first','second':'other'}
        results['focus_activation'].append(activation('other','other'))
        assert_reference_tab_topology(results)
        for field, value in [('target_after', {'targetId':'first'}),('window_after',{'windowId':8}),
                             ('window_after',{'windowId':True}),('handle','foreign')]:
            bad=copy.deepcopy(results);bad['focus_activation'][1][field]=value
            with self.subTest(field=field,value=value),self.assertRaises(ValueError):assert_reference_tab_topology(bad)
    def test_missing_or_duplicate_activation_cannot_supply_tab_authority(self):
        results=self.results();results['focus_windows']={'first':'first','second':'other'}
        with self.assertRaises(ValueError):assert_reference_tab_topology(results)
        results['focus_activation'] += [activation('other','other'),activation('other','other')]
        with self.assertRaises(ValueError):assert_reference_tab_topology(results)

if __name__=='__main__':unittest.main()

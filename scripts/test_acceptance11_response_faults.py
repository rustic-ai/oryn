"""New response-entry, evidence and receipt-route regressions; no native build needed."""
import copy
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest import mock

import workflow_native_adapter as adapter
import workflow_raw_evidence as raw

FIXTURES=adapter.ROOT/'test-harness/workflows'
sys.path.insert(0,str(FIXTURES))
from workflow_faults import ResponseFaults
import workflow_fixture


class ResponseEntryTests(unittest.TestCase):
    def gate(self,root):
        return dict(token='a'*32,route='receipt',marker=str(root/'response-entered.json'))

    def test_marker_only_after_exact_route_status_and_ordinal_are_claimed(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();faults=ResponseFaults(root/'db');gate=self.gate(root)
            faults.arm('case',gate['token'],'receipt',1,200,1,response_gate=gate)
            for route,status in [('other',200),('receipt',403),('receipt',200)]:
                faults.before_response('case',route,status)
                self.assertFalse(Path(gate['marker']).exists())
                self.assertEqual(faults.snapshot('case')[0]['terminal'],'armed')
            faults.before_response('case','receipt',200)
            marker=json.loads(Path(gate['marker']).read_text());row=faults.snapshot('case')[0]
            self.assertTrue(adapter.valid_response_marker(gate,marker,row))
            self.assertEqual(row['terminal'],'released')
            original=Path(gate['marker']).read_bytes()
            faults.before_response('case','receipt',200)
            self.assertEqual(Path(gate['marker']).read_bytes(),original)
            self.assertEqual(len(faults.snapshot('case')),1)

    def test_marker_published_after_durable_entry_before_response_wait(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();faults=ResponseFaults(root/'db');gate=self.gate(root)
            faults.arm('case',gate['token'],'receipt',300,200,response_gate=gate)
            observations=[]
            class Wait:
                def wait(self,seconds):
                    row=faults.snapshot('case')[0]
                    observations.append((seconds,row['terminal'],json.loads(Path(gate['marker']).read_text())['started_ns']==row['started_ns']))
            faults.pending['case'][3]=Wait()
            faults.before_response('case','receipt',200)
            self.assertEqual(observations,[(.3,'withholding',True)])

    def test_foreign_or_reused_marker_cannot_arm(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();faults=ResponseFaults(root/'db');gate=self.gate(root)
            with self.assertRaises(ValueError):faults.arm('case','b'*32,'receipt',1,response_gate=gate)
            Path(gate['marker']).write_text('{}')
            with self.assertRaises(ValueError):faults.arm('case',gate['token'],'receipt',1,response_gate=gate)

    def test_native_entry_and_saved_ledger_marker_must_all_agree(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();faults=ResponseFaults(root/'db');gate=self.gate(root)
            faults.arm('case',gate['token'],'receipt',1,200,response_gate=gate);faults.before_response('case','receipt',200)
            marker=json.loads(Path(gate['marker']).read_text())
            case=SimpleNamespace(database=root/'db',identity='case',fault={'kind':'cancellation'},response_gate=gate)
            original=dict(fault_evidence=dict(injected=True,native=dict(interrupted_inflight=True,reaped=True),response_entered=True,response_gate=gate,response_marker=marker),recovery=[{'index':0}])
            self.assertIs(adapter.read_fault_evidence(case,original)['injected'],True)
            for change in ('token','started_ns','native_witness'):
                session=copy.deepcopy(original);saved=copy.deepcopy(marker)
                if change=='token':saved['token']='b'*32
                elif change=='started_ns':saved['started_ns']=True
                else:session['fault_evidence']['response_entered']=False
                self.assertIs(adapter.read_fault_evidence(case,session,response_marker=saved)['injected'],False)
            self.assertIs(adapter.read_fault_evidence(case,original,response_marker=None)['injected'],False)

    def test_raw_marker_cannot_be_borrowed_from_another_case(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();claim={'database':{'path':'artifacts/case/oracle.sqlite3'},'response_marker':{'path':'artifacts/case/response-entered.json','sha256':'a'*64}}
            gate={'token':'a'*32,'route':'receipt','marker':str(root/'artifacts/case/response-entered.json')}
            case=SimpleNamespace(fault={'kind':'cancellation'},response_route='receipt')
            artifact=mock.Mock(return_value=b'{"schema_version":1}')
            self.assertEqual(raw.response_marker_bytes(root,claim,{'response_gate':gate},case,artifact),{'schema_version':1})
            for field in ('path','route','marker'):
                c=copy.deepcopy(claim);g=copy.deepcopy(gate)
                if field=='path':c['response_marker']['path']='artifacts/other/response-entered.json'
                elif field=='marker':g['marker']=str(root/'artifacts/other/response-entered.json')
                else:g['route']='other'
                with self.assertRaises(ValueError):raw.response_marker_bytes(root,c,{'response_gate':g},case,artifact)


class FaultVerdictTests(unittest.TestCase):
    def test_real_injection_with_failed_recovery_keeps_failure_without_missing_injection(self):
        with tempfile.TemporaryDirectory() as name:
            directory=Path(name)/'artifacts'/'original';directory.mkdir(parents=True)
            runner=adapter.MixedCellAdapter('forms.validation',Path('/unused'),Path('/unused'))
            native=dict(injected=True,native=dict(interrupted_inflight=True,reaped=True))
            supplement=dict(phase='before_commit',fault='worker_failure',injected=adapter.actual_commit_injection(native),
                recovery_passed=False,errors=['commit inspection failed: malformed command'],
                resource_accounting={'complete':True,'errors':[]},process_cleanup={'group_settled':True,'errors':[]},raw_wave={})
            attempt=dict(cell='forms.validation',fault='worker_failure',attempt_id='original',context_count=1,mode='fault_injection')
            with mock.patch.object(runner,'recipes',return_value=[]),mock.patch.object(adapter,'run_commit_supplement',return_value=[supplement]):
                result=runner(attempt,directory,threading.Event())
            self.assertFalse(result['passed']);self.assertFalse(result['functional_passed'])
            self.assertTrue(any('malformed command' in error for error in result['errors']))
            self.assertNotIn('required actual fault injection missing',result['errors'])
            evidence=json.loads((directory/'fault-evidence.json').read_text())
            self.assertTrue(evidence['injected']);self.assertFalse(evidence['records'][0]['recovery_passed'])


class FormsReceiptRouteTests(unittest.TestCase):
    def test_v4_real_receipt_handler_enters_hold_only_after_cookie_authority(self):
        forms=adapter.load('acceptance11_forms_v4',FIXTURES/'forms_fixture_v4.py')
        with tempfile.TemporaryDirectory() as name:
            root=Path(name).resolve();wrapped=workflow_fixture.instrument(forms.Fixture,root/'db','forms');fixture=wrapped(root/'db')
            identity='c'*32;fixture.command(dict(command='create',case_id=identity,scenario='reentrant'))
            marker=root/'response-entered.json';gate=dict(token='a'*32,marker=str(marker),route='receipt')
            fixture.command(dict(command='arm_response_fault',case_id=identity,token='a'*32,route='receipt',delay_ms=1,expected_status=200,response_gate=gate))
            handlers=[]
            def server(address,handler):
                handlers.append(handler);return mock.Mock(server_port=1234)
            with mock.patch.object(forms,'ThreadingHTTPServer',side_effect=server),mock.patch.object(forms.sys,'stdin',io.StringIO('')),mock.patch.object(forms.sys,'stdout',io.StringIO()):
                forms.serve(root/'db',fixture_type=lambda _:fixture)
            handler=handlers[0].__new__(handlers[0]);handler.path='/advanced/'+identity+'/receipt';handler.command='GET';handler.reply=mock.Mock()
            handler.headers={'Cookie':'form_session=wrong'};handler.handle_request()
            self.assertEqual(handler.reply.call_args.args[0],403);self.assertFalse(marker.exists())
            self.assertEqual(fixture.workflow_response_faults.snapshot(identity)[0]['terminal'],'armed')
            handler.headers={'Cookie':'form_session='+fixture.row(identity)['session']};handler.handle_request()
            self.assertEqual(handler.reply.call_args.args[0],200);self.assertTrue(marker.exists())
            self.assertEqual(fixture.workflow_response_faults.snapshot(identity)[0]['terminal'],'released')


if __name__=='__main__':unittest.main()

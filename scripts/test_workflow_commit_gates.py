"""Source regressions for actual pre/post mutation gates (execution deferred)."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
import unittest

HERE=Path(__file__).resolve().parents[1]/'test-harness'/'workflows'
sys.path.insert(0,str(HERE))
from workflow_faults import CommitGates
from workflow_fixture import instrument


class Backend:
    def __init__(self):self.effects=[]
    def command(self,message):return dict(orders=[body.decode() for body in self.effects])
    def post(self,case,route,body,cookie):
        self.effects.append(body)
        return 303,'',[('Location','/receipt')]


class CommitGateTests(unittest.TestCase):
    def test_no_matching_request_never_publishes_marker(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);gate=CommitGates(root/'db')
            gate.arm('case','token','save','before_commit',root/'marker',root/'release')
            self.assertIsNone(gate.claim('case','read',b'body'))
            self.assertFalse((root/'marker').exists())
            self.assertEqual(gate.snapshot('case')[0]['terminal'],'armed')

    def exercise(self,phase,release_valid=True):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=instrument(Backend,root/'db','shopping')()
            fixture.command(dict(command='arm_commit_gate',case_id='case',token='token',route='save',phase=phase,marker=str(root/'marker'),release=str(root/'release'),timeout_ms=1000))
            result=[];worker=threading.Thread(target=lambda:result.append(fixture.post('case','save',b'original=body','private-cookie')))
            worker.start()
            deadline=time.monotonic()+2
            while not (root/'marker').exists() and time.monotonic()<deadline:time.sleep(.001)
            self.assertTrue((root/'marker').exists())
            self.assertEqual(fixture.effects,[] if phase=='before_commit' else [b'original=body'])
            release=dict(schema_version=1,token='token' if release_valid else 'foreign',phase=phase,worker_reaped=True,interrupted_inflight=True)
            temporary=root/'release.part';temporary.write_text(json.dumps(release));temporary.rename(root/'release')
            worker.join(2);self.assertFalse(worker.is_alive())
            ledger=fixture.workflow_commit_gates.snapshot('case')[0]
            self.assertNotIn('cookie',ledger);self.assertNotIn('body',ledger)
            self.assertIsNotNone(ledger['started_ns']);self.assertGreaterEqual(ledger['released_ns'],ledger['started_ns'])
            if not release_valid:self.assertEqual(ledger['terminal'],'invalid_release')
            else:self.assertEqual(ledger['terminal'],'aborted_before_commit' if phase=='before_commit' else 'released_after_commit')
            self.assertEqual(result[0][0],499 if phase=='before_commit' else 303)
            self.assertEqual(fixture.effects,[] if phase=='before_commit' else [b'original=body'])
            # One-shot consumption: a deliberate original-body retry is the only
            # operation allowed to produce an effect after a precommit abort.
            if phase=='before_commit':
                fixture.post('case','save',b'original=body','private-cookie')
                self.assertEqual(fixture.effects,[b'original=body'])

    def test_before_commit_aborts_original_before_retry(self):self.exercise('before_commit')
    def test_after_commit_keeps_original_effect(self):self.exercise('after_commit')
    def test_foreign_release_cannot_authorize_success(self):self.exercise('before_commit',False)

    def test_timeout_never_invokes_original_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=instrument(Backend,root/'db','shopping')()
            fixture.command(dict(command='arm_commit_gate',case_id='case',token='token',route='save',phase='before_commit',marker=str(root/'marker'),release=str(root/'release'),timeout_ms=1))
            self.assertEqual(fixture.post('case','save',b'x','cookie')[0],499)
            self.assertEqual(fixture.effects,[])
            self.assertEqual(fixture.workflow_commit_gates.snapshot('case')[0]['terminal'],'deadline_aborted')

    def test_ordinal_and_exact_request_bytes_are_persisted_privately(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);gates=CommitGates(root/'db')
            gates.arm('case','token','save','before_commit',root/'marker',root/'release',skip_matches=1)
            self.assertIsNone(gates.claim('case','save',b'first'))
            self.assertIsNotNone(gates.claim('case','save',b'a=1&a=2','cookie'))
            self.assertIsNone(gates.claim('case','save',b'third'))
            with sqlite3.connect(root/'db') as db:
                self.assertEqual(db.execute('SELECT body,cookie FROM workflow_commit_gates').fetchone(),(b'a=1&a=2','cookie'))



class CommitRecoveryTests(unittest.TestCase):
    def prepare(self,root,phase):
        fixture=instrument(Backend,root/'db','shopping')()
        fixture.workflow_commit_gates.arm('case','token','save',phase,root/'marker',root/'release',timeout_ms=1000)
        worker=threading.Thread(target=lambda:fixture.post('case','save',b'original=body','cookie'))
        worker.start();deadline=time.monotonic()+2
        while not (root/'marker').exists() and time.monotonic()<deadline:time.sleep(.001)
        self.assertTrue((root/'marker').exists())
        temporary=root/'ready';temporary.write_text(json.dumps(dict(schema_version=1,token='token',phase=phase,worker_reaped=True,interrupted_inflight=True)));temporary.rename(root/'release')
        worker.join(2);self.assertFalse(worker.is_alive())
        return fixture

    def test_before_commit_inspection_exposes_original_body_only_to_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=self.prepare(root,'before_commit')
            self.assertEqual(fixture.workflow_outcome('case','token','foreign')[0],403)
            self.assertEqual(fixture.workflow_outcome('case','foreign','cookie')[0],403)
            response=fixture.workflow_outcome('case','token','cookie')
            self.assertEqual(response[0],200);self.assertIn('not_committed',response[1])
            self.assertIn('Retry pending request',response[1]);self.assertIn('name="original" value="body"',response[1])
            self.assertEqual(fixture.post('case','save',b'original=changed','cookie')[0],409)
            self.assertEqual(fixture.effects,[])

    def test_after_commit_inspection_does_not_offer_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=self.prepare(root,'after_commit')
            response=fixture.workflow_outcome('case','token','cookie')
            self.assertEqual(response[0],200);self.assertIn('>committed<',response[1])
            self.assertNotIn('<form',response[1]);self.assertIn('Continue committed request',response[1])
            self.assertEqual(fixture.effects,[b'original=body'])

    def test_changed_durable_state_rejects_recovery_decision(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);fixture=self.prepare(root,'after_commit')
            fixture.effects.append(b'concurrent=edit')
            self.assertEqual(fixture.workflow_outcome('case','token','cookie')[0],409)

if __name__=='__main__':unittest.main()

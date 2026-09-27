"""Bounded cell concurrency, ordered repetitions and cooperative cancellation."""
import json
import os
from pathlib import Path
import signal
import tempfile
import threading
import unittest
from workflow_execution import run_recovery_lanes, cancellation_signals, execute_contract
from workflow_contract import DEFAULT_CONTRACT, planned_attempts


class RecoveryParallelTests(unittest.TestCase):
    def test_four_cells_overlap_without_overlapping_repetitions(self):
        attempts=[{'cell':str(cell),'sample':sample} for sample in range(3) for cell in range(8)]
        barrier=threading.Barrier(4);lock=threading.Lock();active=set();order={};peak=[]
        def perform(attempt):
            cell=attempt['cell']
            with lock:
                self.assertNotIn(cell,active);active.add(cell);peak.append(len(active))
                order.setdefault(cell,[]).append(attempt['sample'])
            barrier.wait(timeout=5)
            with lock:active.remove(cell)
        run_recovery_lanes(attempts,perform,workers=4,stop=threading.Event())
        self.assertEqual(max(peak),4)
        self.assertEqual(order,{str(cell):[0,1,2] for cell in range(8)})

    def test_one_lane_preserves_cell_repetition_order(self):
        seen=[];attempts=[{'cell':c,'sample':n} for n in range(2) for c in ['a','b']]
        run_recovery_lanes(attempts,lambda a:seen.append((a['cell'],a['sample'])),workers=1,stop=threading.Event())
        self.assertEqual(seen,[('a',0),('a',1),('b',0),('b',1)])

    def test_worker_failure_cancels_and_joins_active_peer(self):
        stop=threading.Event();barrier=threading.Barrier(2);cleaned=threading.Event();seen=[]
        def perform(attempt):
            seen.append((attempt['cell'],attempt['sample']))
            barrier.wait(timeout=5)
            if attempt['cell']=='a':raise OSError('journal failed')
            self.assertTrue(stop.wait(5));cleaned.set()
        attempts=[{'cell':c,'sample':n} for n in range(2) for c in ['a','b']]
        with self.assertRaisesRegex(OSError,'journal failed'):
            run_recovery_lanes(attempts,perform,workers=2,stop=stop)
        self.assertTrue(cleaned.is_set());self.assertTrue(stop.is_set())
        self.assertTrue(all(sample==0 for _,sample in seen))

    def test_product_failure_does_not_drop_repetitions(self):
        seen=[];attempts=[{'cell':'a','sample':n} for n in range(30)]
        def fail(attempt):seen.append(attempt['sample']);return {'passed':False}
        run_recovery_lanes(attempts,fail,stop=threading.Event())
        self.assertEqual(seen,list(range(30)))

    def test_invalid_lane_count_precedes_dispatch(self):
        for workers in [0,5,True,1.5]:
            with self.subTest(workers=workers),self.assertRaises(ValueError):
                run_recovery_lanes([],lambda _:self.fail('dispatched'),workers=workers,stop=threading.Event())

    def test_signal_sets_stop_and_restores_prior_handlers(self):
        stop=threading.Event();before={s:signal.getsignal(s) for s in (signal.SIGTERM,signal.SIGINT)}
        with cancellation_signals(stop):
            os.kill(os.getpid(),signal.SIGTERM)
            self.assertTrue(stop.is_set())
            os.kill(os.getpid(),signal.SIGINT)
        self.assertEqual(before,{s:signal.getsignal(s) for s in before})

    def test_cancellation_preserves_all_688_originals_and_no_further_dispatch(self):
        contract=json.loads(DEFAULT_CONTRACT.read_text());plan=planned_attempts(contract)
        stop=threading.Event();seen=[]
        def cancel(attempt,directory,event):
            seen.append(attempt['attempt_id']);event.set()
            return {'passed':False,'errors':['cancelled during native cleanup']}
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'raw'
            summary=execute_contract(contract,{'parent_sha256':'a'*64,'worker_sha256':'b'*64,'source_tree_sha256':'c'*64},
                {plan[0]['cell']:cancel},output,{},stop=stop)
            rows=[json.loads(s) for s in (output/'attempts.jsonl').read_text().splitlines()]
        self.assertEqual(len(seen),1)
        self.assertEqual(len(rows),688)
        self.assertEqual({r['attempt_id'] for r in rows},{r['attempt_id'] for r in plan})
        self.assertFalse(summary['structurally_valid'])
        self.assertTrue(summary['infrastructure_errors'])

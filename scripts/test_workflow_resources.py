import unittest
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from workflow_resources import parse_processes,descendants,reconcile_resources,percentile

class ResourceTests(unittest.TestCase):
    def test_tree_retains_descendant_processes_without_name_guessing(self):
        rows=parse_processes('1 0 5\n2 1 7\n3 2 11\n4 0 13\n')
        self.assertEqual([row['pid'] for row in descendants(rows,1)],[1,2,3])
        self.assertEqual(rows[3]['rss_bytes'],11*1024)
    def test_invalid_or_duplicate_process_rows_fail(self):
        for text in ('1 0 -2','1 0 2\n1 0 3','not a process','1 2'):
            with self.assertRaises(ValueError):parse_processes(text)
    def test_aggregate_uses_simultaneous_samples_not_sum_of_peaks(self):
        native=dict(parent_pid=1,sessions=[dict(context_id=10,resource_audit=dict(complete=True,records=[dict(worker_pid=2,page_id=20),dict(worker_pid=3,page_id=21)]))])
        raw=dict(schema_version=1,parent_pid=1,samples=[dict(monotonic_ns=1,processes=[dict(pid=1,rss_bytes=10),dict(pid=2,rss_bytes=100),dict(pid=3,rss_bytes=20)]),dict(monotonic_ns=2,processes=[dict(pid=1,rss_bytes=10),dict(pid=2,rss_bytes=20),dict(pid=3,rss_bytes=100)])])
        result=reconcile_resources(raw,native)
        self.assertTrue(result['complete']);self.assertEqual(result['aggregate_sampled_peak_bytes'],130)
        self.assertEqual(sum(row['sampled_peak_rss_bytes'] for row in result['workers']),200)
    def test_unobserved_native_worker_is_missing_evidence(self):
        raw=dict(schema_version=1,parent_pid=1,samples=[dict(monotonic_ns=1,processes=[dict(pid=1,rss_bytes=10)])])
        native=dict(parent_pid=1,sessions=[dict(context_id=1,resource_audit=dict(complete=True,records=[dict(worker_pid=9,page_id=1)]))])
        self.assertFalse(reconcile_resources(raw,native)['complete'])
    def test_unknown_descendant_is_not_only_informational_metadata(self):
        raw=dict(schema_version=1,parent_pid=1,samples=[dict(monotonic_ns=1,processes=[
            dict(pid=1,rss_bytes=10),dict(pid=2,rss_bytes=20),dict(pid=3,rss_bytes=30)])])
        native=dict(parent_pid=1,sessions=[dict(context_id=1,resource_audit=dict(
            complete=True,records=[dict(worker_pid=2,page_id=1)]))])
        result=reconcile_resources(raw,native)
        self.assertFalse(result['complete'])
        self.assertEqual(result['aggregate_sampled_peak_bytes'],60)
        self.assertEqual(result['instantaneous_groups'][0]['unknown_descendants'],[3])
    def test_percentile_rejects_nonfinite_or_absent_data(self):
        for values in ([],[float('nan')],[float('inf')],[-1],[True]):
            with self.assertRaises(ValueError):percentile(values,.95)

class LifecycleResourceTests(unittest.TestCase):
    def test_q_recomputes_accounting_from_pinned_native_and_os_records(self):
        path=Path(__file__).with_name('qualify-workflow-q.py')
        spec=importlib.util.spec_from_file_location('q_resource_test',path)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        raw,native=self.evidence()
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            def save(name,value):
                data=json.dumps(value).encode();(root/name).write_bytes(data)
                return dict(path=name,sha256=hashlib.sha256(data).hexdigest())
            accounting=reconcile_resources(raw,native)
            wave=dict(accounting=save('accounting.json',accounting),
                      samples=save('samples.json',raw),native_evidence=save('native.json',native))
            self.assertEqual(module.resource_accounting(root,wave),accounting)
            # Even freshly pinned altered native evidence cannot retain the
            # previous successful derived accounting result.
            native['worker_lifecycle']['processes'][0]['leases'][0]['ended_unix_ns']=101
            wave['native_evidence']=save('native.json',native)
            with self.assertRaisesRegex(ValueError,'differs from pinned'):
                module.resource_accounting(root,wave)

    def test_q_profile_accounting_is_recomputed_not_accepted_from_summary(self):
        spec=importlib.util.spec_from_file_location('q_profile_test',Path(__file__).with_name('qualify-workflow-q.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        samples,native=self.evidence();samples.update(errors=[],overflow=False)
        raw=dict(runtime=native['runtime'],process_lifecycle=native['worker_lifecycle'])
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            def save(name,value):
                data=json.dumps(value).encode();(root/name).write_bytes(data)
                return dict(path=name,sha256=hashlib.sha256(data).hexdigest())
            row={'artifacts':{'resource-samples.json':save('samples.json',samples),
                'resource-accounting.json':save('accounting.json',reconcile_resources(samples,native))}}
            module.profile_accounting(root,row,raw)
            samples['samples'][0]['processes'][1]['rss_bytes']=999
            row['artifacts']['resource-samples.json']=save('samples.json',samples)
            with self.assertRaisesRegex(ValueError,'differs from raw'):
                module.profile_accounting(root,row,raw)

    def test_malformed_lifecycle_data_fails_without_attribution_or_exception(self):
        for field,value in [('launch_id',[]),('worker_pid',True),('reaped_unix_ns','100'),
                            ('exit_code',False),('signal',0),('leases',None)]:
            with self.subTest(field=field,value=value):
                raw,native=self.evidence()
                native['worker_lifecycle']['processes'][0][field]=value
                result=reconcile_resources(raw,native)
                self.assertFalse(result['complete'])
                self.assertEqual(result['workers'],[])
        for value in (None,[],{'processes':None},{'processes':[None]}):
            raw,native=self.evidence();native['worker_lifecycle']=value
            self.assertFalse(reconcile_resources(raw,native)['complete'])

    def test_lease_interval_must_be_contained_by_original_process_lifetime(self):
        for start,end in [(9,100),(10,101),(50,49),(True,100),(10,None)]:
            with self.subTest(start=start,end=end):
                raw,native=self.evidence()
                native['worker_lifecycle']['processes'][0]['leases'][0].update(
                    started_unix_ns=start,ended_unix_ns=end)
                self.assertFalse(reconcile_resources(raw,native)['complete'])

    def test_malformed_os_measurements_cannot_enter_aggregate(self):
        for field,value in [('rss_bytes',-1),('rss_bytes',True),('rss_bytes','20'),('pid',[])]:
            raw,native=self.evidence();raw['samples'][0]['processes'][1][field]=value
            result=reconcile_resources(raw,native)
            self.assertFalse(result['complete'])
            self.assertIsNone(result['aggregate_sampled_peak_bytes'])

    def evidence(self):
        worker=dict(launch_id=1,worker_pid=2,parent_pid=1,role='page',binary_sha256='a'*64,spawned_unix_ns=10,reaped_unix_ns=100,exit_code=0,signal=None,leases=[dict(context_id=10,page_id=20,started_unix_ns=10,ended_unix_ns=100)])
        verification=dict(worker,launch_id=2,worker_pid=3,role='verification',leases=[])
        native=dict(parent_pid=1,runtime=dict(worker_binary_sha256='a'*64),sessions=[dict(context_id=10)],worker_lifecycle=dict(schema_version=1,complete=True,processes=[worker,verification]))
        raw=dict(schema_version=1,parent_pid=1,samples=[dict(monotonic_ns=1,unix_started_ns=20,unix_ended_ns=30,processes=[dict(pid=1,rss_bytes=10),dict(pid=2,rss_bytes=20),dict(pid=3,rss_bytes=100)])])
        return raw,native
    def test_verification_worker_counts_in_aggregate_not_page_percentile(self):
        raw,native=self.evidence();result=reconcile_resources(raw,native)
        self.assertTrue(result['complete']);self.assertEqual(result['aggregate_sampled_peak_bytes'],130)
        self.assertEqual(result['worker_sampled_peak_p50_bytes'],20)
        self.assertEqual(result['instantaneous_groups'][0]['auxiliary_worker_rss_bytes'],100)
    def test_complete_flag_does_not_hide_missing_checked_reap(self):
        raw,native=self.evidence();native['worker_lifecycle']['processes'][0]['reaped_unix_ns']=None
        self.assertFalse(reconcile_resources(raw,native)['complete'])
    def test_pid_reuse_requires_unique_launch_interval(self):
        raw,native=self.evidence();first=native['worker_lifecycle']['processes'][0]
        native['worker_lifecycle']['processes'].append(dict(first,launch_id=3))
        self.assertFalse(reconcile_resources(raw,native)['complete'])
    def test_wrong_binary_and_lease_owner_fail(self):
        for field,value in [('binary_sha256','b'*64),('parent_pid',99)]:
            raw,native=self.evidence();native['worker_lifecycle']['processes'][0][field]=value
            self.assertFalse(reconcile_resources(raw,native)['complete'])
    def test_backwards_wall_clock_is_not_repaired_by_guessing(self):
        raw,native=self.evidence();raw['samples'][0]['unix_ended_ns']=1
        self.assertFalse(reconcile_resources(raw,native)['complete'])

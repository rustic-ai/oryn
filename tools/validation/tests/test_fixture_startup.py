"""Synthetic service clocks/process receipts; never opens native fixture ports."""
from contextlib import ExitStack, nullcontext, contextmanager
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

from oryn_validation import fixture_startup as subject, wpt, prerequisites, processes
from oryn_validation.inputs import snapshot_input
from oryn_validation.manifests import manifest_digest


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


def bound_fixture(root):
    path = root/'run.json'; path.write_text('{}')
    record = snapshot_input(root, 'run.json')
    return {'store_root': root, 'bundle_root': root/'bundle', 'path': path,
            'sha256': record['sha256'], 'records': [record],
            'run': {'immutable_inputs': [], 'outputs': {'results': 'output'}},
            'assets': {'python': root/'python', 'wpt_checkout': root, 'fixture_relay': root/'relay'}}


def terminated_service(root, *, forced=False, missing=False):
    raw = {'cleanup_complete': True, 'cleanup_errors': [], 'forced_cleanup': forced,
           'error': 'owned execution cancelled by signal 15', 'timed_out': False, 'cancellation_signal': 15}
    result = dict(raw, service_stop_requested=True)
    if not missing:
        write(root/'process-result.json', raw); write(root/'service-result.json', result)
    return SimpleNamespace(result=result, failure=None, thread=SimpleNamespace(is_alive=lambda: False), output=root)


class StartupPhases(unittest.TestCase):
    def run_services(self, root, *, admission_after=90, bind=False, entry_failure=False):
        bound = bound_fixture(root)
        selected = {'fixture': {'ports': {'http': [8000, 8001]}}}
        clock = SimpleNamespace(ns=1_000_000_000)
        services, connections = [], []
        def advance(seconds): clock.ns += int(seconds*1_000_000_000)
        lease = SimpleNamespace(checkpoint=lambda: advance(5))
        class Service:
            def __init__(self, argv, *, cwd, output, env, timeout_seconds, grace_seconds):
                self.argv, self.cwd, self.output, self.env = argv, cwd, output, env
                self.timeout = timeout_seconds; self.result = None; self.failure = None
                self.thread = SimpleNamespace(is_alive=lambda: self.result is None)
            def __enter__(self):
                if entry_failure:
                    raise RuntimeError('service entry failed')
                self.started = clock.ns
                self.preflight = {'argv': self.argv, 'cwd': str(self.cwd), 'started_monotonic_ns': self.started,
                    'timeout_seconds': self.timeout, 'owner_pid': 66, 'child_pid': 77, 'child_identity': [[1, 2], 66]}
                write(self.output/'preflight.json', self.preflight)
                return self
            def check(self):
                if clock.ns >= self.started+self.timeout*1_000_000_000:
                    raise RuntimeError('overall service expired')
                ready = self.output/'admission-ready.json'
                if admission_after is not None and clock.ns >= self.started+admission_after*1_000_000_000 and not ready.exists():
                    write(ready, {'schema_version': 1, 'kind': 'topology-service-admitted',
                        'run_manifest_sha256': bound['sha256'], 'inputs_sha256': manifest_digest(bound['records']),
                        'nonce': self.env[subject.NONCE], 'child_pid': 77, 'child_identity': [[1, 2], 66],
                        'admitted_monotonic_ns': clock.ns})
            def __exit__(self, *args):
                terminal = terminated_service(self.output)
                self.result = terminal.result
        def connection(address, **kwargs):
            connections.append(clock.ns)
            if not bind:
                raise OSError('not bound')
            return nullcontext()
        with ExitStack() as stack:
            stack.enter_context(patch.object(wpt, 'relay_environment'))
            stack.enter_context(patch.object(wpt, 'relay_commands', return_value=[]))
            stack.enter_context(patch.object(processes, 'OwnedService', Service))
            stack.enter_context(patch.object(subject.time, 'monotonic_ns', side_effect=lambda: clock.ns))
            stack.enter_context(patch.object(subject.time, 'sleep', side_effect=advance))
            stack.enter_context(patch.object(wpt.socket, 'create_connection', side_effect=connection))
            if bind and not entry_failure:
                with wpt.fixture_services(bound, selected, output=root/'output', environment={}, lease=lease,
                    timeout_seconds=600, upstream=True, services=services):
                    self.assertEqual(services[0].timeout, 600)
            else:
                expected = RuntimeError if entry_failure else TimeoutError
                with self.assertRaises(expected):
                    with wpt.fixture_services(bound, selected, output=root/'output', environment={}, lease=lease,
                        timeout_seconds=600, upstream=True, services=services):
                        self.fail('failed startup must not yield')
        return bound, services, connections

    def test_admission_longer_than_thirty_seconds_precedes_bind_clock(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            bound, services, connections = self.run_services(root, bind=True)
            admitted = subject._read(services[0].output/'admission-ready.json')
            ready = subject.verify_startup(bound, services[0].output, [8000, 8001])
            self.assertGreater(admitted['admitted_monotonic_ns'], 30_000_000_000)
            self.assertTrue(all(value >= admitted['admitted_monotonic_ns'] for value in connections))
            self.assertLessEqual(ready['bound_monotonic_ns']-ready['bind_started_monotonic_ns'],30_000_000_000)
            self.assertTrue(subject.settlement(None, services, process_attempted=False)['cleanup_complete'])

    def test_never_ready_has_bounded_admission_and_retains_clean_service(self):
        with tempfile.TemporaryDirectory() as temporary:
            _, services, connections = self.run_services(Path(temporary).resolve(), admission_after=None)
            self.assertEqual(connections, [])
            self.assertEqual(len(services), 1)
            self.assertTrue(subject.settlement(None, services, process_attempted=False)['cleanup_complete'])

    def test_bind_timeout_keeps_original_failure_and_accounts_for_service(self):
        with tempfile.TemporaryDirectory() as temporary:
            _, services, connections = self.run_services(Path(temporary).resolve())
            self.assertTrue(connections)
            self.assertEqual(len(services), 1)
            self.assertTrue(subject.settlement(None, services, process_attempted=False)['cleanup_complete'])

    def test_entry_failure_is_accounted_for_but_not_fabricated_as_settled(self):
        with tempfile.TemporaryDirectory() as temporary:
            _, services, _ = self.run_services(Path(temporary).resolve(), entry_failure=True)
            self.assertEqual(len(services), 1)
            self.assertFalse(subject.settlement(None, services, process_attempted=False)['cleanup_complete'])

    def test_saved_startup_rejects_nonce_birth_deadline_population_and_missing_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); bound, services, _=self.run_services(root, bind=True)
            directory=services[0].output
            for name, mutate in [
                ('preflight.json', lambda value:value.update(timeout_seconds=1200)),
                ('admission-ready.json', lambda value:value.update(child_identity=[[1, 3], 66])),
                ('admission-ready.json', lambda value:value.update(inputs_sha256='0'*64)),
                ('admission-ready.json', lambda value:value.update(admitted_monotonic_ns=599_000_000_000)),
                ('bind-ready.json', lambda value:value.update(nonce='a'*64)),
                ('bind-ready.json', lambda value:value.update(ports=[8000])),
                ('bind-ready.json', lambda value:value.update(bound_monotonic_ns=700_000_000_000))]:
                path=directory/name; original=subject._read(path); broken=copy.deepcopy(original); mutate(broken); write(path,broken)
                with self.assertRaises(ValueError): subject.verify_startup(bound,directory,[8000,8001])
                write(path,original)
            (directory/'admission-ready.json').unlink()
            with self.assertRaises(OSError): subject.verify_startup(bound,directory,[8000,8001])

    def test_publish_requires_full_lease_admission_and_refuses_duplicate_signal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve(); bound=bound_fixture(root)
            directory=root/'output/services/upstream';directory.mkdir(parents=True)
            lease=MagicMock(admission={'inputs_sha256':manifest_digest(bound['records'])})
            with patch.dict(subject.os.environ,{subject.NONCE:'b'*64}), patch.object(processes,'supervisor') as supervisor:
                supervisor.return_value.process_identity.return_value=((1,2),66)
                subject.publish_admission(bound,lease)
                lease.checkpoint.assert_called_once()
                with self.assertRaisesRegex(ValueError,'already exists'):subject.publish_admission(bound,lease)
                (directory/'admission-ready.json').unlink()
                lease.admission=None
                with self.assertRaisesRegex(ValueError,'custody differs'):subject.publish_admission(bound,lease)


class Settlement(unittest.TestCase):
    def test_clean_known_services_settle_failed_original_but_unknown_process_does_not(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            service=terminated_service(root/'service')
            self.assertTrue(subject.settlement(None,[service],process_attempted=False)['cleanup_complete'])
            self.assertFalse(subject.settlement(None,[service],process_attempted=True)['cleanup_complete'])
            self.assertTrue(subject.settlement(None,[],process_attempted=False)['cleanup_complete'])
            for selected in (terminated_service(root/'forced',forced=True),terminated_service(root/'missing',missing=True)):
                self.assertFalse(subject.settlement(None,[selected],process_attempted=False)['cleanup_complete'])
            service.thread=SimpleNamespace(is_alive=lambda:True)
            self.assertFalse(subject.settlement(None,[service],process_attempted=False)['cleanup_complete'])


class OwnerFailureIntegration(unittest.TestCase):
    def exercise(self, root, *, owner, failure):
        from test_worker_environment import fixture
        bound=fixture(root)
        path=root/'run.json';path.write_text('{}');output=root/'output'
        bound.update(path=path,sha256=wpt._digest(path))
        policy=root/'tls.json';policy.write_text('{}')
        bound['assets'].update(python=root/'python',wpt_checkout=root,fixture_tls_policy=policy,fixture_relay=root/'relay')
        bound['run'].update(run_id='original',profile='standard-block',suite={'name':'fixture-topology'},
            outputs={'results':'output','events':'output/mozlog.jsonl'})
        selected={'collection':'core','python_environment':{},'fixture':{'ports':{'https':[8443]}},'fixture_path':policy}
        original_fixtures=wpt.fixture_services
        class PartialService:
            def __init__(self,argv,*,output,**kwargs):
                self.output=output;self.result=None;self.failure=None;self.thread=None
            def __enter__(self):
                if self.output.name=='second':raise RuntimeError('primary second entry failure')
                return self
            def check(self):pass
            def __exit__(self,*args):self.result=terminated_service(self.output).result
        @contextmanager
        def fixtures(*args,services,**kwargs):
            if failure=='second_entry':
                with original_fixtures(*args,services=services,**kwargs):yield services
                return
            service=terminated_service(output/'services/upstream',missing=failure=='receipt')
            service.check=lambda:None
            write(service.output/'preflight.json',{'started_monotonic_ns':subject.time.monotonic_ns()-90_000_000_000,'timeout_seconds':600})
            services.append(service)
            if failure=='entry':raise RuntimeError('primary fixture entry failure')
            yield services
        def launch(*args,**kwargs):
            if failure=='launch':raise RuntimeError('primary unknown launch')
            (output/'result.json').write_text('{"errors": []}')
            write(output/'process/preflight.json',{'started_monotonic_ns':subject.time.monotonic_ns(), 'timeout_seconds':kwargs['timeout_seconds']})
            if failure=='parser':
                (output/'process/stderr.log').write_text("argument --product: invalid choice: 'oryn'\n")
            return {'returncode':2 if failure=='parser' else 0,'error':None,'elapsed_ns':0,'cleanup_complete':True,'cleanup_errors':[],
                    'forced_cleanup':False,'timed_out':False,'cancellation_signal':None}
        with ExitStack() as stack:
            target=wpt if owner=='wpt' else prerequisites
            stack.enter_context(patch.object(target,'load_inputs',return_value=bound))
            stack.enter_context(patch.object(target,'InputLease'))
            stack.enter_context(patch('oryn_validation.bundle_ownership.registered_bundle_owner',return_value='build'))
            stack.enter_context(patch.object(wpt,'python_environment',return_value={}))
            stack.enter_context(patch.object(wpt,'relay_environment',return_value={}))
            stack.enter_context(patch.object(wpt,'fixture_services',side_effect=fixtures))
            if failure=='second_entry':
                stack.enter_context(patch.object(processes,'OwnedService',PartialService))
                stack.enter_context(patch.object(wpt,'relay_commands',return_value=[('first',['relay'],('127.0.0.1',8000)),('second',['relay'],('127.0.0.1',8001))]))
                stack.enter_context(patch.object(wpt.socket,'create_connection',return_value=nullcontext()))
            stack.enter_context(patch('oryn_validation.fixture_tls.verify_receipts',
                side_effect=ValueError('missing TLS activation') if failure=='parser' else None))
            if owner=='wpt':
                resources=stack.enter_context(patch.object(processes,'ResourceLease'))
                spawn=stack.enter_context(patch.object(processes,'run_owned',side_effect=launch))
                stack.enter_context(patch.object(wpt,'official_admission',return_value=selected))
                stack.enter_context(patch.object(wpt,'execution_policy',return_value={'timeout_seconds':600,'relay_timeout_seconds':600}))
                stack.enter_context(patch.object(wpt,'plan_command',return_value=['unused']))
                call=lambda:wpt.execute(path,store_root=root,bundle_root=bound['bundle_root'],output=output)
            else:
                resources=stack.enter_context(patch.object(prerequisites,'ResourceLease'))
                spawn=stack.enter_context(patch.object(prerequisites,'run_owned',side_effect=launch))
                stack.enter_context(patch.object(prerequisites,'validate_production'))
                stack.enter_context(patch.object(prerequisites,'_selected',return_value=(None,policy,selected['fixture'])))
                call=lambda:prerequisites.execute(path,store_root=root,bundle_root=bound['bundle_root'],stage='topology')
            with self.assertRaisesRegex((ValueError,RuntimeError),
                {'entry':'primary fixture entry failure','launch':'primary unknown launch','receipt':'settlement failed','second_entry':'primary second entry failure','parser':'upstream WPT failed with exit status 2.*missing TLS activation'}[failure]):call()
            if failure in ('entry','second_entry'):spawn.assert_not_called()
            settled=resources.return_value.__enter__.return_value.mark_settled.call_args.args[0]
        self.assertEqual(settled['cleanup_complete'],failure in ('entry','parser'))
        self.assertEqual(settled['process_attempted'],failure not in ('entry','second_entry'))
        self.assertEqual(len(settled['fixture_services']),2 if failure=='second_entry' else 1)
        self.assertEqual(subject._read(output/'resource-settlement.json'),settled)
        if failure=='receipt':
            execution=subject._read(output/'execution.json')
            self.assertTrue(execution['errors'])
        if failure=='parser':
            execution=subject._read(output/'execution.json')
            self.assertEqual(execution['returncode'],2)
            self.assertFalse(execution['custody_complete'])
            self.assertIn('exit status 2',execution['errors'][0])
            self.assertIn('missing TLS activation',execution['errors'][0])
            self.assertIn('process/stderr.log',execution['artifacts'])
        return settled

    def test_parser_exit_and_secondary_missing_tls_are_retained_with_clean_settlement(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.exercise(Path(temporary).resolve(),owner='wpt',failure='parser')

    def test_entry_failure_preserves_services_and_releases_only_verified_clean_resources(self):
        for owner in ('wpt','prerequisite'):
            with self.subTest(owner=owner),tempfile.TemporaryDirectory() as temporary:
                self.exercise(Path(temporary).resolve(),owner=owner,failure='entry')

    def test_later_service_entry_failure_retains_all_siblings_and_unknown_entry(self):
        for owner in ('wpt','prerequisite'):
            with self.subTest(owner=owner),tempfile.TemporaryDirectory() as temporary:
                settled=self.exercise(Path(temporary).resolve(),owner=owner,failure='second_entry')
                self.assertTrue(settled['fixture_services'][0]['cleanup_complete'])
                self.assertIsNone(settled['fixture_services'][1])

    def test_missing_service_terminal_receipt_fails_otherwise_successful_original(self):
        for owner in ('wpt','prerequisite'):
            with self.subTest(owner=owner),tempfile.TemporaryDirectory() as temporary:
                self.exercise(Path(temporary).resolve(),owner=owner,failure='receipt')

    def test_unknown_attempted_collector_launch_remains_unsettled_with_primary_error(self):
        for owner in ('wpt','prerequisite'):
            with self.subTest(owner=owner),tempfile.TemporaryDirectory() as temporary:
                self.exercise(Path(temporary).resolve(),owner=owner,failure='launch')


class CollectorBudget(unittest.TestCase):
    def test_remaining_earliest_service_budget_is_recomputed_from_saved_preflights(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            services=[]
            for name,start in [('relay',1_000_000_000),('upstream',21_000_000_000)]:
                service=SimpleNamespace(output=root/'services'/name)
                write(service.output/'preflight.json',{'started_monotonic_ns':start,'timeout_seconds':600})
                services.append(service)
            with patch.object(subject.time,'monotonic_ns',return_value=101_000_000_000):
                value=subject.collector_budget(services)
            self.assertEqual(value['timeout_seconds'],500)
            write(root/'collector-budget.json',value)
            preflight={'started_monotonic_ns':102_000_000_000,'timeout_seconds':500}
            subject.verify_collector_budget(root,preflight,{'elapsed_ns':1_000_000_000})
            with self.assertRaisesRegex(ValueError,'remaining service lifetime'):
                subject.verify_collector_budget(root,dict(preflight,timeout_seconds=600),{'elapsed_ns':1_000_000_000})
            with self.assertRaisesRegex(ValueError,'remaining service lifetime'):
                subject.verify_collector_budget(root,preflight,{'elapsed_ns':500_000_000_000})
            with patch.object(subject.time,'monotonic_ns',return_value=602_000_000_000):
                with self.assertRaises(TimeoutError):subject.collector_budget(services)


class BatchedPortWait(unittest.TestCase):
    PORTS = [8000,8001,8443,8444,9000,8888,8889,8002,8003,8445,8446]

    def simulate(self, *, checkpoint_seconds=3, available=True, overall_seconds=600, first_round_missing=False):
        clock=SimpleNamespace(ns=1_000_000_000)
        observations=[]; checkpoints=[]; attempts=[]
        def advance(seconds):clock.ns+=int(seconds*1_000_000_000)
        def checkpoint():
            checkpoints.append(clock.ns);observations.append('checkpoint');advance(checkpoint_seconds)
        service=MagicMock()
        lease=SimpleNamespace(checkpoint=checkpoint)
        def connect(address,timeout):
            port=address[1];attempts.append(port);observations.append(port)
            if not available or (first_round_missing and port==9000 and attempts.count(9000)==1):
                advance(timeout)
                raise OSError('not listening')
            return nullcontext()
        with patch.object(subject.time,'monotonic_ns',side_effect=lambda:clock.ns), \
             patch.object(subject.time,'sleep',side_effect=advance), \
             patch.object(subject.socket,'create_connection',side_effect=connect):
            arguments=dict(overall_deadline_ns=1_000_000_000+overall_seconds*1_000_000_000)
            if not available or checkpoint_seconds*2>=min(30,overall_seconds):
                with self.assertRaises(TimeoutError):subject.wait_for_ports(service,lease,self.PORTS,**arguments)
                result=None
            else:
                result=subject.wait_for_ports(service,lease,self.PORTS,**arguments)
        return result,observations,checkpoints,attempts,service

    def test_eleven_bound_ports_use_two_full_slow_checkpoints_inside_same_thirty_seconds(self):
        result,observations,checkpoints,attempts,service=self.simulate()
        self.assertEqual(result['poll_rounds'],1)
        self.assertEqual(result['completed_monotonic_ns']-result['bind_started_monotonic_ns'],6_000_000_000)
        self.assertEqual(observations,['checkpoint',*sorted(self.PORTS),'checkpoint'])
        self.assertEqual(len(checkpoints),2)
        self.assertEqual(set(attempts),set(self.PORTS))
        self.assertGreaterEqual(service.check.call_count,2*len(self.PORTS)+3)

    def test_each_retry_round_checks_complete_inputs_then_only_pending_ports(self):
        result,_,checkpoints,attempts,_=self.simulate(first_round_missing=True)
        self.assertEqual(result['poll_rounds'],2)
        self.assertEqual(len(checkpoints),3)
        self.assertEqual(attempts,sorted(self.PORTS)+[9000])

    def test_unbound_final_custody_and_overall_expiration_do_not_extend_deadline(self):
        self.simulate(available=False)
        self.simulate(checkpoint_seconds=16)
        self.simulate(overall_seconds=5)

    def test_real_input_drift_during_connections_is_rejected_at_final_full_checkpoint(self):
        from oryn_validation.inputs import InputLease
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve();source=root/'input';source.write_text('original')
            records=[snapshot_input(root,'input')]
            clock=SimpleNamespace(ns=1_000_000_000);calls=[]
            with patch.object(subject.time,'monotonic_ns',side_effect=lambda:clock.ns), \
                 patch.object(subject.socket,'create_connection') as connect:
                def connection(address,timeout):
                    calls.append(address[1])
                    if len(calls)==5:source.write_text('modified')
                    return nullcontext()
                connect.side_effect=connection
                helper_rejected = False
                # The outer assertion covers the separate mandatory exit hash.
                # The inner assertion must succeed before we even reach exit:
                # a helper that returns 'bound' would raise AssertionError here.
                with self.assertRaises(ValueError):
                    with InputLease(records,store_root=root) as lease:
                        original=lease.checkpoint
                        def full_checkpoint():
                            clock.ns+=3_000_000_000
                            original()
                        lease.checkpoint=full_checkpoint
                        with self.assertRaises(ValueError):
                            subject.wait_for_ports(MagicMock(),lease,self.PORTS,overall_deadline_ns=601_000_000_000)
                        helper_rejected = True
                self.assertTrue(helper_rejected)
                self.assertEqual(len(calls),11)

    def test_first_checkpoint_drift_or_service_failure_never_reports_bound(self):
        with patch.object(subject.socket,'create_connection') as connect:
            lease=MagicMock();lease.checkpoint.side_effect=ValueError('input drift')
            with self.assertRaisesRegex(ValueError,'input drift'):
                subject.wait_for_ports(MagicMock(),lease,self.PORTS,overall_deadline_ns=subject.time.monotonic_ns()+600_000_000_000)
            connect.assert_not_called()
            service=MagicMock();service.check.side_effect=RuntimeError('service exited')
            with self.assertRaisesRegex(RuntimeError,'service exited'):
                subject.wait_for_ports(service,MagicMock(),self.PORTS,overall_deadline_ns=subject.time.monotonic_ns()+600_000_000_000)
            connect.assert_not_called()

"""Synthetic launch regressions: no native processes or fixture services."""
import copy
from contextlib import ExitStack, nullcontext
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from oryn_validation.inputs import snapshot_input
from oryn_validation import worker_environment as subject
from oryn_validation import prerequisites, wpt, typed_provenance


def fixture(root):
    worker = root / 'bundle/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
    worker.parent.mkdir(parents=True)
    worker.write_bytes(b'synthetic signed-worker bytes'); worker.chmod(0o755)
    bundle = root / 'bundle'
    record = snapshot_input(bundle, str(worker.relative_to(bundle)), scope='bundle')
    run = {'assets': {'worker': record}, 'immutable_inputs': [record],
           'build': {'build_id': 'selected', 'manifest_sha256': 'a' * 64}}
    return {'run': run, 'store_root': root, 'bundle_root': bundle, 'assets': {'worker': worker}, 'records': [record]}


class WorkerSelection(unittest.TestCase):
    def test_selected_worker_is_sealed_role_not_runtime_relative_or_inherited(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = fixture(Path(temporary).resolve())
            expected = str(bound['assets']['worker'])
            with patch.dict(os.environ, {'ORYN_PAGE_WORKER': '/hostile/worker'}):
                env = dict(os.environ); env.update(subject.worker_environment(bound))
                receipt = subject.assert_worker_environment(bound, env)
                self.assertEqual(receipt['path'], expected)
                self.assertNotEqual(env['ORYN_PAGE_WORKER'], str(bound['bundle_root'] / 'bin/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'))
                with self.assertRaisesRegex(ValueError, 'environment differs'):
                    subject.assert_worker_environment(bound)
            receipt['path'] = '/other/worker'
            with self.assertRaisesRegex(ValueError, 'recorded worker'):
                subject.verify_worker_binding(receipt, bound['run'], bound['bundle_root'])

    def test_unsealed_undeclared_or_substituted_worker_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = fixture(Path(temporary).resolve())
            for mutate in (lambda b: b['run']['immutable_inputs'].clear(),
                           lambda b: b['run']['assets']['worker'].update(scope='store'),
                           lambda b: b['run']['assets']['worker'].update(executable=False),
                           lambda b: b['assets'].update(worker=b['bundle_root'] / 'bin/worker')):
                invalid = copy.deepcopy(bound); mutate(invalid)
                with self.assertRaises(ValueError):
                    subject.worker_environment(invalid)

    def test_retained_binding_is_checked_against_original_bundle_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = fixture(Path(temporary).resolve())
            receipt = subject.worker_binding(bound['run'], '/original/build/runtime')
            subject.verify_worker_binding(receipt, bound['run'], '/original/build/runtime')
            with self.assertRaisesRegex(ValueError, 'recorded worker'):
                subject.verify_worker_binding(receipt, bound['run'], bound['bundle_root'])

    def test_typed_snapshot_rejects_worker_environment_before_admission_lease(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = fixture(Path(temporary).resolve())
            bound['assets']['runner_source'] = typed_provenance.SOURCE_ROOT
            with patch.object(typed_provenance, 'load_inputs', return_value=bound), \
                 patch.dict(os.environ, {'ORYN_VALIDATION_STORE_ROOT': str(bound['store_root']),
                     'ORYN_VALIDATION_BUNDLE_ROOT': str(bound['bundle_root']), 'ORYN_PAGE_WORKER': '/hostile'}):
                with self.assertRaisesRegex(ValueError, 'environment differs'):
                    typed_provenance._bound('/unused/run.json')

    def test_native_driver_boundaries_reject_post_admission_env_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); bound = fixture(root)
            bound.update(path=root / 'run.json')
            bound['assets'].update(runtime=root/'runtime', driver=root/'driver',
                reference_browser=root/'chrome', reference_driver=root/'chromedriver', python=root/'python')
            fixture_path = root/'fixture.json'; fixture_path.write_text('{}')
            output = root/'output'; output.mkdir()
            lease = MagicMock(); launched = []
            prerequisites.legacy_algorithms()
            import wpt_native_controls as native
            class Driver:
                def __init__(self, *args, check_inputs, **kwargs):
                    self.check = check_inputs; self.cleanup = None; self.wire = MagicMock(log=[])
                def __enter__(self):
                    self.check(); self.check()
                    launched.append(os.environ['ORYN_PAGE_WORKER'])
                    return self
                def __exit__(self, *args):
                    self.cleanup = {'complete': True, 'errors': []}
            def mutate(*args, **kwargs):
                os.environ['ORYN_PAGE_WORKER'] = '/injected/after-navigation'
                return kwargs['results']
            with patch.object(prerequisites, 'native_contract', return_value=({'tls': {}}, fixture_path, {'tls': {'ca': {'path': 'ca.pem'}}})), \
                 patch.object(native, 'Fixture', return_value=nullcontext()), patch.object(native, 'focus_lease', return_value=nullcontext()), \
                 patch.object(native, 'Driver', Driver), patch.object(native, 'browser_controls', side_effect=mutate), \
                 patch.object(prerequisites, 'validate_native_observations'), patch.object(prerequisites, 'run_owned') as spawn, \
                 patch.dict(os.environ, subject.worker_environment(bound)):
                result = prerequisites._collect_native(bound, lease, output)
            self.assertEqual(launched, [str(bound['assets']['worker'])])
            self.assertIn('environment differs', result['native']['errors'][0])
            self.assertIn('environment differs', result['reference']['errors'][0])
            self.assertIn('environment differs', result['errors'][0])
            self.assertEqual(result['worker_binding'], subject.selected_worker(bound))
            spawn.assert_not_called()
            self.assertGreaterEqual(lease.checkpoint.call_count, 4)

    def test_wpt_owner_overwrites_hostile_override_before_upstream_spawn(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve(); bound = fixture(root)
            path = root/'run.json'; path.write_text('{}')
            bound.update(path=path, sha256=wpt._digest(path))
            policy = root/'tls.json'; policy.write_text('{}')
            bound['assets'].update(fixture_tls_policy=policy, wpt_checkout=root/'wpt')
            bound['run'].update(outputs={'results': 'run', 'events': 'run/mozlog.jsonl'}, profile='standard-block')
            selected = {'collection': 'core', 'python_environment': {}, 'fixture': {}}
            def launch(*args, **kwargs):
                self.assertEqual(kwargs['env']['ORYN_PAGE_WORKER'], str(bound['assets']['worker']))
                raise RuntimeError('stop synthetic launch')
            with ExitStack() as stack:
                for name, value in [('load_inputs', bound), ('official_admission', selected),
                    ('execution_policy', {'timeout_seconds': 10, 'relay_timeout_seconds': 10}),
                    ('python_environment', {}), ('relay_environment', {}), ('plan_command', ['not-executed'])]:
                    stack.enter_context(patch.object(wpt, name, return_value=value))
                stack.enter_context(patch('oryn_validation.bundle_ownership.registered_bundle_owner', return_value='build'))
                stack.enter_context(patch.object(wpt, 'InputLease'))
                stack.enter_context(patch('oryn_validation.processes.ResourceLease'))
                stack.enter_context(patch.object(wpt, 'fixture_services', return_value=nullcontext([])))
                stack.enter_context(patch('oryn_validation.processes.run_owned', side_effect=launch))
                stack.enter_context(patch.dict(os.environ, {'ORYN_PAGE_WORKER': '/injected/worker'}))
                with self.assertRaisesRegex(RuntimeError, 'stop synthetic launch'):
                    wpt.execute(path, store_root=root, bundle_root=bound['bundle_root'], output=root/'run')

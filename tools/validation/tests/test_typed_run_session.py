"""Real input custody around a stubbed expensive production gate; no browser."""
import copy
import json
import os
from pathlib import Path
import pickle
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation import inputs, typed_provenance as t, wpt
from oryn_wptrunner import provenance as p


class TypedRunSessionTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(); self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name).resolve()
        self.path = self.root / 'run.json'; self.path.write_text('{"kind":"run"}')
        self.data = self.root / 'data'; self.data.mkdir()
        (self.data / 'payload').write_text('original')
        self.records = [inputs.snapshot_input(self.root, 'data', kind='directory')]
        assets = {'runner_source': t.SOURCE_ROOT}
        for name in ('runtime', 'driver', 'worker', 'worker_manifest'):
            f = self.root / name; f.write_text(name); f.chmod(0o755)
            record = inputs.snapshot_input(self.root, name, scope='bundle')
            self.records.append(record); assets[name] = f
        roles = {name: next(x for x in self.records if x['path'] == name)
                 for name in ('runtime', 'driver', 'worker', 'worker_manifest')}
        self.bound = dict(path=self.path, store_root=self.root, bundle_root=self.root,
            records=self.records, sha256=p.digest(self.path), assets=assets,
            run={'assets': roles, 'immutable_inputs': self.records,
                 'build': {'build_id': 'test', 'manifest_sha256': '0'*64}})
        self.value = {'typed_run': {'run_id': 'test'}, 'path': str(self.path),
            'sha256': p.digest(self.path), 'fixture': {'nested': [1]},
            'candidate': {'artifacts': {name: {'path': str(assets[name]),
                'sha256': roles[name]['sha256']} for name in ('runtime', 'driver')}}}
        self.addCleanup(patch.stopall)
        patch.dict(os.environ, {'ORYN_VALIDATION_STORE_ROOT': str(self.root),
            'ORYN_VALIDATION_BUNDLE_ROOT': str(self.root), 'ORYN_PAGE_WORKER': str(assets['worker']),
            'ORYN_CORE_COLLECTION': 'core'}, clear=False).start()
        patch.object(p, '_RUN_ADMISSION', None).start()
        patch.object(wpt, '_resolve_inputs', side_effect=lambda *a, **k: copy.deepcopy(self.bound)).start()
        self.admit = patch.object(t, 'official_admission', return_value={
            'production': {'signature': {'signature': 'valid'}}, 'urls': ['/test']}).start()
        patch.object(t, '_project', side_effect=lambda *a: copy.deepcopy(self.value)).start()
        self.hashes = patch.object(inputs, 'verify_inputs', wraps=inputs.verify_inputs).start()
        self.python = patch.object(wpt, 'python_environment', return_value={}).start()
        self.relay = patch.object(wpt, 'relay_environment', return_value={}).start()
        self.reference = patch('oryn_validation.prerequisites.reference_environment', return_value=({}, {})).start()
        self.signature = patch('oryn_validation.builds.verify_worker_signature',
            return_value={'signature': 'valid'}).start()

    def scope(self):
        return p.run_admission(self.path, None, self.root/'runtime', self.root/'driver')

    def get(self):
        return p.load_run_contract(self.path, self.root/'runtime', self.root/'driver')

    def test_one_admission_one_final_hash_and_no_static_replay(self):
        with self.scope():
            for _ in range(6): self.get()
            bound, selected = p.admitted_execution_inputs(self.path)
            selected['urls'].append('/changed'); bound['records'].clear()
            self.assertEqual(p.admitted_execution_inputs(self.path)[1]['urls'], ['/test'])
            self.assertEqual(self.hashes.call_count, 1)
            self.assertEqual(self.admit.call_count, 1)
        self.assertEqual(self.hashes.call_count, 2)
        self.assertEqual(self.admit.call_count, 1)
        self.python.assert_called_once(); self.relay.assert_called_once(); self.signature.assert_called_once()
        self.reference.assert_called_once()

    def test_semantic_failure_finalizes_admitted_lease_once(self):
        self.admit.side_effect = ValueError('semantic rejection')
        with self.assertRaisesRegex(ValueError, 'semantic rejection'):
            with self.scope(): self.fail('admitted')
        self.assertEqual(self.hashes.call_count, 2)
        self.assertIsNone(p._RUN_ADMISSION)

    def test_change_during_semantics_rejected_before_execution(self):
        def mutate(*a, **k):
            (self.data/'extra').write_text('injected')
            return {'production': {'signature': {}}}
        self.admit.side_effect = mutate
        with self.assertRaisesRegex(ValueError, 'metadata or membership changed'):
            with self.scope(): self.fail('admitted')
        self.assertEqual(self.hashes.call_count, 2)

    def test_changed_then_restored_is_sticky(self):
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                f = self.data/'payload'; before = f.stat(); f.write_text('modified')
                os.utime(f, ns=(before.st_atime_ns, before.st_mtime_ns))
                with self.assertRaisesRegex(ValueError, 'metadata'): self.get()
                f.write_text('original')
                with self.assertRaisesRegex(ValueError, 'revoked'): self.get()

    def test_environment_substitutions_revoke_even_after_restoring(self):
        for key in ('ORYN_PAGE_WORKER', 'ORYN_VALIDATION_STORE_ROOT', 'ORYN_VALIDATION_BUNDLE_ROOT'):
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'final verification failed'):
                with self.scope():
                    original = os.environ[key]; os.environ[key] = str(self.root/'wrong')
                    try:
                        with self.assertRaises(ValueError): self.get()
                    finally: os.environ[key] = original

    def test_final_drift_preserves_primary_error(self):
        error = RuntimeError('test failed')
        with self.assertRaises(RuntimeError) as caught:
            with self.scope():
                (self.data/'payload').write_text('modified')
                raise error
        self.assertIs(caught.exception, error)
        self.assertTrue(error._oryn_run_integrity_failed)
        self.assertTrue(any('integrity' in note for note in error.__notes__))
        self.assertEqual(self.hashes.call_count, 2)

    def test_live_signature_is_not_replaced_by_input_digest(self):
        self.signature.return_value = {'signature': 'different'}
        with self.assertRaisesRegex(ValueError, 'signature differs'):
            with self.scope(): self.get()

    def test_final_successful_systemexit_cannot_hide_drift(self):
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope():
                (self.data/'payload').write_text('modified')
                raise SystemExit(0)

    def test_fork_serialization_and_closed_execution_rejected(self):
        with self.scope():
            session = p._RUN_ADMISSION.typed_session
            with self.assertRaises(TypeError): pickle.dumps(session)
            with patch.object(os, 'getpid', return_value=session.pid+1):
                with self.assertRaisesRegex(ValueError, 'process boundary'): session.execution_inputs()
        with self.assertRaisesRegex(ValueError, 'not active'): session.execution_inputs()
        with self.assertRaisesRegex(ValueError, 'active typed'): p.admitted_execution_inputs(self.path)

    def test_projection_mutation_is_isolated(self):
        with self.scope():
            self.get()['fixture']['nested'].append(2)
            self.assertEqual(self.get()['fixture']['nested'], [1])

    def test_verify_bound_requires_exact_live_closure(self):
        with inputs.InputLease(self.records, store_root=self.root, bundle_root=self.root) as lease:
            lease.verify_bound(self.records, store_root=self.root, bundle_root=self.root)
            with self.assertRaises(inputs.InputError):
                lease.verify_bound([], store_root=self.root, bundle_root=self.root)
            with patch.object(os, 'getpid', return_value=lease._pid+1), self.assertRaises(inputs.InputError):
                lease.verify_bound(self.records, store_root=self.root, bundle_root=self.root)
        with self.assertRaises(inputs.InputError):
            lease.verify_bound(self.records, store_root=self.root, bundle_root=self.root)

    def test_omitted_build_input_cannot_escape_the_final_sweep(self):
        extra = self.root/'unleased-build-input'; extra.write_text('build input')
        omitted = inputs.snapshot_input(self.root, extra.name)
        with inputs.InputLease(self.records, store_root=self.root, bundle_root=self.root) as lease:
            lease.verify_subset(self.records[:1])
            with self.assertRaisesRegex(inputs.InputError, 'absent from active lease'):
                lease.verify_subset([omitted])
            with self.assertRaisesRegex(inputs.InputError, 'absent from active lease'):
                lease.verify_subset([self.records[0], self.records[0]])

    def test_drift_during_live_final_check_is_caught_by_final_digest(self):
        def mutate(*a, **k):
            (self.data/'payload').write_text('modified')
            return {'signature': 'valid'}
        self.signature.side_effect = mutate
        with self.assertRaisesRegex(ValueError, 'final verification failed'):
            with self.scope(): self.get()
        self.assertEqual(self.hashes.call_count, 2)

    def test_failed_live_check_does_not_skip_final_digest(self):
        self.python.side_effect = ValueError('Python observation failed')
        with self.assertRaisesRegex(ValueError, 'Python observation failed'):
            with self.scope(): self.get()
        self.assertEqual(self.hashes.call_count, 2)
        self.signature.assert_called_once()

    def test_changed_then_restored_during_final_live_check_is_rejected(self):
        def mutate(*a, **k):
            f = self.data/'payload'; f.write_text('modified'); f.write_text('original')
            return {'signature': 'valid'}
        self.signature.side_effect = mutate
        with self.assertRaisesRegex(ValueError, 'metadata or membership changed'):
            with self.scope(): self.get()
        self.assertEqual(self.hashes.call_count, 2)

    def test_final_environment_change_and_manifest_bytes_are_checked(self):
        original = os.environ['ORYN_CORE_COLLECTION']
        def change(*a, **k):
            os.environ['ORYN_CORE_COLLECTION'] = 'changed'
            return {'signature': 'valid'}
        self.signature.side_effect = change
        try:
            with self.assertRaisesRegex(ValueError, 'collection changed'):
                with self.scope(): self.get()
        finally:
            os.environ['ORYN_CORE_COLLECTION'] = original

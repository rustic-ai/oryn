"""All-cell raw custody and independent SQLite/native replay contracts, unexecuted."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from completion_provenance import digest
from workflow_evidence import artifact
import workflow_native_adapter as adapter
import workflow_raw_evidence as raw

spec = importlib.util.spec_from_file_location('raw_research_controls', Path(__file__).resolve().parents[1] / 'test_oil_research_outcomes.py')
controls = importlib.util.module_from_spec(spec)
spec.loader.exec_module(controls)


class RawFamilyReplayTests(unittest.TestCase):
    def setUp(self):
        self.control = controls.ResearchTests()
        self.control.setUp()
        self.addCleanup(self.control.tearDown)
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.recipe = ('research', 1, 'search_paginate_compare')
        self.case = next(case for case in self.control.contract['cases'] if case['id'] == self.recipe[2])
        data = self.control.records(self.case)
        frames = self.control.frames(self.case)
        prepared = raw.prepared_recipe(self.recipe, controls.BASE, None)
        self.session = dict(id='context-0', context_id=1, page_id=1, adversarial_for=None,
            errors=[], cleanup_errors=[], workers_retired=True, secret_labels=[], recovery=[], fault_evidence=None,
            steps=[dict(index=i, error=None, expected_error_matched=True, timed_out=False, outputs=frames if i == 0 else []) for i in range(len(prepared.lines))])
        self.plan = dict(schema_version=1, attempt_id='a', sessions=[dict(id='context-0',
            steps=[dict(oil=line) for line in prepared.lines], uploads=[], fault=None, recovery_steps=[])])
        self.native = dict(schema_version=1, attempt_id='a', infrastructure_errors=[],
            runtime=dict(sandboxed=True, worker_binary_sha256='b' * 64), sessions=[self.session],
            native_activity=dict(schema_version=1, overflow=False, operations=[dict(context_id=1, start_ns=10, end_ns=20, terminal='completed', operation_id=1)]))
        self.candidate = dict(worker_sha256='b' * 64, candidate_manifest_sha256='c' * 64)
        self.attempt = dict(attempt_id='a', cell='research.compare', context_count=1, mode='serial', fault=None)
        outcome, self.actions, errors = adapter.replay_case_outcome(prepared, self.session, data)
        self.assertEqual(errors, [])
        for action in self.actions:
            action['recipe'] = '0:0:' + action['recipe']
        self.observed = [dict(case=self.recipe[2], contexts=[outcome])]
        database = self.root / 'sealed.sqlite3'
        database.write_bytes(self.control.database.read_bytes())
        self.claim = dict(fixture_cleanup=dict(returncode=0, forced=False), recipe=list(self.recipe), session='context-0', base=controls.BASE,
            database=dict(path=database.name, sha256=digest(database)), sealed_oracle=self.save('sealed.json', data))
        self.wave = dict(primary=list(self.recipe), cases=[self.claim],
            native_plan=self.save('plan.json', self.plan), native_evidence=self.save('native.json', self.native))
        self.resource_check = patch.object(raw, 'check_resources').start()
        self.addCleanup(patch.stopall)
        self.manifest = dict(schema_version=1, attempt_id='a', waves=[self.wave], supplements=[])
        self.attempt['fault_evidence'] = self.save('faults.json', dict(fault=None, injected=False, records=[]))
        self.attempt['adversarial_evidence'] = self.save('probes.json', [])
        self.attempt['context_activity'] = self.save('activity.json', adapter.native_activity(self.native, 1))
        self.attempt['context_topology'] = self.save('topology.json', [adapter.context_topology(self.native, 1)])
        self.attempt['resource_evidence'] = self.save('resources.json', [dict(wave=0, native_evidence=self.wave['native_evidence'])])

    def save(self, name, value):
        path = self.root / name
        path.write_text(json.dumps(value))
        return dict(path=name, sha256=digest(path))

    def check(self):
        return raw.validate(self.root, self.save('manifest.json', self.manifest), self.attempt,
                            self.candidate, artifact, self.observed, self.actions)

    def test_actual_research_database_and_original_validator_replay(self):
        # Uses real fixture SQLite and the original validator, not a mocked pass.
        self.assertEqual(self.check(), self.observed)

    def test_changed_raw_database_cannot_hide_behind_matching_projection(self):
        self.control.fixture.db.execute('DELETE FROM visits')
        self.control.fixture.db.commit()
        path = self.root / self.claim['database']['path']
        path.write_bytes(self.control.database.read_bytes())
        self.claim['database']['sha256'] = digest(path)
        with self.assertRaisesRegex(ValueError, 'SQLite and sealed'):
            self.check()
        self.claim['sealed_oracle'] = self.save('sealed.json', self.control.records(dict(visits=[])))
        with self.assertRaisesRegex(ValueError, 'independent family outcome'):
            self.check()

    def test_rehashed_native_output_still_reaches_independent_validator(self):
        next(frame for frame in self.session['steps'][0]['outputs'] if frame['kind'] == 'structured')['value'] = 'invented'
        self.wave['native_evidence'] = self.save('native.json', self.native)
        with self.assertRaisesRegex(ValueError, 'independent family outcome'):
            self.check()

    def test_task_fault_authority_candidate_and_step_controls(self):
        original_plan, original_native = copy.deepcopy(self.plan), copy.deepcopy(self.native)
        mutations = [
            lambda: self.plan['sessions'][0]['steps'][0].update(oil='observe'),
            lambda: self.plan['sessions'][0].update(uploads=[dict(label='foreign', path='/tmp/x', mime='text/plain')]),
            lambda: self.plan['sessions'][0].update(fault=dict(kind='worker_failure')),
            lambda: self.native['runtime'].update(worker_binary_sha256='d' * 64),
            lambda: self.native.update(attempt_id='another'),
            lambda: self.native['sessions'][0]['steps'].pop(),
            lambda: self.native['sessions'][0]['steps'][0].update(expected_error_matched=False),
            lambda: self.native['sessions'][0].update(workers_retired=False),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                self.plan, self.native = copy.deepcopy(original_plan), copy.deepcopy(original_native)
                mutate()
                self.wave['native_plan'] = self.save('plan.json', self.plan)
                self.wave['native_evidence'] = self.save('native.json', self.native)
                with self.assertRaises(ValueError):
                    self.check()

    def test_complete_wave_context_recipe_and_projection_denominator(self):
        original = copy.deepcopy(self.manifest)
        for mutation in (
                lambda: self.manifest.update(waves=[]),
                lambda: self.manifest['waves'].append(copy.deepcopy(self.wave)),
                lambda: self.manifest['waves'][0].update(cases=[]),
                lambda: self.manifest['waves'][0]['cases'][0].update(recipe=['research', 1, 'empty_results']),
                lambda: self.manifest.update(supplements=[{}])):
            self.manifest = copy.deepcopy(original)
            mutation()
            with self.assertRaises(ValueError):
                self.check()
        self.manifest = original
        self.actions[0]['action_id'] += 100
        with self.assertRaisesRegex(ValueError, 'outcome/action projection'):
            self.check()

    def test_all_29_cells_and_every_concrete_recipe_have_replay_sources(self):
        cells = [*adapter.RESEARCH_RECIPES, *adapter.RECIPES]
        self.assertEqual(len(cells), 29)
        prefixes = dict(research='research', forms='form', shopping='shop', saas='saas', booking='booking')
        for cell in cells:
            recipes = adapter.MixedCellAdapter(cell, '/unused', '/unused').recipes()
            for recipe in recipes:
                family, version, name = recipe
                contract = adapter.read_case_contract(*recipe)
                case = next(row for row in contract['cases'] if row['id'] == name)
                prefix = 'advanced' if family == 'forms' and case.get('extended') else prefixes[family]
                base = 'http://127.0.0.1:12345/' + prefix + '/' + 'a' * 32
                faults = (None, 'delayed_response', 'worker_failure', 'cancellation', 'duplicate_delivery') if name == adapter.PREFERRED.get(cell) else (None,)
                for fault in faults:
                    with self.subTest(cell=cell, recipe=recipe, fault=fault):
                        prepared = raw.prepared_recipe(recipe, base, fault)
                        self.assertTrue(prepared.lines)
                        self.assertTrue(callable(prepared.receiver.snapshot))
                        self.assertTrue(callable(prepared.runner.validate) if family != 'forms' else callable(adapter.validate_forms))

    def test_sqlite_snapshot_uses_verified_bytes_and_rejects_malformed_database(self):
        encoded = self.control.database.read_bytes()
        expected = self.control.records(dict(visits=[]))
        self.assertEqual(raw.snapshot_bytes(controls.runner.receiver, encoded, controls.CASE), expected)
        with self.assertRaisesRegex(ValueError, 'SQLite evidence'):
            raw.snapshot_bytes(controls.runner.receiver, b'not a database', controls.CASE)

    def test_malformed_nested_payloads_raise_typed_evidence_errors(self):
        original = copy.deepcopy(self.manifest)
        for value in (None, [], 4, "not an object"):
            with self.subTest(manifest=value):
                self.manifest = value
                with self.assertRaises(ValueError):
                    self.check()
        for change in (
                lambda m: m.update(waves=[None]),
                lambda m: m['waves'][0].update(cases=[None]),
                lambda m: m['waves'][0]['cases'][0].update(database=[]),
                lambda m: m['waves'][0]['cases'][0].update(base=[]),
                lambda m: m.update(supplements=[None])):
            self.manifest = copy.deepcopy(original); change(self.manifest)
            with self.assertRaises(ValueError):
                self.check()
        self.manifest = original
        for group in ('sessions', 'runtime'):
            for value in (None, [None], 1):
                native = copy.deepcopy(self.native); native[group] = value
                with self.assertRaises(ValueError):
                    raw.check_run(self.plan, native, 'a', self.candidate)
        for result in (None, [], dict(index=0, outputs=[None])):
            session = dict(steps=[result])
            with self.assertRaises(ValueError):
                raw.check_steps([dict(oil='observe')], session)

    def test_duplicate_json_fields_and_nonfinite_numbers_reject(self):
        for encoded in ('{"a":1,"a":2}', '{"x":NaN}', '{"x":Infinity}', '{"x":-Infinity}'):
            with self.assertRaises(ValueError):
                raw.decode(encoded)
        self.assertEqual(raw.decode('{"schema_version":1,"cases":[]}'), dict(schema_version=1, cases=[]))

    def test_required_commit_phases_cannot_be_omitted(self):
        attempt = dict(self.attempt, cell='booking.cancel_retry', fault='cancellation')
        with self.assertRaisesRegex(ValueError, 'supplement denominator'):
            raw.validate_supplements(self.root, [], attempt, self.candidate, artifact)


class RawResourceCustodyTests(unittest.TestCase):
    def test_cleanup_and_accounting_are_recomputed_for_every_wave(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def saved(name, value):
                path = root / name; path.write_text(json.dumps(value))
                return dict(path=name, sha256=digest(path))
            samples = dict(parent_pid=10, samples=[{}], errors=[], overflow=False)
            cleanup = dict(parent_pid=10, returncode=0, errors=[])
            accounting = dict(complete=True, errors=[], groups=[])
            wave = dict(samples=saved('samples.json', samples), process_cleanup=saved('cleanup.json', cleanup), accounting=saved('accounting.json', accounting))
            native = dict(sessions=[])
            with patch('workflow_resources.reconcile_resources', return_value=accounting) as recompute:
                raw.check_resources(root, wave, native, artifact)
                recompute.assert_called_once_with(samples, native)
                for update in (dict(parent_pid=11), dict(returncode=False), dict(errors=['survivor'])):
                    wave['process_cleanup'] = saved('cleanup.json', dict(cleanup, **update))
                    with self.assertRaisesRegex(ValueError, 'cleanup'):
                        raw.check_resources(root, wave, native, artifact)
                wave['process_cleanup'] = saved('cleanup.json', cleanup)
                recompute.return_value = dict(complete=False, errors=['unowned worker'])
                with self.assertRaisesRegex(ValueError, 'accounting'):
                    raw.check_resources(root, wave, native, artifact)


class RawRecoveryAndProbeTests(unittest.TestCase):
    def test_commit_replay_requires_exact_retry_token_and_typed_phase_times(self):
        from types import SimpleNamespace
        case = SimpleNamespace(family='booking', case={}, identity='a' * 32, receiver=None)
        session = dict(context_id=1, fault_evidence=dict(injected=True),
                       recovery=[dict(stage='resolve_commit', state='not_committed')])
        ledger = [dict(token='a' * 32, phase='before_commit', body_sha256='b' * 64,
                       started_ns=10, released_ns=20, terminal='aborted_before_commit', response_status=303)]
        reads = [dict(token='a' * 32, state='not_committed')]
        retries = [dict(token='a' * 32, body_sha256='b' * 64, matched=1)]
        with patch.object(adapter, 'project_observed', return_value={'reservations': ['one']}), patch.object(adapter, 'project_expected', return_value={'reservations': ['one']}):
            self.assertEqual(adapter.replay_commit_outcome(case, session, {}, ledger, reads, retries, 'before_commit')[1], [])
            for field, value in (('token', 'foreign'), ('matched', True), ('body_sha256', 'c' * 64)):
                changed = copy.deepcopy(retries); changed[0][field] = value
                self.assertTrue(adapter.replay_commit_outcome(case, session, {}, ledger, reads, changed, 'before_commit')[1])
            for field, value in (('started_ns', True), ('released_ns', 9), ('phase', 'after_commit')):
                changed = copy.deepcopy(ledger); changed[0][field] = value
                self.assertTrue(adapter.replay_commit_outcome(case, session, {}, changed, reads, retries, 'before_commit')[1])
            committed_ledger = [dict(ledger[0], phase='after_commit', terminal='released_after_commit')]
            committed_reads = [dict(reads[0], state='committed')]
            committed_session = dict(session, recovery=[dict(stage='resolve_commit', state='committed')])
            self.assertEqual(adapter.replay_commit_outcome(case, committed_session, {}, committed_ledger, committed_reads, [], 'after_commit')[1], [])
            self.assertTrue(adapter.replay_commit_outcome(case, committed_session, {}, committed_ledger, committed_reads, retries, 'after_commit')[1])

    def test_confinement_replay_reads_foreign_case_and_retains_secret_redaction(self):
        import hashlib
        from types import SimpleNamespace
        from unittest.mock import Mock
        base = 'http://127.0.0.1:12345/advanced/' + 'a' * 32
        case = SimpleNamespace(family='forms', case=dict(scenario='confinement'), base=base,
                               identity='a' * 32, origin='http://127.0.0.1:12345', receiver=Mock())
        case.receiver.snapshot.return_value = dict(requests=[], receipts=[])
        secret = 'd' * 64
        probe = dict(kind='forms', id='adversary-0', owner='context-0', case_id='b' * 32,
                     secret_sha256=hashlib.sha256(secret.encode()).hexdigest())
        steps = [dict(oil=f'goto "{base}/receipt"'), dict(oil='extract text --selector "#denied"'),
                 dict(oil='goto "http://127.0.0.1:12345/advanced/' + 'b' * 32 + '/start"'),
                 dict(oil='upload "Authorized document" $file:authorized', expected_error='upload label is unknown'),
                 dict(oil='type "Full name" $secret:confined', expected_error='secret label is unknown'), dict(oil='observe')]
        inputs = {'context-0': dict(secrets=[dict(label='confined', path='/evidence/synthetic-secret-0', origin=case.origin)]),
                  'adversary-0': dict(adversarial_for='context-0', uploads=[], fault=None, steps=steps)}
        results = dict(context_id=2, secret_labels=[], recovery=[], steps=[
            dict(index=i, expected_error_matched=True, timed_out=False, error=step.get('expected_error'),
                 outputs=[dict(kind='structured', value='Context access denied')] if i == 1 else []) for i, step in enumerate(steps)])
        sessions = {'context-0': dict(secret_labels=['confined']), 'adversary-0': results}
        self.assertTrue(raw.replay_probe(case, probe, 0, inputs, sessions, Path('/sealed'))['passed'])
        case.receiver.snapshot.assert_called_with(Path('/sealed'), 'b' * 32)
        changed = copy.deepcopy(sessions); changed['adversary-0']['steps'][-1]['outputs'] = [dict(kind='structured', value=secret)]
        with self.assertRaises(ValueError):
            raw.replay_probe(case, probe, 0, inputs, changed, Path('/sealed'))
        case.receiver.snapshot.return_value = dict(requests=[dict(status=303)], receipts=[])
        with self.assertRaisesRegex(ValueError, 'durable mutation'):
            raw.replay_probe(case, probe, 0, inputs, sessions, Path('/sealed'))

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
        outputs=[[] for _ in prepared.lines]
        action_steps=[i for i,line in enumerate(prepared.lines) if line.split()[0] in ('click','type','select')]
        actions=[frame for frame in frames if frame['kind']=='action']
        self.assertEqual(len(actions),len(action_steps))
        for index,frame in zip(action_steps,actions):outputs[index].append(copy.deepcopy(frame))
        for frame in frames:
            if frame['kind']=='structured':
                owner=frame['command_provenance'];index=owner['step']
                self.assertEqual(prepared.lines[index],owner['oil'])
                self.assertTrue(prepared.lines[index].startswith('extract '))
                output=copy.deepcopy(frame);del output['command_provenance']
                outputs[index].append(output)
        # The shared control has one source observation per explicit observe,
        # followed by a final-location observation at the last command.
        observe_steps=[i for i,line in enumerate(prepared.lines) if line=='observe']
        observations=[frame for frame in frames if frame['kind']=='observation']
        self.assertEqual(len(observations),len(observe_steps)+1)
        for index,frame in zip(observe_steps,observations[:-1]):outputs[index].append(copy.deepcopy(frame))
        outputs[observe_steps[-1]].append(copy.deepcopy(observations[-1]))
        self.assertEqual(sum(map(len,outputs)),len(frames))
        self.session = dict(id='context-0', context_id=1, page_id=1, adversarial_for=None,
            errors=[], cleanup_errors=[], workers_retired=True, secret_labels=[], recovery=[], fault_evidence=None,
            steps=[dict(index=i, error=None, expected_error_matched=True, timed_out=False, outputs=outputs[i]) for i in range(len(prepared.lines))])
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
        index=next(i for i,step in enumerate(self.plan['sessions'][0]['steps']) if step['oil'].startswith('extract '))
        next(frame for frame in self.session['steps'][index]['outputs'] if frame['kind'] == 'structured')['value'] = 'invented'
        self.wave['native_evidence'] = self.save('native.json', self.native)
        with self.assertRaisesRegex(ValueError, 'independent family outcome'):
            self.check()

    def test_rehashed_wrong_step_or_duplicate_extraction_cannot_hide_behind_values(self):
        original=copy.deepcopy(self.native)
        extracts=[i for i,step in enumerate(self.plan['sessions'][0]['steps']) if step['oil'].startswith('extract ')]
        first,second=extracts[:2]
        for mutation in ('move','duplicate','swap'):
            with self.subTest(mutation=mutation):
                self.native=copy.deepcopy(original)
                steps=self.native['sessions'][0]['steps']
                if mutation=='move':
                    steps[second]['outputs'].extend(steps[first]['outputs']);steps[first]['outputs']=[]
                elif mutation=='duplicate':
                    steps[first]['outputs'].append(copy.deepcopy(steps[first]['outputs'][0]))
                else:
                    steps[first]['outputs'],steps[second]['outputs']=steps[second]['outputs'],steps[first]['outputs']
                self.wave['native_evidence']=self.save('native.json',self.native)
                with self.assertRaisesRegex(ValueError,'independent family outcome'):
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
            cleanup = dict(parent_pid=10, process_group=10, group_settled=True, returncode=0, errors=[])
            accounting = dict(complete=True, errors=[], groups=[])
            wave = dict(samples=saved('samples.json', samples), process_cleanup=saved('cleanup.json', cleanup), accounting=saved('accounting.json', accounting))
            native = dict(sessions=[])
            with patch('workflow_resources.reconcile_resources', return_value=accounting) as recompute:
                raw.check_resources(root, wave, native, artifact)
                recompute.assert_called_once_with(samples, native)
                for update in (dict(parent_pid=11), dict(returncode=False), dict(errors=['survivor']),
                               dict(group_settled=False), dict(group_settled=None),
                               dict(process_group=11), dict(process_group=True), dict(process_group=None)):
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


class SaaSDatabaseBundleTests(unittest.TestCase):
    def setUp(self):
        from types import SimpleNamespace
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.identity='a'*32;self.database=self.root/'app.sqlite3'
        self.receiver=adapter.load('raw_bundle_saas',adapter.FIXTURES/'saas_fixture_v2.py')
        fixture=self.receiver.Fixture(self.database);fixture.origin='http://127.0.0.1:41004'
        fixture.command(dict(command='create',case_id=self.identity,scenario='normal'))
        with fixture.provider.db:
            fixture.provider.audit(self.identity,'token_exchange',400)
        fixture.db.close();fixture.provider.close()
        self.case=SimpleNamespace(family='saas',version=2,database=self.database)
        def saved(path):return dict(path=str(path.relative_to(self.root)),sha256=digest(path))
        self.claim=adapter.preserved_case_databases(self.case,saved)
        self.expected=self.receiver.snapshot(self.database,self.identity)

    def snapshot(self,claim=None,version=2):
        from types import SimpleNamespace
        case=SimpleNamespace(family='saas',version=version)
        with raw.case_database_bytes(self.root,self.claim if claim is None else claim,case,artifact) as path:
            self.assertNotEqual(path,self.database)
            return self.receiver.snapshot(path,self.identity)

    def test_real_oauth_snapshot_survives_verified_private_bundle_for_v2_and_v4(self):
        for version in (2,4):
            self.assertEqual(self.snapshot(version=version),self.expected)
        self.assertEqual(self.expected['oauth']['events'],[dict(action='token_exchange',outcome=400)])
        self.assertEqual(self.expected['oauth']['tokens'],[])

    def test_missing_sidecar_does_not_fall_back_to_existing_adjacent_file(self):
        claim=dict(self.claim);del claim['oauth_database']
        self.assertTrue(Path(str(self.database)+'.oauth').is_file())
        with self.assertRaisesRegex(ValueError,'OAuth database'):self.snapshot(claim)

    def test_foreign_reference_hash_change_and_corrupt_bytes_are_rejected(self):
        claim=copy.deepcopy(self.claim);claim['oauth_database']['path']='foreign.sqlite3.oauth'
        with self.assertRaisesRegex(ValueError,'owner/path'):self.snapshot(claim)
        claim=copy.deepcopy(self.claim);claim['oauth_database']['sha256']='0'*64
        with self.assertRaises(ValueError):self.snapshot(claim)
        path=Path(str(self.database)+'.oauth');path.write_bytes(b'not SQLite')
        self.claim['oauth_database']['sha256']=digest(path)
        with self.assertRaisesRegex(ValueError,'SQLite'):self.snapshot()

    def test_rehashed_oauth_mutation_changes_independent_durable_outcome(self):
        import sqlite3
        path=Path(str(self.database)+'.oauth')
        with sqlite3.connect(path) as db:db.execute('UPDATE oauth_events SET outcome=200')
        self.claim['oauth_database']['sha256']=digest(path)
        self.assertNotEqual(self.snapshot()['oauth'],self.expected['oauth'])

    def test_commit_bundle_retains_gate_tables_and_oauth_assertions(self):
        import sqlite3
        with sqlite3.connect(self.database) as db:
            db.execute('CREATE TABLE workflow_commit_gates(token TEXT,phase TEXT)')
            db.execute("INSERT INTO workflow_commit_gates VALUES('bound-token','before_commit')")
        self.claim['database']['sha256']=digest(self.database)
        with raw.case_database_bytes(self.root,self.claim,self.case,artifact) as path:
            self.assertEqual(self.receiver.snapshot(path,self.identity),self.expected)
            with sqlite3.connect(f'file:{path}?mode=ro',uri=True) as db:
                self.assertEqual(db.execute('SELECT token,phase FROM workflow_commit_gates').fetchall(),[('bound-token','before_commit')])

    def test_ordinary_family_bundle_rejects_unexpected_oauth_database(self):
        from types import SimpleNamespace
        with self.assertRaisesRegex(ValueError,'unexpected raw OAuth'):
            with raw.case_database_bytes(self.root,self.claim,SimpleNamespace(family='research',version=3),artifact):pass

"""Prerequisite gate/custody tests; no native UI, fixture ports or WPT run."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, MagicMock

from oryn_validation import prerequisites as subject
from oryn_validation.production import _module
from oryn_validation.inputs import snapshot_input


class NativePrerequisiteContracts(unittest.TestCase):
    def stage_fixture(self, root, *, mutate_preflight=None, omit_artifact=False):
        """Synthetic process receipts test the joins, never attest a real run."""
        from test_manifests import run
        def write(relative, value):
            path = root / relative; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value)); return path
        def pin(path):
            return snapshot_input(root, str(path.relative_to(root)), kind='directory' if path.is_dir() else 'file')
        fixture = write('fixture.json', {'tls': {'name': 'test'}, 'web_features': {}})
        registry = write('registry.json', {'profiles': {'standard-block': {'fixture': 'fixture'}}})
        python = root / 'python'; python.write_text('not executed')
        stage = run(); stage['suite']['name'] = 'native-prerequisites'
        stage['assets'] = {'python': pin(python), 'fixture': pin(fixture), 'core_profile_registry': pin(registry)}
        stage['immutable_inputs'] = list(stage['assets'].values())
        stage['outputs'] = {'events': 'runs/native/events.jsonl', 'results': 'runs/native/output', 'status': 'status/native.json'}
        run_path = write('manifests/native.json', stage)
        stage_record = pin(run_path)
        out = root / stage['outputs']['results']
        argv = [str(python), '-B', '-m', 'oryn_validation.prerequisites', 'collect',
                '--stage', 'native', '--run', str(run_path), '--store-root', str(root),
                '--bundle-root', str(root), '--output', str(out)]
        preflight = {'argv': argv, 'child_pid': 123, 'owner_pid': 122, 'child_identity': [[123456, 17], 122]}
        if mutate_preflight: mutate_preflight(preflight)
        process = {'argv': preflight['argv'], 'returncode': 0, 'timed_out': False, 'error': None,
                   'cleanup_complete': True, 'cleanup_errors': [], 'forced_cleanup': False, 'cancellation_signal': None}
        digest = subject.manifest_digest([*stage['immutable_inputs'], stage_record])
        fixed = {'result': ('result.json', {'observed': 'synthetic'}),
                 'process': ('process/process-result.json', process), 'preflight': ('process/preflight.json', preflight),
                 'custody': ('custody.json', {'run_manifest_sha256': subject._hash(run_path), 'errors': [],
                    'admission': {'inputs_sha256': digest}, 'finalization': {'inputs_sha256': digest}})}
        paths = {name: write(str(out.relative_to(root) / relative), value) for name, (relative, value) in fixed.items()}
        execution = {'schema_version': 1, 'kind': 'typed-prerequisite-execution', 'stage': 'native',
                     'run_id': stage['run_id'], 'run_manifest_sha256': subject._hash(run_path),
                     'build': stage['build'], 'errors': [],
                     'artifacts': {relative: subject._hash(paths[name]) for name, (relative, _) in fixed.items()}}
        if omit_artifact:
            write(str(out.relative_to(root) / 'omitted.json'), {'retained': True})
        paths['execution'] = write(str(out.relative_to(root) / 'execution.json'), execution)
        paths['run'] = run_path; paths['output'] = out
        bound = {'run': copy.deepcopy(stage), 'store_root': root, 'bundle_root': root,
                 'assets': {'fixture': fixture, 'core_profile_registry': registry, 'python': python}}
        for name, path in paths.items():
            role = 'native_prerequisites_' + name
            bound['assets'][role] = path; bound['run']['assets'][role] = pin(path)
        bound['records'] = list(bound['run']['assets'].values())
        return bound

    def test_stage_evidence_joins_complete_original_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = self.stage_fixture(Path(temporary).resolve())
            self.assertEqual(subject._stage_evidence(bound, 'native_prerequisites', 'native'), {'observed': 'synthetic'})

    def test_stage_evidence_rejects_substituted_launch_and_birth(self):
        changes = [lambda value: value['argv'].__setitem__(8, '/unrelated/run.json'),
                   lambda value: value.update(child_identity=[[123456, 17], 999])]
        for mutate in changes:
            with tempfile.TemporaryDirectory() as temporary:
                bound = self.stage_fixture(Path(temporary).resolve(), mutate_preflight=mutate)
                with self.assertRaises(ValueError):
                    subject._stage_evidence(bound, 'native_prerequisites', 'native')

    def test_stage_evidence_rejects_output_omitted_from_execution(self):
        with tempfile.TemporaryDirectory() as temporary:
            bound = self.stage_fixture(Path(temporary).resolve(), omit_artifact=True)
            with self.assertRaisesRegex(ValueError, 'membership'):
                subject._stage_evidence(bound, 'native_prerequisites', 'native')

    def test_stage_evidence_rejects_changed_build_and_incomplete_custody(self):
        for mutate in (lambda bound: bound['run']['build'].update(build_id='other-build'),
                       lambda bound: bound.update(records=[])):
            with tempfile.TemporaryDirectory() as temporary:
                bound = self.stage_fixture(Path(temporary).resolve()); mutate(bound)
                with self.assertRaises(ValueError):
                    subject._stage_evidence(bound, 'native_prerequisites', 'native')

    def modern_fixture(self):
        fixtures = _module('oryn_native_prerequisite_fixtures', 'scripts/tests/test_wpt_native_controls.py')
        record = fixtures.accepted_fixture()
        import wpt_native_controls as native
        record['errors'] = []
        for name in ('native', 'reference'):
            controls = record[name]['controls']
            stages = native.CONTROL_STAGES if name == 'native' else native.CONTROL_STAGES[:5]
            controls['control_execution'] = {stage: {'status': 'completed'} for stage in stages}
            controls['trusted_title'] = native.MARKER
            controls['capabilities'] = {'acceptInsecureCerts': False}
            record[name]['commands'] = [value for value in controls.values()
                                         if isinstance(value, dict) and 'response' in value]
        return record

    def test_current_native_tls_focus_and_fault_semantics_retained(self):
        self.assertIsInstance(subject.validate_native_observations(self.modern_fixture()), dict)

    def test_tls_fixture_urls_and_raw_navigation_must_match(self):
        record = self.modern_fixture()
        urls = {kind: 'https://' + kind + '.test/native-prerequisite'
                for kind in ('trusted', 'untrusted', 'wrong_hostname')}
        for browser in ('native', 'reference'):
            controls = record[browser]['controls']; controls['tls_urls'] = urls; controls['session_id'] = 'session'
            for kind, url in urls.items():
                controls['tls_' + kind].update(method='POST', path='/session/session/url', body={'url': url})
        subject.validate_native_urls(record, urls)
        record['reference']['controls']['tls_untrusted']['body'] = {'url': urls['trusted']}
        with self.assertRaisesRegex(ValueError, 'navigation differs'):
            subject.validate_native_urls(record, urls)

    def test_success_boolean_cannot_replace_native_observations(self):
        with self.assertRaises(ValueError):
            subject.validate_native_observations({'prerequisites_passed': True, 'errors': []})

    def test_historical_missing_stage_map_is_not_current_evidence(self):
        record = self.modern_fixture()
        del record['reference']['controls']['control_execution']
        with self.assertRaisesRegex(ValueError, 'stages incomplete'):
            subject.validate_native_observations(record)

    def test_raw_response_must_match_transcript(self):
        record = self.modern_fixture()
        record['native']['commands'] = record['native']['commands'][:-1]
        with self.assertRaisesRegex(ValueError, 'raw command transcript'):
            subject.validate_native_observations(record)

    def test_actual_negative_controls_cannot_be_replaced_with_timeouts(self):
        record = self.modern_fixture()
        record['tls_probes']['wrong_hostname']['verify_code'] = 20
        with self.assertRaisesRegex(ValueError, 'name rejection'):
            subject.validate_native_observations(record)

    def test_focus_and_crash_cleanup_failures_remain_failures(self):
        for mutate in (lambda value: value['reference']['controls'].update(focus_samples=[True]),
                       lambda value: value['native']['controls']['worker_crash']['response']['value'].update(reaped=False),
                       lambda value: value['native']['cleanup'].update(complete=False)):
            value = self.modern_fixture(); mutate(value)
            with self.assertRaises(ValueError):
                subject.validate_native_observations(value)

    def test_fixture_and_driver_legacy_default_still_checks_generation(self):
        subject.legacy_algorithms()
        import wpt_native_controls as native
        with patch.object(native, 'check_execution_generation', side_effect=ValueError('legacy admission')) as guard:
            with self.assertRaisesRegex(ValueError, 'legacy admission'):
                native.Fixture({}).__enter__()
            with self.assertRaisesRegex(ValueError, 'legacy admission'):
                native.Driver('/not/launched', [], '/not/created').__enter__()
            self.assertEqual(guard.call_count, 2)

    def test_typed_fixture_guard_runs_at_actual_server_launch(self):
        subject.legacy_algorithms()
        import wpt_native_controls as native
        guard = MagicMock(side_effect=[None, ValueError('input drift')])
        with self.assertRaisesRegex(ValueError, 'input drift'):
            native.Fixture({'tls': {'trusted': {'url': 'https://web-platform.test:1/native-prerequisite'}}},
                           check_inputs=guard).__enter__()
        self.assertEqual(guard.call_count, 2)

    def test_typed_driver_guard_runs_immediately_before_popen(self):
        subject.legacy_algorithms()
        import wpt_native_controls as native
        with tempfile.TemporaryDirectory() as temporary:
            reserve = MagicMock()
            reserve.__enter__.return_value.getsockname.return_value = ('127.0.0.1', 1234)
            guard = MagicMock(side_effect=[None, ValueError('input drift')])
            with patch.object(native.socket, 'socket', return_value=reserve), patch.object(native.subprocess, 'Popen') as popen:
                with self.assertRaisesRegex(ValueError, 'input drift'):
                    native.Driver('/not/launched', [], Path(temporary) / 'driver', check_inputs=guard).__enter__()
                popen.assert_not_called()
            self.assertEqual(guard.call_count, 2)

    def test_failed_process_or_cleanup_is_never_admitted(self):
        clean = {'returncode': 0, 'timed_out': False, 'error': None, 'cleanup_complete': True,
                 'cleanup_errors': [], 'forced_cleanup': False, 'cancellation_signal': None}
        subject._success(clean)
        for key, value in [('returncode', 1), ('timed_out', True), ('cleanup_complete', False),
                           ('cleanup_errors', ['orphan']), ('forced_cleanup', True), ('cancellation_signal', 15)]:
            with self.assertRaisesRegex(ValueError, 'custody'):
                subject._success(dict(clean, **{key: value}))

    def test_collector_persists_drift_as_failed_custody(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source = root / 'input'; source.write_text('original')
            runfile = root / 'run.json'; runfile.write_text('{}')
            output = root / 'results'; output.mkdir()
            bound = {'run': {'suite': {'name': 'native-prerequisites'}, 'outputs': {'results': 'results'}},
                     'path': runfile, 'sha256': subject._hash(runfile),
                     'store_root': root, 'bundle_root': root,
                     'records': [snapshot_input(root, 'input'), snapshot_input(root, 'run.json')]}
            def mutate(*_):
                source.write_text('modified')
                return {'errors': []}
            with patch.object(subject, 'validate_production'), patch.object(subject, '_selected'), \
                 patch.object(subject, 'validate_runtime_readiness'), \
                 patch.object(subject, '_collect_native', side_effect=mutate):
                with self.assertRaisesRegex(ValueError, 'changed'):
                    subject.collect(bound, 'native', output)
            custody = json.loads((output / 'custody.json').read_text())
            self.assertTrue(custody['errors'])
            self.assertIsNone(custody['finalization'])


class PrerequisiteLaunchEnvironment(unittest.TestCase):
    def exercise(self, root, *, drift=False, child_failure=False):
        from contextlib import ExitStack
        from oryn_validation import wpt, bundle_ownership
        runfile = root / 'run.json'; runfile.write_text('{}')
        output = root / 'results'
        bound = {'run': {'suite': {'name': 'native-prerequisites'}, 'outputs': {'results': 'results'},
                         'run_id': 'test', 'build': {'build_id': 'test'}},
                 'path': runfile, 'sha256': subject._hash(runfile), 'store_root': root, 'bundle_root': root,
                 'records': [], 'assets': {'python': root / 'python'}}
        overlay = {'PYTHONHOME': '/verified/python', 'PYTHONNOUSERSITE': '1'}
        def child(*args, **kwargs):
            self.assertEqual(kwargs['env']['PYTHONHOME'], '/verified/python')
            self.assertEqual(kwargs['env']['PYTHONNOUSERSITE'], '1')
            self.assertNotIn('PYTHONSTARTUP', kwargs['env'])
            self.assertNotIn('DYLD_INSERT_LIBRARIES', kwargs['env'])
            self.assertNotIn('ORYN_WPT_INHERITED', kwargs['env'])
            self.assertEqual(kwargs['env']['PYTHONPATH'], str(subject.SOURCE_ROOT / 'tools/validation'))
            (output / 'result.json').write_text('{"errors": []}')
            if child_failure:
                raise RuntimeError('primary child failure')
            return {'returncode': 0, 'timed_out': False, 'error': None, 'cleanup_complete': True,
                    'cleanup_errors': [], 'forced_cleanup': False, 'cancellation_signal': None}
        with ExitStack() as stack:
            stack.enter_context(patch.object(subject, 'load_inputs', return_value=bound))
            stack.enter_context(patch.object(subject, 'validate_production'))
            stack.enter_context(patch.object(subject, '_selected', return_value=(None, None, {})))
            owner = stack.enter_context(patch.object(bundle_ownership, 'registered_bundle_owner', return_value='sealed-build'))
            environment = stack.enter_context(patch.object(wpt, 'python_environment',
                side_effect=[overlay, ValueError('system dependency drift') if drift else overlay]))
            stack.enter_context(patch.object(subject, 'InputLease'))
            resources = stack.enter_context(patch.object(subject, 'ResourceLease'))
            stack.enter_context(patch.object(subject, 'run_owned', side_effect=child))
            stack.enter_context(patch.dict(subject.os.environ, {'PYTHONHOME': '/hostile',
                'PYTHONPATH': '/hostile', 'PYTHONSTARTUP': '/hostile',
                'DYLD_INSERT_LIBRARIES': '/hostile.dylib', 'ORYN_WPT_INHERITED': '1'}))
            if child_failure:
                with self.assertRaisesRegex(RuntimeError, 'primary child failure'):
                    subject.execute(runfile, store_root=root, bundle_root=root, stage='native')
            elif drift:
                with self.assertRaisesRegex(ValueError, 'system dependency drift'):
                    subject.execute(runfile, store_root=root, bundle_root=root, stage='native')
            else:
                self.assertEqual(subject.execute(runfile, store_root=root, bundle_root=root, stage='native'), {'errors': []})
            self.assertEqual(environment.call_count, 2)
            owner.assert_called_once_with(bound)
            self.assertEqual(resources.call_args.kwargs['objects'], ['sealed-build'])
        return json.loads((output / 'execution.json').read_text())

    def test_verified_environment_replaces_inherited_overrides_and_rechecks_after_settlement(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = self.exercise(Path(temporary).resolve())
            self.assertEqual(receipt['errors'], [])

    def test_final_system_dependency_drift_rejects_successful_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = self.exercise(Path(temporary).resolve(), drift=True)
            self.assertIn('system dependency drift', receipt['errors'][0])

    def test_final_environment_failure_preserves_primary_child_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = self.exercise(Path(temporary).resolve(), drift=True, child_failure=True)
            self.assertIn('primary child failure', receipt['errors'][0])
            self.assertIn('system dependency drift', receipt['errors'][1])


if __name__ == '__main__':
    unittest.main()

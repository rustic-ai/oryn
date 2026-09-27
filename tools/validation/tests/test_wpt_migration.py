import tempfile
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from oryn_validation.admission import require_migrated_gates
from oryn_validation.wpt import _check_config, plan_command, EXECUTION_POLICY, execution_policy


class TypedWptTests(unittest.TestCase):
    def test_typed_admission_rejection_cannot_fall_back_to_legacy_candidate(self):
        from oryn_validation.admission import legacy_algorithms
        legacy_algorithms()
        from oryn_wptrunner import provenance
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory) / 'run.json'
            run.write_text('{"kind":"run","schema_version":1}')
            with patch('oryn_validation.typed_provenance.load_contract', side_effect=ValueError('typed production rejected')), \
                    patch.object(provenance, '_candidate_scope') as legacy:
                with self.assertRaisesRegex(ValueError, 'typed production rejected'):
                    provenance.load_contract(run)
                legacy.assert_not_called()

    def test_reviewed_source_ports_no_longer_have_static_migration_blocks(self):
        for collection in ('core', 'canaries', 'placement'):
            require_migrated_gates(collection)

    def test_unbound_or_absolute_config_cannot_enter_runtime_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / 'runner.ini'
            config.write_text('[manifest:official]\ntests=/tmp/undeclared\nmetadata=meta\nmanifest=manifest.json\nurl_base=/\n')
            with self.assertRaisesRegex(ValueError, 'relocatable'):
                _check_config(config, {})
            config.write_text('[manifest:official]\ntests=undeclared\nmetadata=meta\nmanifest=manifest.json\nurl_base=/\n')
            with self.assertRaisesRegex(ValueError, 'absent from typed closure'):
                _check_config(config, {})

    def test_command_keeps_original_upstream_execution_and_normal_tls(self):
        assets = {name: Path('/store') / name for name in
                  ('python', 'runtime', 'driver', 'runner_config', 'include', 'wpt_checkout', 'core_metadata', 'core_manifest')}
        bound = dict(assets=assets, run={'workers': 6}, path=Path('/store/run.json'),
                     store_root=Path('/store'), bundle_root=Path('/store/build'))
        fixture = {'tls': {name: {'path': name + '.pem'} for name in ('ca', 'certificate', 'key')}}
        selected = dict(fixture=fixture, fixture_path=Path('/store/config/fixture.json'))
        command = plan_command(bound, selected, '/store/run')
        for flag, value in (('--processes', '6'), ('--repeat', '1'), ('--rerun', '1'),
                            ('--retry-unexpected', '0'), ('--ssl-type', 'pregenerated')):
            self.assertEqual(command[command.index(flag) + 1], value)
        self.assertIn('--no-manifest-update', command)
        self.assertIn('--no-manifest-download', command)
        for flag, role in (('--tests', 'wpt_checkout'), ('--metadata', 'core_metadata'), ('--manifest', 'core_manifest')):
            self.assertEqual(command[command.index(flag) + 1], str(assets[role]))
        self.assertEqual(command[command.index('--ca-cert-path') + 1], '/store/config/ca.pem')

    def test_original_deadline_is_immutable_and_controls_remain_serial_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'policy.json'
            path.write_text(json.dumps(EXECUTION_POLICY))
            bound = {'assets': {'execution_policy': path}}
            self.assertEqual(execution_policy(bound, 'core')['timeout_seconds'], 345600)
            self.assertEqual(execution_policy(bound, 'canaries')['timeout_seconds'], 43200)
            with self.assertRaisesRegex(ValueError, 'override'):
                execution_policy(bound, 'core', 43200)
            path.write_text(json.dumps(dict(EXECUTION_POLICY, core_timeout_seconds=1)))
            with self.assertRaisesRegex(ValueError, 'reviewed original budgets'):
                execution_policy(bound, 'core')

    def test_metadata_must_belong_to_the_selected_collection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            config = root / 'runner.ini'
            assets = {name: root / name for name in ('wpt_checkout', 'core_manifest', 'core_metadata', 'foreign_metadata')}
            config.write_text('[manifest:official]\ntests=wpt_checkout\nmetadata=foreign_metadata\nmanifest=core_manifest\nurl_base=/\n')
            with self.assertRaisesRegex(ValueError, 'collection metadata'):
                _check_config(config, assets)

    def test_python_environment_must_join_exact_run_asset_roles(self):
        from oryn_validation.wpt import python_environment
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            identity = {'environment': {'path': 'env'}, 'python': {'path': 'env/bin/python'},
                        'dependency_lock': {'path': 'env/dependency-lock.json'}, 'probe': {'path': 'env-probe'}}
            path = root / 'identity.json'
            path.write_text(json.dumps(identity))
            bound = {'store_root': root, 'assets': {'python_environment_identity': path},
                     'run': {'assets': {'python_environment': identity['environment'], 'python': identity['python'],
                                      'python_dependency_lock': identity['dependency_lock'], 'probe': identity['probe']}}}
            with patch('oryn_validation.environments.runtime_environment', return_value={'PYTHONHOME': str(root / 'env')}) as verify:
                self.assertEqual(python_environment(bound), {'PYTHONHOME': str(root / 'env')})
                verify.assert_called_once_with(identity, root)
                bound['run']['assets']['python'] = {'path': 'legacy/bin/python'}
                with self.assertRaisesRegex(ValueError, 'substitutes declared run input'):
                    python_environment(bound)
                self.assertEqual(verify.call_count, 1)


class RelayEnvironmentBindings(unittest.TestCase):
    def test_relay_identity_cannot_substitute_or_omit_current_inputs(self):
        from oryn_validation import fixture_relay, wpt
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = {'path': 'relay/bin/socat'}
            environment = {'path': 'relay'}
            identity_record = {'path': 'relay-identity.json'}
            identity = {'executable': executable, 'environment': environment}
            path = root / 'relay-identity.json'; path.write_text(json.dumps(identity))
            roles = {'fixture_relay': executable, 'fixture_relay_environment': environment,
                     'fixture_relay_identity': identity_record}
            bound = {'store_root': root, 'assets': {'fixture_relay_identity': path,
                'fixture_relay': root / 'relay/bin/socat', 'fixture_relay_environment': root / 'relay'},
                'run': {'assets': roles, 'immutable_inputs': list(roles.values())}}
            with patch.object(fixture_relay, 'verify_relay', return_value={'valid': True}) as verify:
                self.assertEqual(wpt.relay_environment(bound), {'valid': True})
                verify.assert_called_once_with(identity, root)
                bound['run']['immutable_inputs'].pop()
                with self.assertRaisesRegex(ValueError, 'absent from immutable'):
                    wpt.relay_environment(bound)
                bound['run']['immutable_inputs'].append(identity_record)
                bound['run']['assets']['fixture_relay'] = {'path': 'other/socat'}
                with self.assertRaisesRegex(ValueError, 'substitutes input'):
                    wpt.relay_environment(bound)


class UpstreamFailurePreservation(unittest.TestCase):
    def test_parser_failure_keeps_returncode_and_original_log_when_tls_is_absent(self):
        from oryn_validation.wpt import verify_upstream_completion
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for code in (2, 1, 64):
                failure = ValueError('missing TLS activation')
                with self.subTest(code=code), self.assertRaisesRegex(RuntimeError, 'exit status ' + str(code)) as caught:
                    verify_upstream_completion({'returncode': code, 'error': None, 'timed_out': False}, output,
                                               lambda: (_ for _ in ()).throw(failure))
                self.assertIn(str(output / 'process/stderr.log'), str(caught.exception))
                self.assertIn('missing TLS activation', str(caught.exception))
                self.assertIs(caught.exception.__cause__, failure)

    def test_expected_canary_exit_one_keeps_tls_and_later_outcome_validation(self):
        from oryn_validation.wpt import verify_upstream_completion
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for name in ('mozlog.jsonl', 'wptreport.json'):
                (output / name).write_text('{}')
            process = {'returncode': 1, 'error': None, 'timed_out': False}
            verify_upstream_completion(process, output, lambda: None)
            failure = ValueError('bad TLS receipt')
            with self.assertRaises(ValueError) as caught:
                verify_upstream_completion(process, output, lambda: (_ for _ in ()).throw(failure))
            self.assertIs(caught.exception, failure)

    def test_abnormal_exit_is_not_accepted_even_with_logs_and_valid_tls(self):
        from oryn_validation.wpt import verify_upstream_completion
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            for name in ('mozlog.jsonl', 'wptreport.json'):
                (output / name).write_text('{}')
            with self.assertRaisesRegex(RuntimeError, 'exit status 2'):
                verify_upstream_completion({'returncode': 2, 'error': None, 'timed_out': False}, output, lambda: None)

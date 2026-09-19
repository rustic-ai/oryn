"""Integrated evidence must preserve denominators, provenance and oracle checks."""
import copy
import importlib.util
from pathlib import Path
import unittest
from unittest import mock
import tempfile

spec = importlib.util.spec_from_file_location("integrated_outcomes", Path(__file__).with_name("run-oil-integrated-outcomes.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class IntegratedOutcomeTests(unittest.TestCase):
    def test_timeout_racing_group_exit_keeps_timeout_and_reaps_child(self):
        process = mock.Mock(pid=43210)
        process.wait.side_effect = [runner.subprocess.TimeoutExpired(['runner'], 1), 0, 0]
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(runner.subprocess, 'Popen', return_value=process), \
                mock.patch.object(runner.os, 'killpg', side_effect=ProcessLookupError):
            result = runner.execute(['runner'], Path(directory) / 'run', {}, 1)
        self.assertTrue(result['timed_out'])
        self.assertEqual(result['returncode'], 0)
        self.assertTrue(result['descendants_clean'])
        self.assertEqual(process.wait.call_count, 3)

    def test_observed_descendant_leak_is_not_erased_by_exit_before_kill(self):
        process = mock.Mock(pid=43211)
        process.wait.return_value = 0
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(runner.subprocess, 'Popen', return_value=process), \
                mock.patch.object(runner.os, 'killpg', side_effect=[None, ProcessLookupError]):
            result = runner.execute(['runner'], Path(directory) / 'run', {}, 1)
        self.assertFalse(result['descendants_clean'])
        self.assertFalse(result['timed_out'])
        self.assertEqual(process.wait.call_count, 2)

    def test_candidate_manifest_uses_current_worker_contract_not_other_wire_versions(self):
        protocols = dict(worker=14, context=45, agent=15)
        source = "c" * 64
        runner.validate_candidate_manifest(dict(source_tree_sha256=source, context_service_protocol_version=45, agent_ipc_protocol_version=15, protocol_version=14), source, protocols)
        for protocol in (11, 45, 15, True, "14", None):
            with self.subTest(protocol=protocol), self.assertRaises(ValueError):
                runner.validate_candidate_manifest(dict(source_tree_sha256=source, context_service_protocol_version=45, agent_ipc_protocol_version=15, protocol_version=protocol), source, protocols)
        for manifest in (None, [], {}, dict(source_tree_sha256="d" * 64, protocol_version=14)):
            with self.subTest(manifest=manifest), self.assertRaises(ValueError):
                runner.validate_candidate_manifest(manifest, source, protocols)
        valid = dict(source_tree_sha256=source, protocol_version=14, context_service_protocol_version=45, agent_ipc_protocol_version=15)
        for field in ('context_service_protocol_version', 'agent_ipc_protocol_version'):
            for value in (None, True, '45', valid[field]+1):
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    runner.validate_candidate_manifest(dict(valid, **{field: value}), source, protocols)
            missing=dict(valid);missing.pop(field)
            with self.assertRaises(ValueError):
                runner.validate_candidate_manifest(missing, source, protocols)
        # A subsequent source bump changes the accepted candidate identity.
        runner.validate_candidate_manifest(dict(source_tree_sha256=source, context_service_protocol_version=45, agent_ipc_protocol_version=15, protocol_version=16), source, dict(protocols, worker=16))

    def test_candidate_identity_covers_manifest_and_all_execution_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ("parent", "worker", "framework", "manifest")]
            for path in paths:
                path.write_bytes(path.name.encode())
            expected = runner.candidate_identity(*paths, "source-a")
            fields = ("parent_sha256", "worker_sha256", "framework_build_sha256", "worker_manifest_sha256")
            for path, field in zip(paths, fields):
                original = path.read_bytes()
                path.write_bytes(original + b"changed")
                changed = runner.candidate_identity(*paths, "source-a")
                self.assertNotEqual(changed[field], expected[field])
                self.assertEqual({key for key in expected if expected[key] != changed[key]}, {field})
                path.write_bytes(original)
            self.assertNotEqual(runner.candidate_identity(*paths, "source-b"), expected)

    def setUp(self):
        self.candidate = dict(parent_sha256="a" * 64, worker_sha256="b" * 64, source_tree_sha256="c" * 64,
                              framework_build_sha256="f" * 64)
        self.report = dict(self.candidate, source_matches_signed_build=True,
                           artifact_identities_unchanged=True, fixture_cleanup={"returncode": 0, "forced": False},
                           planned=3, passed=3, status="passed", records=[
                               dict(case="one", sample=sample, passed=True, errors=[], timed_out=False, returncode=0,
                                    final_database_verified=True) for sample in (1, 2, 3)])

    def errors(self, report):
        return runner.validate_summary(report, self.candidate, ["one"], 3, "research")

    def test_complete_regression_report_is_accepted_without_claiming_full_contract(self):
        self.assertEqual(self.errors(self.report), [])

    def test_duplicate_cannot_replace_missing_sample(self):
        changed = copy.deepcopy(self.report)
        changed["records"][2] = copy.deepcopy(changed["records"][1])
        self.assertIn("missing, duplicate or unexpected planned attempts", self.errors(changed))

    def test_candidate_cleanup_oracle_and_failed_attempt_controls(self):
        mutations = [
            lambda report: report.update(parent_sha256="d" * 64),
            lambda report: report.update(source_matches_signed_build=False),
            lambda report: report.update(fixture_cleanup={"returncode": 0, "forced": True}),
            lambda report: report["records"][0].update(final_database_verified=False),
            lambda report: report["records"][0].update(passed=False),
            lambda report: report["records"][0].update(sample=True),
        ]
        for mutation in mutations:
            changed = copy.deepcopy(self.report)
            mutation(changed)
            self.assertTrue(self.errors(changed))

    def test_fixture_cleanup_requires_typed_exit_and_forced_fields(self):
        for cleanup in ({"returncode": False, "forced": False},
                        {"returncode": 0, "forced": 0},
                        {"returncode": 0.0, "forced": False},
                        {"returncode": 0}, None, 1):
            changed = copy.deepcopy(self.report)
            changed["fixture_cleanup"] = cleanup
            self.assertIn("fixture cleanup was not successful", self.errors(changed))
        for cleanup in (True, {"returncode": 0, "forced": False}):
            changed = copy.deepcopy(self.report)
            changed["fixture_cleanup"] = cleanup
            self.assertEqual(self.errors(changed), [])

    def test_framework_summary_requires_each_owned_process_cleanup(self):
        report = copy.deepcopy(self.report)
        report['build_manifest_sha256'] = self.candidate['framework_build_sha256']
        for record in report['records']:
            record['framework'] = record.pop('case')
            record.update(returncode=0, process_cleanup=dict(parent_reaped=True, descendants_clean=True, forced=False, errors=[]))
        self.assertEqual(runner.validate_summary(report, self.candidate, ['one'], 3, 'framework'), [])
        for cleanup in (None, {}, dict(parent_reaped=True, descendants_clean=False, forced=True, errors=[])):
            changed = copy.deepcopy(report)
            changed['records'][0]['process_cleanup'] = cleanup
            self.assertTrue(runner.validate_summary(changed, self.candidate, ['one'], 3, 'framework'))

        for digest in (None, "e" * 64):
            changed = copy.deepcopy(report)
            changed['build_manifest_sha256'] = digest
            self.assertIn('candidate mismatch: framework_build_sha256',
                          runner.validate_summary(changed, self.candidate, ['one'], 3, 'framework'))

    def test_success_requires_explicit_artifact_and_process_evidence(self):
        changed = copy.deepcopy(self.report)
        del changed['artifact_identities_unchanged']
        self.assertTrue(self.errors(changed))
        for field, values in {'returncode': [None, False, 1, '0'],
                              'timed_out': [None, 0, True],
                              'errors': [None, '', ['failure']]}.items():
            for value in values:
                with self.subTest(field=field, value=value):
                    changed = copy.deepcopy(self.report)
                    changed['records'][0][field] = value
                    self.assertTrue(self.errors(changed))
            changed = copy.deepcopy(self.report)
            del changed['records'][0][field]
            self.assertTrue(self.errors(changed))

    def test_forms_uses_its_existing_final_receiver_oracle(self):
        changed = copy.deepcopy(self.report)
        for record in changed["records"]:
            del record["final_database_verified"]
            record["final_oracle_verified"] = True
        self.assertEqual(runner.validate_summary(changed, self.candidate, ["one"], 3, "forms"), [])
        self.assertTrue(self.errors(changed))

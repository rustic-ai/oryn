"""Profile rejection regression sources; run after the global generation gate."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("v8_profile", ROOT / "scripts/validate-v8-runtime-profile.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

class ProfileValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.profile = {
            "schema_version": 6, "profile": "core-full", "target": "aarch64-apple-darwin",
            "v8_version": "150.4.0", "v8_revision": "ac1e23989121713ca642f6650b34deff7b686896",
            "rusty_v8_revision": "5c15a6995c9bb4bacd3e341b59fff32c909c80bf",
            "icu_revision": "ee5f27adc28bd3f15b2c293f726d14d2e336cbd5",
            "v8_heap_sandbox": True, "pointer_compression": True,
            "gn_args": "v8_drumbrake_bounds_checks=true v8_enable_sandbox=true v8_enable_pointer_compression=true",
        }
        self.profile["v8_regression_manifest_sha256"] = MODULE.digest(ROOT / "patches/v8/tests/manifest.json")
        for field, name in MODULE.PATCHES.items():
            self.profile[field] = MODULE.digest(ROOT / "patches/v8" / (name + ".patch"))
        for name in MODULE.ARTIFACTS:
            path = Path(self.temp.name) / name
            path.write_bytes(name.encode())
            self.profile[name + "_path"] = str(path)
            self.profile[name + "_size_bytes"] = path.stat().st_size
            self.profile[name + "_sha256"] = MODULE.digest(path)

        import rusty_v8_source_provenance
        self.profile.update(rusty_v8_source_provenance.validate(ROOT))
        self.profile["rusty_v8_archive_compatibility"] = dict(cpp_changes=False, crate_version="150.4.0",
            archive_sha256=self.profile["archive_sha256"], binding_sha256=self.profile["binding_sha256"])

        compiler = Path(self.temp.name) / "compiler"
        compiler.write_bytes(b"pinned compiler")
        self.bridge = {"schema_version": 1, "target": self.profile["target"],
            "libcxx_namespace": "__Cr", "libcxx_abi_version": 2,
            "stdlib_linkage": "included-in-rusty-v8", "flags": ["-nostdinc++"],
            "compiler": str(compiler), "inputs": [{"path": str(compiler),
                "sha256": MODULE.digest(compiler), "size_bytes": compiler.stat().st_size}]}
        for key in ("archive", "binding"):
            self.bridge[key] = {"sha256": self.profile[key + "_sha256"],
                                "size_bytes": self.profile[key + "_size_bytes"]}
        self.write_bridge()

    def write_bridge(self):
        path = Path(self.profile["bridge_abi_path"])
        path.write_text(json.dumps(self.bridge))
        self.profile["bridge_abi_size_bytes"] = path.stat().st_size
        self.profile["bridge_abi_sha256"] = MODULE.digest(path)

    def test_bridge_selected_archive_and_retained_compiler_rejected_on_drift(self):
        self.bridge["archive"]["sha256"] = "0" * 64
        self.write_bridge()
        with self.assertRaisesRegex(ValueError, "bridge selected artifact mismatch"):
            MODULE.validate(self.profile, ROOT)
        self.bridge["archive"]["sha256"] = self.profile["archive_sha256"]
        self.write_bridge()
        Path(self.bridge["compiler"]).write_bytes(b"changed compiler")
        with self.assertRaisesRegex(ValueError, "bridge retained input drift"):
            MODULE.validate(self.profile, ROOT)

    def test_current_inputs_and_exact_target(self):
        MODULE.validate(self.profile, ROOT, "core-full", "aarch64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "target mismatch"):
            MODULE.validate(self.profile, ROOT, "core-full", "x86_64-unknown-linux-gnu")

    def test_relocated_source_manifest_requires_explicit_opt_in_and_identical_selected_bytes(self):
        candidate = copy.deepcopy(self.profile)
        retained = Path(self.temp.name) / 'retained-source-manifest.json'
        retained.write_bytes(Path(candidate['rusty_v8_source_manifest_path']).read_bytes())
        candidate['rusty_v8_source_manifest_path'] = str(retained)
        with self.assertRaisesRegex(ValueError, 'Rust V8 selected source mismatch'):
            MODULE.validate(candidate, ROOT)
        MODULE.validate(candidate, ROOT, allow_relocated_source_manifest=True)
        # Even internally consistent hashes on a different retained manifest
        # cannot replace the current selected source manifest.
        retained.write_bytes(retained.read_bytes() + b' ')
        candidate['rusty_v8_source_manifest_size_bytes'] = retained.stat().st_size
        candidate['rusty_v8_source_manifest_sha256'] = MODULE.digest(retained)
        with self.assertRaisesRegex(ValueError, 'Rust V8 selected source mismatch'):
            MODULE.validate(candidate, ROOT, allow_relocated_source_manifest=True)

    def test_relocation_does_not_relax_selected_source_tree_or_patch_checks(self):
        for field in ('rusty_v8_source_tree_sha256', 'rusty_v8_weak_teardown_patch_sha256',
                      'rusty_v8_patch_metadata_sha256'):
            candidate = copy.deepcopy(self.profile)
            candidate[field] = '0' * 64
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'Rust V8 selected source mismatch'):
                MODULE.validate(candidate, ROOT, allow_relocated_source_manifest=True)

    def test_each_missing_or_stale_patch_is_rejected(self):
        for field in MODULE.PATCHES:
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.profile)
                candidate[field] = "0" * 64
                with self.assertRaisesRegex(ValueError, "patch mismatch"):
                    MODULE.validate(candidate, ROOT)

    def test_missing_reattachment_and_regression_provenance_rejected(self):
        for field in ("frame_context_reattachment_patch_sha256", "html_constructor_deferred_prototype_patch_sha256", "v8_regression_manifest_sha256"):
            candidate = copy.deepcopy(self.profile)
            del candidate[field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                MODULE.validate(candidate, ROOT)

    def test_rust_source_and_generated_binding_compatibility_are_mandatory(self):
        for field in ("rusty_v8_source_tree_sha256", "rusty_v8_source_manifest_sha256",
                      "rusty_v8_weak_teardown_patch_sha256", "rusty_v8_patch_metadata_sha256"):
            candidate = copy.deepcopy(self.profile)
            candidate[field] = "0" * 64
            with self.subTest(field=field), self.assertRaises(ValueError):
                MODULE.validate(candidate, ROOT)
        candidate = copy.deepcopy(self.profile)
        candidate["rusty_v8_archive_compatibility"]["binding_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "compatibility mismatch"):
            MODULE.validate(candidate, ROOT)

    def test_modified_archive_rejected_even_if_length_unchanged(self):
        path = Path(self.profile["archive_path"])
        path.write_bytes(b"x" * path.stat().st_size)
        with self.assertRaisesRegex(ValueError, "digest mismatch: archive"):
            MODULE.validate(self.profile, ROOT)

    def test_old_schema_and_conflicting_bounds_options_rejected(self):
        candidate = copy.deepcopy(self.profile)
        candidate["schema_version"] = 1
        with self.assertRaisesRegex(ValueError, "schema 6"):
            MODULE.validate(candidate, ROOT)
        candidate = copy.deepcopy(self.profile)
        candidate["gn_args"] += " v8_drumbrake_bounds_checks=false"
        with self.assertRaisesRegex(ValueError, "conflicting engine option"):
            MODULE.validate(candidate, ROOT)


class TypedWorkerValidatorShellTests(unittest.TestCase):
    def test_legacy_and_typed_validator_arguments_under_system_bash_nounset(self):
        import re
        import subprocess
        source = (ROOT / 'scripts/build-sandboxed-worker.sh').read_text()
        for variable in ('profile_validation', 'runtime_validation'):
            block = re.search(r'^' + variable + r'=\(.*?^"\$\{' + variable + r'\[@\]\}"$',
                              source, re.M | re.S).group(0)
            setup = """set -euo pipefail
repo_root='/source with spaces'
v8_profile_manifest='/profile with spaces'
runtime_file='/runtime with spaces'
RUSTY_V8_SRC_BINDING_PATH='/binding with spaces'
RUSTY_V8_ARCHIVE='/archive with spaces'
typed_source_identity="${1:-}"
function rustc() { printf 'host: aarch64-apple-darwin\n'; }
function python3() { printf '%s\n' "$@"; }
"""
            for typed in ('', '/identity'):
                with self.subTest(variable=variable, typed=bool(typed)):
                    result = subprocess.run(['/bin/bash', '-c', setup + block, '--', typed],
                                            check=True, capture_output=True, text=True)
                    arguments = result.stdout.splitlines()
                    self.assertIn('/profile with spaces', arguments)
                    self.assertEqual('--allow-relocated-source-manifest' in arguments, bool(typed))


class RuntimeProvenanceTests(unittest.TestCase):
    """Receipt custody and owned-producer behavior, with no real build/process."""
    def setUp(self):
        from types import SimpleNamespace
        from unittest import mock
        self.mock = mock
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        spec = importlib.util.spec_from_file_location('runtime_provenance_tested', ROOT / 'scripts/runtime_provenance.py')
        self.provenance = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.provenance)
        spec = importlib.util.spec_from_file_location('runtime_provenance_producer_tested', ROOT / 'scripts/prepare-runtime-provenance.py')
        self.producer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.producer)
        self.producer.ROOT = self.root
        scripts = self.root / 'scripts'
        scripts.mkdir()
        names = ('runtime_provenance.py', 'prepare-runtime-provenance.py', 'build-v8-runtime-profile.sh',
                 'validate-v8-runtime-profile.py', 'rusty_v8_source_provenance.py', 'generation_gate.py', 'custody.py')
        for name in names:
            (scripts / name).write_text(name)
        source_rows = [dict(path='scripts/' + name, sha256=self.provenance.digest(scripts / name)) for name in names]
        self.gate = self.root / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-49-2026-09-18.json'
        self.gate.parent.mkdir(parents=True)
        artifacts = self.root / 'artifacts'
        artifacts.mkdir()
        self.lease = artifacts / 'lease.json'
        self.write(self.lease, dict(kind='repair49_whole_source_and_review_lease_inputs',
                                  expected_declaration_path=str(self.gate.relative_to(self.root)),
                                  sources=source_rows, source_count=len(source_rows)))
        self.write(self.gate, dict(requirements={'Q-05': dict(implementation_sources=source_rows, test_sources=[])},
                                  review_sources=[dict(path=str(self.lease.relative_to(self.root)),
                                                       sha256=self.provenance.digest(self.lease))]))
        self.gate_record = self.provenance.record(self.gate)
        self.lease_record = self.provenance.record(self.lease)
        self.profile_dir = artifacts / 'profile'
        self.profile_dir.mkdir()
        self.profile_path = self.profile_dir / 'core-full-manifest.json'
        self.write(self.profile_path, dict(schema_version=6, profile='core-full'))
        self.worker = self.profile_dir / 'worker'
        self.worker.write_bytes(b'exact worker')
        self.abi = self.profile_dir / 'abi'
        self.abi.write_bytes(b'exact ABI')
        self.outputs = [self.provenance.record(path) for path in (self.profile_path, self.worker, self.abi)]
        self.profile_record = self.outputs[0]
        self.producers = [self.provenance.record(scripts / name) for name in names[:3] + ('custody.py',)]
        self.start = artifacts / 'start.json'
        self.command = ['bash', str(scripts / 'build-v8-runtime-profile.sh'), 'core-full', str(self.profile_dir)]
        self.write(self.start, self.start_value(self.command))
        self.log = artifacts / 'build.log'
        self.log.write_text('actual owned output')
        self.build_path = artifacts / 'build-receipt.json'
        self.build = dict(schema_version=1, kind='oryn_owned_runtime_build', generation_gate=self.gate_record,
                          whole_source_lease=self.lease_record, command=self.command, profile_name='core-full',
                          output_directory=str(self.profile_dir), process=self.process(), before=self.check(),
                          after=self.check(), success=True, producer_sources=self.producers, inputs=[],
                          profile=self.profile_record, outputs=self.outputs, engine_reuse=None,
                          log=self.provenance.record(self.log), start_receipt=self.provenance.record(self.start))
        self.build['custody_source'] = self.producers[-1]
        self.build['environment'] = dict(ORYN_GENERATION_GATE=str(self.gate),
            ORYN_GENERATION_GATE_SHA256=self.gate_record['sha256'],
            ORYN_REPAIR49_SUPERVISED_GROUP='runtime-profile-build',
            GN=str(self.root / 'gn'), NINJA=str(self.root / 'ninja'),
            CLANG_BASE_PATH=str(self.root / 'clang'), LIBCLANG_PATH=str(self.root / 'libclang'),
            SDKROOT=str(self.root / 'sdk'))
        for path in (self.root / 'gn', self.root / 'ninja', self.root / 'clang/bin/clang',
                     self.root / 'libclang/libclang.dylib', self.root / 'sdk/SDKSettings.json'):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'pinned tool')
            self.build['inputs'].append(self.provenance.record(path))
        self.receipt_path = artifacts / 'runtime-provenance.json'
        self.refresh()
        self.Namespace = SimpleNamespace

    def write(self, path, value):
        path.write_text(json.dumps(value))

    def process(self, code=0):
        return dict(returncode=code, pid=321, group_settled=True, execution_error=None,
                    cleanup_errors=[], events=[dict(event='started', pid=321, group_id=321)])

    def check(self):
        return dict(errors=[], declaration_sha256=self.gate_record['sha256'],
                    lease_sha256=self.lease_record['sha256'], source_count=7)

    def start_value(self, command):
        return dict(pid=321, group_id=321, command=command, generation_gate=str(self.gate),
                    generation_gate_sha256=self.gate_record['sha256'], supervision='runtime-profile-build')

    def refresh(self):
        self.write(self.build_path, self.build)
        self.receipt = dict(schema_version=1, kind='oryn_runtime_provenance', status='complete',
                            generation_gate=self.gate_record, whole_source_lease=self.lease_record,
                            profile=self.profile_record, build_receipt=self.provenance.record(self.build_path),
                            engine_reuse=None, inputs=self.provenance.merge_records(self.outputs, self.producers))
        self.write(self.receipt_path, self.receipt)

    def validate(self):
        with self.mock.patch.object(self.provenance, 'profile_inputs', return_value=(dict(profile='core-full'), self.outputs)):
            return self.provenance.validate_runtime_provenance(
                self.receipt_path, self.provenance.digest(self.receipt_path), root=self.root,
                gate_path=self.gate, gate_sha256=self.gate_record['sha256'], lease_path=self.lease,
                lease_sha256=self.lease_record['sha256'], profile_path=self.profile_path,
                profile_sha256=self.profile_record['sha256'])

    def test_complete_receipt_returns_original_runtime_and_producer_inputs(self):
        checked = self.validate()
        self.assertIn(self.provenance.record(self.worker), checked['records'])
        self.assertIn(self.provenance.record(self.start), checked['records'])
        self.assertIn(self.producers[0], checked['records'])

    def test_failed_boolean_exit_and_unsettled_builds_are_rejected(self):
        for key, value in (('returncode', 101), ('returncode', False), ('group_settled', False),
                           ('execution_error', 'launch failed'), ('cleanup_errors', ['live child'])):
            with self.subTest(key=key, value=value):
                self.build['process'] = self.process()
                self.build['process'][key] = value
                self.refresh()
                with self.assertRaisesRegex(ValueError, 'failed or unsettled'):
                    self.validate()

    def test_wrong_generation_and_lease_identity_are_rejected(self):
        for field in ('generation_gate', 'whole_source_lease'):
            self.refresh()
            self.receipt[field] = dict(self.receipt[field], sha256='0' * 64)
            self.write(self.receipt_path, self.receipt)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'generation mismatch'):
                self.validate()

    def test_source_custody_population_and_failure_are_rejected(self):
        for change in ({'source_count': 6}, {'errors': ['drift']}, {'lease_sha256': '0' * 64}):
            self.build['after'] = dict(self.check(), **change)
            self.refresh()
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'source custody failed'):
                self.validate()

    def test_unrelated_start_receipt_is_rejected_despite_valid_hash(self):
        for field, value in (('command', ['true']), ('pid', 999), ('group_id', 999),
                             ('generation_gate_sha256', '0' * 64), ('supervision', 'unowned')):
            changed = dict(self.start_value(self.command), **{field: value})
            self.write(self.start, changed)
            self.build['start_receipt'] = self.provenance.record(self.start)
            self.refresh()
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'start identity'):
                self.validate()

    def test_stale_worker_abi_and_producer_bytes_are_rejected(self):
        for path in (self.worker, self.abi, Path(self.producers[0]['path'])):
            old = path.read_bytes()
            path.write_bytes(old + b' changed')
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.validate()
            path.write_bytes(old)

    def test_profile_and_custom_reuse_relabel_are_rejected(self):
        self.build['output_directory'] = str(self.root / 'artifacts/another')
        self.build['command'][-1] = self.build['output_directory']
        self.refresh()
        with self.assertRaisesRegex(ValueError, 'not an output'):
            self.validate()
        self.build['output_directory'] = str(self.profile_dir)
        self.build['command'][-1] = str(self.profile_dir)
        self.build['engine_reuse'] = {'claimed': 'old archive'}
        self.refresh()
        self.receipt['engine_reuse'] = self.build['engine_reuse']
        self.write(self.receipt_path, self.receipt)
        with self.assertRaisesRegex(ValueError, 'custom engine reuse'):
            self.validate()

    def test_json_parser_rejects_transient_second_read_substitution(self):
        original = Path.read_bytes
        expected = self.provenance.digest(self.gate)
        def substituted(path):
            return b'{"different":true}' if path == self.gate else original(path)
        with self.mock.patch.object(Path, 'read_bytes', substituted):
            with self.assertRaisesRegex(ValueError, 'parsed JSON bytes differ'):
                self.provenance.read_json(self.gate, expected)
        self.assertEqual(self.provenance.digest(self.gate), expected)

    def test_same_byte_symlink_retarget_is_rejected(self):
        twin = self.worker.with_name('worker-twin')
        twin.write_bytes(self.worker.read_bytes())
        alias = self.worker.with_name('worker-alias')
        alias.symlink_to(self.worker)
        captured = self.provenance.record(alias)
        alias.unlink()
        alias.symlink_to(twin)
        with self.assertRaisesRegex(ValueError, 'identity changed'):
            self.provenance.check_records([captured])

    def test_conflicting_asserted_hashes_are_never_replaced(self):
        item = self.provenance.record(self.worker)
        with self.assertRaisesRegex(ValueError, 'conflicting input assertion'):
            self.provenance.merge_records([item], [dict(item, sha256='0' * 64)])

    def producer_fixture(self, code=0, fail_profile=False):
        args = self.Namespace(gate=self.gate, gate_sha256=self.gate_record['sha256'],
            whole_source_lease=self.lease, whole_source_lease_sha256=self.lease_record['sha256'],
            custody=self.root / 'scripts/custody.py', custody_sha256=self.producers[-1]['sha256'],
            output_directory=self.root / 'artifacts/new-profile',
            evidence_directory=self.root / 'artifacts/new-evidence', profile='core-full',
            engine_source=self.root / 'target/engine', gn=self.root / 'gn', ninja=self.root / 'ninja',
            clang_base=self.root / 'clang', libclang=self.root / 'libclang', sdkroot=self.root / 'sdk')
        for path in (args.gn, args.ninja, args.clang_base / 'bin/clang',
                     args.libclang / 'libclang.dylib', args.sdkroot / 'SDKSettings.json'):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'pinned tool')
        owner = self
        class Admission:
            def __init__(self, path, expected):
                owner.assertEqual((path, expected), (owner.gate, owner.gate_record['sha256']))
            def check(self):
                return self
            def environment(self, environment):
                return dict(environment, ORYN_GENERATION_GATE=str(owner.gate),
                            ORYN_GENERATION_GATE_SHA256=owner.gate_record['sha256'])
        captured = {}
        def dispatch(command, environment, start_path):
            captured.update(command=command, environment=environment)
            self.write(start_path, self.start_value(command))
            if type(code) is int and code == 0:
                args.output_directory.mkdir()
                self.write(args.output_directory / 'core-full-manifest.json', {'schema_version': 6, 'profile': 'core-full'})
            return self.process(code)
        def configure_candidate(gate_path, gate_sha256, lease_path, lease_sha256):
            self.assertEqual((gate_path, gate_sha256, lease_path, lease_sha256),
                             (self.gate, self.gate_record['sha256'], self.lease, self.lease_record['sha256']))
            number = self.provenance.candidate_number(self.root, gate_path)
            return dict(number=number, supervisor_key=f'ORYN_REPAIR{number}_SUPERVISED_GROUP')
        custody = self.Namespace(ROOT=self.root, GATE=self.gate, check=lambda *unused: self.check(),
                                 configure_candidate=configure_candidate, run_owned_dispatch=dispatch)
        def loaded(path, name):
            return custody if path == args.custody else self.Namespace(GenerationLease=Admission) if path.name == 'generation_gate.py' else self.provenance
        def profile(path, expected, root):
            if fail_profile:
                raise ValueError('canonical profile rejected')
            return {'profile': 'core-full'}, [self.provenance.record(path, expected)]
        return args, captured, loaded, profile

    def test_recorder_bootstraps_without_runtime_receipt_and_publishes_after_success(self):
        args, captured, loaded, profile = self.producer_fixture()
        with self.mock.patch.object(self.producer, 'load', side_effect=loaded), self.mock.patch.object(
                self.provenance, 'profile_inputs', side_effect=profile):
            self.assertEqual(self.producer.run(args), 0)
        result = args.evidence_directory / 'runtime-provenance.json'
        self.assertTrue(result.is_file())
        receipt = json.loads(result.read_bytes())
        self.assertIsNone(receipt['engine_reuse'])
        self.assertEqual(captured['command'], ['bash', str(self.root / 'scripts/build-v8-runtime-profile.sh'),
                                             'core-full', str(args.output_directory)])
        self.assertEqual(captured['environment']['ORYN_GENERATION_GATE_SHA256'], self.gate_record['sha256'])
        self.assertNotIn('RUSTY_V8_ARCHIVE', captured['environment'])
        self.assertNotIn('V8_FROM_SOURCE', captured['environment'])

    def test_recorder_preserves_failure_without_minting_provenance(self):
        args, captured, loaded, profile = self.producer_fixture(code=101)
        with self.mock.patch.object(self.producer, 'load', side_effect=loaded), self.mock.patch.object(
                self.provenance, 'profile_inputs', side_effect=profile) as validator:
            self.assertEqual(self.producer.run(args), 101)
            validator.assert_not_called()
        self.assertFalse((args.evidence_directory / 'runtime-provenance.json').exists())
        receipt = json.loads((args.evidence_directory / 'build-receipt.json').read_bytes())
        self.assertFalse(receipt['success'])
        self.assertEqual(receipt['process']['returncode'], 101)
        self.assertTrue((args.evidence_directory / 'build.log').is_file())

    def test_recorder_success_exit_does_not_override_rejected_profile(self):
        args, captured, loaded, profile = self.producer_fixture(fail_profile=True)
        with self.mock.patch.object(self.producer, 'load', side_effect=loaded), self.mock.patch.object(
                self.provenance, 'profile_inputs', side_effect=profile):
            self.assertEqual(self.producer.run(args), 1)
        self.assertFalse((args.evidence_directory / 'runtime-provenance.json').exists())
        receipt = json.loads((args.evidence_directory / 'build-receipt.json').read_bytes())
        self.assertEqual(receipt['validation_error'], 'canonical profile rejected')

    def test_recorder_rejects_existing_output_before_dispatch(self):
        args, captured, loaded, profile = self.producer_fixture()
        args.output_directory.mkdir()
        with self.mock.patch.object(self.producer, 'load', side_effect=loaded):
            with self.assertRaisesRegex(ValueError, 'existing output'):
                self.producer.run(args)
        self.assertEqual(captured, {})


    def test_recorder_rejects_dotdot_alias_overlap_before_dispatch(self):
        args, captured, loaded, profile = self.producer_fixture()
        alias_parent = self.root / 'artifacts/alias-parent'
        alias_parent.mkdir()
        args.output_directory = alias_parent / '..' / 'new-evidence'
        with self.mock.patch.object(self.producer, 'load', side_effect=loaded):
            with self.assertRaisesRegex(ValueError, 'overlapping output'):
                self.producer.run(args)
        self.assertEqual(captured, {})


    def test_missing_owned_custody_or_tool_record_is_rejected(self):
        self.build['custody_source'] = self.provenance.record(self.log)
        self.refresh()
        with self.assertRaisesRegex(ValueError, 'custody source missing'):
            self.validate()
        self.build['custody_source'] = self.producers[-1]
        self.build['inputs'] = []
        self.refresh()
        with self.assertRaisesRegex(ValueError, 'tool/SDK input missing'):
            self.validate()

    def test_canonical_validator_cannot_replace_captured_runtime_bytes(self):
        fixture = ProfileValidationTests('test_current_inputs_and_exact_target')
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        output = Path(fixture.temp.name).resolve()
        path = output / 'core-full-manifest.json'
        self.write(path, fixture.profile)
        (output / 'worker-runtime-info.json').write_text('{}')
        (output / 'rusty-v8-source-inputs.json').write_text('{}')
        def mutate(profile, *unused):
            Path(profile['worker_path']).write_bytes(b'replaced after capture')
        with self.mock.patch.object(self.provenance, 'load_module', return_value=self.Namespace(validate=mutate)):
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                self.provenance.profile_inputs(path, self.provenance.digest(path), ROOT)




    def select_candidate(self, number):
        gate = json.loads(self.gate.read_bytes())
        self.gate = self.root / f'docs/oryn2/GENERATION-COMPLETE-REPAIR-{number}-2026-09-18.json'
        lease = json.loads(self.lease.read_bytes())
        lease['kind'] = f'repair{number}_whole_source_and_review_lease_inputs'
        lease['expected_declaration_path'] = str(self.gate.relative_to(self.root))
        self.write(self.lease, lease)
        self.lease_record = self.provenance.record(self.lease)
        gate['review_sources'] = [dict(path=str(self.lease.relative_to(self.root)), sha256=self.lease_record['sha256'])]
        self.write(self.gate, gate)
        self.gate_record = self.provenance.record(self.gate)
        self.build['generation_gate'], self.build['whole_source_lease'] = self.gate_record, self.lease_record
        self.build['before'], self.build['after'] = self.check(), self.check()
        environment = self.build['environment']
        for key in list(environment):
            if key.startswith('ORYN_REPAIR') and key.endswith('_SUPERVISED_GROUP'):
                del environment[key]
        environment.update(ORYN_GENERATION_GATE=str(self.gate), ORYN_GENERATION_GATE_SHA256=self.gate_record['sha256'])
        environment[f'ORYN_REPAIR{number}_SUPERVISED_GROUP'] = 'runtime-profile-build'
        self.write(self.start, self.start_value(self.command))
        self.build['start_receipt'] = self.provenance.record(self.start)
        self.refresh()

    def test_exact_candidate49_and_candidate50_receipts_remain_valid(self):
        self.assertEqual(self.provenance.candidate_number(self.root, self.gate), '49')
        self.validate()
        self.select_candidate(50)
        self.assertEqual(self.provenance.candidate_number(self.root, self.gate), '50')
        self.validate()

    def test_candidate_kind_and_expected_path_must_match_explicit_gate(self):
        self.select_candidate(50)
        original = json.loads(self.lease.read_bytes())
        for change, message in (({'kind': 'repair49_whole_source_and_review_lease_inputs'}, 'wrong whole-source lease'),
                                ({'expected_declaration_path': 'docs/oryn2/GENERATION-COMPLETE-REPAIR-49-2026-09-18.json'}, 'names another declaration')):
            lease = dict(original, **change)
            self.write(self.lease, lease)
            lease_sha = self.provenance.digest(self.lease)
            gate = json.loads(self.gate.read_bytes())
            gate['review_sources'][0]['sha256'] = lease_sha
            self.write(self.gate, gate)
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, message):
                self.provenance.gate_inputs(self.root, self.gate, self.provenance.digest(self.gate), self.lease, lease_sha)

    def test_candidate_hash_and_noncanonical_declaration_are_rejected(self):
        for gate_sha, lease_sha in (('0' * 64, self.lease_record['sha256']),
                                    (self.gate_record['sha256'], '0' * 64)):
            with self.subTest(gate_sha=gate_sha), self.assertRaisesRegex(ValueError, 'digest mismatch'):
                self.provenance.gate_inputs(self.root, self.gate, gate_sha, self.lease, lease_sha)
        outside = self.root / 'artifacts' / self.gate.name
        outside.write_bytes(self.gate.read_bytes())
        alias = self.gate.with_name('GENERATION-COMPLETE-REPAIR-50-2026-09-18.json')
        alias.symlink_to(self.gate)
        for path in (outside, alias, self.gate.with_name('untyped.json')):
            with self.subTest(path=path), self.assertRaisesRegex(ValueError, 'canonical'):
                self.provenance.candidate_number(self.root, path)

    def test_candidate50_cannot_borrow_candidate49_supervision(self):
        self.select_candidate(50)
        del self.build['environment']['ORYN_REPAIR50_SUPERVISED_GROUP']
        self.build['environment']['ORYN_REPAIR49_SUPERVISED_GROUP'] = 'runtime-profile-build'
        self.refresh()
        with self.assertRaisesRegex(ValueError, 'environment generation mismatch'):
            self.validate()



class PublicV8HeaderProvenanceTests(unittest.TestCase):
    """Check the actual vendored API source before a costly runtime build."""
    def test_public_api_manifest_tracks_all_current_engine_patch_pins(self):
        manifest = json.loads((ROOT / 'vendor/v8-api/manifest.json').read_text())
        # Shared Wasm size changes private engine code and has no public-API field.
        public_fields = set(MODULE.PATCHES) - {'shared_wasm_size_patch_sha256'}
        self.assertEqual({key for key in manifest if key.endswith('_patch_sha256')}, public_fields)
        for field, name in MODULE.PATCHES.items():
            patch = ROOT / 'patches/v8' / (name + '.patch')
            metadata = json.loads(patch.with_suffix('.json').read_text())
            with self.subTest(patch=name):
                actual = MODULE.digest(patch)
                self.assertEqual(actual, metadata['patch_sha256'])
                self.assertEqual(manifest['v8_revision'], metadata['v8_base_revision'])
                if field in public_fields:
                    # Even a private-code-only extension changes the whole patch identity.
                    self.assertEqual(manifest[field], actual)
                for prerequisite, expected in metadata.get('required_patch_sha256', {}).items():
                    self.assertEqual(MODULE.digest(ROOT / 'patches/v8' / (prerequisite + '.patch')), expected)

    def test_public_header_bytes_closure_and_final_patch_outputs_match(self):
        import re
        manifest = json.loads((ROOT / 'vendor/v8-api/manifest.json').read_text())
        include = (ROOT / 'vendor/v8-api/include').resolve()
        actual_names = {str(path.relative_to(include)) for path in include.rglob('*.h')}
        self.assertEqual(actual_names, set(manifest['headers']))
        for name, expected in manifest['headers'].items():
            with self.subTest(header=name):
                self.assertEqual(MODULE.digest(include / name), expected)
        final_files = {}
        for name in MODULE.PATCHES.values():
            metadata = json.loads((ROOT / 'patches/v8' / (name + '.json')).read_text())
            final_files.update(metadata['patched_files'])
        for relative, expected in final_files.items():
            if relative.startswith('include/'):
                name = relative.removeprefix('include/')
                with self.subTest(patched_header=name):
                    self.assertIn(name, manifest['headers'])
                    self.assertEqual(manifest['headers'][name], expected)
        pending, visited, external = ['v8.h'], set(), set()
        while pending:
            name = pending.pop()
            if name in visited:
                continue
            visited.add(name)
            header = include / name
            for quoted in re.findall(r'^\s*#\s*include\s*"([^"]+)"', header.read_text(), re.MULTILINE):
                candidates = (header.parent / quoted, include / quoted)
                target = next((path.resolve() for path in candidates if path.is_file()), None)
                if target is None:
                    external.add(quoted)
                else:
                    self.assertTrue(target.is_relative_to(include))
                    pending.append(str(target.relative_to(include)))
        self.assertEqual(visited, set(manifest['headers']))
        self.assertEqual(external, set(manifest['conditional_external_headers']))

if __name__ == "__main__":
    unittest.main()

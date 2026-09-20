"""Independent adversarial collection/custody tests for the full Python harness."""
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
HARNESS = ROOT / 'scripts/qualification'


def load(name):
    spec = importlib.util.spec_from_file_location('python_harness_review_' + name, HARNESS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


LAUNCH = load('launch')
RUNNER = load('runner')
PASSING = 'import unittest\nclass Example(unittest.TestCase):\n def test_value(self): self.assertEqual(2+2,4)\n'


class PythonRegressionHarnessTests(unittest.TestCase):
    def run_fixture(self, bodies):
        with tempfile.TemporaryDirectory() as directory:
            paths = []
            for index, body in enumerate(bodies):
                path = Path(directory) / ('test_' + str(index) + '.py')
                path.write_text(body)
                paths.append(str(path))
            return RUNNER.run_sources(paths, stream=io.StringIO())

    def preflight(self, record):
        record.update(run_id='current', generation_gate_sha256='a' * 64,
                      source_identities=[dict(path=row['path'], sha256='b' * 64) for row in record['sources']])
        return dict(run_id=record['run_id'], generation_gate_sha256=record['generation_gate_sha256'],
                    source_identities=record['source_identities'], test_sources=[row['path'] for row in record['sources']])

    def test_complete_pass_retains_every_source_and_terminal(self):
        record = self.run_fixture([PASSING, PASSING])
        preflight = self.preflight(record)
        self.assertEqual([row['count'] for row in record['sources']], [1, 1])
        self.assertEqual(record['tests'], 2)
        self.assertEqual([row['status'] for row in record['test_outcomes']], ['passed', 'passed'])
        LAUNCH.validate_results(record, preflight)

    def test_zero_collection_and_system_exit_zero_cannot_omit_a_source(self):
        record = self.run_fixture(['raise SystemExit(0)\n', 'value=42\n', PASSING])
        self.assertEqual(len(record['sources']), 3)
        self.assertEqual([row['count'] for row in record['sources']], [0, 0, 1])
        self.assertEqual(len(record['collection_errors']), 2)
        self.assertEqual(record['tests'], 1)
        with self.assertRaises(ValueError):
            LAUNCH.validate_results(record, self.preflight(record))

    def test_skip_expected_failure_and_subtest_failure_remain_nonpassing(self):
        variants = [
            "import unittest\n@unittest.skip('missing signed CLI')\nclass Example(unittest.TestCase):\n def test_value(self): pass\n",
            "import unittest\nclass Example(unittest.TestCase):\n @unittest.expectedFailure\n def test_value(self): self.fail('not repaired')\n",
            "import unittest\nclass Example(unittest.TestCase):\n def test_value(self):\n  with self.subTest(value=1): self.assertEqual(1,2)\n",
        ]
        for body in variants:
            with self.subTest(body=body):
                record = self.run_fixture([body])
                self.assertNotEqual(record['test_outcomes'][0]['status'], 'passed')
                with self.assertRaises(ValueError):
                    LAUNCH.validate_results(record, self.preflight(record))

    def test_class_setup_failure_retains_unstarted_test_obligations(self):
        record = self.run_fixture(["import unittest\nclass Example(unittest.TestCase):\n @classmethod\n def setUpClass(cls): raise RuntimeError('fixture setup failed')\n def test_one(self): pass\n def test_two(self): pass\n"])
        self.assertEqual(record['discovered_tests'], 2)
        self.assertEqual(record['tests'], 0)
        self.assertFalse(record['execution_complete'])
        self.assertEqual([row['status'] for row in record['test_outcomes']], ['not_run', 'not_run'])
        self.assertEqual(len(record['errors']), 1)

    def test_old_run_hash_source_and_terminal_substitution_are_rejected(self):
        record = self.run_fixture([PASSING])
        preflight = self.preflight(record)
        for field, value in [('run_id', 'old'), ('generation_gate_sha256', 'c' * 64),
                             ('source_identities', []), ('test_outcomes', []), ('tests', 0),
                             ('execution_complete', False)]:
            with self.subTest(field=field):
                altered = copy.deepcopy(record)
                altered[field] = value
                with self.assertRaises(ValueError):
                    LAUNCH.validate_results(altered, preflight)
        with self.assertRaises(ValueError):
            LAUNCH.validate_results(None, preflight)

    def test_nested_review_does_not_replace_outer_module_registration(self):
        sentinel = object()
        with mock.patch.dict(sys.modules, {'fq_regression_0': sentinel}):
            self.run_fixture([PASSING])
            self.assertIs(sys.modules['fq_regression_0'], sentinel)

    def test_recursive_discovery_preserves45_and_rejects_missing_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            historical = ['scripts/test_historical_' + str(index) + '.py' for index in range(45)]
            extras = ['scripts/tests/deep/test_nested.py', 'tools/wptrunner-oryn/tests/deep/test_plugin.py', 'extra/test_bound.py']
            bound = {}
            for source in historical + extras:
                path = root / source
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(PASSING)
                bound[source] = hashlib.sha256(path.read_bytes()).hexdigest()
            previous = root / 'previous.json'
            previous.write_text(json.dumps(dict(test_sources=historical)))
            with mock.patch.object(LAUNCH, 'HISTORICAL_SHA256', LAUNCH.digest(previous)):
                identities = LAUNCH.regression_sources(root, previous, bound)
                self.assertEqual({item['path'] for item in identities}, set(historical + extras))
                del bound[extras[0]]
                with self.assertRaisesRegex(ValueError, 'not bound'):
                    LAUNCH.regression_sources(root, previous, bound)
            with mock.patch.object(LAUNCH, 'HISTORICAL_SHA256', '0' * 64):
                with self.assertRaisesRegex(ValueError, 'historical'):
                    LAUNCH.regression_sources(root, previous, bound)

    def test_python13_population_preserves103_and_rejects_missing_extra_or_duplicate(self):
        historical = ['scripts/test_preserved_' + str(index) + '.py' for index in range(103)]
        with tempfile.TemporaryDirectory() as directory:
            previous = Path(directory) / 'previous.json'
            previous.write_text(json.dumps(dict(test_sources=historical)))
            complete = [dict(path=path, sha256='a' * 64)
                        for path in sorted(historical + [LAUNCH.ADMISSION_REGRESSION])]
            with mock.patch.object(LAUNCH, 'PREVIOUS_PREFLIGHT_SHA256', LAUNCH.digest(previous)):
                LAUNCH.validate_population(complete, previous)
                variants = {
                    'missing prior': [row for row in complete if row['path'] != historical[0]],
                    'missing admission': [row for row in complete if row['path'] != LAUNCH.ADMISSION_REGRESSION],
                    'extra source': complete + [dict(path='scripts/test_unexpected.py', sha256='b' * 64)],
                    'duplicate instead of prior': [row for row in complete if row['path'] != historical[0]] + [complete[0]],
                }
                for reason, identities in variants.items():
                    with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, 'exactly'):
                        LAUNCH.validate_population(identities, previous)

    def test_python13_population_rejects_changed_or_incomplete_prior_record(self):
        historical = ['scripts/test_preserved_' + str(index) + '.py' for index in range(103)]
        with tempfile.TemporaryDirectory() as directory:
            previous = Path(directory) / 'previous.json'
            previous.write_text(json.dumps(dict(test_sources=historical)))
            expected = LAUNCH.digest(previous)
            previous.write_text(json.dumps(dict(test_sources=historical[:-1])))
            with mock.patch.object(LAUNCH, 'PREVIOUS_PREFLIGHT_SHA256', expected), \
                 self.assertRaisesRegex(ValueError, 'record changed'):
                LAUNCH.validate_population([], previous)
            for paths in (historical[:-1], historical[:-1] + [historical[0]]):
                previous.write_text(json.dumps(dict(test_sources=paths)))
                with mock.patch.object(LAUNCH, 'PREVIOUS_PREFLIGHT_SHA256', LAUNCH.digest(previous)), \
                     self.assertRaisesRegex(ValueError, 'all 103'):
                    LAUNCH.validate_population([], previous)

    def test_native_prerequisites_require_both_explicit_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'native'
            binary.write_text('fixture bytes only; never executed')
            binary.chmod(0o700)
            inputs, errors = LAUNCH.bind_native_inputs({'ORYN_TEST_NATIVE_BIN': str(binary)})
            self.assertEqual(set(inputs), {'ORYN_TEST_NATIVE_BIN'})
            self.assertTrue(any('ORYN_PAGE_WORKER' in item for item in errors))
            inputs, errors = LAUNCH.bind_native_inputs({'ORYN_TEST_NATIVE_BIN': str(binary), 'ORYN_PAGE_WORKER': str(binary)})
            self.assertFalse(errors)
            self.assertEqual(inputs['ORYN_TEST_NATIVE_BIN']['sha256'], LAUNCH.digest(binary))

    def test_retargeted_aliases_cannot_change_captured_dispatch_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            original, replacement = root / 'original', root / 'replacement'
            original.write_text('captured executable bytes; never executed')
            replacement.write_text('different executable bytes; never executed')
            original.chmod(0o700)
            replacement.chmod(0o700)
            aliases = [root / 'cli-alias', root / 'worker-alias']
            for alias in aliases:
                alias.symlink_to(original)
            environment = dict(zip(('ORYN_TEST_NATIVE_BIN', 'ORYN_PAGE_WORKER'), map(str, aliases)))
            identities, errors = LAUNCH.bind_native_inputs(environment)
            self.assertFalse(errors)
            for alias in aliases:
                alias.unlink()
                alias.symlink_to(replacement)
            self.assertTrue(all(alias.resolve() == replacement for alias in aliases))
            for name, identity in identities.items():
                selected = Path(environment[name])
                self.assertEqual(selected, original.resolve())
                self.assertEqual(str(selected), identity['path'])
                self.assertEqual(LAUNCH.digest(selected), identity['sha256'])
                self.assertNotEqual(LAUNCH.digest(replacement), identity['sha256'])

    def test_launcher_requires_inherited_generation_before_any_child(self):
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(LAUNCH.subprocess, 'Popen') as child:
            with self.assertRaisesRegex(ValueError, 'generation declaration'):
                LAUNCH.main()
            child.assert_not_called()

    def test_missing_or_malformed_result_file_cannot_reuse_a_success(self):
        record = self.run_fixture([PASSING])
        preflight = self.preflight(record)
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / 'test-results.json'
            with self.assertRaises(OSError):
                LAUNCH.read_results(result, preflight)
            result.write_text('{broken')
            with self.assertRaises(ValueError):
                LAUNCH.read_results(result, preflight)
            record['run_id'] = 'previous invocation'
            result.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, 'different invocation'):
                LAUNCH.read_results(result, preflight)

    def test_launcher_reaps_after_metadata_failure_and_keyboard_interrupt(self):
        for metadata_failure in (True, False):
            with self.subTest(metadata_failure=metadata_failure), tempfile.TemporaryDirectory() as directory:
                process = mock.Mock(pid=123456)
                process.poll.return_value = None
                process.wait.side_effect = [-9] if metadata_failure else [KeyboardInterrupt(), -9]
                admission = mock.Mock(sha256='a' * 64)
                with mock.patch.object(LAUNCH.subprocess, 'Popen', return_value=process) as child, \
                     mock.patch.object(LAUNCH, 'persist', side_effect=OSError('receipt write failed') if metadata_failure else None), \
                     mock.patch.object(LAUNCH.os, 'killpg', side_effect=[None, ProcessLookupError()]) as kill, \
                     mock.patch.object(LAUNCH.signal, 'signal'):
                    record = LAUNCH.run_child(['fixture-command'], {}, Path(directory), {'run_id': 'run'}, admission)
                self.assertIsNotNone(record['execution_error'])
                self.assertEqual(record['returncode'], -9)
                self.assertEqual(record['pid'], process.pid)
                self.assertFalse(record['cleanup_errors'])
                self.assertTrue(child.call_args.kwargs['start_new_session'])
                kill.assert_any_call(process.pid, LAUNCH.signal.SIGKILL)
                process.wait.assert_called_with(timeout=15)

    def test_descendant_survival_remains_failed_custody_after_forced_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            process = mock.Mock(pid=123456)
            process.poll.return_value = 0
            process.wait.return_value = 0
            with mock.patch.object(LAUNCH.subprocess, 'Popen', return_value=process), \
                 mock.patch.object(LAUNCH.os, 'killpg') as kill, \
                 mock.patch.object(LAUNCH.signal, 'signal'):
                record = LAUNCH.run_child(['fixture-command'], {}, Path(directory), {'run_id': 'run'}, mock.Mock(sha256='a' * 64))
            self.assertEqual(record['returncode'], 0)
            self.assertTrue(record['cleanup_errors'])
            kill.assert_any_call(process.pid, LAUNCH.signal.SIGKILL)

    def test_sibling_collision_lazy_import_and_teardown_keep_source_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            paths = []
            log = root / 'fixtures.log'
            for name in ('first', 'second'):
                folder = root / name; folder.mkdir()
                (folder / 'shared_fixture.py').write_text('VALUE = ' + repr(name) + '\n')
                source = folder / 'test_case.py'
                source.write_text(
                    'import unittest\nfrom pathlib import Path\nimport shared_fixture\n'
                    'EXPECTED = ' + repr(name) + '\nLOG = Path(' + repr(str(log)) + ')\n'
                    'def record(phase):\n import shared_fixture\n assert shared_fixture.VALUE == EXPECTED\n'
                    ' with LOG.open("a") as stream: stream.write(EXPECTED + ":" + phase + "\\n")\n'
                    'def setUpModule(): record("module_setup")\n'
                    'def tearDownModule(): record("module_teardown")\n'
                    'class Example(unittest.TestCase):\n'
                    ' @classmethod\n def setUpClass(cls): record("class_setup")\n'
                    ' @classmethod\n def tearDownClass(cls): record("class_teardown")\n'
                    ' def test_value(self):\n  import shared_fixture\n  self.assertEqual(shared_fixture.VALUE, EXPECTED)\n  record("test")\n')
                paths.append(str(source))
            before = sys.path[:]
            record = RUNNER.run_sources(paths, stream=io.StringIO(), root=root)
            self.assertEqual([row['status'] for row in record['test_outcomes']], ['passed', 'passed'])
            self.assertFalse(record['collection_errors']); self.assertFalse(record['errors'])
            self.assertEqual(log.read_text().splitlines(), [name + ':' + phase for name in ('first','second')
                             for phase in ('module_setup','class_setup','test','class_teardown','module_teardown')])
            self.assertEqual(sys.path, before)
            self.assertNotIn('shared_fixture', sys.modules)

    def test_relative_import_uses_actual_package_initializer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); package = root / 'package'; package.mkdir()
            (package / '__init__.py').write_text('TOKEN = "initialized"\n')
            (package / 'helper.py').write_text('VALUE = 42\n')
            source = package / 'test_relative.py'
            source.write_text('import unittest\nfrom . import TOKEN\nfrom .helper import VALUE\n'
                              'class Example(unittest.TestCase):\n def test_relative(self):\n'
                              '  from .helper import VALUE as later\n'
                              '  self.assertEqual((TOKEN, VALUE, later), ("initialized",42,42))\n')
            record = RUNNER.run_sources([str(source)], stream=io.StringIO(), root=root)
            self.assertFalse(record['collection_errors']); self.assertEqual(record['tests'], 1)
            self.assertEqual(record['test_outcomes'][0]['status'], 'passed')

    def test_repository_namespace_overrides_foreign_scripts_and_restores_it(self):
        import types
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); folder = root / 'scripts' / 'tests'; folder.mkdir(parents=True)
            (root / 'scripts' / 'evidence_assertions.py').write_text('VALUE = "repository"\n')
            source = folder / 'test_package.py'
            source.write_text('import unittest\nfrom scripts.evidence_assertions import VALUE\n'
                              'class Example(unittest.TestCase):\n def test_package(self):\n'
                              '  from scripts.evidence_assertions import VALUE as later\n'
                              '  self.assertEqual((VALUE,later),("repository","repository"))\n')
            foreign = types.ModuleType('scripts'); foreign.__path__ = [str(root / 'foreign')]
            foreign_child = types.ModuleType('scripts.evidence_assertions'); foreign_child.VALUE = 'foreign'
            before = sys.path[:]
            with mock.patch.dict(sys.modules, {'scripts':foreign, 'scripts.evidence_assertions':foreign_child}):
                record = RUNNER.run_sources([str(source)], stream=io.StringIO(), root=root)
                self.assertIs(sys.modules['scripts'], foreign)
                self.assertIs(sys.modules['scripts.evidence_assertions'], foreign_child)
            self.assertEqual(sys.path, before)
            self.assertFalse(record['collection_errors']); self.assertEqual(record['test_outcomes'][0]['status'], 'passed')

    def test_import_exception_restores_paths_and_preexisting_sibling(self):
        import types
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'sibling.py').write_text('VALUE = "local"\n')
            source = root / 'test_broken.py'
            source.write_text('import sys\nimport sibling\nsys.path.insert(0,"import-leak")\nraise RuntimeError("collection failed")\n')
            original = types.ModuleType('sibling'); original.__file__ = str(root / 'foreign' / 'sibling.py')
            before = sys.path[:]
            with mock.patch.dict(sys.modules, {'sibling':original}):
                record = RUNNER.run_sources([str(source)], stream=io.StringIO(), root=root)
                self.assertIs(sys.modules['sibling'], original)
            self.assertEqual(sys.path, before)
            self.assertEqual(len(record['collection_errors']),1)
            self.assertEqual(record['sources'][0]['count'],0)

    def test_keyboard_interrupt_restores_import_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); source = root / 'test_interrupt.py'
            source.write_text('import sys\nsys.path.insert(0,"interrupt-leak")\nraise KeyboardInterrupt()\n')
            before = sys.path[:]; sentinel = object()
            with mock.patch.dict(sys.modules, {'fq_regression_0':sentinel}):
                with self.assertRaises(KeyboardInterrupt), RUNNER.SourceImports(source, root, 0) as context:
                    context.load()
                self.assertIs(sys.modules['fq_regression_0'],sentinel)
            self.assertEqual(sys.path,before)

    def test_nested_run_preserves_outer_module_and_lazy_sibling(self):
        import types
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve(); nested = root / 'nested'; nested.mkdir()
            (root / 'sibling.py').write_text('VALUE="outer"\n')
            (nested / 'sibling.py').write_text('VALUE="inner"\n')
            inner = nested / 'test_inner.py'
            inner.write_text('import unittest\nclass Inner(unittest.TestCase):\n def test_inner(self):\n'
                             '  import sibling\n  self.assertEqual(sibling.VALUE,"inner")\n')
            source = root / 'test_outer.py'
            source.write_text('import unittest,sys,io\nimport sibling\nfrom harness_fixture_bridge import runner\n'
                              'class Outer(unittest.TestCase):\n def test_outer(self):\n'
                              '  before=sys.modules[__name__]\n'
                              '  record=runner.run_sources([' + repr(str(inner)) + '],stream=io.StringIO(),root=' + repr(str(root)) + ')\n'
                              '  self.assertEqual(record["test_outcomes"][0]["status"],"passed")\n'
                              '  self.assertIs(sys.modules[__name__],before)\n'
                              '  import sibling as later\n  self.assertIs(later,sibling)\n  self.assertEqual(later.VALUE,"outer")\n')
            bridge=types.ModuleType('harness_fixture_bridge'); bridge.runner=RUNNER
            with mock.patch.dict(sys.modules,{'harness_fixture_bridge':bridge}):
                record=RUNNER.run_sources([str(source)],stream=io.StringIO(),root=root)
            self.assertFalse(record['collection_errors']);self.assertEqual(record['test_outcomes'][0]['status'],'passed')

    def test_imported_test_class_cannot_inflate_source_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve(); fixture=root/'borrowed_fixture.py'; fixture.write_text(PASSING)
            source=root/'test_borrowed.py';source.write_text('from borrowed_fixture import Example\n')
            record=RUNNER.run_sources([str(source)],stream=io.StringIO(),root=root)
            self.assertTrue(any('another source' in row['traceback'] for row in record['collection_errors']))
            with self.assertRaises(ValueError):LAUNCH.validate_results(record,self.preflight(record))

    def test_live_reference_methods_dispatch_only_explicit_chromium(self):
        environment = dict(ORYN_WPT_BINARY='/fixture/native', ORYN_WPT_CHECKOUT='/fixture/wpt',
                           ORYN_WPT_NODE='/fixture/node path', ORYN_WPT_CHROMIUM='/fixture/pinned CfT browser')
        source = ROOT / 'scripts/tests/test_wpt_session_live.py'
        names = ('test_chromium_fixture_completion_parity',
                 'test_chromium_parser_time_focus_is_verified_before_tests',
                 'test_chromium_rejects_completion_without_test_entry_focus')
        with mock.patch.dict(os.environ, environment), RUNNER.SourceImports(source, ROOT, 10000) as context:
            module = context.load()
            completion = dict(harness=0, tests=[dict(name='positive', status=0)])
            def reference(argv, **kwargs):
                if argv[3].endswith('/unreported-focus.html'):
                    return LAUNCH.subprocess.CompletedProcess(argv, 1, '', 'Chromium document-entry focus unverified (missing observation)')
                payload = dict(completion=completion, initial_focus_verified=True, entry_focus_verified=True,
                               entry_focus_phase='report_ready', focus_isolation='host_serialized')
                return LAUNCH.subprocess.CompletedProcess(argv, 0, json.dumps(payload), '')
            with mock.patch.object(module.subprocess, 'run', side_effect=reference) as command, \
                 mock.patch.object(module, 'WptSession') as native:
                native.return_value.run.return_value = dict(error=None, completion=completion, resource_evidence_complete=True)
                native.return_value.close.return_value = dict(returncode=0)
                for name in names:
                    case = module.SignedSessionTests(name); case.base = 'http://fixture.invalid'
                    getattr(case, name)()
            self.assertEqual(command.call_count,11)
            for call in command.call_args_list:
                self.assertEqual(call.args[0][0], environment['ORYN_WPT_NODE'])
                self.assertEqual(call.args[0][2], environment['ORYN_WPT_CHROMIUM'])
        missing_browser = dict(environment); missing_browser.pop('ORYN_WPT_CHROMIUM')
        with mock.patch.dict(os.environ, missing_browser, clear=True), RUNNER.SourceImports(source, ROOT, 10000) as context:
            module = context.load()
            for name in names:
                self.assertTrue(getattr(getattr(module.SignedSessionTests,name),'__unittest_skip__',False))

class SignedPythonOverlayTests(unittest.TestCase):
    """Synthetic files and mocked processes; no native executable is launched."""
    def setUp(self):
        import contextlib
        import types
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.root = Path(directory).resolve()
        self.overlay = self.module(HARNESS / 'signed_inputs.py', 'overlay49_test')
        self.custody = self.module(HARNESS / 'custody.py', 'custody49_test')
        self.base = self.root / 'artifacts/completion-validation-2026-09-18'
        self.base.mkdir(parents=True)
        for module in (self.overlay, self.custody):
            self.stack.enter_context(mock.patch.object(module, 'ROOT', self.root))
            self.stack.enter_context(mock.patch.object(module, 'BASE', self.base))
        self.stack.enter_context(mock.patch.object(self.custody, 'GATE', self.root / 'gate.json'))
        self.stack.enter_context(mock.patch.object(self.overlay, 'source_identities', return_value={}))
        self.stack.enter_context(mock.patch.object(self.custody, 'bound_source'))
        for source in ('scripts/native_protocol_versions.py', 'crates/oryn-native/src/worker.rs',
                       'crates/oryn-native/src/context_service.rs', 'crates/oryn-native/src/agent_ipc.rs'):
            destination = self.root / source
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((ROOT / source).read_bytes())
        self.admission = types.SimpleNamespace(environment=lambda env: dict(env), check=lambda: None)
        self.lease = self.base / 'lease.json'
        self.lease.write_text('{}')
        self.good_process = dict(returncode=0, execution_error=None, cleanup_errors=[], group_settled=True)
        self.release = self.root / 'target/current/release'
        self.release.mkdir(parents=True)
        raw = self.release / 'oryn-page-worker'; raw.write_bytes(b'raw build'); raw.chmod(0o700)
        self.profile = dict(worker_path=str(raw), worker_sha256=self.custody.sha(raw), worker_size_bytes=9,
                            archive_sha256='a'*64, binding_sha256='b'*64, bridge_abi_sha256='c'*64)
        self.profile_path = self.base / 'profile.json'; self.profile_path.write_text(json.dumps(self.profile))
        self.profile_ref = dict(path=str(self.profile_path), sha256=self.custody.sha(self.profile_path))
        self.runtime = dict(profile=self.profile_ref,
                            records=[self.overlay.identity(raw), self.overlay.identity(self.profile_path)],
                            sdkroot=str(self.root), sdkroot_resolved=str(self.root),
                            environment=dict(ORYN_V8_PROFILE_MANIFEST=str(self.profile_path)))
        self.provenance_ref=dict(path=str(self.base/'runtime-provenance.json'),sha256='9'*64)
        self.provenance_load=self.stack.enter_context(mock.patch.object(self.custody,'load_runtime_provenance',return_value=self.provenance_ref))
        self.provenance_check=self.stack.enter_context(mock.patch.object(self.custody,'check_runtime_provenance',return_value=dict(errors=[])))
        self.runtime_load = self.stack.enter_context(mock.patch.object(self.custody, 'load_runtime', return_value=self.runtime))
        paths = self.overlay.layout(self.profile)
        for name in ('cli', 'worker'):
            paths[name].parent.mkdir(parents=True, exist_ok=True)
            paths[name].write_bytes(('prepared ' + name).encode()); paths[name].chmod(0o700)
        paths['worker_manifest'].parent.mkdir(parents=True)
        paths['bundle_profile'].write_bytes(self.profile_path.read_bytes())
        manifest = dict(bundle_id=self.overlay.BUNDLE_ID, team_id=self.overlay.TEAM_ID,
                        signing_cert_sha1=self.overlay.CERT_SHA1, worker_build_sha256=self.profile['worker_sha256'],
                        worker_build_bytes=9, v8_profile_sha256=self.profile_ref['sha256'], source_tree_sha256='d'*64)
        paths['worker_manifest'].write_text(json.dumps(manifest))
        self.paths = paths
        import security_prebuilt
        self.security = types.SimpleNamespace(bundle_inventory=security_prebuilt.bundle_inventory,
            verify_signature=mock.Mock(), source_hash=lambda: 'd'*64)
        self.stack.enter_context(mock.patch.object(self.overlay, 'security_module', return_value=self.security))
        self.artifacts = {name:self.overlay.identity(paths[name]) for name in
                          ('cli', 'worker', 'worker_manifest', 'bundle_profile')}
        protocols = self.overlay.module_at(self.root / 'scripts/native_protocol_versions.py', 'protocols49_test')
        versions = protocols.versions(self.root)
        self.parent = dict(native_v8=True, build_profile='release', execution_mode='sandboxed_worker',
            sandboxed=True, sandbox_state='app_sandbox', worker_bundle_id=self.overlay.BUNDLE_ID,
            worker_team_id=self.overlay.TEAM_ID, worker_signing_cert_sha1=self.overlay.CERT_SHA1,
            worker_binary_sha256=self.artifacts['worker']['sha256'], v8_archive_sha256='a'*64,
            v8_binding_sha256='b'*64, v8_bridge_abi_sha256='c'*64,
            worker_protocol_version=versions['worker'], context_service_protocol_version=versions['context'],
            agent_ipc_protocol_version=versions['agent'])
        for name, data in (('build.log', 'canonical log'), ('build-process.json', json.dumps(self.good_process)),
                           ('parent.json', json.dumps(self.parent)), ('parent-process.json', json.dumps(self.good_process))):
            (self.base / name).write_text(data)
        self.whole = dict(errors=[], declaration_sha256='e'*64, lease_sha256='f'*64)
        self.value = dict(schema_version=1, kind=self.overlay.KIND, mode='canonical_preparation', prepared=True,
            generation=dict(path=str(self.custody.GATE.resolve()), sha256='e'*64),
            whole_lease=dict(path=str(self.lease), sha256='f'*64), producer_sources={},
            command=self.overlay.COMMAND, process=self.good_process,
            whole_source_before=self.whole, whole_source_after=self.whole,
            signing_identity=self.overlay.CERT_SHA1, profile=self.profile_ref, runtime_provenance=self.provenance_ref,
            artifacts=self.artifacts, overlay=self.overlay.overlay_for(self.artifacts),
            source_tree_sha256='d'*64, bundle_files=self.security.bundle_inventory(paths['worker']),
            log=self.overlay.identity(self.base/'build.log'), process_receipt=self.overlay.identity(self.base/'build-process.json'),
            parent_runtime_command=[self.artifacts['cli']['resolved_path'], 'native', '--runtime-info'],
            parent_runtime_process=self.good_process, parent_runtime_output=self.overlay.identity(self.base/'parent.json'),
            parent_runtime_process_receipt=self.overlay.identity(self.base/'parent-process.json'))
        self.receipt = self.base / 'preparation.json'
        self.write_receipt()

    @staticmethod
    def module(path, name):
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        return module

    def write_receipt(self):
        self.receipt.write_text(json.dumps(self.value))
        return self.custody.sha(self.receipt)

    def admit(self):
        return self.overlay.load_inputs(self.receipt, self.custody.sha(self.receipt), self.custody,
                                        self.lease, 'f'*64, 'e'*64)

    def test_exact_three_overlay_keeps_baseline_and_generation_last(self):
        baseline = dict(PATH='pinned-node:/bin', ORYN_GENERATION_GATE='current', EXTRA='unchanged')
        overlay = self.value['overlay']
        result = self.overlay.effective_environment(baseline, overlay, self.admission)
        self.assertEqual({key:result[key] for key in self.overlay.KEYS}, overlay)
        self.assertEqual({key:value for key,value in result.items() if key not in self.overlay.KEYS}, baseline)
        for changed in (dict(result, EXTRA='changed'), dict(result, UNDECLARED='injected')):
            with self.assertRaises(ValueError): self.overlay.validate_environment(baseline, changed, overlay)
        for key in self.overlay.KEYS:
            with self.assertRaises(ValueError):
                self.overlay.effective_environment(dict(baseline, **{key:'inherited'}), overlay, self.admission)
        self.admission.environment = lambda env: dict(env, ORYN_GENERATION_GATE='stale')
        with self.assertRaises(ValueError): self.overlay.effective_environment(baseline, overlay, self.admission)

    def test_admission_requires_exact_receipt_generation_lease_and_successful_preparation(self):
        accepted = self.admit()
        self.assertEqual(accepted['value']['overlay'], self.value['overlay'])
        self.security.verify_signature.assert_called_once()
        original = copy.deepcopy(self.value)
        variants = [dict(prepared=False), dict(command=['cargo','build']), dict(producer_sources={'unbound':'0'*64}),
                    dict(generation=dict(path='old',sha256='e'*64)), dict(whole_lease=dict(path=str(self.lease),sha256='0'*64)),
                    dict(process=dict(self.good_process,group_settled=False))]
        for changed in variants:
            self.value = dict(original, **changed); self.write_receipt()
            with self.assertRaises(ValueError): self.admit()
        self.value = original; digest = self.write_receipt()
        self.receipt.write_text(self.receipt.read_text()+' ')
        with self.assertRaises(ValueError):
            self.overlay.load_inputs(self.receipt,digest,self.custody,self.lease,'f'*64,'e'*64)

    def test_receipt_payload_is_parsed_from_the_same_digest_checked_buffer(self):
        expected=self.custody.sha(self.receipt)
        original=self.receipt.read_bytes()
        reader=self.overlay.read_identity
        checker=self.custody.runtime_records_check
        def capture(path):
            row,data=reader(path)
            if Path(path)==self.receipt:
                self.receipt.write_text('{"prepared":false}')
            return row,data
        def restore(rows):
            self.receipt.write_bytes(original)
            return checker(rows)
        with mock.patch.object(self.overlay,'read_identity',side_effect=capture), \
             mock.patch.object(self.custody,'runtime_records_check',side_effect=restore):
            state=self.overlay.load_inputs(self.receipt,expected,self.custody,self.lease,'f'*64,'e'*64)
        self.assertTrue(state['value']['prepared'])
        self.assertEqual(state['receipt']['sha256'],expected)

    def test_runtime_provenance_is_checked_on_admission_and_after_cleanup(self):
        state=self.admit()
        self.provenance_load.assert_called_once_with(self.provenance_ref['path'],self.provenance_ref['sha256'],
            self.lease,'f'*64,'e'*64,self.runtime)
        self.provenance_check.return_value=dict(errors=['profile build belongs to old generation'])
        with self.assertRaises(ValueError):self.overlay.check_inputs(state,self.custody)
        self.provenance_load.side_effect=ValueError('wrong actual generation')
        with self.assertRaisesRegex(ValueError,'wrong actual generation'):self.admit()

    def test_raw_worker_profile_manifest_and_parent_attestation_are_not_interchangeable(self):
        original = self.paths['worker_manifest'].read_bytes()
        for field,value in [('worker_build_sha256','0'*64),('worker_build_bytes',10),
                            ('v8_profile_sha256','1'*64),('signing_cert_sha1','ADHOC'),('source_tree_sha256','2'*64)]:
            altered = json.loads(original); altered[field]=value
            self.paths['worker_manifest'].write_text(json.dumps(altered))
            self.value['artifacts']['worker_manifest']=self.overlay.identity(self.paths['worker_manifest'])
            self.value['bundle_files']=self.security.bundle_inventory(self.paths['worker'])
            self.write_receipt()
            with self.assertRaises(ValueError): self.admit()
        self.paths['worker_manifest'].write_bytes(original)
        for field,value in [('sandboxed',False),('worker_binary_sha256','0'*64),('v8_bridge_abi_sha256','f'*64),
                            ('context_service_protocol_version',0),('build_profile','debug')]:
            with self.assertRaises(ValueError):
                self.overlay.validate_parent(dict(self.parent,**{field:value}),self.profile,self.artifacts,self.custody)

    def test_complete_bundle_drift_and_live_signature_failure_invalidate_after_custody(self):
        state=self.admit()
        extra=self.paths['worker'].parents[1]/'Resources/extra'; extra.write_bytes(b'unlisted')
        with self.assertRaises(ValueError): self.overlay.check_inputs(state,self.custody)
        extra.unlink()
        self.paths['bundle_profile'].write_bytes(b'changed')
        with self.assertRaises(ValueError): self.overlay.check_inputs(state,self.custody)
        self.paths['bundle_profile'].write_bytes(self.profile_path.read_bytes())
        self.security.verify_signature.side_effect=ValueError('certificate invalid')
        with self.assertRaisesRegex(ValueError,'certificate invalid'): self.overlay.check_inputs(state,self.custody)

    def test_original_alias_retarget_and_validator_time_mutation_cannot_be_adopted(self):
        alias=self.base/'receipt-alias'; alias.symlink_to(self.receipt)
        other=self.base/'same-bytes'; other.write_bytes(self.receipt.read_bytes())
        def retarget(*args):
            alias.unlink(); alias.symlink_to(other)
            return self.runtime
        self.runtime_load.side_effect=retarget
        with self.assertRaises(ValueError):
            self.overlay.load_inputs(alias,self.custody.sha(alias),self.custody,self.lease,'f'*64,'e'*64)
        self.runtime_load.side_effect=None
        state=self.admit()
        def mutate(candidate,run): self.paths['cli'].write_bytes(b'replaced')
        self.security.verify_signature.side_effect=mutate
        with self.assertRaises(ValueError): self.overlay.check_inputs(state,self.custody)

    def test_signed_wpt_input_requires_same_cli_even_when_other_executable_bytes_match(self):
        env=dict(self.value['overlay']); inputs,errors=LAUNCH.bind_python_native_inputs(env)
        self.assertFalse(errors); self.assertEqual(set(inputs),set(self.overlay.KEYS))
        other=self.base/'other-cli'; other.write_bytes(self.paths['cli'].read_bytes());other.chmod(0o700)
        env=dict(self.value['overlay'],ORYN_WPT_BINARY=str(other))
        _,errors=LAUNCH.bind_python_native_inputs(env)
        self.assertTrue(any('differs' in error for error in errors))
        env=dict(self.value['overlay']);env.pop('ORYN_WPT_BINARY')
        self.assertTrue(LAUNCH.bind_python_native_inputs(env)[1])

    def test_signature_adapter_allows_only_absolute_read_only_codesign_commands(self):
        def verify(candidate,run):
            run(['codesign','--verify','--strict','/bundle'],check=True,capture_output=True)
            with self.assertRaises(ValueError):run(['codesign','--sign','identity','/bundle'])
            with self.assertRaises(ValueError):run(['cargo','build'])
        self.security.verify_signature.side_effect=verify
        with mock.patch.object(self.overlay.subprocess,'run') as command:
            self.overlay.signature_check(self.security,self.paths['worker'],self.paths['worker_manifest'])
        command.assert_called_once_with(['/usr/bin/codesign','--verify','--strict','/bundle'],timeout=30,check=True,capture_output=True)

    def test_preparation_forces_canonical_build_branch_and_explicit_profile_directory(self):
        inherited=dict(ORYN_SECURITY_PREBUILT_CANDIDATE='old',ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256='old',
                       CARGO_TARGET_DIR='/old',ORYN_PAGE_WORKER='/old',ORYN_TEST_NATIVE_BIN='/old',ORYN_WPT_BINARY='/old')
        with mock.patch.dict(os.environ,inherited,clear=True):
            env,paths=self.overlay.preparation_environment(self.admission,self.custody,self.runtime,self.overlay.CERT_SHA1)
        self.assertEqual(env['CARGO_TARGET_DIR'],str(self.release.parent))
        self.assertEqual(env['ORYN_CODESIGN_IDENTITY'],self.overlay.CERT_SHA1)
        for key in (*self.overlay.KEYS,'ORYN_SECURITY_PREBUILT_CANDIDATE','ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256'):
            self.assertNotIn(key,env)
        self.assertEqual(paths['worker'],self.paths['worker'])
        with self.assertRaises(ValueError):
            self.overlay.preparation_environment(self.admission,self.custody,self.runtime,'-')

    def test_failed_build_or_cleanup_never_publishes_signed_acceptance_inputs(self):
        import types
        for index,process in enumerate((dict(self.good_process,returncode=101),dict(self.good_process,cleanup_errors=['survivor']))):
            output=self.base/('attempt-'+str(index))
            args=types.SimpleNamespace(output=output,whole_lease=self.lease,whole_lease_sha256='f'*64,
                gate_sha256='e'*64,runtime_profile=self.profile_path,runtime_profile_sha256=self.profile_ref['sha256'],
                signing_identity=self.overlay.CERT_SHA1,runtime_provenance=Path(self.provenance_ref['path']),
                runtime_provenance_sha256=self.provenance_ref['sha256'])
            admission=types.SimpleNamespace(check=lambda:None,environment=lambda env:dict(env,
                ORYN_GENERATION_GATE=str(self.custody.GATE),ORYN_GENERATION_GATE_SHA256='e'*64))
            def run(command,env,start_receipt=None,log_path=None):
                Path(log_path).write_text('failed canonical build'); return process
            with mock.patch.object(self.custody,'check',return_value=self.whole), \
                 mock.patch.object(self.custody,'run_owned_dispatch',side_effect=run) as execute:
                self.assertEqual(self.overlay.prepare(args,self.custody,admission),1)
            execute.assert_called_once()
            self.assertEqual(execute.call_args.args[0],self.overlay.COMMAND)
            self.assertFalse((output/'signed-inputs.json').exists())
            self.assertFalse(json.loads((output/'preparation.json').read_bytes())['prepared'])

    def test_canonical_build_mutating_profile_owned_raw_worker_is_rejected_before_parent_launch(self):
        import types
        output=self.base/'mutated-raw-preparation'
        args=types.SimpleNamespace(output=output,whole_lease=self.lease,whole_lease_sha256='f'*64,
            gate_sha256='e'*64,runtime_profile=self.profile_path,runtime_profile_sha256=self.profile_ref['sha256'],
            signing_identity=self.overlay.CERT_SHA1,runtime_provenance=Path(self.provenance_ref['path']),
            runtime_provenance_sha256=self.provenance_ref['sha256'])
        admission=types.SimpleNamespace(check=lambda:None,environment=lambda env:dict(env,
            ORYN_GENERATION_GATE=str(self.custody.GATE),ORYN_GENERATION_GATE_SHA256='e'*64))
        def run(command,env,start_receipt=None,log_path=None):
            Path(log_path).write_text('build completed but replaced a profile input')
            Path(self.profile['worker_path']).write_bytes(b'different build')
            return dict(self.good_process)
        with mock.patch.object(self.custody,'check',return_value=self.whole), \
             mock.patch.object(self.custody,'run_owned_dispatch',side_effect=run) as execute:
            self.assertEqual(self.overlay.prepare(args,self.custody,admission),1)
        execute.assert_called_once()
        result=json.loads((output/'preparation.json').read_bytes())
        self.assertFalse(result['prepared'])
        self.assertTrue(result['runtime_after']['errors'])
        self.assertIn('changed inputs owned',result['preparation_error'])
        self.assertFalse((output/'signed-inputs.json').exists())

    def test_successful_preparation_records_real_commands_then_acceptance_never_rebuilds(self):
        import types
        output=self.base/'complete-preparation'
        args=types.SimpleNamespace(output=output,whole_lease=self.lease,whole_lease_sha256='f'*64,
            gate_sha256='e'*64,runtime_profile=self.profile_path,runtime_profile_sha256=self.profile_ref['sha256'],
            signing_identity=self.overlay.CERT_SHA1,runtime_provenance=Path(self.provenance_ref['path']),
            runtime_provenance_sha256=self.provenance_ref['sha256'])
        admission=types.SimpleNamespace(check=lambda:None,environment=lambda env:dict(env,
            ORYN_GENERATION_GATE=str(self.custody.GATE),ORYN_GENERATION_GATE_SHA256='e'*64))
        def run(command,env,start_receipt=None,log_path=None):
            Path(log_path).write_text(json.dumps(self.parent) if command[-2:]==['native','--runtime-info'] else 'canonical build log')
            return dict(self.good_process)
        with mock.patch.object(self.custody,'check',return_value=self.whole), \
             mock.patch.object(self.custody,'run_owned_dispatch',side_effect=run) as execute:
            self.assertEqual(self.overlay.prepare(args,self.custody,admission),0)
            self.assertEqual([call.args[0] for call in execute.call_args_list],
                [self.overlay.COMMAND,[self.artifacts['cli']['resolved_path'],'native','--runtime-info']])
        reference=json.loads((output/'signed-inputs.json').read_bytes())
        with mock.patch.object(self.custody,'run_owned_dispatch',side_effect=AssertionError('acceptance must not execute a build')):
            state=self.overlay.load_inputs(reference['path'],reference['sha256'],self.custody,self.lease,'f'*64,'e'*64)
            self.overlay.check_inputs(state,self.custody)
        self.assertEqual(state['value']['artifacts'],self.artifacts)
        self.assertTrue(state['value']['prepared'])
        self.assertEqual(state['value']['parent_runtime_command'],[self.artifacts['cli']['resolved_path'],'native','--runtime-info'])

    def test_baseline_checker_receives_only_baseline_environment_and_fresh14_outputs(self):
        checker=self.base/'checker.py'; checker.write_text('# synthetic checker')
        config=self.base/'environment.json';config.write_text('{}')
        baseline=dict(ORYN_WPT_PYTHON='/venv/python',PATH='node:/bin',ORYN_REPAIR49_SUPERVISED_GROUP='outer-owned-session')
        def run(command,env):
            self.assertFalse(set(env)&set(self.overlay.KEYS))
            self.assertNotIn('ORYN_REPAIR49_SUPERVISED_GROUP',env)
            self.assertIn('--check-process-environment',command)
            output=Path(command[command.index('--output')+1]);output.write_text('{"status":"passed"}')
            return dict(self.good_process)
        with mock.patch.object(self.custody,'ENVIRONMENT',config),mock.patch.object(self.custody,'ENVIRONMENT_SHA',self.custody.sha(config)), \
             mock.patch.object(self.custody,'ENVIRONMENT_CHECKER',checker),mock.patch.object(self.custody,'ENVIRONMENT_CHECKER_SHA',self.custody.sha(checker)), \
             mock.patch.object(self.custody,'run_owned_dispatch',side_effect=run):
            before=self.custody.environment_check(baseline,'before')
            after=self.custody.environment_check(baseline,'after',before['sha256'])
        self.assertEqual(before['scope'],'baseline82_without_signed_overlay')
        self.assertIn('python14-base-environment-before',before['path'])
        self.assertIn('python14-base-environment-after',after['path'])

    def test_all_twelve_former_signed_skip_tests_are_selected_by_the_explicit_overlay(self):
        environment=dict(self.value['overlay'],ORYN_WPT_CHECKOUT='/fixture/wpt',ORYN_WPT_NODE='/fixture/node',ORYN_WPT_CHROMIUM='/fixture/chrome')
        handoff=json.loads((ROOT/'artifacts/completion-validation-2026-09-18/repair48-downstream-dispatch-handoff-01.json').read_bytes())
        rows=handoff['python12_skipped_outcomes']
        self.assertEqual(len(rows),12)
        sources={ 'scripts.test_native_cli_errors':'scripts/test_native_cli_errors.py',
                  'scripts.tests.test_wpt_direct_fixture':'scripts/tests/test_wpt_direct_fixture.py',
                  'scripts.tests.test_wpt_session_live':'scripts/tests/test_wpt_session_live.py' }
        selected=0
        with mock.patch.dict(os.environ,environment,clear=True):
            for prefix,relative in sources.items():
                with RUNNER.SourceImports(ROOT/relative,ROOT,11000+selected) as context:
                    module=context.load()
                    for row in rows:
                        if '.'+prefix+'.' not in row['test']:continue
                        cls,name=row['test'].rsplit('.',2)[1:]
                        case=getattr(module,cls);method=getattr(case,name)
                        self.assertFalse(getattr(case,'__unittest_skip__',False))
                        self.assertFalse(getattr(method,'__unittest_skip__',False))
                        selected+=1
        self.assertEqual(selected,12)

class CandidateExecutionIdentityTests(unittest.TestCase):
    """Small real metadata graphs; all execution in these regressions is mocked."""
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory=Path(self.temporary.name).resolve()
        self.sequence=0

    def candidate(self,number):
        import types
        self.sequence+=1
        root=self.directory/str(self.sequence);root.mkdir()
        base=root/'artifacts/completion-validation-2026-09-18';base.mkdir(parents=True)
        (root/'docs/oryn2').mkdir(parents=True);(root/'scripts').mkdir()
        helper=HARNESS/'custody.py'
        custody=SignedPythonOverlayTests.module(helper,'candidate_execution_test_'+str(self.sequence))
        custody.ROOT=root;custody.BASE=base;custody.CANDIDATE_BINDING=None
        def write(path,value):path.write_text(json.dumps(value));return dict(path=str(path.relative_to(root)),sha256=custody.sha(path))
        review=base/'review.json';review_ref=write(review,{'reviewed':True})
        prior_gate=None;prior_lease=None;sources=[];reviews=[review_ref]
        gates={};leases={}
        for current in range(48,number+1):
            source=root/'scripts'/('test_source_'+str(current)+'.py');source.write_text('VALUE='+str(current)+'\n')
            sources=sources+[dict(path=str(source.relative_to(root)),sha256=custody.sha(source))]
            gate=root/'docs/oryn2'/f'GENERATION-COMPLETE-REPAIR-{current}-2026-09-18.json'
            lease=base/f'repair{current}-whole-source-lease-inputs-01.json'
            if prior_gate is not None:reviews=reviews+[prior_lease,prior_gate]
            value=dict(schema_version=1,kind=f'repair{current}_whole_source_and_review_lease_inputs',
                baseline=prior_lease,expected_declaration_path=str(gate.relative_to(root)),
                authored_discovery_roots=['scripts'],sources=list(sources),source_count=len(sources),review_sources=list(reviews))
            lease_ref=write(lease,value)
            declaration=dict(review_sources=reviews+[lease_ref],requirements={},
                previous_declaration=prior_gate,parent_declaration=prior_gate)
            gate_ref=write(gate,declaration)
            gates[current]=gate;leases[current]=lease
            prior_gate,prior_lease=gate_ref,lease_ref
        custody.HISTORICAL_LEASE=leases[48];custody.HISTORICAL_LEASE_SHA=custody.sha(leases[48])
        return types.SimpleNamespace(custody=custody,root=root,base=base,gate=gates[number],lease=leases[number],
            gates=gates,leases=leases,number=number,write=write)

    def configure(self,case):
        return case.custody.configure_candidate(case.gate,case.custody.sha(case.gate),case.lease,case.custody.sha(case.lease))

    def refresh_lease(self,case,value):
        old=json.loads(case.gate.read_bytes())
        ref=case.write(case.lease,value)
        old['review_sources']=[row for row in old['review_sources'] if row['path']!=ref['path']]+[ref]
        case.write(case.gate,old)

    def status_transition(self):
        case = self.candidate(50)
        c = case.custody
        name = 'docs/oryn2/BUILD-AND-QUALIFICATION-PROGRESS-2026-09-18.md'
        target = case.root / name
        target.write_text('new status\n')
        archive = case.base / 'retained-status.md'
        archive.write_text('old status\n')
        archive_ref = dict(path=str(archive.relative_to(case.root)), sha256=c.sha(archive))
        core = dict(path=name, before_sha256=c.sha(archive), after_sha256=c.sha(target),
                    archived_before=archive_ref)
        review = case.base / 'status-transition-review.json'
        review_ref = case.write(review, dict(approved=True, review_identity_transitions=[core]))
        transition = dict(core, review=review_ref)
        new = dict(path=name, sha256=c.sha(target))
        data = dict(sources=[new], review_sources=[new, archive_ref, review_ref],
                    review_identity_transitions=[transition])
        previous = dict(review_sources=[dict(path=name, sha256=c.sha(archive))])
        bound = {(row['path'], row['sha256']) for row in data['review_sources']}
        return case, data, previous, bound, archive, review

    def test_status_review_transition_retains_exact_archived_bytes_and_explicit_review(self):
        case, data, previous, bound, archive, review = self.status_transition()
        case.custody.retained_review_identities(data, previous, bound)
        archive.write_text('substituted old status\n')
        with self.assertRaisesRegex(ValueError, 'archive bytes'):
            case.custody.retained_review_identities(data, previous, bound)

    def test_status_review_transition_rejects_unreviewed_or_unbound_replacements(self):
        import copy
        for mutation in ('missing', 'duplicate', 'source', 'archive_binding', 'wrong_before',
                         'outside', 'unapproved', 'wrong_review'):
            with self.subTest(mutation=mutation):
                case, data, previous, bound, archive, review = self.status_transition()
                row = data['review_identity_transitions'][0]
                if mutation == 'missing': data['review_identity_transitions'] = []
                elif mutation == 'duplicate': data['review_identity_transitions'].append(copy.deepcopy(row))
                elif mutation == 'source': data['sources'] = []
                elif mutation == 'archive_binding': bound.remove((row['archived_before']['path'], row['archived_before']['sha256']))
                elif mutation == 'wrong_before': row['before_sha256'] = '0' * 64
                elif mutation == 'outside': row['archived_before']['path'] = '../outside.md'
                else:
                    value = dict(approved=mutation != 'unapproved', review_identity_transitions=[])
                    ref = case.write(review, value)
                    data['review_sources'][-1] = ref
                    row['review'] = ref
                    bound = {(r['path'], r['sha256']) for r in data['review_sources']}
                with self.assertRaises(ValueError):
                    case.custody.retained_review_identities(data, previous, bound)

    def test_status_transition_cannot_waive_other_historical_reviews(self):
        case, data, previous, bound, archive, review = self.status_transition()
        previous['review_sources'].append(dict(path='scripts/qualification/custody.py', sha256='a' * 64))
        with self.assertRaisesRegex(ValueError, 'historical review'):
            case.custody.retained_review_identities(data, previous, bound)
        case, data, previous, bound, archive, review = self.status_transition()
        row = data['review_identity_transitions'][0]
        row['path'] = 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'
        with self.assertRaisesRegex(ValueError, 'explicit current source'):
            case.custody.retained_review_identities(data, previous, bound)

    def test_candidates49_and50_use_exact_metadata_and_immediate_predecessor_with_floor(self):
        for number in (49,50):
            case=self.candidate(number);binding=self.configure(case);c=case.custody
            self.assertEqual(binding['number'],str(number))
            self.assertEqual(binding['supervisor_key'],f'ORYN_REPAIR{number}_SUPERVISED_GROUP')
            self.assertEqual(c.PREVIOUS_LEASE,case.leases[number-1])
            self.assertEqual(c.HISTORICAL_LEASE,case.leases[48])
            self.assertEqual(self.configure(case),binding)
            self.assertFalse(c.check(case.lease,c.sha(case.lease),c.sha(case.gate))['errors'])

    def test_configured_helper_cannot_switch_candidate_or_digest(self):
        case=self.candidate(50);c=case.custody;binding=self.configure(case)
        with self.assertRaisesRegex(ValueError,'already bound'):
            c.configure_candidate(case.gates[49],c.sha(case.gates[49]),case.leases[49],c.sha(case.leases[49]))
        with self.assertRaises(ValueError):c.configure_candidate(case.gate,'0'*64,case.lease,c.sha(case.lease))
        self.assertEqual(self.configure(case),binding)
        self.assertEqual(c.GATE,case.gate)

    def test_candidate_rejects_alias_unbound_lease_and_filename_metadata_mismatch(self):
        case=self.candidate(50);c=case.custody
        alias=case.base/'gate-alias';alias.symlink_to(case.gate)
        with self.assertRaises(ValueError):c.configure_candidate(alias,c.sha(case.gate),case.lease,c.sha(case.lease))
        with self.assertRaises(ValueError):c.configure_candidate(case.gate,'0'*64,case.lease,c.sha(case.lease))
        for field,value in [('kind','repair49_whole_source_and_review_lease_inputs'),('expected_declaration_path','docs/old.json')]:
            case=self.candidate(50);data=json.loads(case.lease.read_bytes());data[field]=value;self.refresh_lease(case,data)
            with self.assertRaisesRegex(ValueError,'metadata'):self.configure(case)
        case=self.candidate(50);gate=json.loads(case.gate.read_bytes());gate['review_sources']=[];case.write(case.gate,gate)
        with self.assertRaises(ValueError):self.configure(case)

    def test_predecessor_cannot_skip49_or_substitute_a_different_declared_parent(self):
        case=self.candidate(50);data=json.loads(case.lease.read_bytes())
        data['baseline']=dict(path=str(case.leases[48].relative_to(case.root)),sha256=case.custody.sha(case.leases[48]))
        self.refresh_lease(case,data)
        with self.assertRaisesRegex(ValueError,'predecessor'):self.configure(case)
        case=self.candidate(50);gate=json.loads(case.gate.read_bytes());gate['parent_declaration']={'path':'different','sha256':'0'*64};case.write(case.gate,gate)
        with self.assertRaisesRegex(ValueError,'predecessor'):self.configure(case)

    def test_preserved_sources_reviews_and_discovery_still_reject_shrink_and_extra_files(self):
        case=self.candidate(50);value=json.loads(case.lease.read_bytes());value['sources']=value['sources'][1:];value['source_count']=len(value['sources']);self.refresh_lease(case,value)
        self.configure(case);c=case.custody
        self.assertTrue(any('floor source' in error for error in c.check(case.lease,c.sha(case.lease),c.sha(case.gate))['errors']))
        case=self.candidate(50);self.configure(case);c=case.custody
        (case.root/'scripts/test_unbound.py').write_text('UNBOUND=True\n')
        self.assertTrue(any('added:' in error for error in c.check(case.lease,c.sha(case.lease),c.sha(case.gate))['errors']))
        case=self.candidate(50);gate=json.loads(case.gate.read_bytes());gate['review_sources']=[row for row in gate['review_sources'] if not row['path'].endswith('/review.json')];case.write(case.gate,gate)
        self.configure(case);c=case.custody
        self.assertTrue(c.check(case.lease,c.sha(case.lease),c.sha(case.gate))['errors'])

    def test_fresh_output_admission_keeps_existing_evidence_and_rejects_aliases(self):
        case=self.candidate(50);c=case.custody
        output=c.fresh_output_directory(case.base/'attempt')
        evidence=output/'whole-custody.json';evidence.write_text('original receipt')
        with self.assertRaises(ValueError):c.fresh_output_directory(output)
        alias=case.base/'output-alias';alias.symlink_to(output,target_is_directory=True)
        with self.assertRaises(ValueError):c.fresh_output_directory(alias)
        with self.assertRaises(ValueError):c.fresh_output_directory(case.root/'outside-artifacts')
        self.assertEqual(evidence.read_text(),'original receipt')
        self.assertNotEqual(c.fresh_output_directory(case.base/'attempt-2'),output)

    def test_successor_supervision_discards_stale_tags_and_generation_environment_wins_last(self):
        import types
        case=self.candidate(50);self.configure(case);c=case.custody
        runtime=dict(profile=dict(path='/profile',sha256='a'*64),environment={})
        admission=types.SimpleNamespace(environment=lambda env:dict(env,ORYN_GENERATION_GATE=str(case.gate),ORYN_GENERATION_GATE_SHA256=c.sha(case.gate)))
        with mock.patch.dict(os.environ,{'ORYN_REPAIR49_SUPERVISED_GROUP':'old','ORYN_GENERATION_GATE':'old'},clear=True):
            env,command=c.build_environment(admission,False,case.base/'native.py',c.sha(case.gate),runtime)
        self.assertNotIn('ORYN_REPAIR49_SUPERVISED_GROUP',env)
        self.assertEqual(env['ORYN_REPAIR50_SUPERVISED_GROUP'],'outer-owned-session')
        self.assertEqual(env['ORYN_GENERATION_GATE'],str(case.gate))
        with mock.patch.object(LAUNCH,'SUPERVISOR_KEY',c.SUPERVISOR_KEY),mock.patch.object(LAUNCH.os,'getpid',return_value=10),mock.patch.object(LAUNCH.os,'getpgrp',return_value=10):
            self.assertEqual(LAUNCH.supervision(env)['mode'],'outer-owned')
            self.assertEqual(LAUNCH.supervision({'ORYN_REPAIR49_SUPERVISED_GROUP':'outer-owned-session'})['mode'],'standalone')

    def test_root_dispatch_passes_candidate_profile_and_fresh_child_output_without_renumbering_helpers(self):
        import types
        case=self.candidate(50);c=case.custody;output=case.base/'native-attempt'
        profile=case.base/'profile.json';profile.write_text('{}');provenance=case.base/'provenance.json';provenance.write_text('{}')
        runtime=dict(profile=dict(path=str(profile),sha256=c.sha(profile)),records=[],environment={})
        provenance_value=dict(path=str(provenance),sha256=c.sha(provenance))
        process=dict(returncode=0,execution_error=None,cleanup_errors=[],group_settled=True)
        admission=types.SimpleNamespace(check=lambda:None,environment=lambda env:dict(env,
            ORYN_GENERATION_GATE=str(case.gate),ORYN_GENERATION_GATE_SHA256=c.sha(case.gate)))
        argv=['custody','native35','--gate',str(case.gate),'--gate-sha256',c.sha(case.gate),
            '--whole-lease',str(case.lease),'--whole-lease-sha256',c.sha(case.lease),'--output-directory',str(output),
            '--runtime-profile',str(profile),'--runtime-profile-sha256',c.sha(profile),
            '--runtime-provenance',str(provenance),'--runtime-provenance-sha256',c.sha(provenance)]
        old_path=sys.path[:]
        self.addCleanup(lambda:sys.path.__setitem__(slice(None),old_path))
        with mock.patch.object(sys,'argv',argv),mock.patch.dict(sys.modules,{c.__name__:c,'generation_gate':types.SimpleNamespace(GenerationLease=lambda *a,**k:admission)}), \
             mock.patch.object(c,'bound_harness'),mock.patch.object(c,'load_runtime',return_value=runtime), \
             mock.patch.object(c,'runtime_check',return_value={'errors':[]}),mock.patch.object(c,'load_runtime_provenance',return_value=provenance_value), \
             mock.patch.object(c,'check_runtime_provenance',return_value={'errors':[]}),mock.patch.object(c,'run_owned_dispatch',return_value=process) as execute, \
             mock.patch('sys.stdout',new_callable=io.StringIO):
            self.assertEqual(c.main(),0)
        command,env=execute.call_args.args
        self.assertEqual(command[1],str(case.root/'scripts/qualification/native.py'))
        self.assertEqual(command[command.index('--gate')+1],str(case.gate))
        self.assertEqual(command[command.index('--gate-sha256')+1],c.sha(case.gate))
        self.assertEqual(command[command.index('--output-directory')+1],str(output/'native'))
        self.assertEqual(env['ORYN_REPAIR50_SUPERVISED_GROUP'],'outer-owned-session')
        self.assertTrue((output/'whole-custody.json').is_file())
        self.assertFalse((case.base/'repair49-native35-whole-custody.json').exists())

    def test_signed_receipt_cannot_cross_candidates_even_when_profile_and_binary_match(self):
        overlay=SignedPythonOverlayTests.module(HARNESS/'signed_inputs.py','candidate_signed_test')
        case=self.candidate(50);self.configure(case);c=case.custody
        artifacts=dict(cli={'resolved_path':'/cli'},worker={'resolved_path':'/worker'})
        whole=dict(errors=[],declaration_sha256=c.sha(case.gate),lease_sha256=c.sha(case.lease))
        value=dict(schema_version=1,kind='repair50_signed_python_inputs',mode='canonical_preparation',prepared=True,
            generation=dict(path=str(case.gate),sha256=c.sha(case.gate)),whole_lease=dict(path=str(case.lease),sha256=c.sha(case.lease)),
            producer_sources={},command=overlay.COMMAND,process=dict(returncode=0,execution_error=None,cleanup_errors=[],group_settled=True),
            whole_source_before=whole,whole_source_after=whole,signing_identity=overlay.CERT_SHA1,
            artifacts=artifacts,overlay=overlay.overlay_for(artifacts))
        with mock.patch.object(overlay,'source_identities',return_value={}):
            overlay.validate_record(value,c,case.lease,c.sha(case.lease),c.sha(case.gate))
            for changed in (dict(value,kind='repair49_signed_python_inputs'),dict(value,generation=dict(path=str(case.gates[49]),sha256=c.sha(case.gates[49])))):
                with self.assertRaises(ValueError):overlay.validate_record(changed,c,case.lease,c.sha(case.lease),c.sha(case.gate))

    def test_python_launcher_propagates_exact_candidate_and_fresh_output_to_runner(self):
        import types
        case=self.candidate(50);c=case.custody
        harness=case.root/'scripts/qualification';harness.mkdir()
        for name in ('launch.py','runner.py'):(harness/name).write_text('# fixture source\n')
        gate=json.loads(case.gate.read_bytes());gate['requirements']={'Q':dict(implementation_sources=[],test_sources=[dict(path=str((harness/name).relative_to(case.root)),sha256=c.sha(harness/name)) for name in ('launch.py','runner.py')])};case.write(case.gate,gate)
        self.configure(case)
        binary=case.base/'native';binary.write_text('synthetic executable');binary.chmod(0o700)
        native=dict(ORYN_TEST_NATIVE_BIN=str(binary),ORYN_PAGE_WORKER=str(binary),ORYN_WPT_BINARY=str(binary))
        def environment(env=None):
            return dict(os.environ if env is None else env,**native,ORYN_GENERATION_GATE=str(case.gate),ORYN_GENERATION_GATE_SHA256=c.sha(case.gate))
        lease=types.SimpleNamespace(path=case.gate,sha256=c.sha(case.gate),check=lambda:None,environment=environment)
        helper=types.SimpleNamespace(check_inputs=mock.Mock())
        signed=dict(receipt={'sha256':'a'*64})
        source=case.root/'scripts/test_source_50.py'
        identities=[dict(path=str(source.relative_to(case.root)),sha256=c.sha(source))]
        output=case.base/'python-attempt'
        argv=['launch','--gate',str(case.gate),'--gate-sha256',lease.sha256,'--output-directory',str(output),
            '--whole-lease',str(case.lease),'--whole-lease-sha256',c.sha(case.lease),
            '--signed-inputs',str(case.base/'signed.json'),'--signed-inputs-sha256','a'*64]
        old_cwd=os.getcwd();old_path=sys.path[:]
        self.addCleanup(lambda:os.chdir(old_cwd));self.addCleanup(lambda:sys.path.__setitem__(slice(None),old_path))
        process=dict(returncode=0,execution_error=None,cleanup_errors=[],pid=123,supervision={'mode':'outer-owned'})
        with mock.patch.object(LAUNCH,'__file__',str(harness/'launch.py')),mock.patch.object(LAUNCH,'SUPERVISOR_KEY','ORYN_REPAIR49_SUPERVISED_GROUP'), \
             mock.patch.object(sys,'argv',argv),mock.patch.dict(sys.modules,{'generation_gate':types.SimpleNamespace(inherited_generation_lease=lambda:lease)}), \
             mock.patch.dict(os.environ,{'ORYN_REPAIR50_SUPERVISED_GROUP':'outer-owned-session'}), \
             mock.patch.object(LAUNCH.os,'getpid',return_value=123),mock.patch.object(LAUNCH.os,'getpgrp',return_value=123), \
             mock.patch.object(LAUNCH,'regression_sources',return_value=identities) as discovery,mock.patch.object(LAUNCH,'validate_population') as population, \
             mock.patch.object(LAUNCH,'admit_signed_inputs',return_value=(helper,c,signed)),mock.patch.object(LAUNCH,'run_child',return_value=process) as child, \
             mock.patch.object(LAUNCH,'read_results'),mock.patch('sys.stdout',new_callable=io.StringIO):
            self.assertEqual(LAUNCH.main(),0)
        command=child.call_args.args[0]
        self.assertEqual(command[1],str(harness/'runner.py'))
        self.assertEqual(command[0],str(case.base/'fq-python-venv/bin/python'))
        self.assertEqual(discovery.call_args.args[:2],(case.root,case.base/'fq-python-regressions-07/preflight.json'))
        self.assertEqual(command[command.index('--gate')+1],str(case.gate))
        self.assertEqual(command[command.index('--gate-sha256')+1],lease.sha256)
        self.assertEqual(command[command.index('--output-directory')+1],str(output))
        self.assertEqual(command[-1],str(source))
        self.assertEqual(child.call_args.args[1]['ORYN_REPAIR50_SUPERVISED_GROUP'],'outer-owned-session')
        population.assert_called_once_with(identities,case.base/'fq-python-regressions-12/preflight.json')
        self.assertTrue((output/'preflight.json').is_file())
        self.assertFalse((harness/'preflight.json').exists())

    def test_python_runner_rejects_different_explicit_gate_before_collection(self):
        import types
        case=self.candidate(50);output=case.base/'child';output.mkdir()
        harness=case.root/'scripts/qualification';harness.mkdir()
        lease=types.SimpleNamespace(path=case.gate,sha256=case.custody.sha(case.gate))
        argv=['runner','--gate',str(case.gates[49]),'--gate-sha256',case.custody.sha(case.gates[49]),
            '--output-directory',str(output),str(case.root/'scripts/test_source_50.py')]
        old_path=sys.path[:];self.addCleanup(lambda:sys.path.__setitem__(slice(None),old_path))
        with mock.patch.object(RUNNER,'__file__',str(harness/'runner.py')),mock.patch.object(sys,'argv',argv), \
             mock.patch.dict(sys.modules,{'generation_gate':types.SimpleNamespace(inherited_generation_lease=lambda:lease)}), \
             mock.patch.object(RUNNER,'run_sources') as collect:
            with self.assertRaisesRegex(ValueError,'explicit candidate'):RUNNER.main()
        collect.assert_not_called()
        self.assertFalse((output/'test-results.json').exists())


class CanonicalQualificationPathTests(unittest.TestCase):
    def test_canonical_helpers_keep_repository_root_and_require_current_source_bindings(self):
        custody=SignedPythonOverlayTests.module(HARNESS/'custody.py','canonical_custody_paths')
        signed=SignedPythonOverlayTests.module(HARNESS/'signed_inputs.py','canonical_signed_paths')
        native=SignedPythonOverlayTests.module(HARNESS/'native.py','canonical_native_paths')
        for module in (custody,signed,native):
            self.assertEqual(module.ROOT,ROOT)
        self.assertEqual(Path(signed.custody_module().__file__).resolve(),HARNESS/'custody.py')
        with mock.patch.object(custody,'bound_source') as bound:
            identities=signed.source_identities(custody)
        self.assertIn('scripts/qualification/custody.py',identities)
        self.assertIn('scripts/qualification/signed_inputs.py',identities)
        self.assertNotIn('artifacts/completion-validation-2026-09-18/repair49-root-execution-custody.py',identities)
        self.assertIn(mock.call(HARNESS/'custody.py'),bound.call_args_list)
        with tempfile.TemporaryDirectory() as directory:
            gate=Path(directory)/'gate.json'
            paths=[HARNESS/name for name in ('custody.py','launch.py','runner.py')]+[custody.ENVIRONMENT_CHECKER]
            records=[dict(path=str(path.relative_to(ROOT)),sha256=custody.sha(path)) for path in paths]
            gate.write_text(json.dumps(dict(requirements={'Q':dict(implementation_sources=records,test_sources=[])})))
            with mock.patch.object(custody,'GATE',gate):
                custody.bound_harness(HARNESS/'launch.py',True)
                records[0]=dict(path='artifacts/completion-validation-2026-09-18/repair49-root-execution-custody.py',sha256=custody.sha(HARNESS/'custody.py'))
                gate.write_text(json.dumps(dict(requirements={'Q':dict(implementation_sources=records,test_sources=[])})))
                with self.assertRaisesRegex(ValueError,'not bound'):
                    custody.bound_harness(HARNESS/'launch.py',True)

    def test_canonical_non_test_helpers_preserve104_without_discovery_exceptions(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory).resolve()
            previous=['scripts/test_preserved_'+str(index)+'.py' for index in range(103)]
            required=previous+[LAUNCH.ADMISSION_REGRESSION]
            helpers=['scripts/qualification/'+name for name in ('custody.py','signed_inputs.py','native.py','launch.py','runner.py')]
            bound={}
            for source in required+helpers:
                path=root/source;path.parent.mkdir(parents=True,exist_ok=True);path.write_text('VALUE=1\n')
                bound[source]=LAUNCH.digest(path)
            historical=root/'historical.json';historical.write_text(json.dumps(dict(test_sources=previous[:45])))
            recent=root/'previous.json';recent.write_text(json.dumps(dict(test_sources=previous)))
            with mock.patch.object(LAUNCH,'HISTORICAL_SHA256',LAUNCH.digest(historical)), \
                 mock.patch.object(LAUNCH,'PREVIOUS_PREFLIGHT_SHA256',LAUNCH.digest(recent)):
                identities=LAUNCH.regression_sources(root,historical,bound)
                self.assertEqual({row['path'] for row in identities},set(required))
                self.assertEqual(len(identities),104)
                LAUNCH.validate_population(identities,recent)
                unexpected=root/'scripts/qualification/test_unexpected.py';unexpected.write_text('VALUE=2\n')
                with self.assertRaisesRegex(ValueError,'not bound'):
                    LAUNCH.regression_sources(root,historical,bound)
                bound[str(unexpected.relative_to(root))]=LAUNCH.digest(unexpected)
                with self.assertRaisesRegex(ValueError,'exactly'):
                    LAUNCH.validate_population(LAUNCH.regression_sources(root,historical,bound),recent)

    def test_canonical_runner_default_import_root_is_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            source=Path(directory)/'test_example.py';source.write_text(PASSING)
            with mock.patch.object(RUNNER,'SourceImports',wraps=RUNNER.SourceImports) as imports:
                result=RUNNER.run_sources([str(source)],stream=io.StringIO())
            self.assertEqual(imports.call_args.args[1],ROOT)
            self.assertFalse(result['collection_errors'])
            self.assertEqual(result['test_outcomes'][0]['status'],'passed')


if __name__ == '__main__':
    unittest.main()

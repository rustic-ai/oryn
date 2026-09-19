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
HARNESS = ROOT / 'artifacts/completion-validation-2026-09-18/fq-python-regressions-12'


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


if __name__ == '__main__':
    unittest.main()

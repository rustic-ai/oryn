"""Per-source collection and terminal outcomes for one admitted Python run."""
import hashlib
import importlib
import importlib.util
import importlib.machinery
import json
import os
from pathlib import Path
import sys
import traceback
import types
import unittest


def cases(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from cases(test)
        else:
            yield test


class SourceImports:
    """Keep one source's sibling/package imports live through fixture teardown."""
    def __init__(self, source, root, index):
        self.source = Path(source).resolve()
        self.root = Path(root).resolve()
        self.base = self.root if self.source.is_relative_to(self.root) else self.source.parent
        self.prefix = 'fq_regression_' + str(index)
        relative = self.source.relative_to(self.base).with_suffix('')
        self.name = self.prefix + '.' + '.'.join(relative.parts)
        self.targets = {path.stem: path for path in self.source.parent.glob('*.py') if path.stem != '__init__'}
        self.targets.update({path.name: path for path in self.source.parent.iterdir()
                             if path.is_dir() and path.name.isidentifier() and path.name != '__pycache__'})
        if (self.root / 'scripts').is_dir():
            self.targets['scripts'] = self.root / 'scripts'
        self.roots = set(self.targets) | {self.prefix}
        self.loaded = None

    def owns(self, name):
        return name.partition('.')[0] in self.roots

    @staticmethod
    def namespace(name, path):
        module = types.ModuleType(name)
        module.__package__ = name
        module.__path__ = [str(path)]
        module.__spec__ = importlib.machinery.ModuleSpec(name, loader=None, is_package=True)
        module.__spec__.submodule_search_locations = module.__path__
        return module

    def matches(self, name, module):
        target = self.targets.get(name.partition('.')[0])
        if target is None:
            return False
        filename = getattr(module, '__file__', None)
        if filename:
            resolved = Path(filename).resolve()
            return resolved == target if target.is_file() else resolved.is_relative_to(target)
        return any(Path(path).resolve() == target for path in getattr(module, '__path__', ()))

    def __enter__(self):
        self.previous_path = sys.path[:]
        self.previous_modules = {name: module for name, module in sys.modules.copy().items() if self.owns(name)}
        try:
            sys.path[:] = list(dict.fromkeys(map(str, (self.source.parent, self.root, self.root / 'scripts')))) + self.previous_path
            for name, module in self.previous_modules.items():
                if self.loaded is not None or not self.matches(name, module):
                    sys.modules.pop(name, None)
            if self.loaded is not None:
                sys.modules.update(self.loaded)
            else:
                sys.modules[self.prefix] = self.namespace(self.prefix, self.base)
                scripts = self.root / 'scripts'
                if scripts.is_dir() and not (scripts / '__init__.py').exists():
                    # An installed regular package otherwise outranks a local namespace.
                    sys.modules['scripts'] = self.namespace('scripts', scripts)
            return self
        except BaseException:
            self.restore()
            raise

    def restore(self):
        for name in list(sys.modules):
            if self.owns(name):
                sys.modules.pop(name, None)
        sys.modules.update(self.previous_modules)
        sys.path[:] = self.previous_path

    def __exit__(self, *_):
        self.loaded = {name: module for name, module in sys.modules.copy().items() if self.owns(name)}
        self.restore()

    def load(self):
        importlib.import_module(self.name.rpartition('.')[0])
        spec = importlib.util.spec_from_file_location(self.name, self.source)
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        return module


class SourceSuites(unittest.TestSuite):
    """Each inner suite owns complete unittest class/module fixture teardown."""
    def __init__(self):
        super().__init__()
        self.contexts = []

    def add_source(self, tests, context):
        self.addTest(tests)
        self.contexts.append(context)

    def run(self, result, debug=False):
        # Do not enter an outer unittest fixture scope. Inner top-level suites
        # tear down their modules before their import contexts are restored.
        for tests, context in zip(self, self.contexts):
            if result.shouldStop:
                break
            with context:
                try:
                    tests.run(result, debug=debug)
                finally:
                    try:
                        if getattr(result, '_testRunEntered', False):
                            tests._tearDownPreviousClass(None, result)
                            tests._handleModuleTearDown(result)
                    finally:
                        result._testRunEntered = False
                        result._previousTestClass = None
        return result


class LedgerResult(unittest.TextTestResult):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.outcomes = {}
        self.finished = []

    def addSuccess(self, test):
        super().addSuccess(test)
        self.outcomes.setdefault(test.id(), 'passed')

    def addFailure(self, test, err):
        super().addFailure(test, err)
        self.outcomes[test.id()] = 'failed'

    def addError(self, test, err):
        super().addError(test, err)
        self.outcomes[test.id()] = 'error'

    def addSkip(self, test, reason):
        super().addSkip(test, reason)
        self.outcomes[test.id()] = 'skipped'

    def addExpectedFailure(self, test, err):
        super().addExpectedFailure(test, err)
        self.outcomes[test.id()] = 'expected_failure'

    def addUnexpectedSuccess(self, test):
        super().addUnexpectedSuccess(test)
        self.outcomes[test.id()] = 'unexpected_success'

    def addSubTest(self, test, subtest, err):
        super().addSubTest(test, subtest, err)
        if err is not None:
            self.outcomes[test.id()] = 'subtest_failed'

    def stopTest(self, test):
        self.finished.append(test.id())
        super().stopTest(test)


class LedgerRunner(unittest.TextTestRunner):
    resultclass = LedgerResult

    def _makeResult(self):
        self.result = super()._makeResult()
        return self.result


def run_sources(paths, startup_errors=(), stream=None, root=None):
    root = Path(root) if root is not None else Path(__file__).resolve().parent.parents[2]
    suite = SourceSuites()
    collection_errors = list(startup_errors)
    sources = []
    for index, path in enumerate(paths):
        row = dict(path=path, count=0, test_ids=[])
        sources.append(row)
        print('TEST_SOURCE ' + path, flush=True)
        try:
            context = SourceImports(path, root, index)
            with context:
                module = context.load()
                loader = unittest.TestLoader()
                tests = loader.loadTestsFromModule(module)
                collected = list(cases(tests))
                row['test_ids'] = [test.id() for test in collected]
                for test in collected:
                    if test.__class__.__module__ != module.__name__:
                        collection_errors.append(dict(path=path, traceback='test class belongs to another source: ' + test.id()))
                row['count'] = len(row['test_ids'])
                if not row['count']:
                    collection_errors.append(dict(path=path, traceback='source collected zero unittest cases'))
                collection_errors.extend(dict(path=path, traceback=detail) for detail in loader.errors)
            suite.add_source(tests, context)
        except (Exception, SystemExit):
            detail = traceback.format_exc()
            collection_errors.append(dict(path=path, traceback=detail))
            print(detail, flush=True)
    identifiers = [name for row in sources for name in row['test_ids']]
    if len(set(identifiers)) != len(identifiers):
        collection_errors.append(dict(path=None, traceback='duplicate collected test identifiers'))
    print('DISCOVERED_TEST_COUNT ' + str(len(identifiers)), flush=True)
    execution_errors = []
    runner = LedgerRunner(verbosity=2, stream=stream)
    try:
        result = runner.run(suite)
    except BaseException:
        result = runner.result
        execution_errors.append(traceback.format_exc())
    return dict(tests=result.testsRun, discovered_tests=len(identifiers), sources=sources,
                test_outcomes=[dict(test=name, status=result.outcomes.get(name, 'not_run')) for name in identifiers],
                execution_complete=bool(identifiers) and sorted(result.finished) == sorted(identifiers),
                failures=[dict(test=str(test), traceback=trace) for test, trace in result.failures],
                errors=[dict(test=str(test), traceback=trace) for test, trace in result.errors],
                skipped=[dict(test=str(test), reason=reason) for test, reason in result.skipped],
                expected_failures=[dict(test=str(test), traceback=trace) for test, trace in result.expectedFailures],
                unexpected_successes=[str(test) for test in result.unexpectedSuccesses],
                collection_errors=collection_errors, execution_errors=execution_errors)


def main():
    output = Path(__file__).resolve().parent
    root = output.parents[2]
    sys.path.insert(0, str(root / 'scripts'))
    from generation_gate import inherited_generation_lease
    lease = inherited_generation_lease()
    if lease.path != (root / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-40-2026-09-18.json').resolve():
        raise ValueError('Python child requires the repair40 declaration')
    preflight = json.loads((output / 'preflight.json').read_text())
    if preflight['generation_gate_sha256'] != lease.sha256 or Path(preflight['generation_gate']).resolve() != lease.path:
        raise ValueError('Python child did not inherit the admitted generation')
    if os.environ.get('ORYN_PYTHON_RUN_ID') != preflight['run_id']:
        raise ValueError('Python child invocation differs from preflight')
    expected = [str(root / source['path']) for source in preflight['source_identities']]
    if sys.argv[1:] != expected:
        raise ValueError('Python child source population differs from preflight')
    for source, path in zip(preflight['source_identities'], expected):
        if hashlib.sha256(Path(path).read_bytes()).hexdigest() != source['sha256']:
            raise ValueError('Python child source changed: ' + source['path'])
    startup_errors = []
    try:
        import localpaths  # noqa: F401; pinned upstream WPT package setup
    except Exception:
        startup_errors.append(dict(path='localpaths', traceback=traceback.format_exc()))
    record = run_sources(expected, startup_errors, root=root)
    for row, source in zip(record['sources'], preflight['source_identities']):
        row['path'] = source['path']
    record.update(run_id=preflight['run_id'], generation_gate_sha256=lease.sha256,
                  source_identities=preflight['source_identities'])
    try:
        lease.check()
    except Exception:
        record['execution_errors'].append(traceback.format_exc())
    with (output / 'test-results.json').open('x') as stream:
        json.dump(record, stream, indent=2)
        stream.write('\n')
    print('FINAL_COUNTS ' + json.dumps(dict(tests=record['tests'], discovered_tests=record['discovered_tests'],
          **{key: len(record[key]) for key in ('failures', 'errors', 'skipped', 'expected_failures', 'unexpected_successes', 'collection_errors', 'execution_errors')})), flush=True)
    failed = any(record[key] for key in ('failures', 'errors', 'skipped', 'expected_failures', 'unexpected_successes', 'collection_errors', 'execution_errors'))
    return int(failed or not record['execution_complete'])


if __name__ == '__main__':
    sys.exit(main())

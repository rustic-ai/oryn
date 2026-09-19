"""Per-source collection and terminal outcomes for one admitted Python run."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import traceback
import unittest


def cases(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from cases(test)
        else:
            yield test


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


def run_sources(paths, startup_errors=(), stream=None):
    suite = unittest.TestSuite()
    collection_errors = list(startup_errors)
    sources = []
    saved_modules = {}
    for index, path in enumerate(paths):
        row = dict(path=path, count=0, test_ids=[])
        sources.append(row)
        print('TEST_SOURCE ' + path, flush=True)
        try:
            spec = importlib.util.spec_from_file_location('fq_regression_' + str(index), path)
            module = importlib.util.module_from_spec(spec)
            saved_modules[spec.name] = sys.modules.get(spec.name)
            sys.modules[spec.name] = module
            spec.loader.exec_module(module)
            loader = unittest.TestLoader()
            tests = loader.loadTestsFromModule(module)
            row['test_ids'] = [test.id() for test in cases(tests)]
            row['count'] = len(row['test_ids'])
            if not row['count']:
                collection_errors.append(dict(path=path, traceback='source collected zero unittest cases'))
            collection_errors.extend(dict(path=path, traceback=detail) for detail in loader.errors)
            suite.addTests(tests)
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
    for name, previous in saved_modules.items():
        if previous is None:
            sys.modules.pop(name, None)
        else:
            sys.modules[name] = previous
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
    if lease.path != (root / 'docs/oryn2/GENERATION-COMPLETE-REPAIR-39-2026-09-18.json').resolve():
        raise ValueError('Python child requires the repair39 declaration')
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
    record = run_sources(expected, startup_errors)
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

#!/usr/bin/env python3
"""Pinned Python bootstrap; all test algorithms remain upstream-owned."""
import argparse
from pathlib import Path
import sys
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--checkout', type=Path, required=True)
    parser.add_argument('--generation-gate', type=Path, required=True)
    parser.add_argument('mode', choices=['run', 'manifest', 'protocol-controls'])
    args, remaining = parser.parse_known_args()
    from wpt_official_workflow import generation_gate
    from generation_gate import GenerationLease
    import os
    expected = os.environ.get("ORYN_GENERATION_GATE_SHA256")
    inherited = os.environ.get("ORYN_GENERATION_GATE")
    if inherited is not None and Path(inherited).resolve() != args.generation_gate.resolve():
        raise ValueError("upstream child generation declaration differs from parent")
    admission = GenerationLease(args.generation_gate, validator=generation_gate, expected=expected)
    checkout = args.checkout.resolve()
    from wpt_full_qualification import REVISION
    revision = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    if revision != REVISION:
        raise ValueError('upstream runner checkout revision changed')
    changes = subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain', '--untracked-files=all', '--', 'tools'], text=True)
    if changes.strip():
        raise ValueError('pinned upstream tooling has local modifications')
    admission.check()
    sys.path.insert(0, str(checkout / 'tools'))
    import localpaths  # noqa: F401 -- registers pinned upstream dependencies
    if args.mode == 'protocol-controls':
        import json
        import unittest
        parser = argparse.ArgumentParser()
        parser.add_argument('--output', type=Path, required=True)
        options = parser.parse_args(remaining)
        admission.check()
        suite = unittest.defaultTestLoader.discover(str(Path(__file__).resolve().parents[1] / 'tools/wptrunner-oryn/tests'), pattern='test_protocol_faults.py')
        def identities(item):
            if isinstance(item, unittest.TestSuite):
                return [name for child in item for name in identities(child)]
            return [item.id()]
        names = identities(suite)
        admission.check()
        result = unittest.TestResult(); suite.run(result)
        admission.check()
        with options.output.open('x') as stream:
            json.dump({'schema_version': 1, 'tests': names, 'tests_run': result.testsRun,
                       'successful': result.wasSuccessful(), 'failures': [(test.id(), error) for test, error in result.failures],
                       'errors': [(test.id(), error) for test, error in result.errors],
                       'skipped': [(test.id(), reason) for test, reason in result.skipped]}, stream)
        return int(not result.wasSuccessful())
    if args.mode == 'manifest':
        parser = argparse.ArgumentParser()
        parser.add_argument('--tests', required=True)
        parser.add_argument('--output', required=True)
        parser.add_argument('--url-base', required=True)
        options = parser.parse_args(remaining)
        from manifest.manifest import load_and_update
        admission.check()
        load_and_update(options.tests, options.output, options.url_base,
                        update=True, rebuild=True, parallel=False)
        admission.check()
        return 0
    from wptrunner import wptrunner
    sys.argv = [sys.argv[0], *remaining]
    admission.check()
    result = wptrunner.main()
    admission.check()
    return result


if __name__ == '__main__':
    sys.exit(main())

#!/usr/bin/env python3
"""Pinned Python bootstrap; all test algorithms remain upstream-owned."""
import argparse
from pathlib import Path
import sys
import subprocess


def run_upstream(main, remaining, admission, gate_path):
    # Parse only our existing exact option names; upstream still owns its CLI.
    selector = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    selector.add_argument('--oryn-run-contract')
    selector.add_argument('--binary')
    selector.add_argument('--webdriver-binary')
    selected, _ = selector.parse_known_args(remaining)
    from oryn_wptrunner.provenance import run_admission, _checked_exit
    # Outer generation finalization also runs if admission/full-finalization fails.
    with _checked_exit(admission.final_check):
        admission.check()
        with run_admission(selected.oryn_run_contract, gate_path,
                           selected.binary, selected.webdriver_binary):
            from oryn_wptrunner.recovery import manager_recovery
            with manager_recovery():
                return main()


def process_exit(main):
    """Reserve 65 for rejected integrity, distinct from ordinary WPT exit 1."""
    try:
        return main()
    except BaseException as failure:
        if not getattr(failure, '_oryn_run_integrity_failed', False):
            raise
        import traceback
        traceback.print_exception(failure)
        raise SystemExit(65) from failure


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
        parser.add_argument('--subdir', required=True)
        parser.add_argument('--require-test', nargs=2, action='append', required=True,
                            metavar=('TYPE', 'URL'))
        options = parser.parse_args(remaining)
        from manifest.manifest import load_and_update
        tests = Path(options.tests).resolve()
        # Upstream intentionally classifies files at the manifest root as
        # support. Select a real child directory under its unchanged parent;
        # keep that relative directory in both manifest IDs and served URLs.
        if (Path(options.subdir).is_absolute() or '\\' in options.subdir
                or any(part in {'', '.', '..'} for part in options.subdir.split('/'))):
            raise ValueError('local manifest requires a canonical child subdirectory')
        selected = (tests / options.subdir).resolve()
        if not selected.is_relative_to(tests) or not selected.is_dir():
            raise ValueError('local manifest subdirectory is missing or escapes its root')
        if (not options.url_base.startswith('/') or not options.url_base.endswith('/')
                or '//' in options.url_base or '?' in options.url_base or '#' in options.url_base
                or any(part in {'.', '..'} for part in options.url_base.split('/'))):
            raise ValueError('local manifest requires a canonical URL mount')
        required = {tuple(item) for item in options.require_test}
        if (len(required) != len(options.require_test)
                or any(kind not in {'testharness', 'reftest'}
                       or not url.startswith(options.url_base + options.subdir + '/')
                       for kind, url in required)):
            raise ValueError('required local manifest tests differ from the selected scope')
        admission.check()
        generated = load_and_update(str(tests), options.output, options.url_base,
                        update=True, rebuild=True, parallel=False,
                        paths_to_update=[options.subdir],
                        cache_root=str(Path(options.output).resolve().parent / (Path(options.output).name + ".cache")))
        admission.check()
        discovered = {(kind, test.id)
                      for kind, _, items in generated.itertypes('testharness', 'reftest')
                      for test in items}
        missing = required - discovered
        if missing:
            raise ValueError('required local manifest tests missing or wrong type: '
                             + repr(sorted(missing)))
        return 0
    from wptrunner import wptrunner
    sys.argv = [sys.argv[0], *remaining]
    return run_upstream(wptrunner.main, remaining, admission, args.generation_gate)


if __name__ == '__main__':
    sys.exit(process_exit(main))

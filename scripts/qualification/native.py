"""Native launcher35: explicit candidate, fresh output and validated schema6 inputs."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'artifacts/completion-validation-2026-09-18'
OUTPUT = BASE
COMMAND = ['cargo', 'test', '--locked', '--no-fail-fast', '-p', 'oryn-native', '-p', 'oryn-js',
           '-p', 'oryn-dom', '-p', 'oryn-webidl', '-p', 'oryn-layout', '-p', 'oryn-common',
           '-p', 'oryn-net', '-p', 'oryn-render-core', '--features',
           'oryn-native/v8-host,oryn-native/in-process-probe,oryn-native/v8-sandbox,oryn-native/core-full',
           '--', '--nocapture']
CHECK = ['cargo', 'run', '--locked', '--offline', '-p', 'oryn-webidl', '--', 'check',
         'crates/oryn-webidl/idl/browser.webidl', 'crates/oryn-js/src/generated/browser.rs']


def save(name, value):
    with (OUTPUT / name).open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def main():
    global OUTPUT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--gate', type=Path, required=True)
    parser.add_argument('--gate-sha256', required=True)
    parser.add_argument('--output-directory', type=Path, required=True)
    parser.add_argument('--runtime-profile', type=Path, required=True)
    parser.add_argument('--runtime-profile-sha256', required=True)
    parser.add_argument('--runtime-provenance', type=Path, required=True)
    parser.add_argument('--runtime-provenance-sha256', required=True)
    parser.add_argument('--whole-lease', type=Path, required=True)
    parser.add_argument('--whole-lease-sha256', required=True)
    args = parser.parse_args()
    os.chdir(ROOT)
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(ROOT / 'scripts'))
    from generation_gate import inherited_generation_lease
    result = dict(exit_code=1, phase='generation_admission')
    start = time.monotonic()
    lease, runtime, custody, provenance = None, None, None, None
    try:
        lease = inherited_generation_lease()
        gate = args.gate.resolve()
        if lease.path != gate or lease.sha256 != args.gate_sha256:
            raise ValueError('native launcher did not inherit the explicit candidate')
        helper = ROOT / 'scripts/qualification/custody.py'
        # The outer wrapper already admits the full helper graph; reject an
        # unbound helper before importing its runtime-profile validator.
        bindings = {row['sha256'] for item in json.loads(gate.read_bytes())['requirements'].values()
                    for role in ('implementation_sources', 'test_sources') for row in item[role]
                    if row['path'] == str(helper.relative_to(ROOT))}
        import hashlib
        if bindings != {hashlib.sha256(helper.read_bytes()).hexdigest()}:
            raise ValueError('native35 custody helper is not uniquely source-bound')
        spec = importlib.util.spec_from_file_location('repair49_native_custody', helper)
        custody = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(custody)
        candidate = custody.configure_candidate(args.gate,args.gate_sha256,args.whole_lease,args.whole_lease_sha256)
        if (os.environ.get(candidate['supervisor_key']) != 'outer-owned-session' or os.getpid() != os.getpgrp()):
            raise ValueError('native launcher must lead the outer-owned process group')
        OUTPUT = custody.fresh_output_directory(args.output_directory)
        custody.bound_source(Path(__file__).resolve())
        runtime = custody.load_runtime(args.runtime_profile, args.runtime_profile_sha256)
        provenance = custody.load_runtime_provenance(args.runtime_provenance,args.runtime_provenance_sha256,
            args.whole_lease,args.whole_lease_sha256,lease.sha256,runtime)
        base_env = custody.runtime_environment(os.environ, runtime)
        base_env.pop(candidate['supervisor_key'], None)
        # No environment overlay follows the actual frozen generation binding.
        env = lease.environment(base_env)
        if env['ORYN_GENERATION_GATE'] != str(gate) or env['ORYN_GENERATION_GATE_SHA256'] != lease.sha256:
            raise ValueError('native35 final environment changed generation')
        result.update(declaration_sha256=lease.sha256, runtime_profile=runtime['profile'], runtime_provenance=provenance)
        save('native-js-tests-35.preflight.json', dict(command=COMMAND, registry_command=CHECK,
             declaration_sha256=lease.sha256, runtime_profile=runtime['profile'],
             runtime_inputs=runtime['records'], runtime_environment=runtime['environment'], runtime_provenance=provenance))
        def intact():
            lease.check()
            errors = (custody.runtime_check(runtime)['errors']
                      + custody.check_runtime_provenance(provenance,runtime)['errors'])
            if errors:
                raise ValueError('native35 runtime input drift: ' + repr(errors))
        intact()
        result['phase'] = 'registry_check'
        with (OUTPUT / 'webidl-check-35.log').open('x') as log:
            code = subprocess.call(CHECK, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
        save('webidl-check-35.json', dict(exit_code=code, declaration_sha256=lease.sha256,
             command=CHECK, runtime_profile=runtime['profile']))
        result.update(exit_code=code, log=str((OUTPUT / 'webidl-check-35.log').relative_to(ROOT)))
        intact()
        if not code:
            result['phase'] = 'tests'
            with (OUTPUT / 'native-js-tests-35.log').open('x') as log:
                code = subprocess.call(COMMAND, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            result.update(exit_code=code, log=str((OUTPUT / 'native-js-tests-35.log').relative_to(ROOT)))
            intact()
    except BaseException as error:
        result['execution_error'] = repr(error)
        result['exit_code'] = result.get('exit_code') or 1
    finally:
        result['elapsed_seconds'] = time.monotonic() - start
        if lease is not None:
            try:
                lease.check()
            except Exception as error:
                result['generation_error'] = repr(error)
        if runtime is not None:
            result['runtime_after'] = custody.runtime_check(runtime)
        if provenance is not None:
            result['runtime_provenance_after'] = custody.check_runtime_provenance(provenance,runtime)
        if OUTPUT != BASE:
            save('native-js-tests-35.json', result)
    print(json.dumps(result), flush=True)
    return result['exit_code'] or int(bool(result.get('execution_error') or result.get('generation_error')
                        or result.get('runtime_after', {}).get('errors')
                        or result.get('runtime_provenance_after', {}).get('errors')))


if __name__ == '__main__':
    sys.exit(main())

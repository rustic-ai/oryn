#!/usr/bin/env python3
"""Build auxiliary security executables before the compiled candidate is frozen."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess

from completion_provenance import ROOT, digest
from generation_gate import inherited_generation_lease
from security_prebuilt import BUILD_COMMANDS, bundle_inventory, preparation_artifacts, source_hash, verify_signature


def record(path):
    path = path.resolve()
    return dict(path=str(path), sha256=digest(path))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    lease = inherited_generation_lease()
    profile_path = args.profile.resolve()
    profile_record = record(profile_path)
    spec = importlib.util.spec_from_file_location('profile_validation', ROOT / 'scripts/validate-v8-runtime-profile.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    profile = module.validate(json.loads(profile_path.read_text()), ROOT, 'core-full', 'aarch64-apple-darwin')
    identity = source_hash()
    worker = ROOT/'target/release/OrynPageWorker.app/Contents/MacOS/oryn-page-worker'
    worker_manifest = worker.parents[1]/'Resources/worker-manifest.json'
    verify_signature({'artifacts':{'worker':worker,'worker_manifest':worker_manifest}})
    bundle_files = bundle_inventory(worker)
    output = args.output.resolve()
    # A fresh output owns its Cargo directory; it cannot overwrite release inputs.
    output.mkdir(parents=True, exist_ok=False)
    target = output / 'cargo-target'
    environment = lease.environment(dict(os.environ, CARGO_TARGET_DIR=str(target),
        RUSTY_V8_ARCHIVE=profile['archive_path'], RUSTY_V8_SRC_BINDING_PATH=profile['binding_path'],
        ORYN_V8_BRIDGE_ABI=profile['bridge_abi_path'], ORYN_STARTUP_SNAPSHOT=profile['startup_snapshot_path'],
        ORYN_ICU_DATA_FINGERPRINT_PATH=profile['icu_data_path']))
    for name in ('V8_FROM_SOURCE', 'ORYN_ICU_DATA_PATH', 'ORYN_LITE_ICU_DATA',
                 'ORYN_SECURITY_PREBUILT_CANDIDATE', 'ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256'):
        environment.pop(name, None)
    result = dict(schema_version=1, source_tree_sha256=identity, engine_profile=profile_record,
        generation_gate=dict(path=str(lease.path),sha256=lease.sha256), builds={}, bundle_files=bundle_files,
        producer_sources={name:digest(ROOT/'scripts'/name) for name in
                          ('prepare-security-prebuilt.py','security_prebuilt.py')})
    for name, command in BUILD_COMMANDS.items():
        lease.check()
        log = output / (name + '.log')
        with log.open('xb') as stream:
            completed = subprocess.run(command, cwd=ROOT, env=environment, stdout=stream, stderr=subprocess.STDOUT)
        lease.check()
        if completed.returncode:
            raise ValueError('security preparation build failed: ' + name)
        executable = (target/'debug/oryn-page-worker' if name == 'debug_worker'
                      else target/'release/examples'/name)
        result['builds'][name] = dict(argv=command, returncode=completed.returncode,
                                     executable=record(executable), log=record(log))
        if (source_hash() != identity or record(profile_path) != profile_record
                or bundle_inventory(worker) != bundle_files):
            raise ValueError('source/profile/bundle drift during security preparation')
    lease.check()
    manifest = output/'security-prebuilt.json'
    manifest.write_text(json.dumps(result, indent=2)+'\n')
    artifacts = dict(preparation_artifacts(result), security_prebuilt=record(manifest))
    (output/'security-prebuilt-artifacts.json').write_text(json.dumps(artifacts, indent=2)+'\n')
    lease.check()
    print(output/'security-prebuilt-artifacts.json')


if __name__ == '__main__':
    main()

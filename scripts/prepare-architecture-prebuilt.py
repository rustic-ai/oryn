#!/usr/bin/env python3
"""Build architecture inputs in three isolated outputs before candidate freeze.

Both engine profiles must be explicitly selected. They may reference identical
compatible engine bytes, but the legacy build never inherits the current choice.
This publishes input provenance only; final qualification executes these inputs.
"""
import argparse
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile

from architecture_prebuilt import (ROOT, BUILD_COMMANDS, TREE_COMMAND, SOURCE_INPUTS,
    PRODUCER_SOURCES, LEGACY_COMMIT, HARNESS, engine_inputs, engine_environment,
    exact_record, record, source_hash, preparation_artifacts, manifest_inputs, archive_sources)
from generation_gate import inherited_generation_lease


def extract_legacy(archive, output):
    """The archived Git tree must contain only owned regular files/directories."""
    with tarfile.open(archive, 'r:') as source:
        seen = set()
        for member in source.getmembers():
            name = PurePosixPath(member.name)
            if (name.is_absolute() or '..' in name.parts or member.name in seen
                    or not (member.isfile() or member.isdir())):
                raise ValueError('legacy architecture archive contains an unsafe entry')
            seen.add(member.name)
        source.extractall(output, filter='data')


def prepare(profile, legacy_profile, output, lease):
    lease.check(); identity = source_hash(); manifest_inputs()
    engines = {'current': engine_inputs(profile), 'legacy': engine_inputs(legacy_profile)}
    sources = {name: record(ROOT / relative) for name, relative in SOURCE_INPUTS.items()}
    producers = {name: record(ROOT / 'scripts' / name) for name in PRODUCER_SOURCES}
    output = Path(output).resolve()
    protected = [Path(item['path']) for inputs in engines.values() for item in inputs.values()]
    if output == ROOT or ROOT in output.parents or any(output == p or output in p.parents for p in protected):
        raise ValueError('architecture preparation output must be fresh and outside source/engine inputs')
    lease.check(); output.mkdir(parents=True, exist_ok=False)
    archive = output / 'legacy-source.tar'
    with archive.open('xb') as stream:
        subprocess.run(['git', 'archive', LEGACY_COMMIT], cwd=ROOT, env=lease.environment(),
                       stdout=stream, check=True)
    archive_sources(archive)
    lease.check(); legacy = output / 'legacy-source'; legacy.mkdir()
    extract_legacy(archive, legacy)
    harness = legacy / HARNESS; harness.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(exact_record(sources['harness']), harness)
    if record(harness)['sha256'] != sources['harness']['sha256']:
        raise ValueError('injected legacy harness differs from current frozen fixture source')
    legacy_sources = {path.relative_to(legacy).as_posix(): record(path)
                      for path in sorted(legacy.rglob('*')) if path.is_file()}
    result = {'schema_version': 1, 'source_tree_sha256': identity,
              'generation_gate': {'path': str(lease.path), 'sha256': lease.sha256},
              'sources': sources, 'producer_sources': producers, 'engines': engines,
              'builds': {}, 'legacy_commit': LEGACY_COMMIT, 'legacy_archive': record(archive),
              'legacy_source_root': str(legacy), 'legacy_sources': legacy_sources}
    def recheck():
        lease.check()
        if source_hash() != identity: raise ValueError('source changed during architecture preparation')
        for item in [*sources.values(), *producers.values(), result['legacy_archive']]: exact_record(item)
        for item in legacy_sources.values(): exact_record(item)
        if {path.relative_to(legacy).as_posix() for path in legacy.rglob('*') if path.is_file()} != set(legacy_sources):
            raise ValueError('legacy source membership changed during architecture preparation')
        for role, inputs in engines.items():
            if engine_inputs(exact_record(inputs['profile'])) != inputs:
                raise ValueError('engine changed during architecture preparation: ' + role)
    for name, command in BUILD_COMMANDS.items():
        recheck()
        role = 'legacy' if name == 'legacy' else 'current'
        target = output / (name + '-cargo-target')
        environment = engine_environment(engines[role])
        environment['CARGO_TARGET_DIR'] = str(target)
        environment = lease.environment(environment)
        cwd = legacy if name == 'legacy' else ROOT
        log = output / (name + '-build.log')
        with log.open('xb') as stream:
            completed = subprocess.run(command, cwd=cwd, env=environment,
                                       stdout=stream, stderr=subprocess.STDOUT)
        recheck()
        if completed.returncode != 0: raise ValueError('architecture preparation build failed: ' + name)
        executable = target / ('debug/oryn-webidl' if name == 'webidl' else 'debug/examples/runtime_shadow')
        result['builds'][name] = {'argv': command, 'returncode': completed.returncode,
                                  'executable': record(executable), 'log': record(log)}
    graph = output / 'native-dependency-graph.txt'
    environment = engine_environment(engines['current'])
    environment['CARGO_TARGET_DIR'] = str(output / 'current-cargo-target')
    recheck()
    with graph.open('xb') as stream:
        completed = subprocess.run(TREE_COMMAND, cwd=ROOT, env=lease.environment(environment),
                                   stdout=stream, stderr=subprocess.PIPE)
    recheck()
    if completed.returncode != 0: raise ValueError('architecture dependency graph preparation failed')
    result['dependency_graph'] = {'argv': TREE_COMMAND, 'returncode': completed.returncode,
                                  'output': record(graph)}
    manifest = output / 'architecture-prebuilt.json'
    with manifest.open('x') as stream: json.dump(result, stream, indent=2, sort_keys=True)
    artifacts = dict(preparation_artifacts(result), architecture_prebuilt=record(manifest))
    with (output / 'architecture-prebuilt-artifacts.json').open('x') as stream:
        json.dump(artifacts, stream, indent=2, sort_keys=True)
    recheck()
    return output / 'architecture-prebuilt-artifacts.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('profile', 'legacy-profile', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    args = parser.parse_args()
    print(prepare(args.profile, args.legacy_profile, args.output, inherited_generation_lease()))


if __name__ == '__main__': main()

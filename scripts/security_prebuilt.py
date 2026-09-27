"""Exact frozen security inputs; validation never builds, signs or repairs them."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile

from completion_provenance import ROOT, digest, load_candidate, read_record
from generation_gate import inherited_generation_lease

CANDIDATE_ENV = 'ORYN_SECURITY_PREBUILT_CANDIDATE'
DIGEST_ENV = 'ORYN_SECURITY_PREBUILT_CANDIDATE_SHA256'
EXAMPLES = ('layout_probe', 'layout_benchmark', 'sandbox_worker_integration', 'security_policy_probe')
BUILD_COMMANDS = {
    name: ['cargo', 'build', '--locked', '--release', '-p', 'oryn-native', '--example', name]
    for name in EXAMPLES
}
BUILD_COMMANDS['debug_worker'] = ['cargo', 'build', '--locked', '-p', 'oryn-native',
                                 '--features', 'v8-host,v8-sandbox', '--bin', 'oryn-page-worker']
CANDIDATE_EXAMPLES = {'sandbox_page_benchmark': 'page_benchmark',
                      'page_group_benchmark': 'group_benchmark',
                      'semantic_reference_churn': 'churn'}


def preparation_artifacts(value):
    """Flatten every consumed preparation input into final candidate custody."""
    result = {}
    for name in BUILD_COMMANDS:
        result['security_prebuilt_' + name] = value['builds'][name]['executable']
        result['security_prebuilt_' + name + '_build_log'] = value['builds'][name]['log']
    for name, record in value['bundle_files'].items():
        result['security_bundle_' + hashlib.sha256(name.encode()).hexdigest()] = record
    return result


def bundle_inventory(worker):
    app = Path(worker).parents[2]
    result = {}
    for path in sorted(app.rglob('*')):
        if path.is_symlink():
            raise ValueError('frozen worker bundle contains a mutable symlink')
        if path.is_file():
            result[path.relative_to(app).as_posix()] = dict(path=str(path.resolve()), sha256=digest(path))
    if not result:
        raise ValueError('frozen worker bundle inventory is empty')
    return result


def validate_bundle(value, candidate):
    inventory = value.get('bundle_files')
    if not isinstance(inventory, dict) or not inventory:
        raise ValueError('frozen worker bundle inventory missing')
    if bundle_inventory(candidate['artifacts']['worker']) != inventory:
        raise ValueError('frozen worker bundle file set or bytes changed')
    for name, record in inventory.items():
        key = 'security_bundle_' + hashlib.sha256(name.encode()).hexdigest()
        if candidate['record']['artifacts'].get(key) != record:
            raise ValueError('worker bundle file missing from final candidate custody')


def source_hash():
    return subprocess.check_output([str(ROOT / 'scripts/source-tree-hash.sh')], cwd=ROOT, text=True).strip()


def selected_worker_bundle(worker, profile_path, source):
    """Admit a selected signed bundle after full core-full profile validation.

    The producer validates raw engine records before this bundle/source join.
    """
    worker = Path(worker).resolve()
    if worker.name != 'oryn-page-worker' or worker.parent.name != 'MacOS':
        raise ValueError('selected worker must be a bundled page worker')
    resources = worker.parents[1] / 'Resources'
    manifest_path = resources / 'worker-manifest.json'
    manifest = json.loads(manifest_path.read_text())
    profile_path = Path(profile_path).resolve()
    profile = json.loads(profile_path.read_text())
    if (manifest.get('source_tree_sha256') != source
            or manifest.get('v8_profile_sha256') != digest(profile_path)
            or digest(resources / 'v8-profile-manifest.json') != digest(profile_path)
            or manifest.get('worker_build_sha256') != profile.get('worker_sha256')
            or manifest.get('worker_build_bytes') != profile.get('worker_size_bytes')):
        raise ValueError('selected worker source/profile identity mismatch')
    verify_signature({'artifacts': {'worker': worker, 'worker_manifest': manifest_path}})
    return worker, manifest_path


def validate_preparation(value, candidate, lease):
    records = candidate['record']['artifacts']
    if (type(value.get('schema_version')) is not int or value['schema_version'] != 1
            or value.get('source_tree_sha256') != candidate['record']['source_tree_sha256']
            or value.get('engine_profile') != records['engine_profile']
            or value.get('generation_gate') != dict(path=str(lease.path), sha256=lease.sha256)):
        raise ValueError('security preparation source/profile/generation provenance mismatch')
    if value.get('producer_sources') != {
            name: digest(ROOT / 'scripts' / name)
            for name in ('prepare-security-prebuilt.py', 'security_prebuilt.py')}:
        raise ValueError('security preparation producer changed')
    builds = value.get('builds')
    if not isinstance(builds, dict) or set(builds) != set(BUILD_COMMANDS):
        raise ValueError('security preparation executable inventory incomplete')
    validate_bundle(value, candidate)
    if any(records.get(name) != record for name, record in preparation_artifacts(value).items()):
        raise ValueError('security preparation inputs missing from final candidate custody')
    result = {}
    for name, command in BUILD_COMMANDS.items():
        record = builds[name]
        if record.get('argv') != command or type(record.get('returncode')) is not int or record['returncode'] != 0:
            raise ValueError('security preparation build command failed or changed: ' + name)
        result[name] = read_record(record['executable'])
        read_record(record['log'])
        if not os.access(result[name], os.X_OK):
            raise ValueError('security prepared input is not executable: ' + name)
    return result


def verify_consumed_security(candidate):
    """Final receipt consumption has no temporary build environment to inherit."""
    preparation = candidate['artifacts'].get('security_prebuilt')
    if preparation is None:
        raise ValueError('security receipt lacks frozen preparation custody')
    value = json.loads(preparation.read_text())
    validate_bundle(value, candidate)
    for name, record in preparation_artifacts(value).items():
        if candidate['record']['artifacts'].get(name) != record:
            raise ValueError('security preparation input differs from final candidate')
        read_record(record)
    verify_signature(candidate)
    validate_bundle(value, candidate)


def verify_signature(candidate, run=None):
    """Re-read the live signature; a retained receipt cannot replace codesign."""
    run = subprocess.run if run is None else run
    worker = candidate['artifacts']['worker']
    app = worker.parents[2]
    manifest = json.loads(candidate['artifacts']['worker_manifest'].read_text())
    identity = manifest['signing_cert_sha1']
    selected = os.environ.get('ORYN_CODESIGN_IDENTITY', identity)
    if selected != identity:
        raise ValueError('configured signing identity differs from frozen worker')
    run(['codesign', '--verify', '--strict', '--verbose=2', str(app)], check=True, capture_output=True)
    details = run(['codesign', '-dvvv', str(worker)], check=True, capture_output=True, text=True).stderr
    required = ('Identifier=ai.rustic.oryn.page-worker', 'TeamIdentifier=5HVA9VFF8K',
                'Authority=Apple Development:',
                'Authority=Apple Worldwide Developer Relations Certification Authority',
                'flags=0x10000(runtime)')
    if any(value not in details for value in required):
        raise ValueError('frozen worker signature identity/chain/runtime mismatch')
    dumped = run(['codesign', '-d', '--entitlements', ':-', str(worker)], check=True, capture_output=True).stdout
    entitlements = plistlib.loads(dumped)
    if (set(entitlements) != {'com.apple.security.app-sandbox', 'com.apple.security.cs.allow-jit'}
            or any(value is not True for value in entitlements.values())):
        raise ValueError('frozen worker entitlements mismatch')
    with tempfile.TemporaryDirectory(prefix='oryn-frozen-worker-cert-') as directory:
        prefix = Path(directory) / 'cert'
        run(['codesign', '-d', '--extract-certificates=' + str(prefix), str(worker)],
            check=True, capture_output=True)
        leaf = hashlib.sha1(prefix.with_name('cert0').read_bytes()).hexdigest().upper()
    if leaf != identity:
        raise ValueError('frozen worker signing certificate mismatch')


def bind():
    lease = inherited_generation_lease()
    path, expected = os.environ.get(CANDIDATE_ENV), os.environ.get(DIGEST_ENV)
    if not path or not expected:
        raise ValueError('prebuilt security requires exact frozen candidate and digest')
    candidate = load_candidate(path)
    if candidate['sha256'] != expected:
        raise ValueError('prebuilt security candidate digest mismatch')
    if source_hash() != candidate['record']['source_tree_sha256']:
        raise ValueError('prebuilt security source differs from frozen candidate')
    resources = candidate['artifacts']['worker'].parents[1] / 'Resources'
    if (candidate['artifacts']['worker_manifest'] != (resources / 'worker-manifest.json').resolve()
            or digest(resources / 'v8-profile-manifest.json') != candidate['record']['artifacts']['engine_profile']['sha256']):
        raise ValueError('prebuilt worker bundle differs from frozen manifest/profile')
    preparation = candidate['artifacts'].get('security_prebuilt')
    if preparation is None:
        raise ValueError('candidate lacks pre-freeze security preparation')
    extra = validate_preparation(json.loads(preparation.read_text()), candidate, lease)
    target = os.environ.get('CARGO_TARGET_DIR')
    if not target:
        raise ValueError('prebuilt security requires an isolated check build directory')
    target = Path(target).resolve()
    protected = list(candidate['artifacts'].values()) + list(extra.values())
    if (target == ROOT or ROOT in target.parents
            or any(target == item or target in item.parents for item in protected)):
        raise ValueError('security check build directory overlaps frozen inputs')
    lease.check()
    return candidate, extra


def selected(name, candidate, extra):
    if name in CANDIDATE_EXAMPLES:
        return candidate['artifacts'][CANDIDATE_EXAMPLES[name]]
    if name in extra:
        return extra[name]
    if name in ('runtime', 'worker', 'engine_worker', 'engine_profile', 'worker_manifest'):
        return candidate['artifacts'][name]
    raise ValueError('unknown frozen security executable: ' + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', choices=('check', 'path', 'run'))
    parser.add_argument('name', nargs='?')
    parser.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    candidate, extra = bind()
    if args.operation == 'check':
        verify_signature(candidate)
        bind()
        print(json.dumps({'candidate_sha256': candidate['sha256'], 'validated_prebuilt': True}))
        return 0
    executable = selected(args.name, candidate, extra)
    if args.operation == 'path':
        print(executable)
        return 0
    arguments = args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
    result = subprocess.run([str(executable), *arguments], env=dict(os.environ,
                            ORYN_PAGE_WORKER=str(candidate['artifacts']['worker'])))
    bind()
    return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())

"""Committed-source export and the complete 51-requirement generation barrier.

Preparing declarations is not independent review or permission to launch a build.
No historical candidate paths become dependencies of a new declaration.
"""
from __future__ import annotations

import base64
import json
from pathlib import Path
import shutil

from . import builds
from .inputs import resolve_input, snapshot_input


REQUIREMENTS = {f'{package}-{number:02}' for package, count in
                [('G', 8), ('R', 7), ('P', 6), ('L', 5), ('W', 5), ('S', 5),
                 ('V', 5), ('F', 4), ('Q', 6)] for number in range(1, count + 1)}
PLAN = 'docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md'


def _clean(root, expected_commit=None):
    paths = [x.decode('utf-8') for x in builds._git(root, 'ls-files', '-z').split(b'\0') if x]
    return builds.audit_source(root, effective_inputs=paths, expected_commit=expected_commit)


def _write_new(path, value):
    with Path(path).open('x') as output:
        json.dump(value, output, sort_keys=True, indent=2)
        output.write('\n')


def new_store_path(store, relative):
    builds._relative(relative)
    path = Path(store).resolve(strict=True)
    for part in relative.split('/'):
        path = path / part
        if path.is_symlink():
            raise ValueError('new output contains symlink traversal')
    return path


def _tree_objects(root, commit):
    root_oid = builds._git(root, 'rev-parse', commit + '^{tree}').decode().strip()
    objects = {root_oid}
    for row in builds._git(root, 'ls-tree', '-r', '-d', '-z', commit).split(b'\0'):
        if row:
            metadata, _ = row.split(b'\t', 1)
            objects.add(metadata.decode().split()[2])
    # One read-only child, not one Git process per directory.
    import os
    import subprocess
    ordered = sorted(objects)
    data = subprocess.check_output(['git', '-C', str(root), 'cat-file', '--batch'],
                                   input=('\n'.join(ordered) + '\n').encode(),
                                   env={**os.environ, 'GIT_OPTIONAL_LOCKS': '0'})
    result, position = {}, 0
    for expected in ordered:
        end = data.index(b'\n', position)
        oid, kind, count = data[position:end].decode().split()
        size = int(count)
        raw = data[end + 1:end + 1 + size]
        if oid != expected or kind != 'tree' or data[end + 1 + size:end + 2 + size] != b'\n':
            raise ValueError('Git object export framing differs')
        result[oid] = base64.b64encode(raw).decode()
        position = end + 2 + size
    if position != len(data):
        raise ValueError('unexpected Git object export bytes')
    return result


def export_source(source_root, store_root, *, export_path, identity_path, expected_commit=None):
    """Export exactly one committed tree and raw Git proof, without copying .git."""
    source = Path(source_root).resolve(strict=True)
    store = Path(store_root).resolve(strict=True)
    if store == source or source in store.parents:
        raise ValueError('source exports must live outside the development checkout')
    destination = new_store_path(store, export_path)
    identity_file = new_store_path(store, identity_path)
    if destination.exists() or identity_file.exists() or destination in identity_file.parents:
        raise ValueError('source export and identity need new separate destinations')
    admission = _clean(source, expected_commit)
    commit = admission['commit']
    identity = {'schema_version': 1, 'kind': 'git-source-export', 'commit': commit,
                'commit_object': base64.b64encode(builds._git(source, 'cat-file', 'commit', commit)).decode(),
                'trees': _tree_objects(source, commit)}
    destination.mkdir(parents=True)
    identity_file.parent.mkdir(parents=True, exist_ok=True)
    for name, expected in admission['tracked'].items():
        original = builds._file(source, name)
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, target)
        target.chmod(0o555 if expected['executable'] else 0o444)
        if builds._identity(target) != expected:
            raise ValueError('source changed while exporting: ' + name)
    membership = snapshot_input(store, export_path, scope='store', kind='directory')
    from .production import validate_source_export
    verified = validate_source_export(identity, destination, membership)
    if verified['tree_sha256'] != admission['tree_sha256']:
        raise ValueError('export differs from admitted committed source')
    _write_new(identity_file, identity)
    identity_file.chmod(0o444)
    for directory in sorted(destination.rglob('*'), reverse=True):
        if directory.is_dir():
            directory.chmod(0o555)
    destination.chmod(0o555)
    return {'schema_version': 1, 'source': {key: verified[key] for key in ('commit', 'tree_sha256', 'dirty')},
            'source_export': membership, 'source_identity': snapshot_input(store, identity_path),
            'independent_review_required': True}


def source_info(source_root, source_identity_path):
    """Re-admit current build checkout against the exported raw commit identity."""
    identity = json.loads(Path(source_identity_path).read_text())
    if identity.get('kind') != 'git-source-export' or identity.get('schema_version') != 1:
        raise ValueError('committed Git source export identity required')
    root = Path(source_root).resolve(strict=True)
    admission = _clean(root, identity['commit'])
    if base64.b64encode(builds._git(root, 'cat-file', 'commit', admission['commit'])).decode() != identity['commit_object']:
        raise ValueError('source commit object differs from export')
    if _tree_objects(root, admission['commit']) != identity['trees']:
        raise ValueError('source tree objects differ from export')
    return {key: admission[key] for key in ('commit', 'tree_sha256', 'dirty')}


def prepare_generation_gate(source_root, predecessor_path, output_path, *, expected_commit=None):
    """Refresh only source hashes; preserve requirement mapping and generated flags."""
    root = Path(source_root).resolve(strict=True)
    output_path = Path(output_path).absolute()
    if output_path == root or root in output_path.parents:
        raise ValueError('generation declarations must live outside committed source')
    predecessor = Path(predecessor_path)
    previous_bytes = predecessor.read_bytes()
    previous = json.loads(previous_bytes)
    plan_hash = builds._identity(builds._file(root, PLAN))['sha256']
    if type(previous.get('schema_version')) is not int or previous['schema_version'] not in (1, 2) or previous.get('plan_sha256') != plan_hash:
        raise ValueError('predecessor must bind the unchanged full completion plan')
    requirements = previous.get('requirements')
    if not isinstance(requirements, dict) or set(requirements) != REQUIREMENTS:
        raise ValueError('predecessor must account for all 51 requirements')
    admission = _clean(root, expected_commit)
    refreshed = {}
    for name, item in requirements.items():
        if item.get('implementation_generated') is not True or item.get('tests_generated') is not True:
            raise ValueError('cannot promote incomplete generation: ' + name)
        row = {'implementation_generated': True, 'tests_generated': True}
        for field in ('implementation_sources', 'test_sources'):
            if not isinstance(item.get(field), list) or not item[field]:
                raise ValueError('requirement source mapping missing')
            records = []
            for record in item[field]:
                path = record['path']
                if path not in admission['tracked']:
                    raise ValueError('generation source is not committed: ' + path)
                records.append({'path': path, 'sha256': admission['tracked'][path]['sha256']})
            row[field] = records
        refreshed[name] = row
    gate = {'schema_version': 2, 'kind': 'committed-source-generation',
            'plan_sha256': plan_hash,
            'source': {key: admission[key] for key in ('commit', 'tree_sha256', 'dirty')},
            'requirements': refreshed,
            'history': {'predecessor_sha256': builds._identity(predecessor)['sha256']}}
    import hashlib
    if hashlib.sha256(previous_bytes).hexdigest() != gate['history']['predecessor_sha256']:
        raise ValueError('predecessor generation declaration changed during preparation')
    validate_committed_gate(gate, root)
    _write_new(output_path, gate)
    return {'schema_version': 1, 'gate_sha256': builds._identity(Path(output_path))['sha256'],
            'source': gate['source'], 'requirements': len(refreshed), 'independent_review_required': True}


def validate_committed_gate(gate, source_root):
    """Additional clean-source binding; the shared gate still checks all sources."""
    required = {'schema_version', 'kind', 'plan_sha256', 'source', 'requirements', 'history'}
    if (set(gate) != required or type(gate['schema_version']) is not int or gate['schema_version'] != 2
            or gate['kind'] != 'committed-source-generation'):
        raise ValueError('typed committed generation schema differs')
    source = gate['source']
    if set(source) != {'commit', 'tree_sha256', 'dirty'} or source['dirty'] is not False:
        raise ValueError('generation requires clean committed source')
    if set(gate['history']) != {'predecessor_sha256'}:
        raise ValueError('generation history must be an opaque digest')
    import re
    if not re.fullmatch('[0-9a-f]{64}', gate['history']['predecessor_sha256']):
        raise ValueError('invalid historical generation digest')
    admission = _clean(Path(source_root).resolve(strict=True), source['commit'])
    if admission['tree_sha256'] != source['tree_sha256']:
        raise ValueError('committed generation source tree changed')
    return gate

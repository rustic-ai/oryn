"""Exact opt-in fixture URL repairs; upstream assertion/source bytes stay intact."""
import hashlib
import json
from pathlib import Path
import re
from . import WPT_REVISION

MANIFEST = Path(__file__).with_name('fixture-repairs.json')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate(path, expected_hash, checkout):
    path, root = Path(path).resolve(), Path(checkout).resolve()
    if digest(path) != expected_hash or path.read_bytes() != MANIFEST.read_bytes():
        raise ValueError('fixture repair manifest differs from frozen declared source')
    manifest = json.loads(path.read_text())
    if manifest.get('schema_version') != 1 or manifest.get('upstream_revision') != WPT_REVISION:
        raise ValueError('fixture repair identity mismatch')
    routes = []
    for repair in manifest['repairs']:
        request = repair['request_path']
        if repair['method'] != 'GET' or not re.fullmatch(r'/[A-Za-z0-9_./-]+', request) or '..' in request.split('/'):
            raise ValueError('fixture repair must name an exact local GET path')
        if (root / request.lstrip('/')).exists():
            raise ValueError('fixture repair cannot override an upstream file')
        for prefix in ('source', 'target'):
            source = (root / repair[prefix + '_path']).resolve()
            if root not in source.parents or not source.is_file() or digest(source) != repair[prefix + '_sha256']:
                raise ValueError('fixture repair source/target bytes changed')
        route = ('GET', request, '/' + repair['target_path'])
        if any(existing[1] == request for existing in routes):
            raise ValueError('duplicate fixture repair path')
        routes.append(route)
    return routes

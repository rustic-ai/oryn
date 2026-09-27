"""Versioned fresh input locks without editing the historical v1 behavior contract."""
import copy
import hashlib
import json
from pathlib import Path
from workflow_contract import DEFAULT_CONTRACT, FROZEN_CONTRACT_SHA256, validate_contract


def digest(path): return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def baseline(root):
    path = Path(root) / 'test-harness/workflows/five-family-contract-v1.json'
    if digest(path) != FROZEN_CONTRACT_SHA256:
        raise ValueError('historical workflow behavior contract changed')
    return validate_contract(json.loads(path.read_text()))


def input_paths(root):
    root = Path(root).resolve(); original = baseline(root)
    paths = {entry['path'] for entry in original['regression_inputs']}
    paths.update('benchmarks/app-compatibility/' + name for name in
                 ('fixtures.lock.json', 'fixtures-v1.lock.json', 'fixtures-v2.lock.json'))
    extensions = {'.py', '.json', '.mjs', '.js', '.html', '.css', '.txt', '.toml', '.md'}
    for folder in ('test-harness/workflows', 'tools/wptrunner-oryn'):
        for path in (root / folder).rglob('*'):
            if path.is_file() and path.suffix in extensions and '__pycache__' not in path.parts:
                paths.add(path.relative_to(root).as_posix())
    for path in (root / 'scripts').iterdir():
        if path.is_file() and (path.name.startswith(('workflow', 'wpt', 'completion', 'run-oil-', 'qualify-workflow', 'run-workflow', 'run-wpt-', 'create-wpt-', 'freeze-completion-', 'generation_gate'))
                               or path.suffix == '.sh'
                               or path.name in {'suite_input_lease.py', 'framework_fixture_lock.py', 'build_app_compatibility_framework_corpus.py', 'native_media_qualification.py', 'run-native-media-qualification.py', 'run-full-wpt-baseline.py', 'run-upstream-wpt.py', 'run-official-wpt.py', 'reconcile-full-wpt-qualification.py', 'merge-wpt-qualification-ledger.py'}):
            paths.add(path.relative_to(root).as_posix())
    return sorted(paths)


def snapshot(root, source_tree_sha256):
    root = Path(root).resolve()
    records = []
    for name in input_paths(root):
        path = (root / name).resolve()
        if root not in path.parents or not path.is_file():
            raise ValueError('workflow input escapes source root or is missing')
        records.append({'path': name, 'sha256': digest(path)})
    return {'schema_version': 2, 'base_contract_sha256': FROZEN_CONTRACT_SHA256,
            'source_tree_sha256': source_tree_sha256, 'inputs': records}


def load(path, source_tree_sha256, root):
    lock = json.loads(Path(path).read_text())
    if (lock.get('schema_version') != 2 or lock.get('base_contract_sha256') != FROZEN_CONTRACT_SHA256
            or lock.get('source_tree_sha256') != source_tree_sha256):
        raise ValueError('workflow input lock source or behavior identity mismatch')
    current = snapshot(root, source_tree_sha256)
    if lock != current:
        raise ValueError('workflow input lock is stale, incomplete or substituted')
    contract = copy.deepcopy(baseline(root))
    # Only the separate input lock changes. Behavior cells, sampling, recovery
    # classification, dimensions and all original source bytes remain immutable.
    contract['regression_inputs'] = lock['inputs']
    return validate_contract(contract)


def for_candidate(candidate):
    from workflow_contract import ROOT, load_contract
    path = candidate['artifacts']['workflow_contract']
    if digest(path) == FROZEN_CONTRACT_SHA256:
        return load_contract(path, ROOT)
    return load(path, candidate['record']['source_tree_sha256'], ROOT)

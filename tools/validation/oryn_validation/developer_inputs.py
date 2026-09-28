"""Small startup checks for local development, separate from qualification.

An existing run manifest supplies paths and native build identity, not approval
of the current checkout. No directory hashes, review chains or build probes are
performed unless audit_inputs is explicitly requested by the parent.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess

from .admission import SOURCE_ROOT, legacy_algorithms, read_object
from .inputs import snapshot_input, verify_inputs, resolve_input
from .wpt import _resolve_inputs, _digest, _check_config
from .worker_environment import selected_worker


def source_info():
    def git(*args):
        return subprocess.check_output(['git', '-C', str(SOURCE_ROOT), *args], text=True).strip()
    return {'path': str(SOURCE_ROOT), 'commit': git('rev-parse', 'HEAD'),
            'working_tree_changes': git('status', '--porcelain'),
            'description': 'actual developer checkout; no source export or qualification'}


def python_environment(bound):
    from .environments import _runtime_env
    root = bound['assets']['python_environment']
    if bound['assets']['python'] != root / 'bin/python':
        raise ValueError('Python executable does not belong to selected environment')
    return _runtime_env(root)


def load_developer_inputs(run, *, store_root, bundle_root):
    legacy_algorithms()
    path = Path(run).absolute()
    raw = path.read_bytes()
    template = json.loads(raw)
    if template.get('kind') == 'developer-wpt' and template.get('qualification') == 'not-evaluated':
        bound = template['inputs']
        if (Path(bound['store_root']) != Path(store_root).absolute()
                or Path(bound['bundle_root']) != Path(bundle_root).absolute()):
            raise ValueError('developer configuration roots differ; use the original run template to relocate')
        bound.update(assets={k: Path(v) for k, v in bound['assets'].items()},
                     store_root=Path(store_root).absolute(), bundle_root=Path(bundle_root).absolute(),
                     path=path, sha256=hashlib.sha256(raw).hexdigest())
    else:
        if template.get('kind') != 'run' or template.get('schema_version') != 1:
            raise ValueError('expected a run configuration template')
        # A legacy template can exceed 100 MB because it embeds every dataset file
        # twice. Only configuration roles are needed for local execution. Do not
        # validate, copy, or pass those directory inventories to child processes.
        records = {}
        paths = {}
        store_root, bundle_root = Path(store_root).absolute(), Path(bundle_root).absolute()
        roles = {'runtime', 'driver', 'worker', 'worker_manifest', 'python', 'python_environment',
                 'wpt_checkout', 'core_manifest', 'core_metadata', 'core_runner_config',
                 'core_profile_registry', 'core_include', 'include', 'fixture_relay',
                 'fixture_repairs', 'fixture_tls_policy', 'fixture_ca',
                 'fixture_server_certificate', 'fixture_server_key'}
        def asset_json(role):
            item = template['assets'][role]
            return read_object(resolve_input(bundle_root if item['scope'] == 'bundle' else store_root, item['path']))
        if 'core_profile_registry' in template['assets']:
            roles.add(asset_json('core_profile_registry')['profiles'][template['profile']]['fixture'])
        if 'fixture_tls_policy' in template['assets']:
            policy = asset_json('fixture_tls_policy')
            roles.update(policy['modules'].values())
            roles.update(policy[k] for k in ('response_module', 'handlers_module', 'pipes_module', 'certificate'))
        for role in sorted(roles & template['assets'].keys()):
            record = template['assets'][role]
            if record['scope'] not in ('store', 'bundle') or record['kind'] not in ('file', 'directory'):
                raise ValueError('invalid developer asset: ' + role)
            records[role] = {k: v for k, v in record.items() if k not in ('members', 'directories')}
            paths[role] = resolve_input(bundle_root if record['scope'] == 'bundle' else store_root, record['path'])
        worker = records['worker']
        if worker not in template['immutable_inputs']:
            raise ValueError('worker is absent from the selected build inputs')
        run = {key: template[key] for key in ('build', 'run_id', 'profile', 'runner',
               'original_run_id', 'attempt', 'suite', 'workers')}
        run.update(assets=records, immutable_inputs=[worker])
        bound = dict(run=run, assets=paths, path=path, sha256=hashlib.sha256(raw).hexdigest(),
                     store_root=store_root, bundle_root=bundle_root, audit_template=str(path))
    for role in ('runtime', 'driver', 'python', 'fixture_relay'):
        path = bound['assets'][role]
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError('missing or non-executable developer input: ' + role)
    selected_worker(bound)
    python_environment(bound)
    if not bound['assets']['wpt_checkout'].is_dir():
        raise ValueError('WPT checkout is missing')
    # These are the *actual* current Python sources. Record only the two small
    # files needed by TLS bootstrap, rather than manufacturing a source export.
    bound['assets'].pop('runner_source', None)
    for role, relative in {'fixture_tls_module': 'tools/validation/oryn_validation/fixture_tls.py',
                           'fixture_tls_bootstrap': 'tools/validation/bootstrap/sitecustomize.py'}.items():
        bound['assets'][role] = SOURCE_ROOT / relative
        # Receipt comparison needs identity, not a relocatable store claim.
        bound['run']['assets'][role] = dict(snapshot_input(SOURCE_ROOT, relative),
                                            path=str(SOURCE_ROOT / relative))
    return bound


def select_fixture(bound):
    assets, run = bound['assets'], bound['run']
    if run['suite']['name'] != 'wpt-core':
        raise ValueError('developer WPT requires a Core fixture context')
    registry = read_object(assets['core_profile_registry'])
    fixture_path = assets[registry['profiles'][run['profile']]['fixture']]
    fixture = read_object(fixture_path)
    _check_config(assets['core_runner_config'], assets, 'core')
    assets['runner_config'] = assets['core_runner_config']
    run['assets']['runner_config'] = run['assets']['core_runner_config']
    return {'fixture': fixture, 'fixture_path': fixture_path, 'collection': 'core'}


def audit_inputs(bound):
    """Explicit expensive audit, once in the parent; never implicit on lookup."""
    from .production import validate_production
    original = _resolve_inputs(bound.get('audit_template', bound['path']), store_root=bound['store_root'], bundle_root=bound['bundle_root'])
    verify_inputs(original['records'], store_root=original['store_root'], bundle_root=original['bundle_root'])
    validate_production(original)


def serialize_inputs(bound):
    return {**bound, 'assets': {k: str(v) for k, v in bound['assets'].items()},
            **{k: str(bound[k]) for k in ('path', 'store_root', 'bundle_root')}}


def configured_inputs(config_path, expected_digest):
    legacy_algorithms()
    if _digest(config_path) != expected_digest:
        raise ValueError('developer configuration changed')
    config = read_object(config_path)
    if config.get('kind') != 'developer-wpt' or config.get('qualification') != 'not-evaluated':
        raise ValueError('explicit developer WPT configuration required')
    bound = config['inputs']
    bound.update(assets={k: Path(v) for k, v in bound['assets'].items()},
                 store_root=Path(bound['store_root']), bundle_root=Path(bound['bundle_root']),
                 path=Path(config_path).absolute(), sha256=expected_digest)
    return bound, config


def bootstrap_inputs():
    """Use the small parent configuration; never reload the legacy inventory."""
    return configured_inputs(os.environ['ORYN_WPT_DEVELOPER_CONFIG'],
                             os.environ['ORYN_WPT_DEVELOPER_CONFIG_SHA256'])[0]


def bundle_owner(bound):
    """Protect the selected build from cleanup, without auditing it again."""
    from .retention import retention_lock
    store, bundle = bound['store_root'], bound['bundle_root']
    build_root = store / 'builds' / bound['run']['build']['build_id']
    if not bundle.is_relative_to(build_root) or bundle.name != 'runtime':
        raise ValueError('selected runtime does not belong to the named build')
    with retention_lock(store):
        objects = read_object(store / 'retention.json')['objects']
        owners = [name for name, item in objects.items() if item['kind'] == 'build'
                  and 'removed_ns' not in item and build_root == store / item['path']]
        if len(owners) != 1:
            raise ValueError('selected build needs one registered cleanup owner')
        return owners[0]

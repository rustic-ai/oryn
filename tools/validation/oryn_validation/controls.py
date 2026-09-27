"""Recompute explicitly bound control originals; never accept a saved pass flag."""
from pathlib import Path
from collections import Counter

from .admission import legacy_algorithms, object_digest, read_object
from .manifests import load_manifest
from .inputs import resolve_input, verify_inputs
from .results import read_mozlog, reconcile_wpt


def verify_control(bound, collection):
    """Validate current control run custody, outcomes and independent review.

    Only the named assets below are read. Their typed records belong to the
    caller's immutable closure; a control run's history/outputs are not followed.
    """
    legacy_algorithms()
    from .wpt import _digest
    from wpt_execution_evidence import protocol_custody
    from wpt_canary_qualification import MOUNT, EXPECTED
    from wpt_shared_placement import TEST
    required = {'run', 'report', 'process', 'preflight', 'execution', 'custody', 'raw', 'review', 'output'}
    required |= {'audit'} if collection == 'canaries' else {'context', 'diagnostics', 'live_six'}
    prefix = 'controls_' + collection + '_'
    missing = [prefix + name for name in required if prefix + name not in bound['assets']]
    if missing:
        raise ValueError('control evidence missing typed assets: ' + ', '.join(sorted(missing)))
    output_record = bound['run']['assets'].get(prefix + 'output')
    if not output_record or output_record.get('kind') != 'directory':
        raise ValueError('control requires the entire completed output as an immutable directory')
    verify_inputs([output_record], store_root=bound['store_root'], bundle_root=bound['bundle_root'])
    output_path = Path(bound['assets'][prefix + 'output'])
    paths = {name: Path(bound['assets'][prefix + name]) for name in required - {'output'}}
    run = load_manifest(paths['run'], expected_kind='run')
    current = bound['run']
    if (run['build'] != current['build'] or run['runner'] != current['runner']
            or run['profile'] != current['profile'] or run['suite']['name'] != 'wpt-' + collection
            or run['suite']['sha256'] != current['suite']['sha256']
            or run['suite']['upstream_commit'] != current['suite']['upstream_commit']
            or run['attempt'] != 1 or run['run_id'] != run['original_run_id'] or run['workers'] != 1):
        raise ValueError('control original differs from current build/profile/runner policy')
    # The control must bind the same production and fixture inputs. Do not load
    # its nested assets; compare its typed declarations to the current ones.
    for name in ('runtime', 'driver', 'worker', 'worker_manifest', 'fixture_repairs',
                 'core_profile_registry', 'core_manifest', 'policy', 'coverage'):
        if run.get('assets', {}).get(name) != current['assets'].get(name) or name not in current['assets']:
            raise ValueError('control substitutes a current input: ' + name)
    collection_inputs = {'include', 'runner_config', 'canary_manifest', 'canary_source', 'canary_metadata',
                         'placement_manifest', 'placement_source', 'placement_metadata'}
    for name in (set(run['assets']) & set(current['assets'])) - collection_inputs:
        if run['assets'][name] != current['assets'][name]:
            raise ValueError('control substitutes shared current input: ' + name)
    execution, process, report = (read_object(paths[name]) for name in ('execution', 'process', 'report'))
    build_hash, run_hash = current['build']['manifest_sha256'], _digest(paths['run'])
    if (execution.get('kind') != 'typed-wpt-execution' or execution.get('schema_version') != 1
            or execution.get('run_id') != run['run_id'] or execution.get('run_manifest_sha256') != run_hash
            or execution.get('candidate_sha256') != build_hash or execution.get('profile') != current['profile']
            or execution.get('collection') != collection or execution.get('errors') != []
            or execution.get('custody_complete') is not True):
        raise ValueError('control execution identity or integrity differs')
    root = paths['execution'].parent
    if root != output_path:
        raise ValueError('control completed output directory differs')
    if root != Path(bound['store_root']).absolute() / run['outputs']['results']:
        raise ValueError('control output differs from its original run destination')
    if run['outputs']['events'] != run['outputs']['results'] + '/mozlog.jsonl':
        raise ValueError('control raw event destination differs')
    if any(item not in current['immutable_inputs'] for item in run['immutable_inputs']):
        raise ValueError('control producer input absent from explicit current closure')
    control_assets = {name: resolve_input(bound['bundle_root'] if item['scope'] == 'bundle' else bound['store_root'], item['path'])
                      for name, item in run['assets'].items()}
    registry = read_object(control_assets['core_profile_registry'])
    fixture_path = control_assets[registry['profiles'][run['profile']]['fixture']]
    control_bound = dict(bound, run=run, assets=control_assets, path=paths['run'], sha256=run_hash)
    from .wpt import plan_command, execution_policy, verify_fixture_services
    if execution.get('execution_policy') != execution_policy(control_bound, collection):
        raise ValueError('control process deadline differs from immutable execution policy')
    preflight = read_object(paths['preflight'])
    argv = preflight.get('argv', [])
    if (len(argv) < 12 or argv[1:6] != ['-B', '-m', 'oryn_validation.wpt', 'upstream', '--run']
            or argv[7] != '--store-root' or argv[9] != '--bundle-root' or argv[11] != '--'):
        raise ValueError('control original launch roots/command missing')
    original_store, original_bundle = Path(argv[8]), Path(argv[10])
    if not original_store.is_absolute() or not original_bundle.is_absolute():
        raise ValueError('control launch roots must be absolute')
    origin_assets = {name: (original_bundle if item['scope'] == 'bundle' else original_store) / item['path']
                     for name, item in run['assets'].items()}
    fixture_name = registry['profiles'][run['profile']]['fixture']
    origin = dict(control_bound, assets=origin_assets,
                  path=original_store / paths['run'].relative_to(bound['store_root']),
                  store_root=original_store, bundle_root=original_bundle)
    expected_argv = plan_command(origin, {'collection': collection, 'fixture_path': origin_assets[fixture_name],
                                         'fixture': read_object(fixture_path)}, original_store / run['outputs']['results'])
    birth = preflight.get('child_identity')
    if (process.get('argv') != expected_argv or preflight.get('argv') != expected_argv
            or preflight.get('timeout_seconds') != execution['execution_policy']['timeout_seconds']
            or preflight.get('cwd') != str(origin_assets['wpt_checkout'])
            or not isinstance(birth, list) or len(birth) != 2 or not isinstance(birth[0], list)
            or len(birth[0]) not in (1, 2) or birth[1] != preflight.get('owner_pid')
            or type(preflight.get('child_pid')) is not int or preflight['child_pid'] <= 0):
        raise ValueError('control process command/deadline/birth differs from upstream original policy')
    fixture = read_object(fixture_path)
    verify_fixture_services(control_bound, {'fixture': fixture}, root,
                            original_roots=(original_store, original_bundle))
    from .fixture_tls import verify_receipts
    ports = [port for name, values in fixture['ports'].items() if name.startswith('https') for port in values]
    verify_receipts(root / 'tls-activation', control_assets['fixture_tls_policy'],
                    _digest(control_assets['fixture_tls_policy']), ports, require_live=False, bound=control_bound)
    for name in ('report', 'process', 'preflight', 'custody', 'raw'):
        if not paths[name].is_relative_to(root):
            raise ValueError('control artifact belongs to another execution')
        relative = str(paths[name].relative_to(root))
        if execution.get('artifacts', {}).get(relative) != _digest(paths[name]):
            raise ValueError('control original artifact changed: ' + name)
    if (type(process.get('returncode')) is not int or process.get('forced_cleanup') is not False
            or process.get('cleanup_complete') is not True or process.get('cleanup_errors') != []
            or process.get('timed_out') is not False or process.get('error') is not None
            or process.get('cancellation_signal') is not None
            or process.get('returncode') != (1 if collection == 'canaries' else 0)):
        raise ValueError('control process custody or expected exit failed')
    for field in ('returncode', 'timed_out', 'cleanup_complete'):
        if execution.get(field) != process[field]:
            raise ValueError('control execution summary differs from owned process')
    if execution.get('launch_error') is not None:
        raise ValueError('control execution has a launch error')
    info = report.get('run_info', {})
    for key, expected in {'oryn_candidate_sha256': build_hash, 'oryn_run_contract_sha256': run_hash,
                          'oryn_run_id': run['run_id'], 'oryn_profile_id': current['profile'],
                          'oryn_runner_commit': current['runner']['commit'], 'product': 'oryn',
                          'oryn_executor': 'upstream-wptrunner',
                          'oryn_wpt_revision': current['suite']['upstream_commit']}.items():
        if info.get(key) != expected:
            raise ValueError('control raw report identity differs: ' + key)
    urls = sorted(MOUNT + name for name in EXPECTED) if collection == 'canaries' else [TEST]
    if run['expected'] != {'automated': len(urls), 'assessments': 0}:
        raise ValueError('control run population differs')
    raw_events = list(read_mozlog(paths['raw']))
    observed = reconcile_wpt(urls, raw_events, execution=process, assessment_total=0)
    if not observed['compatibility_complete']:
        raise ValueError('control original raw starts/ends are incomplete or duplicated')
    raw_ends = {event['test']: event['status'] for event in raw_events if event.get('action') == 'test_end'}
    raw_subtests = {url: Counter((event.get('subtest'), event.get('status')) for event in raw_events
                                 if event.get('action') == 'test_status' and event.get('test') == url)
                    for url in urls}
    reported = report.get('results', [])
    if (len(reported) != len(urls) or {item.get('test') for item in reported} != set(urls)
            or any(item.get('status') != raw_ends[item['test']]
                   or Counter((subtest.get('name'), subtest.get('status')) for subtest in item.get('subtests', []))
                   != raw_subtests[item['test']] for item in reported)):
        raise ValueError('control report differs from raw original outcomes')
    protocol_custody(paths['custody'], build_hash)
    if collection == 'canaries':
        from oryn_wptrunner.canary_resource_audit import response_status
        if not any(response_status(event) == 404 for event in raw_events):
            raise ValueError('missing helper has no real wptserve 404 in raw original')
        worker = read_object(bound['assets']['worker_manifest'])
        expected_render = {key: worker[key] for key in
                           ('render_contract_version', 'local_effects_contract_version')}
        if info.get('oryn_rendering_contracts') != expected_render:
            raise ValueError('canary rendering differs from signed worker contract')
        from wpt_canary_qualification import qualify
        result = qualify(paths['report'], paths['execution'], paths['audit'], build_hash)
        if not result['controls_rejected']:
            raise ValueError('five canary outcomes or required failure observations differ')
    else:
        from wpt_shared_placement import qualify
        for name in ('context', 'diagnostics', 'live_six'):
            if not paths[name].is_relative_to(root) or execution['artifacts'].get(str(paths[name].relative_to(root))) != _digest(paths[name]):
                raise ValueError('placement evidence is not bound to original execution')
        context = read_object(paths['context'])
        if context.get('candidate_sha256') != build_hash or context.get('run_contract_sha256') != run_hash:
            raise ValueError('placement context differs from admitted control original')
        result = qualify(read_object(paths['diagnostics']), report, context, _digest(bound['assets']['worker']))
        if not result['qualification_accepted']:
            raise ValueError('native shared placement did not qualify')
        from .live_six import verify as verify_live_six
        verify_live_six(read_object(paths['live_six']), read_object(paths['diagnostics']), context, preflight,
                        worker_path=str(origin_assets['worker']), worker_sha256=_digest(bound['assets']['worker']),
                        runtime_path=str(origin_assets['runtime']), runtime_sha256=_digest(bound['assets']['runtime']))
    review = read_object(paths['review'])
    if set(review) != {'schema_version', 'kind', 'author', 'reviewer', 'run_id', 'inputs', 'result_sha256', 'decision'}:
        raise ValueError('control requires an exact independent review record')
    if (review['schema_version'] != 1 or review['kind'] != 'typed-control-review'
            or review['decision'] != 'accepted' or review['run_id'] != run['run_id']
            or any(not isinstance(review[k], str) or not review[k].strip() for k in ('author', 'reviewer'))
            or review['author'].strip().casefold() == review['reviewer'].strip().casefold()
            or review['inputs'] != {name: _digest(path) for name, path in paths.items() if name != 'review'}
            or review['result_sha256'] != object_digest(result)):
        raise ValueError('independent control review does not bind exact recomputed original')
    return result


def verify_controls(bound):
    return {kind: verify_control(bound, kind) for kind in ('canaries', 'placement')}

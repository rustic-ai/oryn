import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from oryn_validation.admission import legacy_algorithms, object_digest
from oryn_validation.controls import verify_control
from oryn_validation.inputs import snapshot_input
from oryn_validation.worker_environment import worker_binding
from oryn_validation.wpt import _digest, plan_command, EXECUTION_POLICY, relay_commands

legacy_algorithms()
from wpt_canary_qualification import EXPECTED, MOUNT, REVISION, qualify


class TypedControlEvidenceTests(unittest.TestCase):
    def setUp(self):
        # Transport receipt semantics are tested separately with real loader and
        # stable-identity negatives; these fixtures isolate control qualification.
        transport = patch('oryn_validation.fixture_tls.verify_receipts', return_value=[])
        transport.start()
        self.addCleanup(transport.stop)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.output = self.root / 'runs/canaries'
        (self.output / 'process').mkdir(parents=True)
        self.assets = {}
        for name in ('runtime', 'driver', 'worker', 'fixture_repairs', 'core_manifest', 'core_metadata', 'policy',
                     'coverage', 'python', 'include', 'runner_config', 'fixture_relay', 'wpt_checkout', 'fixture_tls_policy'):
            path = self.root / name
            path.write_text(name)
            self.assets[name] = path
        self.write_asset('execution_policy', EXECUTION_POLICY)
        self.write_asset('fixture', {'ports': {'https': [8443]}, 'tls': {key: {'path': key + '.pem'} for key in ('ca', 'certificate', 'key')},
            'endpoint_routes': [dict(protocol=name, port=port, connect_ip=ip) for name, port, ip in
                [('http-local', 8002, '192.168.1.2'), ('https-local', 8445, '192.168.1.2'),
                 ('http-public', 8003, '2001:4860::1'), ('https-public', 8446, '2001:4860::1')]]})
        self.write_asset('core_profile_registry', {'profiles': {'standard-block': {'fixture': 'fixture'}}})
        self.write_asset('worker_manifest', {'render_contract_version': 1, 'local_effects_contract_version': 1})
        records = {name: snapshot_input(self.root, str(path.relative_to(self.root))) for name, path in self.assets.items()}
        self.assets['worker'].chmod(0o755)
        records['worker'] = dict(snapshot_input(self.root, 'worker'), scope='bundle')
        self.run = {'schema_version': 1, 'kind': 'run', 'run_id': 'canaries',
                    'build': {'build_id': 'build', 'manifest_sha256': 'a' * 64},
                    'runner': {'commit': 'b' * 40, 'tree_sha256': 'c' * 64},
                    'suite': {'name': 'wpt-canaries', 'sha256': 'd' * 64, 'upstream_commit': REVISION},
                    'profile': 'standard-block', 'workers': 1, 'expected': {'automated': 5, 'assessments': 0},
                    'immutable_inputs': list(records.values()), 'assets': records, 'environment': [],
                    'outputs': {'results': 'runs/canaries', 'events': 'runs/canaries/mozlog.jsonl', 'status': 'status/canaries'},
                    'history': [], 'attempt': 1, 'original_run_id': 'canaries'}
        self.run_path = self.root / 'control-run.json'
        self.run_path.write_text(json.dumps(self.run))
        self.bound = {'run': copy.deepcopy(self.run), 'store_root': self.root, 'bundle_root': self.root,
                      'assets': self.assets.copy()}
        self.bound['run']['run_id'] = self.bound['run']['original_run_id'] = 'core'
        self.bound['run']['suite']['name'] = 'wpt-core'
        self.report = {'run_info': {'product': 'oryn', 'oryn_candidate_sha256': 'a' * 64,
                        'oryn_run_contract_sha256': _digest(self.run_path), 'oryn_run_id': 'canaries',
                        'oryn_profile_id': 'standard-block', 'oryn_runner_commit': 'b' * 40,
                        'oryn_executor': 'upstream-wptrunner', 'oryn_wpt_revision': REVISION,
                        'oryn_rendering_contracts': {'render_contract_version': 1, 'local_effects_contract_version': 1}},
                       'results': [{'test': MOUNT + name, 'status': status,
                                    'subtests': [{'name': subtest, 'status': substatus}] if subtest else []}
                                   for name, (status, subtest, substatus) in EXPECTED.items()]}

    def test_worker_substitution_rejected_even_with_fresh_independent_review_hashes(self):
        bound = self.evidence(substitute_worker=True)
        with self.assertRaisesRegex(ValueError, 'recorded worker environment'):
            verify_control(bound, 'canaries')

    def write_asset(self, name, value):
        path = self.root / (name + '.json')
        path.write_text(json.dumps(value))
        self.assets[name] = path

    def evidence(self, *, raw_404=True, cleanup=True, reviewer='reviewer', substitute_worker=False):
        paths = {'run': self.run_path, 'report': self.output / 'wptreport.json',
                 'raw': self.output / 'mozlog.jsonl', 'custody': self.output / 'protocol-custody.jsonl',
                 'process': self.output / 'process/process-result.json', 'execution': self.output / 'execution.json',
                 'audit': self.output / 'audit.json', 'review': self.root / 'review.json',
                 'preflight': self.output / 'process/preflight.json'}
        paths['report'].write_text(json.dumps(self.report))
        events = [{'action': 'suite_start'}]
        for row in self.report['results']:
            events += [{'action': 'test_start', 'test': row['test']}]
            events += [{'action': 'test_status', 'test': row['test'], 'subtest': item['name'], 'status': item['status']}
                       for item in row['subtests']]
            events += [{'action': 'test_end', 'test': row['test'], 'status': row['status']}]
        if raw_404:
            events += [{'action': 'log', 'component': 'wptserve', 'level': 'INFO',
                        'message': '404 GET /__oryn_qualification_missing_helper__.js () 0'}]
        events += [{'action': 'suite_end'}]
        paths['raw'].write_text(''.join(json.dumps(event) + '\n' for event in events))
        paths['custody'].write_text(''.join(json.dumps(dict(schema_version=1, session='f' * 32,
            candidate_sha256='a' * 64, success=True, error_type=None, phase=phase)) + '\n' for phase in ('connect', 'teardown')))
        command = plan_command(dict(self.bound, run=self.run, path=self.run_path),
                               {'fixture_path': self.assets['fixture'], 'fixture': json.loads(self.assets['fixture'].read_text()), 'collection': 'canaries'}, self.output)
        process = dict(argv=command, returncode=1, timed_out=False, cleanup_complete=cleanup,
                       cleanup_errors=[], forced_cleanup=False, error=None, cancellation_signal=None)
        paths['process'].write_text(json.dumps(process))
        paths['preflight'].write_text(json.dumps(dict(argv=command, cwd=str(self.assets['wpt_checkout']),
            timeout_seconds=43200, child_identity=[[1, 2], 3], owner_pid=3, child_pid=4)))
        for name, argv, _ in relay_commands(self.assets['fixture_relay'], json.loads(self.assets['fixture'].read_text())):
            folder = self.output / 'services' / name
            folder.mkdir(parents=True, exist_ok=True)
            raw = dict(argv=argv, cleanup_complete=True, cleanup_errors=[], timed_out=False, forced_cleanup=False,
                       error='owned execution cancelled by signal 15', cancellation_signal=15)
            (folder / 'process-result.json').write_text(json.dumps(raw))
            (folder / 'service-result.json').write_text(json.dumps(dict(raw, service_stop_requested=True)))
            (folder / 'preflight.json').write_text(json.dumps(dict(argv=argv, cwd=str(self.assets['wpt_checkout']),
                child_identity=[[1, 2], 3], owner_pid=3, child_pid=4)))
        execution = dict(schema_version=1, kind='typed-wpt-execution', run_id='canaries',
                         run_manifest_sha256=_digest(self.run_path), candidate_sha256='a' * 64,
                         profile='standard-block', collection='canaries', returncode=1, timed_out=False,
                         execution_policy=dict(EXECUTION_POLICY, timeout_seconds=43200),
                         worker_binding=worker_binding(self.run, self.root),
                         cleanup_complete=cleanup, launch_error=None, custody_complete=True, errors=[],
                         artifacts={str(paths[name].relative_to(self.output)): _digest(paths[name])
                                    for name in ('report', 'raw', 'custody', 'process', 'preflight')})
        if substitute_worker:
            execution['worker_binding']['path'] = '/substituted/worker'
        paths['execution'].write_text(json.dumps(execution))
        paths['audit'].write_text(json.dumps(dict(candidate_sha256='a' * 64, report_sha256=_digest(paths['report']),
            cleanup_complete=True, all_workers_reaped=True, native_reference_rendering=True,
            resources={MOUNT+'missing-helper.html': [dict(path='/__oryn_qualification_missing_helper__.js', status=404)]})))
        result = qualify(paths['report'], paths['execution'], paths['audit'], 'a' * 64)
        paths['review'].write_text(json.dumps(dict(schema_version=1, kind='typed-control-review', author='author',
            reviewer=reviewer, run_id='canaries', inputs={name:_digest(path) for name,path in paths.items() if name != 'review'},
            result_sha256=object_digest(result), decision='accepted')))
        self.bound['assets'].update({'controls_canaries_'+name:path for name,path in paths.items()})
        self.bound['assets']['controls_canaries_output'] = self.output
        self.bound['run']['assets']['controls_canaries_output'] = snapshot_input(self.root, 'runs/canaries', kind='directory')
        return self.bound

    def test_real_expected_controls_with_exact_independent_review(self):
        self.assertTrue(verify_control(self.evidence(), 'canaries')['controls_rejected'])

    def test_pass_flags_cannot_replace_real_resource_log_or_cleanup(self):
        with self.assertRaisesRegex(ValueError, 'real wptserve 404'):
            verify_control(self.evidence(raw_404=False), 'canaries')
        with self.assertRaisesRegex(ValueError, 'process custody'):
            verify_control(self.evidence(cleanup=False), 'canaries')

    def test_review_cannot_bless_wrong_outcome_or_self_review(self):
        self.report['results'][0]['status'] = 'PASS'
        with self.assertRaisesRegex(ValueError, 'canary outcomes'):
            verify_control(self.evidence(), 'canaries')
        self.report['results'][0]['status'] = 'OK'
        with self.assertRaisesRegex(ValueError, 'independent control review'):
            verify_control(self.evidence(reviewer='author'), 'canaries')

class TypedPlacementEvidenceTests(unittest.TestCase):
    setUp = TypedControlEvidenceTests.setUp
    write_asset = TypedControlEvidenceTests.write_asset
    evidence = TypedControlEvidenceTests.evidence

    def placement_evidence(self, mutation=None):
        from wpt_shared_placement import TEST, NAMES, qualify as placement_qualify
        self.evidence()
        self.run['suite']['name'] = 'wpt-placement'
        self.run['expected'] = {'automated': 1, 'assessments': 0}
        self.run_path.write_text(json.dumps(self.run))
        run_hash = _digest(self.run_path)
        context = {'nonce': 'a' * 64, 'candidate_sha256': 'a' * 64,
                   'run_contract_sha256': run_hash, 'test': TEST}
        binary = _digest(self.assets['worker'])
        def placement(pid):
            return {'mode': 'child_process', 'attested': True, 'process_id': pid,
                    'binary_sha256': binary, 'bootstrap_sha256': 'e' * 64}
        window = {'kind': 'window', 'id': 1, 'frame': None}
        snapshot = {'schema': 1, 'nonce': context['nonce'], 'capture_id': 'f' * 64, 'parent_pid': 10,
                    'pages': [{'id': 1, 'url': 'https://web-platform.test:8443' + TEST, 'placement': placement(11)}],
                    'agents': [{'id': agent, 'kind': 'Dedicated', 'live': True, 'owners': [window],
                                'script_url': 'https://web-platform.test:8443/__oryn_shared_memory__/shared-memory/worker.js?placement=1',
                                'placement': placement(11 + agent)} for agent in range(2, 8)],
                    'shared_backings': [{'backing_sha256': str(index) * 64, 'wasm': wasm, 'memory64': False,
                                         'growable': growable, 'holders': [window, {'kind': 'agent', 'id': 2}, {'kind': 'agent', 'id': 3}]}
                                        for index, wasm, growable in ((1, False, False), (2, False, True), (3, True, True))]}
        record = {'schema': 1, **context, 'snapshot': snapshot}
        report = {'run_info': dict(self.report['run_info'], oryn_run_contract_sha256=run_hash),
                  'results': [{'test': TEST, 'status': 'OK',
                               'subtests': [{'name': name, 'status': 'PASS'} for name in sorted(NAMES)]}]}
        expected_result = placement_qualify(record, report, context, binary)
        events = [{'action': 'suite_start'}, {'action': 'test_start', 'test': TEST}]
        events += [{'action': 'test_status', 'test': TEST, 'subtest': name, 'status': 'PASS'} for name in sorted(NAMES)]
        events += [{'action': 'test_end', 'test': TEST, 'status': 'OK'}, {'action': 'suite_end'}]
        if mutation == 'same-process':
            record['snapshot']['agents'][0]['placement']['process_id'] = 11
        elif mutation == 'raw-mismatch':
            events[2]['status'] = 'FAIL'
        elif mutation == 'context-substitution':
            context['nonce'] = 'b' * 64
        paths = {'run': self.run_path, 'report': self.output / 'wptreport.json',
                 'raw': self.output / 'mozlog.jsonl', 'custody': self.output / 'protocol-custody.jsonl',
                 'process': self.output / 'process/process-result.json', 'preflight': self.output / 'process/preflight.json',
                 'execution': self.output / 'execution.json', 'context': self.output / 'placement-context.json',
                 'diagnostics': self.output / 'placement.json', 'live_six': self.output / 'live-six.json', 'review': self.root / 'placement-review.json'}
        paths['report'].write_text(json.dumps(report))
        paths['raw'].write_text(''.join(json.dumps(event) + '\n' for event in events))
        paths['context'].write_text(json.dumps(context)); paths['diagnostics'].write_text(json.dumps(record))
        command = plan_command(dict(self.bound, run=self.run, path=self.run_path),
            {'fixture_path': self.assets['fixture'], 'fixture': json.loads(self.assets['fixture'].read_text()), 'collection': 'placement'}, self.output)
        process = json.loads(paths['process'].read_text()); process.update(argv=command, returncode=0)
        if mutation == 'cleanup':
            process['cleanup_complete'] = False
        paths['process'].write_text(json.dumps(process))
        preflight = json.loads(paths['preflight'].read_text()); preflight['argv'] = command
        paths['preflight'].write_text(json.dumps(preflight))
        from test_live_six import proof
        live = proof(snapshot, context, preflight, str(self.assets['worker']), str(self.assets['runtime']),
                     binary, _digest(self.assets['runtime']))
        paths['live_six'].write_text(json.dumps(live))
        execution = json.loads(paths['execution'].read_text())
        execution.update(collection='placement', returncode=0, run_manifest_sha256=run_hash,
            cleanup_complete=process['cleanup_complete'],
            artifacts={str(paths[name].relative_to(self.output)): _digest(paths[name])
                       for name in ('report', 'raw', 'custody', 'process', 'preflight', 'context', 'diagnostics', 'live_six')})
        paths['execution'].write_text(json.dumps(execution))
        (self.output / 'audit.json').unlink()
        paths['review'].write_text(json.dumps(dict(schema_version=1, kind='typed-control-review', author='author',
            reviewer='author' if mutation == 'self-review' else 'reviewer', run_id=self.run['run_id'],
            inputs={name: _digest(path) for name, path in paths.items() if name != 'review'},
            result_sha256=object_digest(expected_result), decision='accepted')))
        self.bound['assets'].update({'controls_placement_' + name: path for name, path in paths.items()})
        self.bound['assets']['controls_placement_output'] = self.output
        self.bound['run']['assets']['controls_placement_output'] = snapshot_input(self.root, 'runs/canaries', kind='directory')
        return self.bound

    def test_integrated_placement_joins_real_qualifier_and_original_custody(self):
        result = verify_control(self.placement_evidence(), 'placement')
        self.assertTrue(result['qualification_accepted'])
        self.assertEqual(set(result['witnesses']), {'fixed', 'growable', 'wasm'})

    def test_consistent_hashes_and_review_cannot_bless_invalid_placement(self):
        cases = {'same-process': 'physical process separation', 'raw-mismatch': 'raw original outcomes',
                 'context-substitution': 'run binding changed', 'cleanup': 'process custody',
                 'self-review': 'independent control review'}
        for mutation, reason in cases.items():
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, reason):
                verify_control(self.placement_evidence(mutation), 'placement')

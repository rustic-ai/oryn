"""Synthetic kernel observations only; no browser or native process launched."""
import copy
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from oryn_validation import live_six as subject


def proof(snapshot, context, preflight, worker_path, runtime_path, worker_hash, runtime_hash):
    workers = {str(row['id']): row['placement']['process_id'] for row in snapshot['agents']}
    parent, root = snapshot['parent_pid'], preflight['child_pid']
    nodes = {str(pid): {'identity': [[pid + 1000, 3], parent], 'path': worker_path} for pid in workers.values()}
    nodes[str(parent)] = {'identity': [[parent + 1000, 3], root], 'path': runtime_path}
    nodes[str(root)] = {'identity': preflight['child_identity'], 'path': '/pinned/python'}
    return {'schema_version': 1, 'kind': 'live-six-native-dedicated-workers',
            **{key: context[key] for key in subject.BINDINGS}, 'capture_id': snapshot['capture_id'],
            'workers': workers, 'worker_sha256': worker_hash, 'runtime_sha256': runtime_hash, 'diagnostic_interval': [2, 3],
            'scans': [{'started_ns': 1, 'ended_ns': 2, 'nodes': copy.deepcopy(nodes)},
                      {'started_ns': 3, 'ended_ns': 4, 'nodes': copy.deepcopy(nodes)}]}


class LiveSixEvidence(unittest.TestCase):
    def fixture(self):
        subject.legacy_algorithms()
        from wpt_shared_placement import TEST
        def placement(pid):
            return {'mode': 'child_process', 'attested': True, 'binary_sha256': 'a' * 64,
                    'process_id': pid, 'bootstrap_sha256': 'c' * 64}
        context = {'test': TEST, 'nonce': 'b' * 64, 'candidate_sha256': 'd' * 64, 'run_contract_sha256': 'e' * 64}
        snapshot = {'schema': 1, 'nonce': context['nonce'], 'capture_id': 'f' * 64, 'parent_pid': 10,
            'pages': [{'id': 1, 'url': 'https://web-platform.test:8443' + TEST, 'placement': placement(11)}],
            'agents': [{'id': i, 'kind': 'Dedicated', 'live': True,
                'script_url': 'https://web-platform.test:8443/__oryn_shared_memory__/shared-memory/worker.js?placement=1',
                'owners': [{'kind': 'window', 'id': 1, 'frame': None}], 'placement': placement(20 + i)} for i in range(2, 8)]}
        preflight = {'child_pid': 5, 'child_identity': [[123, 4], 1]}
        record = proof(snapshot, context, preflight, '/bundle/worker', '/bundle/runtime', 'a' * 64, '9' * 64)
        return record, {'snapshot': snapshot}, context, preflight

    def verify(self, args):
        return subject.verify(*args, worker_path='/bundle/worker', worker_sha256='a' * 64,
                              runtime_path='/bundle/runtime', runtime_sha256='9' * 64)

    def test_exact_six_with_overlapping_kernel_lifetimes(self):
        self.assertEqual(self.verify(self.fixture())['native_dedicated_workers'], 6)

    def test_pid_reuse_or_nonoverlap_cannot_pass(self):
        for mutation in ('birth', 'time', 'parent', 'path'):
            args = self.fixture(); record = args[0]
            if mutation == 'birth': record['scans'][1]['nodes']['22']['identity'][0][0] += 1
            if mutation == 'time': record['scans'][1]['started_ns'] = 1
            if mutation == 'parent':
                for scan in record['scans']: scan['nodes']['22']['identity'][1] = 5
            if mutation == 'path':
                for scan in record['scans']: scan['nodes']['22']['path'] = '/other/worker'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): self.verify(args)

    def test_five_workers_shared_pid_or_foreign_capture_rejected(self):
        for mutation in ('five', 'shared', 'capture', 'hash', 'root'):
            args = self.fixture(); record, diagnostics, context, preflight = args
            if mutation == 'five': diagnostics['snapshot']['agents'].pop()
            if mutation == 'shared': diagnostics['snapshot']['agents'][1]['placement']['process_id'] = 22
            if mutation == 'capture': record['capture_id'] = '0' * 64
            if mutation == 'hash': record['worker_sha256'] = '0' * 64
            if mutation == 'root': preflight['child_identity'][0][0] += 1
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): self.verify(args)

    def test_kernel_scan_rechecks_identity_around_path_observation(self):
        reader = lambda pid: ((pid, 1), 10 if pid == 22 else 5 if pid == 10 else 1)
        path = lambda pid: '/bundle/' + ('worker' if pid == 22 else 'runtime' if pid == 10 else 'python')
        result = subject._scan([22], 5, identity_reader=reader, path_reader=path, clock=iter([1, 2]).__next__)
        self.assertEqual(set(result['nodes']), {'22', '10', '5'})
        changed = iter([((22, 1), 10), ((23, 1), 10)])
        with self.assertRaisesRegex(ValueError, 'changed during kernel observation'):
            subject._scan([22], 5, identity_reader=lambda pid: next(changed), path_reader=path)


    def test_birth_fields_reject_invalid_platform_values(self):
        for birth in ([True, 0], [-1, 0], [1, 1000000], ['nonsense'], ['0'], [1, '2']):
            with self.subTest(birth=birth), self.assertRaisesRegex(ValueError, 'birth fields'):
                subject._identity([birth, 10])

    def test_capture_joins_real_preflight_and_binary_bytes_to_kernel_scans(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); (root / 'process').mkdir()
            _, diagnostics, context, preflight = self.fixture()
            worker = root / 'worker'; worker.write_bytes(b'worker bytes')
            runtime = root / 'runtime'; runtime.write_bytes(b'runtime bytes')
            worker_hash, runtime_hash = subject._hash(worker), subject._hash(runtime)
            snapshot = diagnostics['snapshot']
            for row in snapshot['pages'] + snapshot['agents']:
                row['placement']['binary_sha256'] = worker_hash
            (root / 'process/preflight.json').write_text(json.dumps(preflight))
            expected = proof(snapshot, context, preflight, str(worker), str(runtime), worker_hash, runtime_hash)
            nodes = expected['scans'][0]['nodes']
            reader = lambda pid: nodes[str(pid)]['identity']
            paths = lambda pid: nodes[str(pid)]['path']
            with patch.object(subject, 'supervisor') as supervisor, patch.object(subject, '_path', side_effect=paths):
                supervisor.return_value.process_identity.side_effect = reader
                captured, confirmed = subject.capture(snapshot, context, {'worker': {'path': str(worker), 'sha256': worker_hash},
                    'runtime': {'path': str(runtime), 'sha256': runtime_hash}}, root,
                    diagnostic_reader=lambda: copy.deepcopy(snapshot))
                self.assertEqual(confirmed, snapshot)
            result = subject.verify(captured, diagnostics, context, preflight, worker_path=str(worker),
                worker_sha256=worker_hash, runtime_path=str(runtime), runtime_sha256=runtime_hash)
            self.assertTrue(result['accepted'])

    def test_fresh_diagnostic_is_bracketed_and_population_change_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); (root / 'process').mkdir()
            _, diagnostics, context, preflight = self.fixture()
            (root / 'process/preflight.json').write_text(json.dumps(preflight))
            snapshot = diagnostics['snapshot']
            artifacts = {'worker': {'path': '/worker', 'sha256': 'a' * 64},
                         'runtime': {'path': '/runtime', 'sha256': '9' * 64}}
            order = []
            def scan(*args):
                order.append('kernel'); return {}
            def changed():
                order.append('diagnostic')
                value = copy.deepcopy(snapshot); value['agents'][0]['placement']['process_id'] = 100
                return value
            with patch.object(subject, '_hash', side_effect=lambda path: 'a' * 64 if path == '/worker' else '9' * 64), \
                 patch.object(subject, '_scan', side_effect=scan):
                with self.assertRaisesRegex(ValueError, 'confirming diagnostic changed'):
                    subject.capture(snapshot, context, artifacts, root, diagnostic_reader=changed)
                self.assertEqual(order, ['kernel', 'diagnostic'])
                order.clear()
                with patch.object(subject, '_validate_scans'):
                    subject.capture(snapshot, context, artifacts, root,
                        diagnostic_reader=lambda: (order.append('diagnostic'), copy.deepcopy(snapshot))[1])
                self.assertEqual(order, ['kernel', 'diagnostic', 'kernel'])

    def test_typed_hook_preserves_live_capture_failure_and_original_diagnostics(self):
        from oryn_wptrunner import placement
        _, diagnostics, context, _ = self.fixture()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'placement.json'
            config = dict(context, schema=1, output=str(path), _typed_artifacts={})
            executor = SimpleNamespace(protocol=SimpleNamespace(webdriver=SimpleNamespace(
                send_session_command=lambda *args: diagnostics['snapshot'])))
            with patch.object(subject, 'capture', side_effect=ValueError('kernel observation unavailable')):
                placement.capture(executor, SimpleNamespace(url=context['test']), config)
            self.assertEqual(json.loads(path.read_text())['snapshot'], diagnostics['snapshot'])
            self.assertNotIn('error', json.loads(path.read_text()))
            self.assertIn('kernel observation unavailable', json.loads(path.with_name('live-six.json').read_text())['error'])


if __name__ == '__main__':
    unittest.main()

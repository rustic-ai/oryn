import copy
import unittest
from wpt_shared_placement import qualify, TEST, NAMES


class PlacementTests(unittest.TestCase):
    def evidence(self):
        binary = 'b' * 64
        context = {'nonce': 'a' * 64, 'candidate_sha256': 'c' * 64,
                   'run_contract_sha256': 'd' * 64, 'test': TEST}
        def placement(pid, agent=False):
            return {'mode': 'child_process', 'attested': True, 'process_id': pid,
                    'binary_sha256': binary, 'bootstrap_sha256': 'e' * 64 if agent else None}
        window = {'kind': 'window', 'id': 1, 'frame': None, 'document_generation': 2, 'stream_epoch': 0}
        snapshot = {'schema': 1, 'nonce': context['nonce'], 'capture_id': 'f' * 64, 'parent_pid': 10,
                    'pages': [{'id': 1, 'url': 'https://web-platform.test:8443' + TEST, 'placement': placement(11)}],
                    'agents': [{'id': id, 'kind': 'Dedicated', 'live': True, 'owners': [window],
                                'script_url': 'https://web-platform.test:8443/__oryn_shared_memory__/worker.js?placement=1',
                                'placement': placement(11 + id, True)} for id in (2, 3)],
                    'shared_backings': [{'backing_sha256': str(index) * 64, 'wasm': wasm,
                                         'memory64': False, 'growable': growable, 'maximum_length': 262144,
                                         'holders': [window, {'kind': 'agent', 'id': 2}, {'kind': 'agent', 'id': 3}]}
                                        for index, wasm, growable in ((1, False, False), (2, False, True), (3, True, True))]}
        record = {'schema': 1, **context, 'snapshot': snapshot}
        report = {'run_info': {'oryn_candidate_sha256': context['candidate_sha256'],
                              'oryn_run_contract_sha256': context['run_contract_sha256'],
                              'oryn_wpt_revision': '719d5e38fdd0903a18ed9007aba816c98cc491e0',
                              'oryn_executor': 'upstream-wptrunner'},
                  'results': [{'test': TEST, 'status': 'OK',
                               'subtests': [{'name': name, 'status': 'PASS'} for name in sorted(NAMES)]}]}
        return record, report, context, binary

    def test_distinct_attested_processes_and_actual_test_results_are_both_required(self):
        evidence = self.evidence()
        self.assertEqual(set(qualify(*evidence)['witnesses']), {'fixed', 'growable', 'wasm'})
        for mutation in ('same_pid', 'thread', 'unattested', 'binary', 'bootstrap', 'missing', 'test_fail', 'wrong_nonce', 'wrong_report'):
            record, report, context, binary = copy.deepcopy(evidence)
            agent = record['snapshot']['agents'][0]
            if mutation == 'same_pid': agent['placement']['process_id'] = 11
            elif mutation == 'thread': agent['placement']['mode'] = 'in_process_thread'
            elif mutation == 'unattested': agent['placement']['attested'] = False
            elif mutation == 'binary': agent['placement']['binary_sha256'] = '0' * 64
            elif mutation == 'bootstrap': agent['placement']['bootstrap_sha256'] = None
            elif mutation == 'missing': record['snapshot']['pages'][0]['placement'] = None
            elif mutation == 'test_fail': report['results'][0]['subtests'][0]['status'] = 'FAIL'
            elif mutation == 'wrong_nonce': record['snapshot']['nonce'] = '0' * 64
            else: report['run_info']['oryn_candidate_sha256'] = '0' * 64
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                qualify(record, report, context, binary)

    def test_duplicate_or_foreign_principals_and_partial_mapping_coverage_cannot_qualify(self):
        for mutation in ('duplicate', 'foreign_fixture', 'foreign_owner', 'stale', 'one_process', 'missing_wasm', 'duplicate_backing', 'no_results'):
            record, report, context, binary = self.evidence()
            snapshot = record['snapshot']
            if mutation == 'duplicate': snapshot['agents'].append(copy.deepcopy(snapshot['agents'][0]))
            elif mutation == 'foreign_fixture': snapshot['agents'][0]['script_url'] = 'https://foreign.test/worker.js?placement=1'
            elif mutation == 'foreign_owner': snapshot['agents'][0]['owners'] = [{'kind': 'window', 'id': 99}]
            elif mutation == 'stale': snapshot['agents'][0]['live'] = False
            elif mutation == 'one_process': snapshot['agents'][1]['placement']['process_id'] = 13
            elif mutation == 'missing_wasm': snapshot['shared_backings'].pop()
            elif mutation == 'duplicate_backing': snapshot['shared_backings'][1]['backing_sha256'] = snapshot['shared_backings'][0]['backing_sha256']
            else: report['results'] = []
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                qualify(record, report, context, binary)

import io
import json
import hashlib
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch
import unittest

from oryn_validation.fixture_tls import _Writes, load_contract, verify_receipts
from oryn_validation.wpt import relay_commands, verify_fixture_services


class FixtureTransportTests(unittest.TestCase):
    def normal(self, wire, method='GET', **flags):
        state = _Writes(io.BytesIO())
        state.write(wire)
        state.complete = True
        for key, value in flags.items():
            setattr(state, key, value)
        return state.normal(method)

    def test_only_complete_ordinary_http_framing_can_get_tls_close_notify(self):
        self.assertTrue(self.normal(b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc'))
        for wire in (b'HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabc',
                     b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 3\r\n\r\nabc',
                     b'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n',
                     b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n',
                     b'HTTP/1.1 200 OK\r\nHeader: bad\x00value\r\n\r\n'):
            with self.subTest(wire=wire):
                self.assertFalse(self.normal(wire))
        for flags in ({'failed': True}, {'tainted': True}, {'complete': False}):
            self.assertFalse(self.normal(b'HTTP/1.1 200 OK\r\n\r\nabc', **flags))

    def test_head_and_no_body_responses_preserve_original_semantics(self):
        self.assertTrue(self.normal(b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n', 'HEAD'))
        self.assertFalse(self.normal(b'HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc', 'HEAD'))
        self.assertTrue(self.normal(b'HTTP/1.1 204 No Content\r\n\r\n'))
        self.assertFalse(self.normal(b'HTTP/1.1 204 No Content\r\n\r\na'))

    def test_relays_keep_end_to_end_tls_and_actual_address_spaces(self):
        fixture = {'endpoint_routes': [dict(protocol=name, port=port, connect_ip=ip) for name, port, ip in
                    [('http-local', 8002, '192.168.1.2'), ('https-local', 8445, '192.168.1.2'),
                     ('http-public', 8003, '2001:4860::1'), ('https-public', 8446, '2001:4860::1')]]}
        commands = relay_commands('/store/socat', fixture)
        self.assertEqual(len(commands), 4)
        self.assertEqual(commands[-1][1][-1], 'TCP4:127.0.0.1:8446')
        self.assertIn('bind=[2001:4860::1],ipv6only=1', commands[-1][1][-2])
        self.assertFalse(any('OPENSSL' in argument for _, argv, _ in commands for argument in argv))
        fixture['endpoint_routes'].pop()
        with self.assertRaises(ValueError):
            relay_commands('/store/socat', fixture)

    def test_typed_loader_rejects_substituted_interpreter_certificate_and_module(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            policy = {'schema_version': 2, 'kind': 'wpt_h1_normal_completion_tls_shutdown',
                'unwrap_timeout_seconds': 1,
                'modules': {'wptserve.server': 'server', 'wptrunner.environment': 'environment'},
                'response_module': 'response', 'handlers_module': 'handlers', 'pipes_module': 'pipes',
                'certificate': 'certificate', 'bootstrap_inputs': ['fixture_tls_module', 'fixture_tls_bootstrap']}
            path = root / 'policy.json'
            path.write_text(json.dumps(policy))
            assets = {'fixture_tls_policy': path, 'wpt_checkout': root / 'upstream'}
            for name in ('python', 'certificate', 'server', 'environment'):
                assets[name] = root / name
                assets[name].write_text(name)
            assets['fixture'] = root / 'fixture.json'
            assets['fixture'].write_text(json.dumps({'tls': {'certificate': {'path': 'correct-cert'}}}))
            assets['core_profile_registry'] = root / 'registry.json'
            assets['core_profile_registry'].write_text(json.dumps({'profiles': {'standard-block': {'fixture': 'fixture'}}}))
            bound = {'assets': assets, 'run': {'profile': 'standard-block', 'assets': {'python': {'sha256': hashlib.sha256(assets['python'].read_bytes()).hexdigest()}}}}
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, 'interpreter differs'):
                load_contract(path, digest, bound=bound)
            # Offline validation progresses past a different verifier interpreter,
            # but still rejects the next invalid contract binding.
            with self.assertRaisesRegex(ValueError, 'certificate differs'):
                load_contract(path, digest, bound=bound, executing_bootstrap=False)
            bound['run']['assets']['python']['sha256'] = '0' * 64
            with self.assertRaisesRegex(ValueError, 'admitted interpreter changed'):
                load_contract(path, digest, bound=bound, executing_bootstrap=False)
            assets['python'] = Path(sys.executable).resolve()
            bound['run']['assets']['python']['sha256'] = hashlib.sha256(assets['python'].read_bytes()).hexdigest()
            with self.assertRaisesRegex(ValueError, 'certificate differs'):
                load_contract(path, digest, bound=bound)
            assets['fixture'].write_text(json.dumps({'tls': {'certificate': {'path': 'certificate'}}}))
            with self.assertRaisesRegex(ValueError, 'target module differs'):
                load_contract(path, digest, bound=bound)

    def test_retained_tls_receipts_survive_relocation_but_not_changed_input_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            contract = root / 'policy.json'
            contract.write_text('policy')
            digest = hashlib.sha256(contract.read_bytes()).hexdigest()
            ref = lambda name: {'path': str(root / name), 'sha256': 'a' * 64}
            identities = {'contract': {'scope': 'store', 'path': 'policy.json', 'sha256': digest}}
            value = {'python': ref('python'), 'receipt_inputs': identities, 'certificate': ref('cert'), 'response_module': ref('response'),
                     'modules': {'wptserve.server': ref('server')}, 'bootstrap_inputs': [ref('bootstrap')]}
            receipt = {'schema_version': 3, 'python': ref('python'), 'kind': 'actual_tls_server_bootstrap', 'input_identities': identities,
                       'port': 8443, 'pid': 42, 'birth_identity': [1, 2], 'mode': 'normal_h1_tls_shutdown',
                       'contract': {'path': '/old/policy.json', 'sha256': digest},
                       'certificate': ref('cert'), 'response_module': ref('response'),
                       'server_module': ref('server'), 'bootstrap_inputs': [ref('bootstrap')]}
            for name in ('certificate', 'response_module', 'server_module'):
                receipt[name]['path'] = '/old/' + name
            target = root / 'server-42.json'
            target.write_text(json.dumps(receipt))
            with patch('oryn_validation.fixture_tls.load_contract', return_value=value):
                self.assertEqual(len(verify_receipts(root, contract, digest, [8443], require_live=False)), 1)
                receipt['input_identities'] = {'contract': {'sha256': 'b' * 64}}
                target.write_text(json.dumps(receipt))
                with self.assertRaisesRegex(ValueError, 'typed input identities'):
                    verify_receipts(root, contract, digest, [8443], require_live=False)

    def test_offline_receipts_reject_old_missing_and_forged_interpreter_without_observing_verifier(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            contract = root / 'policy.json'; contract.write_text('policy')
            digest = hashlib.sha256(contract.read_bytes()).hexdigest()
            ref = lambda name: {'path': str(root / name), 'sha256': 'a' * 64}
            identities = {'python': {'scope': 'store', 'path': 'python', 'sha256': 'a' * 64}}
            value = {'python': ref('python'), 'receipt_inputs': identities, 'certificate': ref('cert'),
                     'response_module': ref('response'), 'modules': {'wptserve.server': ref('server')},
                     'bootstrap_inputs': [ref('bootstrap')]}
            receipt = {'schema_version': 3, 'kind': 'actual_tls_server_bootstrap', 'input_identities': identities,
                       'python': ref('python'), 'port': 8443, 'pid': 42, 'birth_identity': [1, 2],
                       'mode': 'normal_h1_tls_shutdown', 'contract': {'path': str(contract), 'sha256': digest},
                       'certificate': ref('cert'), 'response_module': ref('response'), 'server_module': ref('server'),
                       'bootstrap_inputs': [ref('bootstrap')]}
            target = root / 'server-42.json'
            with patch('oryn_validation.fixture_tls.load_contract', return_value=value) as loader:
                target.write_text(json.dumps(receipt))
                self.assertEqual(len(verify_receipts(root, contract, digest, [8443], require_live=False)), 1)
                self.assertFalse(loader.call_args.kwargs['executing_bootstrap'])
                for change in ('old', 'missing', 'forged', 'relative', 'identity'):
                    broken = json.loads(json.dumps(receipt))
                    if change == 'old': broken['schema_version'] = 2
                    elif change == 'missing': del broken['python']
                    elif change == 'forged': broken['python']['sha256'] = 'b' * 64
                    elif change == 'relative': broken['python']['path'] = 'python'
                    else: broken['input_identities']['python']['sha256'] = 'b' * 64
                    target.write_text(json.dumps(broken))
                    with self.subTest(change=change), self.assertRaises(ValueError):
                        verify_receipts(root, contract, digest, [8443], require_live=False)

    def test_service_original_requires_real_settlement_and_exact_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            assets = {'fixture_relay': {'scope': 'store', 'path': 'socat'},
                      'wpt_checkout': {'scope': 'store', 'path': 'wpt'}}
            bound = {'store_root': root, 'bundle_root': root / 'build', 'path': root / 'run.json',
                     'run': {'assets': assets}}
            fixture = {'endpoint_routes': [dict(protocol=name, port=port, connect_ip=ip) for name, port, ip in
                        [('http-local', 8002, '192.168.1.2'), ('https-local', 8445, '192.168.1.2'),
                         ('http-public', 8003, '2001:4860::1'), ('https-public', 8446, '2001:4860::1')]]}
            for name, argv, _ in relay_commands('/old/socat', fixture):
                out = root / 'services' / name
                out.mkdir(parents=True)
                raw = {'argv': argv, 'cleanup_complete': True, 'cleanup_errors': [], 'timed_out': False,
                       'forced_cleanup': False, 'error': 'owned execution cancelled by signal 15', 'cancellation_signal': 15}
                (out / 'process-result.json').write_text(json.dumps(raw))
                (out / 'service-result.json').write_text(json.dumps(dict(raw, service_stop_requested=True)))
                (out / 'preflight.json').write_text(json.dumps({'argv': argv, 'cwd': '/old/wpt',
                    'child_identity': [[1, 2], 3], 'owner_pid': 3, 'child_pid': 4}))
            options = {'original_roots': ('/old', '/old/build')}
            self.assertEqual(len(verify_fixture_services(bound, {'fixture': fixture}, root, **options)), 4)
            target = root / 'services/http-local/service-result.json'
            broken = json.loads(target.read_text()); broken['forced_cleanup'] = True
            target.write_text(json.dumps(broken))
            with self.assertRaisesRegex(ValueError, 'custody/cleanup'):
                verify_fixture_services(bound, {'fixture': fixture}, root, **options)

    def test_child_bootstrap_checks_only_startup_files_not_whole_dataset(self):
        import os
        from oryn_validation.fixture_tls import bootstrap_inputs
        from oryn_validation.inputs import snapshot_input
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            records = {}
            def write(name, value):
                path = root / (name + '.json')
                path.write_text(json.dumps(value))
                records[name] = snapshot_input(root, path.name)
                return path
            policy = {'schema_version': 2, 'kind': 'wpt_h1_normal_completion_tls_shutdown', 'unwrap_timeout_seconds': 1,
                      'modules': {'wptserve.server': 'server', 'wptrunner.environment': 'environment'},
                      'response_module': 'response', 'handlers_module': 'handlers', 'pipes_module': 'pipes',
                      'certificate': 'certificate', 'bootstrap_inputs': ['fixture_tls_module', 'fixture_tls_bootstrap']}
            policy_path = write('fixture_tls_policy', policy)
            write('core_profile_registry', {'profiles': {'standard-block': {'fixture': 'fixture'}}})
            write('fixture', {'tls': {'certificate': {'path': 'certificate.json'}}})
            for name in ('python', 'fixture_tls_module', 'fixture_tls_bootstrap', 'certificate',
                         'response', 'handlers', 'pipes', 'server', 'environment'):
                write(name, name)
            for name in ('runner_source', 'wpt_checkout'):
                (root / name).mkdir()
                records[name] = snapshot_input(root, name, kind='directory')
            # A declared unrelated directory is deliberately absent. Full
            # coordinator admission would reject it; bootstrap must never read it.
            records['large_unrelated'] = dict(records['wpt_checkout'], path='not-present')
            run = {'schema_version': 1, 'kind': 'run', 'run_id': 'bootstrap',
                   'build': {'build_id': 'build', 'manifest_sha256': 'a' * 64},
                   'runner': {'commit': 'b' * 40, 'tree_sha256': 'c' * 64},
                   'suite': {'name': 'wpt-core', 'sha256': 'd' * 64, 'upstream_commit': 'e' * 40},
                   'profile': 'standard-block', 'workers': 6, 'expected': {'automated': 15985, 'assessments': 752},
                   'immutable_inputs': list(records.values()), 'assets': records, 'environment': [],
                   'outputs': {'results': 'runs/result', 'events': 'runs/events', 'status': 'runs/status'},
                   'history': [], 'attempt': 1, 'original_run_id': 'bootstrap'}
            run_path = root / 'run.json'
            run_path.write_text(json.dumps(run))
            run_hash = hashlib.sha256(run_path.read_bytes()).hexdigest()
            env = {'ORYN_WPT_RUN_CONTRACT': str(run_path), 'ORYN_WPT_RUN_CONTRACT_SHA256': run_hash,
                   'ORYN_VALIDATION_STORE_ROOT': str(root), 'ORYN_VALIDATION_BUNDLE_ROOT': str(root)}
            digest = hashlib.sha256(policy_path.read_bytes()).hexdigest()
            with patch.dict(os.environ, env), patch('oryn_validation.fixture_tls.ROOT', root / 'runner_source'):
                bound = bootstrap_inputs(policy_path, digest)
                self.assertNotIn('large_unrelated', bound['assets'])
                (root / 'server.json').write_text('changed')
                with self.assertRaises(ValueError):
                    bootstrap_inputs(policy_path, digest)

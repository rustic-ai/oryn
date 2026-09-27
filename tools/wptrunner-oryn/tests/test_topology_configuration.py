"""Pinned upstream construction/start dispatch without servers or sockets."""
import copy
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

from tools.serve import serve
from wptserve import config as upstream_config
from oryn_validation.wpt import topology_config_class, topology_server_config

PORTS = {'http': [8000, 8001], 'https': [8443, 8444], 'h2': [9000],
         'ws': [8888], 'wss': [8889], 'http-local': [8002], 'http-public': [8003],
         'https-local': [8445], 'https-public': [8446]}


class PinnedTopologyConfiguration(unittest.TestCase):
    def setUp(self):
        self.logger = MagicMock()
        fixture = {'ports': PORTS, 'tls': {'certificate': {'path': 'server.pem'}, 'key': {'path': 'server.key'}}}
        self.config = topology_server_config({'assets': {'wpt_checkout': Path('/pinned-wpt')}},
            {'fixture': fixture, 'fixture_path': Path('/fixtures/fixture.json')})
        defaults = copy.deepcopy(serve.ConfigBuilder._default)
        defaults['ports'].pop('h2', None)
        self.enterContext(patch.object(serve.ConfigBuilder, '_default', defaults))
        self.allocate = self.enterContext(patch.object(upstream_config, 'get_port', return_value=49152))

    def test_old_late_merge_reproduces_missing_http2(self):
        config = self.config
        class LateConfig(serve.ConfigBuilder):
            def __enter__(self):
                self.update(config)
                return super().__enter__()
        with serve.build_config(self.logger, config_cls=LateConfig) as built:
            self.assertNotIn('h2', built.ports)

    def test_constructor_pins_all_eleven_ports_and_http2_start_dispatch(self):
        builder = topology_config_class(serve, self.config)
        before = copy.deepcopy(self.config)
        with serve.build_config(self.logger, config_cls=builder) as built:
            self.assertEqual(dict(built.ports), PORTS)
            self.assertEqual(sum(map(len, built.ports.values())), 11)
            self.assertEqual(built.ssl_config['cert_path'], '/fixtures/server.pem')
            self.assertEqual(built.ssl_config['key_path'], '/fixtures/server.key')
            self.assertEqual(built.doc_root, '/pinned-wpt')
            self.allocate.assert_not_called()
            with patch.object(serve, 'ServerProc') as process, patch.object(serve, 'http2_compatible', return_value=True):
                serve.start_servers(self.logger, built.server_host, built.ports, built.paths, [],
                    built.bind_address, built, SimpleNamespace(), [])
                self.assertEqual(process.call_count, 11)
                schemes = [call.kwargs['scheme'] for call in process.call_args_list]
                self.assertEqual(schemes.count('h2'), 1)
                h2 = [call for call in process.return_value.start.call_args_list
                      if serve.start_http2_server in call.args]
                self.assertEqual(len(h2), 1)
                self.assertIn(9000, h2[0].args)
        self.assertEqual(self.config, before)

    def test_later_override_cannot_silently_remove_or_replace_required_port(self):
        builder = topology_config_class(serve, self.config)
        configured = serve.build_config(self.logger, config_cls=builder)
        configured.update({'ports': {'h2': [9443]}})
        with self.assertRaisesRegex(ValueError, 'exact typed fixture'):
            with configured:
                self.fail('substituted HTTP/2 endpoint must fail before launch')

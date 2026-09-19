import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_fixture_contract as fixture


class FixtureContractTests(unittest.TestCase):
    def test_media_devices_are_explicit_and_unknown_modes_fail_closed(self):
        self.assertTrue(fixture.validate(self.config, self.root)['configured'])
        self.config['media_devices'] = 'synthetic_fixture_v1'
        self.assertFalse(fixture.validate(self.config, self.root)['qualified'])
        for value in (True, None, 'camera', {}, []):
            self.config['media_devices'] = value
            with self.assertRaises(ValueError):
                fixture.validate(self.config, self.root)

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.config = {'schema_version': 1, 'upstream_revision': fixture.REVISION,
                       'certificate_bypass': False, 'third_party_cookies': 'block', 'web_features': {'usermedia_element': 'standard'}, 'tls': {}, 'ports': {}, 'endpoint_routes': [],
                       'hosts': ['web-platform.test', 'www.web-platform.test', 'not-web-platform.test']}
        for name in ('ca', 'certificate', 'key'):
            path = self.root / (name + '.pem')
            path.write_text('synthetic fixture contract identity; not a certificate')
            self.config['tls'][name] = {'path': path.name, 'sha256': fixture.sha256(path)}
        port = 8000
        for protocol, count in fixture.PORTS.items():
            self.config['ports'][protocol] = list(range(port, port + count))
            for value in self.config['ports'][protocol]:
                self.config['endpoint_routes'].append({'protocol': protocol, 'port': value,
                    'connect_ip': '10.23.0.2' if protocol.endswith('-local') else
                                  '93.184.216.34' if protocol.endswith('-public') else '127.0.0.1'})
            port += count

    def test_configuration_is_not_live_qualification(self):
        result = fixture.validate(self.config, self.root)
        self.assertTrue(result['configured'])
        self.assertFalse(result['qualified'])

    def test_hostname_alias_does_not_replace_actual_address_space(self):
        for row in self.config['endpoint_routes']:
            row['connect_ip'] = '127.0.0.1'
        with self.assertRaisesRegex(ValueError, 'actual required IP space'):
            fixture.validate(self.config, self.root)

    def test_reserved_documentation_ip_is_not_public_evidence(self):
        for value in ('192.0.2.10', '198.51.100.3', '203.0.113.1', '0.0.0.0', '::'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                fixture.address_space(value)

    def test_native_and_python_ip_contract_boundaries(self):
        cases = {
            'loopback': ['127.0.0.1', '127.9.8.7', '::1', '::ffff:127.0.0.1'],
            'local': ['10.23.0.2', '172.16.0.1', '172.31.255.254', '192.168.1.1', '169.254.8.1', 'fd01::1', 'fe80::1'],
            'public': ['20.40.50.60', '8.8.8.8', '2603:1030:20e:3::1'],
        }
        for expected, values in cases.items():
            for value in values:
                self.assertEqual(fixture.address_space(value), expected, value)
        for value in ['100.64.0.1', '100.127.255.255', '192.0.0.9', '192.88.99.1',
                      '198.18.0.1', '224.0.0.1', '255.255.255.255', 'ff02::1',
                      '2001::1', '2001:db8::1', '2002::1', '3fff::1', '64:ff9b::1']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                fixture.address_space(value)

    def test_nonexistent_and_noncanonical_names_cannot_be_mapped(self):
        for host in ['nonexistent.web-platform.test', 'www.nonexistent.web-platform.test',
                     'UPPER.web-platform.test', 'x_y.web-platform.test', 'x..web-platform.test',
                     'web-platform.test.', 'web-platform.test.evil', 'web-platform.test']:
            config = copy.deepcopy(self.config)
            config['hosts'].append(host)
            with self.subTest(host=host), self.assertRaisesRegex(ValueError, 'hostname'):
                fixture.validate(config, self.root)

    def test_every_endpoint_and_distinct_port_required(self):
        self.config['endpoint_routes'].pop()
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            fixture.validate(self.config, self.root)

    def test_ca_identity_and_bypass_fail_closed(self):
        self.config['certificate_bypass'] = True
        with self.assertRaisesRegex(ValueError, 'validate certificates'):
            fixture.validate(self.config, self.root)
        self.config['certificate_bypass'] = False
        (self.root / self.config['tls']['ca']['path']).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'identity mismatch'):
            fixture.validate(self.config, self.root)

    def probe(self):
        return {'contract_sha256': hashlib.sha256(json.dumps(self.config, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
                'untrusted_certificate_rejected': True, 'wrong_hostname_rejected': True,
                'endpoints': [{'protocol': row['protocol'], 'port': row['port'],
                               'peer_ip': row['connect_ip'], 'helper_response_verified': True,
                               'certificate_verified': True} for row in self.config['endpoint_routes']]}

    def test_peer_and_certificate_negative_controls_bound_to_config(self):
        probe = self.probe()
        self.assertTrue(fixture.validate(self.config, self.root, probe)['qualified'])
        probe['endpoints'][-1]['peer_ip'] = '127.0.0.1'
        with self.assertRaisesRegex(ValueError, 'peer'):
            fixture.validate(self.config, self.root, probe)
        probe = self.probe()
        probe['untrusted_certificate_rejected'] = False
        with self.assertRaisesRegex(ValueError, 'negative controls'):
            fixture.validate(self.config, self.root, probe)


if __name__ == '__main__':
    unittest.main()

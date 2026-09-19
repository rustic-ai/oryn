"""Validate explicit fixture topology without changing hosts, trust or routing.

The native driver must consume endpoint routes before this can be operational.
Port-aware routing is necessary because pinned LNA wrappers change the port of
one hostname, rather than selecting a different hostname for each address space.
"""
import ipaddress
import hashlib
import json
from pathlib import Path
from wpt_full_qualification import REVISION, sha256

PORTS = {'http': 2, 'https': 2, 'h2': 1, 'ws': 1, 'wss': 1,
         'http-local': 1, 'https-local': 1, 'http-public': 1, 'https-public': 1}
# Deliberately version-independent and identical to native fixture.rs. Python's
# is_global tables change between releases; they cannot define our contract.
IPV4_NON_PUBLIC = [ipaddress.ip_network(cidr) for cidr in (
    '0.0.0.0/8', '100.64.0.0/10', '192.0.0.0/24', '192.0.2.0/24',
    '192.88.99.0/24', '198.18.0.0/15', '198.51.100.0/24', '203.0.113.0/24',
    '224.0.0.0/3')]
IPV4_LOCAL = [ipaddress.ip_network(cidr) for cidr in (
    '10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16', '169.254.0.0/16')]
IPV6_LOCAL = [ipaddress.ip_network(cidr) for cidr in ('fc00::/7', 'fe80::/10')]
IPV6_NON_PUBLIC = [ipaddress.ip_network(cidr) for cidr in ('2001::/23', '2001:db8::/32', '2002::/16', '3fff::/20')]


def canonical_ip(value):
    if not isinstance(value, str) or '%' in value:
        raise ValueError('fixture IP must be an unscoped address string')
    address = ipaddress.ip_address(value)
    if isinstance(address, ipaddress.IPv6Address) and address.ipv4_mapped is not None:
        return address.ipv4_mapped
    return address


def address_space(value):
    address = canonical_ip(value)
    if address.is_loopback:
        return 'loopback'
    if isinstance(address, ipaddress.IPv4Address):
        if any(address in network for network in IPV4_LOCAL):
            return 'local'
        if not any(address in network for network in IPV4_NON_PUBLIC):
            return 'public'
    else:
        if any(address in network for network in IPV6_LOCAL):
            return 'local'
        if address in ipaddress.ip_network('2000::/3') and not any(address in network for network in IPV6_NON_PUBLIC):
            return 'public'
    raise ValueError('fixture IP is reserved/unspecified, not a qualifying address space')


def fixture_host(host):
    if not isinstance(host, str) or not host or len(host) > 253 or not host.isascii():
        return False
    if any(not label or len(label) > 63 or label == 'nonexistent' or label.startswith('-') or label.endswith('-')
           or any(not ('a' <= char <= 'z' or '0' <= char <= '9' or char == '-') for char in label)
           for label in host.split('.')):
        return False
    return any(host == domain or host.endswith('.' + domain) for domain in ('web-platform.test', 'not-web-platform.test'))


def validate(config, root, probe=None):
    features = config.get("web_features")
    if not isinstance(features, dict) or set(features) != {"usermedia_element"} or features["usermedia_element"] not in ("standard", "legacy"):
        raise ValueError("fixture requires explicit native web feature profile")
    if config.get("third_party_cookies") not in ("allow", "block"):
        raise ValueError("fixture requires explicit third_party_cookies allow or block")
    if type(config.get("bluetooth_testing", False)) is not bool:
        raise ValueError("Bluetooth testing must be an explicit boolean")
    if config.get('media_devices', 'none') not in ('none', 'synthetic_fixture_v1'):
        raise ValueError('unknown native fixture media device configuration')
    if config.get('schema_version') != 1 or config.get('upstream_revision') != REVISION:
        raise ValueError('fixture contract version/revision mismatch')
    if config.get('certificate_bypass') is not False:
        raise ValueError('fixture trust must validate certificates')
    root = Path(root).resolve()
    for kind in ('ca', 'certificate', 'key'):
        artifact = config['tls'][kind]
        path = (root / artifact['path']).resolve()
        if root not in path.parents or not path.is_file() or sha256(path) != artifact['sha256']:
            raise ValueError('fixture TLS artifact identity mismatch: ' + kind)
    ports = config['ports']
    if set(ports) != set(PORTS):
        raise ValueError('all upstream fixture protocol variants are required')
    flattened = []
    for protocol, count in PORTS.items():
        values = ports[protocol]
        if len(values) != count or any(type(p) is not int or not 1 <= p <= 65535 for p in values):
            raise ValueError('invalid fixture ports: ' + protocol)
        flattened.extend(values)
    if len(set(flattened)) != len(flattened):
        raise ValueError('ambiguous fixture destination ports')
    routes = config['endpoint_routes']
    expected = {(protocol, port) for protocol, values in ports.items() for port in values}
    actual = {(row['protocol'], row['port']) for row in routes}
    if actual != expected or len(actual) != len(routes):
        raise ValueError('fixture endpoint routing incomplete or duplicate')
    for row in routes:
        space = 'local' if row['protocol'].endswith('-local') else 'public' if row['protocol'].endswith('-public') else 'loopback'
        if address_space(row['connect_ip']) != space:
            raise ValueError('endpoint does not use actual required IP space: ' + row['protocol'])
    hosts = config.get('hosts') or []
    if (not hosts or len(hosts) > 256 or 'web-platform.test' not in hosts
            or any(not fixture_host(host) for host in hosts) or len(set(hosts)) != len(hosts)):
        raise ValueError('invalid canonical positive fixture hostname set')
    if probe is None:
        return {'configured': True, 'qualified': False, 'reason': 'live topology and trust probes not collected'}
    identity = hashlib.sha256(json.dumps(config, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    if probe.get('contract_sha256') != identity:
        raise ValueError('topology probe is not bound to this fixture contract')
    observed = {(row['protocol'], row['port']): row for row in probe.get('endpoints', [])}
    if len(observed) != len(probe.get('endpoints', [])) or set(observed) != expected:
        raise ValueError('missing or duplicate topology probe')
    for route in routes:
        row = observed[(route['protocol'], route['port'])]
        if canonical_ip(row.get('peer_ip')) != canonical_ip(route['connect_ip']) or row.get('helper_response_verified') is not True:
            raise ValueError('probe did not reach the configured fixture peer')
        if route['protocol'].startswith(('https', 'wss', 'h2')) and row.get('certificate_verified') is not True:
            raise ValueError('fixture TLS validation unproven')
    if probe.get('untrusted_certificate_rejected') is not True or probe.get('wrong_hostname_rejected') is not True:
        raise ValueError('TLS negative controls missing')
    return {'configured': True, 'qualified': True}

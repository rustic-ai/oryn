"""Run-local, name-constrained loopback CA; never installed in a system store."""
import hashlib
import atexit
import re
import subprocess
import tempfile
import threading
import uuid
from pathlib import Path

_lock = threading.Lock()
_certificates = {}


def fixture_certificate(checkout):
    checkout = Path(checkout).resolve()
    with _lock:
        if checkout in _certificates:
            return _certificates[checkout][1]
        root = checkout / 'tools/certs'
        names = subprocess.check_output(['openssl', 'x509', '-in', str(root / 'web-platform.test.pem'),
            '-noout', '-ext', 'subjectAltName'], text=True)
        dns = {name.replace('not-web-platform.test', 'alt.localhost').replace('web-platform.test', 'localhost')
               for name in re.findall(r'DNS:([^,\s]+)', names)}
        if not dns or any(name != 'localhost' and not name.endswith('.localhost') for name in dns):
            raise ValueError('unexpected pinned WPT certificate names')
        directory = tempfile.TemporaryDirectory(prefix='oryn-wpt-tls-')
        atexit.register(directory.cleanup)
        folder = Path(directory.name)
        ca = folder / 'ca.pem'
        ca_key = folder / 'ca.key'
        subprocess.run(['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt',
            'ec_paramgen_curve:P-256', '-nodes', '-keyout', str(ca_key), '-out', str(ca),
            '-subj', '/CN=Oryn isolated WPT loopback CA', '-days', '2',
            '-addext', 'basicConstraints=critical,CA:TRUE,pathlen:0',
            '-addext', 'keyUsage=critical,keyCertSign,cRLSign',
            '-addext', 'nameConstraints=critical,permitted;DNS:localhost,permitted;IP:127.0.0.0/255.0.0.0'],
            check=True, capture_output=True)
        extension = folder / 'extensions.cnf'
        extension.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\n'
            'extendedKeyUsage=serverAuth\nsubjectAltName=' + ','.join('DNS:'+name for name in sorted(dns))
            + ',IP:127.0.0.1\n')
        subprocess.run(['openssl', 'req', '-new', '-key', str(root / 'web-platform.test.key'),
            '-subj', '/CN=localhost', '-out', str(folder / 'request.pem')], check=True, capture_output=True)
        certificate = folder / 'localhost.pem'
        subprocess.run(['openssl', 'x509', '-req', '-in', str(folder / 'request.pem'),
            '-CA', str(ca), '-CAkey', str(ca_key),
            '-set_serial', '0x'+uuid.uuid4().hex, '-days', '2', '-extfile', str(extension),
            '-out', str(certificate)], check=True, capture_output=True)
        result = {'certificate':str(certificate), 'key':str(root / 'web-platform.test.key'), 'ca':str(ca),
                  'ca_sha256':hashlib.sha256(ca.read_bytes()).hexdigest(),
                  'certificate_sha256':hashlib.sha256(certificate.read_bytes()).hexdigest()}
        _certificates[checkout] = (directory, result)
        return result

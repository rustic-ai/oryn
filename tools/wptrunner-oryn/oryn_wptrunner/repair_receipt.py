"""Observe declared fixture deliveries after official fixture servers are running.

This collector never starts a server, edits source files or manufactures success.
The caller retains its output as a candidate-bound assessment artifact.
"""
import argparse
import hashlib
import json
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request, urlopen
from .fixture_repairs import MANIFEST, digest, validate


def collect(candidate_path, checkout, origin, fetch=None):
    candidate_path = Path(candidate_path)
    candidate = json.loads(candidate_path.read_text())
    artifacts = candidate.get('artifacts', {})
    # Candidate manifests use a name-keyed artifact mapping.
    repair = artifacts.get('fixture_repairs') or {}
    if repair.get('sha256') != digest(MANIFEST):
        raise ValueError('candidate repair artifact is not the declared manifest')
    validate(repair.get('path', ''), repair['sha256'], checkout)
    parsed = urlsplit(origin)
    if parsed.scheme not in ('http', 'https') or not parsed.hostname or parsed.username or parsed.password or parsed.path not in ('', '/') or parsed.query or parsed.fragment:
        raise ValueError('fixture origin must be an explicit HTTP(S) origin')
    fetch = urlopen if fetch is None else fetch
    requests = []
    for entry in json.loads(MANIFEST.read_text())['repairs']:
        url = origin.rstrip('/') + entry['request_path']
        record = {'method': 'GET', 'request_path': entry['request_path'], 'origin': origin}
        try:
            with fetch(Request(url, headers={'Accept-Encoding': 'identity'}), timeout=30) as response:
                body = response.read(16 * 1024 * 1024 + 1)
                record.update(status=response.status, final_url=response.geturl(),
                              target_sha256=hashlib.sha256(body).hexdigest())
                if len(body) > 16 * 1024 * 1024 or response.geturl() != url:
                    record['error'] = 'oversized or redirected fixture response'
        except Exception as error:
            record['error'] = str(error)
        requests.append(record)
    return {'schema_version': 1, 'candidate_sha256': digest(candidate_path),
            'fixture_repairs_sha256': digest(MANIFEST), 'requests': requests}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--candidate', required=True)
    parser.add_argument('--checkout', required=True)
    parser.add_argument('--origin', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    receipt = collect(args.candidate, args.checkout, args.origin)
    Path(args.output).write_text(json.dumps(receipt, indent=2) + '\n')
    expected = {item['request_path']: item['target_sha256'] for item in json.loads(MANIFEST.read_text())['repairs']}
    if any(item.get('error') or item.get('status') != 200 or item.get('target_sha256') != expected[item['request_path']] for item in receipt['requests']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()

"""Read distinct native wire contracts from their authoritative source constants."""
import argparse
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
SOURCES = {'worker': ('worker.rs', 'WORKER_PROTOCOL_VERSION'),
           'context': ('context_service.rs', 'CONTEXT_SERVICE_PROTOCOL_VERSION'),
           'agent': ('agent_ipc.rs', 'VERSION')}


def versions(root=ROOT):
    result = {}
    for name, (file, constant) in SOURCES.items():
        text = (Path(root) / 'crates/oryn-native/src' / file).read_text()
        matches = re.findall(r'^pub const\s+' + constant + r'\s*:\s*u32\s*=\s*(\d+)\s*;', text, re.MULTILINE)
        if len(matches) != 1 or int(matches[0]) <= 0:
            raise ValueError('missing or ambiguous native protocol constant: ' + name)
        result[name] = int(matches[0])
    return result


def runtime_matches(runtime, root=ROOT):
    """Require all compiled wire identities for current candidate evidence."""
    expected = versions(root)
    return all(type(runtime.get(field)) is int and runtime[field] == expected[name]
               for name, field in [('worker', 'worker_protocol_version'),
                                   ('context', 'context_service_protocol_version'),
                                   ('agent', 'agent_ipc_protocol_version')])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--field', choices=sorted(SOURCES))
    args = parser.parse_args()
    value = versions()
    print(value[args.field] if args.field else json.dumps(value, sort_keys=True))

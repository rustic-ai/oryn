#!/usr/bin/env python3
"""Write a new provenance-only join; never overwrite historical qualification."""
import argparse
import json
import os
from pathlib import Path
from completion_provenance import join


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'components', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    result = join(args.candidate, json.loads(args.components.read_text()))
    with args.output.open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    print(json.dumps(dict(provenance_complete=True, qualified=False)))


if __name__ == '__main__':
    main()

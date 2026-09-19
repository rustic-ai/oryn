#!/usr/bin/env python3
"""Run the pinned native media decoder dependency after generation closes."""
import argparse
from pathlib import Path
from native_media_qualification import qualify

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'generation-gate', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(int(not qualify(args.candidate, args.generation_gate, args.output)['qualified']))

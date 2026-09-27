#!/usr/bin/env python3
"""Join compiled Rust V8 source identity to selected sources and generated binding."""
import argparse
import json
from pathlib import Path
from rusty_v8_source_provenance import FIELDS, digest, validate


def validate_runtime(selected, runtime, binding, archive, root, *, allow_relocated_source_manifest=False):
    current = validate(root)
    for field, expected in current.items():
        if field == 'rusty_v8_source_manifest_path' and allow_relocated_source_manifest:
            path = Path(selected.get(field, ''))
            if (not path.is_absolute() or not path.is_file()
                    or path.stat().st_size != current['rusty_v8_source_manifest_size_bytes']
                    or digest(path) != current['rusty_v8_source_manifest_sha256']):
                raise ValueError('retained Rust V8 source manifest differs from selected source')
            continue
        if selected.get(field) != expected:
            raise ValueError('selected Rust V8 source differs: ' + field)
    for field in FIELDS:
        if runtime.get(field) != current[field]:
            raise ValueError('compiled Rust V8 source differs: ' + field)
    if runtime.get('v8_archive_sha256') != digest(archive):
        raise ValueError('compiled V8 archive differs from selected artifact')
    if runtime.get('v8_binding_sha256') != digest(binding):
        raise ValueError('compiled V8 binding differs from selected artifact')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('selected', type=Path)
    parser.add_argument('runtime', type=Path)
    parser.add_argument('--binding', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument("--allow-relocated-source-manifest", action="store_true")
    args = parser.parse_args()
    validate_runtime(json.loads(args.selected.read_text()), json.loads(args.runtime.read_text()),
                     args.binding, args.archive, Path(__file__).resolve().parents[1],
                     allow_relocated_source_manifest=args.allow_relocated_source_manifest)


if __name__ == '__main__':
    main()

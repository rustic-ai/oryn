"""Exact Cargo-selected Rust V8 sources, independent of the C++ archive identity."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tomllib

VENDOR = 'vendor/rusty-v8'
MANIFEST = VENDOR + '/oryn-source-manifest.json'
PATCH = 'patches/rusty-v8/weak-callback-teardown.patch'
METADATA = 'patches/rusty-v8/weak-callback-teardown.json'
VERSION = '150.4.0'
CHECKSUM = '42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49'
SUPPLEMENTAL = {'third_party/icu/common/icudtl.dat': {
    'sha256': '1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612',
    'source_path': 'target/rusty-v8-src-150.4.0-schema5/third_party/icu/common/icudtl.dat',
    'icu_revision': 'ee5f27adc28bd3f15b2c293f726d14d2e336cbd5'}}
FIELDS = ('rusty_v8_source_manifest_sha256', 'rusty_v8_source_tree_sha256',
          'rusty_v8_weak_teardown_patch_sha256', 'rusty_v8_patch_metadata_sha256')


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def tree_digest(files):
    return hashlib.sha256(json.dumps(files, sort_keys=True, separators=(',', ':'),
                                     ensure_ascii=False).encode()).hexdigest()


def require(value, message):
    if not value:
        raise ValueError('Rust V8 source provenance: ' + message)


def inventory(directory):
    files = {}
    for path in sorted(directory.rglob('*')):
        require(not path.is_symlink(), 'symlink in vendor source')
        if path.is_file() and path != directory / 'oryn-source-manifest.json':
            files[path.relative_to(directory).as_posix()] = digest(path)
    require(files, 'empty vendor source')
    return files


def validate(root):
    root = Path(root).resolve()
    path = root / MANIFEST
    require(not (root / VENDOR).is_symlink() and not path.is_symlink(), 'vendor or manifest symlink')
    raw = path.read_bytes()
    manifest = json.loads(raw)
    require(manifest.get('schema_version') == 1 and manifest.get('crate') == 'v8'
            and manifest.get('version') == VERSION, 'manifest crate/version/schema mismatch')
    require(manifest.get('archive_only') is True, 'Rust-only archive compatibility missing')
    checksum = manifest.get('registry_checksum')
    require(checksum == CHECKSUM, 'upstream registry checksum mismatch')
    files = inventory(root / VENDOR)
    require(manifest.get('files') == files, 'vendor file set or content changed')
    require(manifest.get('tree_sha256') == tree_digest(files), 'vendor tree digest mismatch')
    supplemental = manifest.get('supplemental_files', {})
    require(supplemental == SUPPLEMENTAL, 'supplemental ICU provenance mismatch')
    for name, receipt in supplemental.items():
        require(files.get(name) == receipt['sha256'], 'supplemental ICU source drift')
    base = manifest.get('base_files')
    require(isinstance(base, dict) and set(base) <= set(files), 'base file population mismatch')
    require(not set(base) & set(supplemental), 'supplemental source falsely claimed as upstream')
    metadata_path = root / METADATA
    metadata = json.loads(metadata_path.read_bytes())
    require(metadata.get('schema_version') == 1 and metadata.get('crate') == 'v8'
            and metadata.get('crate_version') == VERSION, 'patch metadata identity mismatch')
    require(metadata.get('cpp_changes') is False and metadata.get('source_basis'),
            'patch baseline or Rust-only disposition missing')
    patch_hash = digest(root / PATCH)
    require(metadata.get('patch_sha256') == patch_hash, 'patch digest mismatch')
    require(manifest.get('patches') == [dict(path=PATCH, sha256=patch_hash)], 'patch selection mismatch')
    before, after = metadata.get('base_files'), metadata.get('patched_files')
    require(isinstance(before, dict) and isinstance(after, dict) and before
            and set(before) == set(after), 'patch source population mismatch')
    require(all((name in ('Cargo.toml', 'build.rs') or PurePosixPath(name).suffix == '.rs' and name.startswith(('src/', 'tests/')))
                and '..' not in PurePosixPath(name).parts for name in before), 'non-Rust patch source')
    require({name for name in files if name not in supplemental and base.get(name) != files[name]} == set(before), 'unreviewed source changes')
    for name in before:
        require(base.get(name) == before[name] and files.get(name) == after[name], 'patch base/result mismatch')
    cargo = tomllib.loads((root / 'Cargo.toml').read_text())
    override = cargo.get('patch', {}).get('crates-io', {}).get('v8', {})
    require(override == {'path': VENDOR}, 'Cargo does not select exact vendored v8')
    crate = tomllib.loads((root / VENDOR / 'Cargo.toml').read_text())['package']
    require(crate['name'] == 'v8' and crate['version'] == VERSION, 'selected crate identity mismatch')
    locked = [p for p in tomllib.loads((root / 'Cargo.lock').read_text())['package'] if p['name'] == 'v8']
    require(len(locked) == 1 and locked[0]['version'] == VERSION and 'source' not in locked[0]
            and 'checksum' not in locked[0], 'Cargo lock does not select local v8')
    require(path.read_bytes() == raw, 'manifest changed during validation')
    return dict(rusty_v8_source_manifest_path=str(path), rusty_v8_source_manifest_size_bytes=len(raw),
                rusty_v8_source_manifest_sha256=hashlib.sha256(raw).hexdigest(),
                rusty_v8_source_tree_sha256=tree_digest(files),
                rusty_v8_weak_teardown_patch_sha256=patch_hash,
                rusty_v8_patch_metadata_sha256=digest(metadata_path))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--output', type=Path)
    parser.add_argument('--verify', type=Path)
    args = parser.parse_args()
    value = validate(args.root)
    if args.verify:
        require(json.loads(args.verify.read_text()) == value, 'inputs changed across build')
    if args.output:
        args.output.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    elif not args.verify:
        print(json.dumps(value, sort_keys=True))


if __name__ == '__main__':
    main()

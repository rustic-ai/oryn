#!/usr/bin/env python3
"""Materialize the pinned Rust-only archive consumer; never build or modify cache."""
import argparse, hashlib, json, subprocess, tarfile
from pathlib import Path
CHECKSUM='42a978ff11f15b24e5c05a7123cf2b68f41e763546699781a924ef4e2cf43a49'
def sha(data): return hashlib.sha256(data).hexdigest()
def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('archive',type=Path)
    ap.add_argument('--destination',type=Path)
    args=ap.parse_args()
    root=Path(__file__).resolve().parents[1]
    target=(args.destination or root/'vendor/rusty-v8').resolve()
    if not target.is_relative_to(root): ap.error('destination must be inside repository')
    if target.exists(): ap.error('destination already exists; existing source is never overwritten')
    if sha(args.archive.read_bytes())!=CHECKSUM:ap.error('upstream crate checksum mismatch')
    patch=root/'patches/rusty-v8/weak-callback-teardown.patch'
    metadata=json.loads(patch.with_suffix('.json').read_text())
    if sha(patch.read_bytes())!=metadata['patch_sha256']:ap.error('reviewed patch checksum mismatch')
    directories=('src/','gen/','tests/','examples/','benches/')
    roots={'Cargo.toml','Cargo.toml.orig','Cargo.lock','build.rs','BUILD.gn','README.md','LICENSE','LICENSE-MIT','tools/download_file.py'}
    files={}
    with tarfile.open(args.archive) as source:
        for entry in source.getmembers():
            name=entry.name.removeprefix('v8-150.4.0/')
            if not (name.startswith(directories) or name in roots):continue
            if not entry.isfile():
                if entry.isdir():continue
                ap.error('nonregular source entry: '+name)
            if '..' in Path(name).parts or Path(name).is_absolute():ap.error('invalid archive path')
            data=source.extractfile(entry).read();files[name]=data
    for name,expected in metadata['base_files'].items():
        if (sha(files[name]) if name in files else None)!=expected:ap.error('patch base mismatch: '+name)
    target.mkdir(parents=True)
    for name,data in files.items():
        path=target/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
    subprocess.run(['git','apply','--directory='+target.relative_to(root).as_posix(),str(patch)],cwd=root,check=True)
    supplemental_name='third_party/icu/common/icudtl.dat'
    supplemental_source='target/rusty-v8-src-150.4.0-schema5/'+supplemental_name
    supplemental_hash='1cf67874b5a87a8363a86fb3f81e3cbbed54d389062dab8fb52308d5cf8c8612'
    supplemental_data=(root/supplemental_source).read_bytes()
    if sha(supplemental_data)!=supplemental_hash: raise RuntimeError('retained ICU checksum mismatch')
    supplemental_target=target/supplemental_name
    supplemental_target.parent.mkdir(parents=True,exist_ok=True)
    supplemental_target.write_bytes(supplemental_data)
    final={p.relative_to(target).as_posix():sha(p.read_bytes()) for p in target.rglob('*') if p.is_file()}
    for name,expected in metadata['patched_files'].items():
        if final.get(name)!=expected:raise RuntimeError('patched source hash mismatch: '+name)
    tree=sha(json.dumps(final,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode())
    manifest={'schema_version':1,'crate':'v8','version':'150.4.0','registry_checksum':CHECKSUM,
      'archive_only':True,'source_basis':{'archive':'v8-150.4.0.crate','sha256':CHECKSUM,'selection':{'directories':list(directories),'root_files':sorted(roots)},'omission_policy':'C++ engine/build toolchain trees omitted; V8_FROM_SOURCE rejected. Use retained verified archive and generated ABI bindings from engine profile.'},
      'supplemental_files':{supplemental_name:{'sha256':supplemental_hash,'source_path':supplemental_source,'icu_revision':'ee5f27adc28bd3f15b2c293f726d14d2e336cbd5'}},
      'base_files':{n:sha(data) for n,data in files.items()},'files':final,'tree_sha256':tree,
      'patches':[{'path':'patches/rusty-v8/weak-callback-teardown.patch','sha256':metadata['patch_sha256']}]}
    (target/'oryn-source-manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    print(tree)
if __name__=='__main__':main()

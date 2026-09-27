"""Reproducible source archive matching source-tree-hash.sh's byte identities."""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import stat
import tarfile

INDEX = '.oryn-source-index.json'
EXCLUDED = {'docs/oryn2/IMPLEMENTATION_STATUS.md', 'docs/oryn2/CAPABILITY_MILESTONES.md'}


def source_digest(records):
    value = hashlib.sha256()
    for record in records:
        value.update(record['path'].encode() + b'\0' + record['sha256'].encode() + b'\n')
    return value.hexdigest()


def inventory(root, paths):
    root = Path(root).resolve(); records = []
    for name in sorted(set(paths), key=lambda name: name.encode()):
        if name in EXCLUDED or name.startswith('benchmarks/evidence/'): continue
        source = root / name
        if not source.is_file(): continue
        if (name == INDEX or '\n' in name or '\r' in name or PurePosixPath(name).is_absolute()
                or '..' in PurePosixPath(name).parts or root not in source.resolve().parents):
            raise ValueError('source snapshot path is not an owned reproducible file')
        data = source.read_bytes()
        records.append({'path': name, 'sha256': hashlib.sha256(data).hexdigest(),
                        'size': len(data), 'mode': stat.S_IMODE(source.stat().st_mode)})
    if not records: raise ValueError('source snapshot is empty')
    return records


def write(path, root, records):
    index = {'schema_version': 1, 'source_tree_sha256': source_digest(records), 'files': records}
    encoded = (json.dumps(index, sort_keys=True, separators=(',', ':')) + '\n').encode()
    with Path(path).open('xb') as target, tarfile.open(fileobj=target, mode='w', format=tarfile.PAX_FORMAT) as archive:
        for record in records:
            data = (Path(root) / record['path']).read_bytes()
            if len(data) != record['size'] or hashlib.sha256(data).hexdigest() != record['sha256']:
                raise ValueError('source changed while capturing snapshot')
            info = tarfile.TarInfo(record['path']); info.size = len(data); info.mode = record['mode']
            archive.addfile(info, io.BytesIO(data))
        info = tarfile.TarInfo(INDEX); info.size = len(encoded); info.mode = 0o644
        archive.addfile(info, io.BytesIO(encoded))
    return index


def validate(path, expected):
    with tarfile.open(path, 'r:') as archive:
        members = archive.getmembers()
        names = [member.name for member in members]
        if len(names) != len(set(names)) or INDEX not in names or len(names) > 200000:
            raise ValueError('source archive index or entry identities invalid')
        for member in members:
            name = PurePosixPath(member.name)
            if not member.isfile() or name.is_absolute() or '..' in name.parts:
                raise ValueError('source archive contains an unsafe entry')
        index_member = members[names.index(INDEX)]
        if index_member.size > 32 * 1024 * 1024: raise ValueError('source archive index oversized')
        index = json.load(archive.extractfile(index_member))
        records = index.get('files', [])
        if (index.get('schema_version') != 1 or index.get('source_tree_sha256') != expected
                or source_digest(records) != expected or names != [record['path'] for record in records] + [INDEX]
                or [record['path'] for record in records] != sorted({record['path'] for record in records}, key=lambda name: name.encode())):
            raise ValueError('source archive does not bind the exact source-tree identity')
        # Exact ordered names were checked above. Reuse their TarInfo objects:
        # TarFile.getmember performs a linear name scan for each call.
        for member, record in zip(members[:-1], records):
            if member.size != record['size'] or member.mode != record['mode']:
                raise ValueError('source archive file metadata changed')
            with archive.extractfile(member) as stream:
                actual = hashlib.file_digest(stream, 'sha256').hexdigest()
            if actual != record['sha256']: raise ValueError('source archive bytes changed')
    return index

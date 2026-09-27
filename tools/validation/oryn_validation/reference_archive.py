"""Exact already-pinned Chrome publisher archive proof; no extraction/download.

The original publisher ZIP omits app signature resources. This contract records
that fact; it does not call the reference app signed-valid or relax Oryn signing.
"""
import hashlib
import os
from pathlib import Path
import stat
import zipfile

from .inputs import _identity, resolve_input
from .manifests import manifest_digest, relative_path

PACKAGE_SHA256 = '6f67faa4b34dd551b53abb6fee24edeae470ab695b0b100ddc4885ff0be6724a'
PACKAGE_SIZE = 191015853
PACKAGE_MEMBERS = 675
APP_PREFIX = 'chrome-mac-arm64/Google Chrome for Testing.app/'


def verify_archive(path):
    path = Path(path).absolute()
    resolve_input(path.parent, path.name)
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode) or before.st_size != PACKAGE_SIZE:
        raise ValueError('reference archive size/type differs from pinned publisher package')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    rows, seen = [], set()
    with os.fdopen(fd, 'rb') as stream:
        if _identity(os.fstat(stream.fileno())) != _identity(before):
            raise ValueError('reference archive changed before opening')
        if hashlib.file_digest(stream, 'sha256').hexdigest() != PACKAGE_SHA256:
            raise ValueError('reference archive differs from pinned publisher SHA256')
        stream.seek(0)
        with zipfile.ZipFile(stream) as archive:
            members = archive.infolist()
            if len(members) != PACKAGE_MEMBERS:
                raise ValueError('reference archive member population differs')
            for member in members:
                name = member.filename[:-1] if member.is_dir() else member.filename
                relative_path(name)
                if name.casefold() in seen:
                    raise ValueError('duplicate or case-aliased reference ZIP member')
                seen.add(name.casefold())
                mode = member.external_attr >> 16
                kind = stat.S_IFMT(mode)
                if member.create_system != 3 or kind not in (stat.S_IFDIR, stat.S_IFREG, stat.S_IFLNK):
                    raise ValueError('reference ZIP member has unsupported type')
                if member.is_dir() != (kind == stat.S_IFDIR) or stat.S_IMODE(mode) > 0o777:
                    raise ValueError('reference ZIP type or mode disagrees with path')
                row = {'path': name, 'mode': stat.S_IMODE(mode)}
                if kind == stat.S_IFDIR:
                    if member.file_size:raise ValueError('reference ZIP directory has data')
                    row['type'] = 'directory'
                elif kind == stat.S_IFLNK:
                    if member.file_size > 4096:raise ValueError('reference link spelling too long')
                    row.update(type='symlink', target=archive.read(member).decode('utf-8'))
                else:
                    with archive.open(member) as content:
                        digest = hashlib.file_digest(content, 'sha256').hexdigest()
                    row.update(type='file', size=member.file_size, sha256=digest,
                               executable=bool(mode & 0o111))
                rows.append(row)
        if _identity(os.fstat(stream.fileno())) != _identity(before) or _identity(path.lstat()) != _identity(before):
            raise ValueError('reference archive changed while validating')
    rows.sort(key=lambda row:row['path'])
    app_root = APP_PREFIX.rstrip('/')
    root = [row for row in rows if row['path'] == app_root]
    if len(root) != 1 or root[0]['type'] != 'directory':
        raise ValueError('pinned reference app root missing from archive')
    body = {'mode': root[0]['mode'], 'files': [], 'directories': [], 'symlinks': []}
    for row in rows:
        if not row['path'].startswith(APP_PREFIX):continue
        name = row['path'][len(APP_PREFIX):]
        if name.endswith('/CodeResources') or '_CodeSignature' in Path(name).parts:
            raise ValueError('publisher archive signature-resource absence differs')
        value = {key:item for key,item in row.items() if key not in ('type','path')}
        value['path'] = name
        if row['type'] == 'symlink':
            value.pop('mode');body['symlinks'].append(value)
        elif row['type'] == 'directory':body['directories'].append(value)
        else:body['files'].append(value)
    return {'package_sha256':PACKAGE_SHA256,'members':len(rows),
            'members_sha256':manifest_digest(rows),'app':body}

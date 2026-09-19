#!/usr/bin/env python3
"""Bind the Oryn C++ bridge to the exact compiler and C++ ABI of a built V8 archive.

Reads build provenance only; never compiles or changes the engine. Keep the
referenced source/toolchain directories with the profile: no system-header fallback.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def record(path):
    path = path.resolve(strict=True)
    return {'path': str(path), 'sha256': digest(path), 'size_bytes': path.stat().st_size}


def absolute_flags(flags, directory):
    result = []
    path_next = None
    for flag in flags:
        if path_next:
            result.append(str((directory / flag).resolve(strict=path_next != '-I')))
            path_next = None
        elif flag in ('-I', '-isystem', '-isysroot', '--sysroot'):
            result.append(flag)
            path_next = flag
        else:
            for prefix in ('-isystem', '-I', '--sysroot='):
                if flag.startswith(prefix) and flag != prefix:
                    flag = prefix + str((directory / flag[len(prefix):]).resolve(strict=prefix != '-I'))
                    break
            if flag.startswith('-fcrash-diagnostics-dir='):
                flag = '-fcrash-diagnostics-dir=' + str((directory / flag.split('=', 1)[1]).resolve())
            result.append(flag)
    if path_next:
        raise ValueError('missing path in GN flags')
    return result


def prepare(source, gn_out, archive, binding, target):
    source, gn_out = source.resolve(strict=True), gn_out.resolve(strict=True)
    ninja = gn_out / 'obj/rusty_v8.ninja'
    variables = dict(re.findall(r'^(\w+) = (.*)$', ninja.read_text(), re.M))
    toolchain = gn_out / 'toolchain.ninja'
    match = re.search(r'^rule cxx\n  command = (.*?) -MD ', toolchain.read_text(), re.M)
    if match is None:
        raise ValueError('missing exact GN C++ compiler')
    compiler_tokens = shlex.split(match.group(1))
    if len(compiler_tokens) != 1:
        raise ValueError('compiler wrappers require explicit audited support')
    compiler = (gn_out / compiler_tokens[0]).resolve(strict=True)
    flags = absolute_flags(shlex.split(' '.join(variables[key] for key in
                          ('defines', 'include_dirs', 'cflags', 'cflags_cc'))), gn_out)
    roots = [source / 'buildtools/third_party/libc++',
             source / 'third_party/libc++/src/include',
             source / 'third_party/libc++abi/src/include',
             compiler.parent.parent / 'lib/clang']
    config = roots[0] / '__config_site'
    text = config.read_text()
    if '#define _LIBCPP_ABI_NAMESPACE __Cr' not in text or '#define _LIBCPP_ABI_VERSION 2' not in text:
        raise ValueError('unsupported Chromium libc++ ABI configuration')
    if '-nostdinc++' not in flags or '-fno-exceptions' not in flags or '-fno-rtti' not in flags:
        raise ValueError('missing required isolated engine C++ flags')
    for root in roots[:3]:
        if not any(str(root) in flag for flag in flags):
            raise ValueError('GN flags omit pinned C++ headers: ' + str(root))
    if not any(flag.startswith('-std=c++') for flag in flags):
        raise ValueError('missing engine language version')
    sdk = flags[flags.index('-isysroot') + 1] if '-isysroot' in flags else None
    files = {compiler, ninja, toolchain}
    for root in roots:
        if not root.is_dir():
            raise ValueError('missing retained compiler/header directory: ' + str(root))
        files.update(p for p in root.rglob('*') if p.is_file())
    return {'schema_version': 1, 'scope': 'archive-bound C++ bridge ABI; not conformance qualification',
            'target': target, 'archive': record(archive), 'binding': record(binding),
            'compiler': str(compiler), 'flags': flags, 'sdkroot': sdk,
            'libcxx_namespace': '__Cr', 'libcxx_abi_version': 2,
            'stdlib_linkage': 'included-in-rusty-v8',
            'header_roots': [str(p) for p in roots],
            'inputs': [record(p) for p in sorted(files)]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('source', 'gn-out', 'archive', 'binding', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--target', required=True)
    args = parser.parse_args()
    manifest = prepare(args.source, args.gn_out, args.archive, args.binding, args.target)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, indent=2) + '\n')
    print(digest(args.output))


if __name__ == '__main__':
    main()

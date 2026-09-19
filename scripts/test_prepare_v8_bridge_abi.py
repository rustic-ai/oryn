"""Archive/bridge ABI provenance tests; execute after the generation gate."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('bridge_abi', ROOT / 'scripts/prepare-v8-bridge-abi.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class BridgeAbiTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'source'
        self.gn = self.root / 'gn'
        self.compiler = self.root / 'clang/bin/clang++'
        self.compiler.parent.mkdir(parents=True)
        self.compiler.write_bytes(b'pinned compiler')
        self.roots = [self.source / 'buildtools/third_party/libc++',
                      self.source / 'third_party/libc++/src/include',
                      self.source / 'third_party/libc++abi/src/include',
                      self.root / 'clang/lib/clang/23/include']
        for root in self.roots:
            root.mkdir(parents=True)
            (root / 'header').write_bytes(b'pinned header')
        (self.roots[0] / '__config_site').write_text(
            '#define _LIBCPP_ABI_NAMESPACE __Cr\n#define _LIBCPP_ABI_VERSION 2\n')
        (self.gn / 'obj').mkdir(parents=True)
        (self.gn / 'toolchain.ninja').write_text('rule cxx\n  command = ' + str(self.compiler) + ' -MD ${in}\n')
        self.ninja = self.gn / 'obj/rusty_v8.ninja'
        self.ninja.write_text('defines = -DV8_ENABLE_SANDBOX\ninclude_dirs = -I' + str(self.roots[0]) +
            '\ncflags = -fno-exceptions -fno-rtti\ncflags_cc = -std=c++23 -nostdinc++ -isystem' +
            str(self.roots[1]) + ' -isystem' + str(self.roots[2]) + '\n')
        self.archive = self.root / 'archive.a'
        self.binding = self.root / 'binding.rs'
        self.archive.write_bytes(b'archive')
        self.binding.write_bytes(b'binding')

    def prepare(self):
        return MODULE.prepare(self.source, self.gn, self.archive, self.binding, 'aarch64-apple-darwin')

    def test_exact_compiler_headers_archive_and_flags_are_bound(self):
        manifest = self.prepare()
        self.assertEqual(manifest['archive']['sha256'], MODULE.digest(self.archive))
        self.assertEqual(manifest['binding']['sha256'], MODULE.digest(self.binding))
        self.assertEqual(manifest['compiler'], str(self.compiler))
        self.assertIn('-std=c++23', manifest['flags'])
        self.assertIn(str(self.roots[2] / 'header'), [x['path'] for x in manifest['inputs']])
        before = manifest['inputs']
        (self.roots[2] / 'header').write_bytes(b'changed header')
        self.assertNotEqual(before, self.prepare()['inputs'])

    def test_system_cpp_fallback_or_namespace_only_config_rejected(self):
        original = self.ninja.read_text()
        self.ninja.write_text(original.replace('-nostdinc++', ''))
        with self.assertRaisesRegex(ValueError, 'isolated engine'):
            self.prepare()
        self.ninja.write_text(original)
        (self.roots[0] / '__config_site').write_text('#define _LIBCPP_ABI_NAMESPACE __Cr\n')
        with self.assertRaisesRegex(ValueError, 'ABI configuration'):
            self.prepare()

    def test_relative_paths_and_missing_path_operand(self):
        flags = MODULE.absolute_flags(['-I../source', '-isystem', '../source'], self.gn)
        self.assertEqual(flags, ['-I' + str(self.source), '-isystem', str(self.source)])
        with self.assertRaisesRegex(ValueError, 'missing path'):
            MODULE.absolute_flags(['-isystem'], self.gn)
        # GN can retain optional generated include directories that do not exist.
        missing = self.gn / 'gen/unused'
        self.assertEqual(MODULE.absolute_flags(['-Igen/unused', '-I', 'gen/unused'], self.gn),
                         ['-I' + str(missing), '-I', str(missing)])


if __name__ == '__main__':
    unittest.main()

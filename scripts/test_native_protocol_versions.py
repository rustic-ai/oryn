import tempfile
from pathlib import Path
import unittest
from native_protocol_versions import versions, SOURCES


class ProtocolVersionsTests(unittest.TestCase):
    def test_independent_wire_versions_never_collapse(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'crates/oryn-native/src'
            source.mkdir(parents=True)
            for name, (file, constant) in SOURCES.items():
                value = {'worker': 13, 'context': 45, 'agent': 14}[name]
                (source / file).write_text(f'pub const {constant}:u32={value};\n')
            self.assertEqual(versions(root), dict(worker=13, context=45, agent=14))
            (source / 'worker.rs').write_text('pub const WORKER_PROTOCOL_VERSION:u32=0;')
            with self.assertRaises(ValueError):
                versions(root)

    def test_duplicate_source_constant_rejects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'crates/oryn-native/src'
            source.mkdir(parents=True)
            (source / 'worker.rs').write_text('pub const WORKER_PROTOCOL_VERSION:u32=13;\npub const WORKER_PROTOCOL_VERSION:u32=14;')
            with self.assertRaisesRegex(ValueError, 'ambiguous'):
                versions(root)

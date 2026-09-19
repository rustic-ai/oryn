"""Distinct source contracts and current-consumer wiring; never starts a worker."""
from pathlib import Path
import sys
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from native_protocol_versions import SOURCES, versions, runtime_matches

class NativeProtocolVersionsTests(unittest.TestCase):
    def test_runtime_requires_every_distinct_wire_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); native=root/'crates/oryn-native/src'; native.mkdir(parents=True)
            for index,(name,(file,constant)) in enumerate(SOURCES.items(),10):
                (native/file).write_text(f'pub const {constant}: u32 = {index};\n')
            runtime={'worker_protocol_version':10,'context_service_protocol_version':11,'agent_ipc_protocol_version':12}
            self.assertTrue(runtime_matches(runtime,root))
            self.assertFalse(runtime_matches(dict(runtime,context_service_protocol_version=12,agent_ipc_protocol_version=11),root))
            for field in runtime:
                for value in (None,0,'10',True,99):
                    with self.subTest(field=field,value=value):
                        self.assertFalse(runtime_matches(dict(runtime,**{field:value}),root))
                omitted=dict(runtime);omitted.pop(field);self.assertFalse(runtime_matches(omitted,root))

    def test_distinct_contracts_are_read_from_their_own_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); native = root / 'crates/oryn-native/src'
            native.mkdir(parents=True)
            for index, (name, (file, constant)) in enumerate(SOURCES.items(), 10):
                (native / file).write_text(f'pub const {constant}: u32 = {index};\n')
            self.assertEqual(versions(root), {'worker': 10, 'context': 11, 'agent': 12})
            file, constant = SOURCES['worker']
            (native / file).write_text(f'pub const {constant}: u32 = 0;\n')
            with self.assertRaisesRegex(ValueError, 'worker'): versions(root)
            (native / file).write_text((f'pub const {constant}: u32 = 10;\n') * 2)
            with self.assertRaisesRegex(ValueError, 'worker'): versions(root)

    def test_shell_checks_bind_source_number_as_json_not_a_literal_or_string(self):
        scripts = Path(__file__).resolve().parents[1]
        for name in ('run-layout-suite.sh', 'benchmark-layout.sh', 'benchmark-sandbox-worker.sh',
                     'run-app-compatibility-suite.sh', 'benchmark-app-compatibility.sh',
                     'run-execution-replay-suite.sh'):
            with self.subTest(name=name):
                source = (scripts / name).read_text()
                self.assertIn('native_protocol_versions.py" --field worker', source)
                self.assertIn('--argjson expected_worker_protocol "$expected_worker_protocol"', source)
                self.assertIn('.worker_protocol_version == $expected_worker_protocol', source)
                self.assertNotIn('.worker_protocol_version == 11', source)

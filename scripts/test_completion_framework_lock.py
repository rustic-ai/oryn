"""Reviewed generation-admission lock evolution preserves all historical inputs."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from framework_fixture_lock import CHANGES, validate

ROOT = Path(__file__).resolve().parents[1]


class FrameworkLockTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.folder = self.root / 'benchmarks/app-compatibility'
        self.folder.mkdir(parents=True)
        (self.root / 'scripts').mkdir()
        for name in ('fixtures.lock.json', 'fixtures-v1.lock.json', 'fixtures-v2.lock.json'):
            shutil.copyfile(ROOT / 'benchmarks/app-compatibility' / name, self.folder / name)
        for name in CHANGES.values():
            shutil.copyfile(ROOT / 'scripts' / name, self.root / 'scripts' / name)

    def test_reviewed_three_entrypoint_admissions_preserve_history(self):
        self.assertEqual(validate(self.root)['revision'], 3)

    def test_source_change_cannot_be_approved_by_rehashing_current_lock(self):
        import hashlib
        name = CHANGES['framework_validator']
        path = self.root / 'scripts' / name
        path.write_bytes(path.read_bytes() + b'\nexit 0\n')
        lock = self.folder / 'fixtures.lock.json'
        value = json.loads(lock.read_text())
        value['harness']['framework_validator'] = hashlib.sha256(path.read_bytes()).hexdigest()
        lock.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            validate(self.root)

    def test_denominator_dependencies_chain_and_history_cannot_change(self):
        path = self.folder / 'fixtures.lock.json'
        original = json.loads(path.read_text())
        for mutate in (lambda x: x['frameworks'].pop('react'),
                       lambda x: x['workspace'].update(pnpm_lock='0' * 64),
                       lambda x: x.update(previous_lock='../foreign.json'),
                       lambda x: x['harness'].update(chromium='0' * 64)):
            changed = copy.deepcopy(original)
            mutate(changed)
            path.write_text(json.dumps(changed))
            with self.assertRaises(ValueError):
                validate(self.root)
        path.write_text(json.dumps(original))
        archived = self.folder / 'fixtures-v2.lock.json'
        archived.write_bytes(archived.read_bytes() + b'\n')
        with self.assertRaisesRegex(ValueError, 'historical'):
            validate(self.root)

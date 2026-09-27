"""Git-object export and new generation declarations, using tiny local fixtures."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from oryn_validation import builds, generation
from oryn_validation.inputs import snapshot_input
from oryn_validation.production import validate_source_export


class GenerationContracts(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / 'source'
        self.source.mkdir()
        self.store = self.root / 'store'
        self.store.mkdir()
        for name in ('Cargo.toml', 'Cargo.lock', 'native.rs', generation.PLAN):
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture\n')
        subprocess.run(['git', '-C', str(self.source), 'init', '-q'], check=True)
        subprocess.run(['git', '-C', str(self.source), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(self.source), '-c', 'user.name=Test', '-c',
                        'user.email=test@example.invalid', '-c', 'core.hooksPath=/dev/null',
                        'commit', '--no-gpg-sign', '-qm', 'fixture'], check=True)

    def export(self):
        result = generation.export_source(self.source, self.store, export_path='sources/fixture',
                                          identity_path='identities/fixture.json')
        self.addCleanup(self.unseal)
        return result

    def unseal(self):
        for path in [self.store, *self.store.rglob('*')]:
            path.chmod(0o755 if path.is_dir() else 0o644)

    def test_export_proves_commit_without_repository_or_cache_copy(self):
        receipt = self.export()
        source = self.store / receipt['source_export']['path']
        identity = json.loads((self.store / receipt['source_identity']['path']).read_text())
        verified = validate_source_export(identity, source, receipt['source_export'])
        self.assertEqual(receipt['source']['commit'], verified['commit'])
        self.assertFalse((source / '.git').exists())
        self.assertFalse((source / 'target').exists())
        self.assertNotIn(str(self.root), json.dumps(receipt))
        self.assertEqual(generation.source_info(self.source, self.store / receipt['source_identity']['path']), receipt['source'])

    def test_modified_export_and_forged_git_objects_fail(self):
        receipt = self.export()
        source = self.store / receipt['source_export']['path']
        identity = json.loads((self.store / receipt['source_identity']['path']).read_text())
        forged = copy.deepcopy(identity)
        forged['commit'] = 'f' * 40
        with self.assertRaisesRegex(ValueError, 'object hash'):
            validate_source_export(forged, source, receipt['source_export'])
        self.unseal()
        (source / 'native.rs').write_text('changed\n')
        membership = snapshot_input(self.store, receipt['source_export']['path'], kind='directory')
        with self.assertRaisesRegex(ValueError, 'blob differs'):
            validate_source_export(identity, source, membership)

    def test_dirty_source_and_overwriting_export_fail(self):
        self.export()
        with self.assertRaises(ValueError):
            self.export()
        (self.source / 'native.rs').write_text('dirty\n')
        with self.assertRaisesRegex(ValueError, 'clean committed'):
            generation.export_source(self.source, self.store, export_path='sources/other', identity_path='identities/other.json')

    def predecessor(self):
        row = {'implementation_generated': True, 'tests_generated': True,
               'implementation_sources': [{'path': 'native.rs', 'sha256': 'a' * 64}],
               'test_sources': [{'path': 'native.rs', 'sha256': 'a' * 64}]}
        gate = {'schema_version': 1, 'plan_sha256': builds._identity(self.source / generation.PLAN)['sha256'],
                'requirements': {name: copy.deepcopy(row) for name in generation.REQUIREMENTS}}
        path = self.root / 'predecessor.json'
        path.write_text(json.dumps(gate))
        return path

    def test_new_gate_keeps_all_51_and_requires_review(self):
        predecessor = self.predecessor()
        output = self.store / 'gate.json'
        receipt = generation.prepare_generation_gate(self.source, predecessor, output)
        self.assertTrue(receipt['independent_review_required'])
        gate = json.loads(output.read_text())
        self.assertEqual(len(gate['requirements']), 51)
        self.assertEqual(set(gate['history']), {'predecessor_sha256'})
        script = Path(__file__).resolve().parents[3] / 'scripts/generation_gate.py'
        spec = importlib.util.spec_from_file_location('fixture_generation_gate', script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        module.generation_gate(output, root=self.source)
        gate['requirements'].pop(next(iter(gate['requirements'])))
        output.write_text(json.dumps(gate))
        with self.assertRaisesRegex(ValueError, 'all 51'):
            module.generation_gate(output, root=self.source)

    def test_generation_cannot_invent_missing_or_unfinished_requirements(self):
        predecessor = self.predecessor()
        value = json.loads(predecessor.read_text())
        value['requirements'][next(iter(value['requirements']))]['tests_generated'] = False
        predecessor.write_text(json.dumps(value))
        with self.assertRaisesRegex(ValueError, 'incomplete generation'):
            generation.prepare_generation_gate(self.source, predecessor, self.store / 'gate.json')

    def mapping_fixture(self):
        predecessor = self.predecessor()
        previous = json.loads(predecessor.read_text())
        previous['requirements']['Q-01']['caller_evidence'] = 'preserved mechanism and original receipts'
        previous['requirements']['Q-01']['package_review'] = 'review.md'
        old = {'path': 'artifacts/obsolete-helper.py', 'sha256': 'c' * 64}
        previous['requirements']['Q-01']['implementation_sources'].append(old)
        predecessor.write_text(json.dumps(previous))
        mapping = {'schema_version': 1, 'kind': 'generation-source-migration',
            'predecessor_sha256': builds._identity(predecessor)['sha256'], 'entries': [{
                'requirement': 'Q-01', 'field': 'implementation_sources', 'previous': old,
                'replacement_paths': ['native.rs'], 'reason': 'Current committed semantic successor'}]}
        return predecessor, previous, mapping

    def commit_mapping(self, mapping):
        path = self.source / 'mapping.json'
        path.write_text(json.dumps(mapping))
        subprocess.run(['git', '-C', str(self.source), 'add', 'mapping.json'], check=True)
        subprocess.run(['git', '-C', str(self.source), '-c', 'user.name=Test', '-c',
                        'user.email=test@example.invalid', '-c', 'core.hooksPath=/dev/null',
                        'commit', '--no-gpg-sign', '-qm', 'mapping'], check=True)
        return path

    def test_explicit_committed_mapping_preserves_all_requirements_roles_and_metadata(self):
        predecessor, previous, mapping = self.mapping_fixture()
        path = self.commit_mapping(mapping)
        output = self.store / 'mapped-gate.json'
        receipt = generation.prepare_generation_gate(self.source, predecessor, output, mapping_path='mapping.json')
        gate = json.loads(output.read_text())
        self.assertEqual(set(gate['requirements']), generation.REQUIREMENTS)
        self.assertEqual(gate['history']['mapping_sha256'], builds._identity(path)['sha256'])
        self.assertEqual(gate['history']['predecessor_sha256'], mapping['predecessor_sha256'])
        self.assertTrue(receipt['independent_review_required'])
        self.assertEqual(gate['requirements']['Q-01']['caller_evidence'], previous['requirements']['Q-01']['caller_evidence'])
        self.assertEqual(gate['requirements']['Q-01']['package_review'], 'review.md')
        for name in generation.REQUIREMENTS:
            for field in ('implementation_sources', 'test_sources'):
                self.assertEqual([row['path'] for row in gate['requirements'][name][field]], ['native.rs'])
            self.assertTrue(gate['requirements'][name]['implementation_generated'])
            self.assertTrue(gate['requirements'][name]['tests_generated'])
        self.assertFalse((self.source / 'artifacts').exists())
        self.assertEqual(json.loads(predecessor.read_text()), previous)

    def test_mapping_cannot_drop_or_reassign_roles_or_bypass_exact_predecessor(self):
        predecessor, _, mapping = self.mapping_fixture()
        mutations = [lambda value: value.update(predecessor_sha256='f' * 64),
                     lambda value: value.update(entries=[]),
                     lambda value: value['entries'][0].update(replacement_paths=[]),
                     lambda value: value['entries'][0].update(replacement_paths=['not-committed.py']),
                     lambda value: value['entries'][0]['previous'].update(sha256='f' * 64),
                     lambda value: value['entries'][0]['previous'].update(path='native.rs', sha256='a' * 64)]
        for index, mutation in enumerate(mutations):
            invalid = copy.deepcopy(mapping)
            mutation(invalid)
            path = self.commit_mapping(invalid)
            with self.assertRaises(ValueError):
                generation.prepare_generation_gate(self.source, predecessor, self.store / ('bad-' + str(index)),
                                                   mapping_path=path)

    def test_mapping_requires_explicit_selection_and_a_clean_committed_file(self):
        predecessor, _, mapping = self.mapping_fixture()
        with self.assertRaisesRegex(ValueError, 'not committed'):
            generation.prepare_generation_gate(self.source, predecessor, self.store / 'absent.json')
        path = self.source / 'mapping.json'
        path.write_text(json.dumps(mapping))
        with self.assertRaisesRegex(ValueError, 'committed'):
            generation.prepare_generation_gate(self.source, predecessor, self.store / 'untracked.json', mapping_path=path)


if __name__ == '__main__':
    unittest.main()

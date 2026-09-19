"""Architecture custody regressions; native/build commands use controlled fixtures."""
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import tarfile
import tempfile
import unittest
from contextlib import ExitStack
from types import SimpleNamespace
from unittest.mock import patch

import architecture_prebuilt as architecture


class Lease:
    def __init__(self, path):
        self.path = path; self.sha256 = architecture.digest(path)
    def check(self):
        if architecture.digest(self.path) != self.sha256: raise ValueError('generation declaration changed')
    def environment(self, base=None):
        self.check()
        return dict(base or {}, ORYN_GENERATION_GATE=str(self.path), ORYN_GENERATION_GATE_SHA256=self.sha256)


class Fixture:
    def __init__(self, directory):
        self.base = Path(directory).resolve(); self.root = self.base / 'repo'; self.root.mkdir()
        self.inputs = self.base / 'inputs'; self.inputs.mkdir()
        self.identity = 'a' * 64
        self.manifest = json.loads(architecture.MANIFEST.read_text())
        self.fixtures = json.loads(architecture.SHADOW_MANIFEST.read_text())
        def write(path, value):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(value) if isinstance(value, (dict, list)) else value)
            return path
        self.write = write
        for name, relative in architecture.SOURCE_INPUTS.items(): write(self.root / relative, name)
        write(self.root / architecture.SOURCE_INPUTS['manifest'], self.manifest)
        write(self.root / architecture.SOURCE_INPUTS['shadow_manifest'], self.fixtures)
        write(self.root / architecture.SOURCE_INPUTS['historical'],
              {'legacy_commit': architecture.LEGACY_COMMIT, 'harness': architecture.HARNESS})
        for name in architecture.PRODUCER_SOURCES: write(self.root / 'scripts' / name, name)
        for path in ('crates/oryn-native/src/rust_executable.rs', 'crates/oryn-native/src/host.rs',
                     'crates/oryn-js/src/v8_bindings.rs'): write(self.root / path, 'native source')
        self.engines = {}
        for role in ('current', 'legacy'):
            self.engines[role] = {name: architecture.record(write(self.inputs / (role + '-' + name), name))
                                  for name in ('profile', *architecture.ENGINE_FIELDS)}
        self.gate = write(self.inputs / 'gate.json', {})
        self.lease = Lease(self.gate)
        legacy_root = self.inputs / 'legacy-source'
        legacy_sources = {name: architecture.record(write(legacy_root / name,
            (self.root / name).read_text() if name == architecture.HARNESS else name))
            for name in ('Cargo.toml', 'Cargo.lock', architecture.HARNESS)}
        archive = self.inputs / 'legacy.tar'
        with tarfile.open(archive, 'w', format=tarfile.PAX_FORMAT,
                          pax_headers={'comment': architecture.LEGACY_COMMIT}) as source:
            for name in ('Cargo.toml', 'Cargo.lock'):
                contents = name.encode(); member = tarfile.TarInfo(name); member.size = len(contents)
                source.addfile(member, io.BytesIO(contents))
        builds = {}
        for name, command in architecture.BUILD_COMMANDS.items():
            executable = write(self.inputs / name, 'executable ' + name); executable.chmod(0o755)
            builds[name] = {'argv': command, 'returncode': 0, 'executable': architecture.record(executable),
                            'log': architecture.record(write(self.inputs / (name + '.log'), 'built'))}
        self.preparation = {
            'schema_version': 1, 'source_tree_sha256': self.identity,
            'generation_gate': architecture.record(self.gate),
            'sources': {name: architecture.record(self.root / path) for name, path in architecture.SOURCE_INPUTS.items()},
            'producer_sources': {name: architecture.record(self.root / 'scripts' / name) for name in architecture.PRODUCER_SOURCES},
            'engines': self.engines, 'builds': builds, 'legacy_commit': architecture.LEGACY_COMMIT,
            'legacy_source_root': str(legacy_root), 'legacy_sources': legacy_sources,
            'legacy_archive': architecture.record(archive),
            'dependency_graph': {'argv': architecture.TREE_COMMAND, 'returncode': 0,
                'output': architecture.record(write(self.inputs / 'tree.txt', 'oryn-dom\noryn-js\noryn-net\n'))}}
        self.prepared = write(self.inputs / 'architecture-prebuilt.json', self.preparation)
        records = dict(architecture.preparation_artifacts(self.preparation),
                       architecture_prebuilt=architecture.record(self.prepared),
                       engine_profile=self.engines['current']['profile'])
        for name in ('worker', 'runtime'): records[name] = architecture.record(write(self.inputs / ('candidate-' + name), name))
        self.candidate = {'sha256': 'b' * 64, 'record': {'source_tree_sha256': self.identity, 'artifacts': records},
                          'artifacts': {name: Path(item['path']) for name, item in records.items()}}
        self.rows = [{'id': row['id'], 'passed': True} for row in self.fixtures['fixtures']]
        self.registry = {'sha256': 'c' * 64, 'value': {
            'required_profile_ids': ['mac-primary'], 'profiles': {'mac-primary': {'platform': 'macOS'}},
            'non_core_claims': {'architecture': 'mac-primary'}}}

    def patches(self):
        import core_profile_contract
        stack = ExitStack()
        stack.enter_context(patch.object(architecture, 'ROOT', self.root))
        stack.enter_context(patch.object(architecture, 'MANIFEST', self.root / architecture.SOURCE_INPUTS['manifest']))
        stack.enter_context(patch.object(architecture, 'SHADOW_MANIFEST', self.root / architecture.SOURCE_INPUTS['shadow_manifest']))
        stack.enter_context(patch.object(architecture, 'engine_inputs', side_effect=lambda path:
            copy.deepcopy(next(value for value in self.engines.values() if Path(value['profile']['path']) == Path(path)))))
        stack.enter_context(patch.object(architecture, 'source_hash', return_value=self.identity))
        stack.enter_context(patch.object(core_profile_contract, 'validate_registry', return_value=self.registry))
        return stack

    def run(self, argv, **kwargs):
        name = Path(argv[0]).name
        if name not in ('current', 'legacy', 'webidl'): raise AssertionError('unexpected build or command: ' + str(argv))
        if 'CARGO_TARGET_DIR' in kwargs['env']: raise AssertionError('inherited Cargo output escaped')
        if name != 'webidl': kwargs['stdout'].write(json.dumps(self.rows).encode())
        return SimpleNamespace(returncode=0)


class ArchitecturePrebuiltTests(unittest.TestCase):
    def test_final_run_uses_three_frozen_executables_without_build_or_input_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory); before = copy.deepcopy(fixture.candidate['record']['artifacts'])
            with fixture.patches(), patch.dict(os.environ, {'CARGO_TARGET_DIR': str(fixture.inputs)}), \
                    patch.object(architecture.subprocess, 'run', side_effect=fixture.run) as run:
                result = architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                self.assertEqual([Path(call.args[0][0]).name for call in run.call_args_list], ['current', 'legacy', 'webidl'])
                self.assertTrue(result['cutover_ready'])
                self.assertEqual(result['pending_release_attestations'], ['macos-app-sandbox'])
                self.assertEqual(result['outside_declared_scope'], ['linux-namespaces-landlock-seccomp'])
                self.assertEqual(result['cases'], 32)
                for item in before.values(): architecture.exact_record(item)

    def test_transitive_mutation_or_missing_membership_prevents_any_launch(self):
        for mutation in ('executable', 'archive', 'log', 'engine', 'historical', 'legacy_source', 'omit', 'extra'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                paths = {'executable': fixture.preparation['builds']['legacy']['executable'],
                         'archive': fixture.preparation['legacy_archive'],
                         'log': fixture.preparation['builds']['current']['log'],
                         'engine': fixture.engines['current']['archive'],
                         'legacy_source': fixture.preparation['legacy_sources']['Cargo.lock'],
                         'historical': fixture.preparation['sources']['historical']}
                if mutation in paths: Path(paths[mutation]['path']).write_text('changed')
                elif mutation == 'omit': del fixture.candidate['record']['artifacts']['architecture_current']
                else: fixture.candidate['record']['artifacts']['architecture_invented'] = fixture.preparation['legacy_archive']
                with fixture.patches(), patch.object(architecture.subprocess, 'run') as run:
                    with self.assertRaises(ValueError): architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                    run.assert_not_called()

    def test_mutation_during_execution_retains_raw_output_and_stops_next_child(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            def mutate(argv, **kwargs):
                completed = fixture.run(argv, **kwargs)
                Path(fixture.preparation['builds']['current']['log']['path']).write_text('replaced during run')
                return completed
            with fixture.patches(), patch.object(architecture.subprocess, 'run', side_effect=mutate) as run:
                with self.assertRaises(ValueError): architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                self.assertEqual(run.call_count, 1)
                self.assertEqual(json.loads((fixture.base / 'run/current.stdout').read_text()), fixture.rows)
                self.assertFalse((fixture.base / 'run/result.json').exists())

    def test_relabelled_legacy_tree_cannot_retain_pinned_archive_identity(self):
        for mutation in ('substitute_source', 'add_source', 'commit_metadata', 'unsafe_archive'):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                value = fixture.preparation
                if mutation == 'substitute_source':
                    item = value['legacy_sources']['Cargo.lock']
                    Path(item['path']).write_text('different locked source')
                    value['legacy_sources']['Cargo.lock'] = architecture.record(item['path'])
                elif mutation == 'add_source':
                    added = fixture.write(Path(value['legacy_source_root']) / 'added.rs', 'unarchived source')
                    value['legacy_sources']['added.rs'] = architecture.record(added)
                else:
                    path = Path(value['legacy_archive']['path'])
                    comment = 'd' * 40 if mutation == 'commit_metadata' else architecture.LEGACY_COMMIT
                    with tarfile.open(path, 'w', format=tarfile.PAX_FORMAT, pax_headers={'comment': comment}) as source:
                        names = ('Cargo.toml', 'Cargo.lock') if mutation == 'commit_metadata' else ('../escape',)
                        for name in names:
                            contents = name.encode(); member = tarfile.TarInfo(name); member.size = len(contents)
                            source.addfile(member, io.BytesIO(contents))
                    value['legacy_archive'] = architecture.record(path)
                # Even a newly relabelled manifest/map cannot break archive→tree custody.
                fixture.write(fixture.prepared, value)
                records = fixture.candidate['record']['artifacts']
                records.update(architecture.preparation_artifacts(value))
                records['architecture_prebuilt'] = architecture.record(fixture.prepared)
                with fixture.patches(), patch.object(architecture.subprocess, 'run') as run:
                    with self.assertRaisesRegex(ValueError, 'pinned archive|pinned commit|unsafe'):
                        architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                    run.assert_not_called()

    def test_failed_exit_preserves_raw_failure_and_never_accepts_green_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            def fail(argv, **kwargs):
                fixture.run(argv, **kwargs); kwargs['stderr'].write(b'original native error')
                return SimpleNamespace(returncode=2)
            with fixture.patches(), patch.object(architecture.subprocess, 'run', side_effect=fail) as run:
                with self.assertRaisesRegex(ValueError, 'executable failed'):
                    architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                self.assertEqual(run.call_count, 1)
                self.assertEqual((fixture.base / 'run/current.stderr').read_text(), 'original native error')
                self.assertEqual(json.loads((fixture.base / 'run/current.execution.json').read_text())['returncode'], 2)

    def test_shadow_denominator_false_boolean_duplicate_and_order_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory); ids = [row['id'] for row in fixture.rows]
            for rows in (fixture.rows[:-1], fixture.rows[::-1], [fixture.rows[0]] * 4,
                         [dict(row, passed=1) for row in fixture.rows],
                         [dict(row, passed=False) for row in fixture.rows]):
                with self.subTest(rows=rows), self.assertRaises(ValueError): architecture.check_shadow(rows, ids)

    def test_all_declared_platforms_are_required_and_unknown_platform_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory)
            fixture.registry['value']['profiles']['secondary'] = {'platform': 'Linux'}
            fixture.registry['value']['required_profile_ids'].append('secondary')
            with fixture.patches():
                result = architecture.software_result(fixture.candidate, {'current': fixture.rows, 'legacy': fixture.rows})
                self.assertEqual(result['pending_release_attestations'], list(architecture.RELEASE_PLATFORMS))
                self.assertEqual(result['outside_declared_scope'], [])
                fixture.registry['value']['profiles']['secondary']['platform'] = 'Windows'
                with self.assertRaisesRegex(ValueError, 'declared platform'): architecture.scope(fixture.candidate)

    def test_missing_or_unreviewed_scope_and_replaced_generation_prevent_launch(self):
        import core_profile_contract
        for failure in ('scope', 'generation'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                with fixture.patches(), patch.object(architecture.subprocess, 'run') as run:
                    if failure == 'scope':
                        with patch.object(core_profile_contract, 'validate_registry', side_effect=ValueError('scope review rejected')):
                            with self.assertRaisesRegex(ValueError, 'scope review'):
                                architecture.execute(fixture.candidate, fixture.base / 'run', fixture.lease)
                    else:
                        replacement = fixture.write(fixture.inputs / 'replacement-gate.json', {})
                        with self.assertRaisesRegex(ValueError, 'another generation'):
                            architecture.execute(fixture.candidate, fixture.base / 'run', Lease(replacement))
                    run.assert_not_called()

    def test_original_security_custody_is_required_for_macos_and_cannot_close_linux(self):
        import completion_execution
        for declared_linux in (False, True):
            with self.subTest(declared_linux=declared_linux), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(directory)
                if declared_linux:
                    fixture.registry['value']['profiles']['linux'] = {'platform': 'Linux'}
                    fixture.registry['value']['required_profile_ids'].append('linux')
                receipt = fixture.base / 'architecture/receipt.json'; receipt.parent.mkdir()
                raw = receipt.parent / 'raw'; raw.mkdir()
                security_receipt = fixture.base / 'security/receipt.json'; security_receipt.parent.mkdir()
                security_path = security_receipt.parent / 'raw/security-result-v3.json'
                security = {'fingerprint': {'source_tree_sha256': fixture.identity,
                    'worker': {'sha256': fixture.candidate['record']['artifacts']['worker']['sha256']},
                    'parent': {'sha256': fixture.candidate['record']['artifacts']['runtime']['sha256']}},
                    'outcome': {'status': 'passed'}}
                fixture.write(receipt, {}); fixture.write(security_receipt, {}); fixture.write(security_path, security)
                with fixture.patches():
                    result = architecture.software_result(fixture.candidate, {'current': fixture.rows, 'legacy': fixture.rows})
                    fixture.write(raw / 'result.json', result)
                    for name in ('current', 'legacy'): fixture.write(raw / (name + '.stdout'), fixture.rows)
                    for name in ('current', 'legacy', 'webidl'):
                        executable = fixture.preparation['builds'][name]['executable']
                        argv = [executable['path']]
                        if name == 'webidl': argv += ['check', str(fixture.root / architecture.SOURCE_INPUTS['idl']),
                                                     str(fixture.root / architecture.SOURCE_INPUTS['generated'])]
                        profile = fixture.engines['legacy' if name == 'legacy' else 'current']['profile']
                        fixture.write(raw / (name + '.execution.json'), {'argv': argv, 'executable': executable,
                            'engine_profile': profile, 'returncode': 0, 'error': None})
                    with patch.object(completion_execution, 'verify_receipt') as verify:
                        with self.assertRaisesRegex(ValueError, 'qualified security'):
                            architecture.qualify_architecture(receipt, fixture.candidate, None, None)
                        if declared_linux:
                            with self.assertRaisesRegex(ValueError, 'linux-namespaces'):
                                architecture.qualify_architecture(receipt, fixture.candidate, security, security_receipt)
                        else:
                            qualified = architecture.qualify_architecture(receipt, fixture.candidate, security, security_receipt)
                            self.assertTrue(qualified['release_ready']); self.assertEqual(len(qualified['case_ledger']), 32)
                            self.assertIn(unittest.mock.call(security_receipt, 'security', fixture.candidate), verify.call_args_list)
                            changed = copy.deepcopy(security); changed['fingerprint']['worker']['sha256'] = 'e' * 64
                            fixture.write(security_path, changed)
                            with self.assertRaisesRegex(ValueError, 'candidate mismatch'):
                                architecture.qualify_architecture(receipt, fixture.candidate, changed, security_receipt)
                            fixture.write(security_path, security)
                            execution_path = raw / 'legacy.execution.json'
                            executed = json.loads(execution_path.read_text())
                            altered = copy.deepcopy(executed); altered['argv'] = ['/substituted/legacy']
                            fixture.write(execution_path, altered)
                            with self.assertRaisesRegex(ValueError, 'executable custody'):
                                architecture.qualify_architecture(receipt, fixture.candidate, security, security_receipt)
                            fixture.write(execution_path, executed)
                    def reject_security(path, kind, candidate):
                        if kind == 'security': raise ValueError('live signature failed')
                    with patch.object(completion_execution, 'verify_receipt', side_effect=reject_security):
                        with self.assertRaisesRegex(ValueError, 'signature'):
                            architecture.qualify_architecture(receipt, fixture.candidate, security, security_receipt)

    def test_preparation_builds_separate_targets_with_explicit_engine_selection(self):
        spec = importlib.util.spec_from_file_location('architecture_preparation',
            Path(__file__).with_name('prepare-architecture-prebuilt.py'))
        prepare = importlib.util.module_from_spec(spec); spec.loader.exec_module(prepare)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(directory); observed = []
            def run(argv, **kwargs):
                if argv[:2] == ['git', 'archive']:
                    with tarfile.open(fileobj=kwargs['stdout'], mode='w', format=tarfile.PAX_FORMAT,
                                      pax_headers={'comment': architecture.LEGACY_COMMIT}) as archive:
                        for name in ('Cargo.lock', 'Cargo.toml'):
                            item = tarfile.TarInfo(name); item.size = 4
                            archive.addfile(item, io.BytesIO(b'lock'))
                elif argv[:2] == ['cargo', 'build']:
                    target = Path(kwargs['env']['CARGO_TARGET_DIR']); observed.append((target, kwargs['cwd'], kwargs['env']))
                    name = 'debug/oryn-webidl' if argv[-1] == 'oryn-webidl' else 'debug/examples/runtime_shadow'
                    executable = fixture.write(target / name, 'built'); executable.chmod(0o755)
                    kwargs['stdout'].write(b'original successful build')
                elif argv == architecture.TREE_COMMAND: kwargs['stdout'].write(b'oryn-dom\noryn-js\noryn-net\n')
                else: raise AssertionError(argv)
                return SimpleNamespace(returncode=0)
            select = lambda path: copy.deepcopy(next(value for value in fixture.engines.values() if Path(value['profile']['path']) == Path(path)))
            with fixture.patches(), patch.object(prepare, 'ROOT', fixture.root), \
                    patch.object(prepare, 'engine_inputs', side_effect=select), \
                    patch.object(prepare, 'source_hash', return_value=fixture.identity), \
                    patch.object(prepare.subprocess, 'run', side_effect=run):
                output = fixture.base / 'prepared'
                mapping = prepare.prepare(fixture.engines['current']['profile']['path'],
                    fixture.engines['legacy']['profile']['path'], output, fixture.lease)
                self.assertEqual(len({row[0] for row in observed}), 3)
                self.assertEqual(observed[0][1], fixture.root)
                self.assertEqual(observed[1][1], output / 'legacy-source')
                self.assertEqual(observed[1][2]['RUSTY_V8_ARCHIVE'], fixture.engines['legacy']['archive']['path'])
                self.assertEqual(observed[0][2]['RUSTY_V8_ARCHIVE'], fixture.engines['current']['archive']['path'])
                self.assertIn('architecture_legacy_archive', json.loads(mapping.read_text()))


if __name__ == '__main__': unittest.main()

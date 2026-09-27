"""Scope, envelope and independent-ledger regressions. No native processes."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import core_profile_contract as profiles
from core_profile_fixtures import ScopeFixture, write


class CoreProfileContractTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(); self.addCleanup(temporary.cleanup)
        self.fixture = ScopeFixture(temporary.name)
        admission = self.fixture.admitted(); admission.start(); self.addCleanup(admission.stop)

    def test_explicit_three_profile_scope_preserves_both_pairs_without_cartesian_expansion(self):
        value = profiles.validate_registry(self.fixture.candidate, required=True)
        self.assertEqual(value['value']['required_profile_ids'], ['primary', 'allow', 'legacy'])
        self.assertEqual(len(value['fixtures']), 3)
        self.assertEqual(profiles.validate_prerequisite_equivalence(self.fixture.candidate)['profile_ids'], ['primary', 'allow', 'legacy'])

    def test_missing_pair_duplicate_profile_and_unknown_selector_reject_even_after_scope_review(self):
        original = copy.deepcopy(self.fixture.registry)
        mutations = [lambda r:r['required_pairs'].pop('features'),
                     lambda r:r['required_profile_ids'].append('primary'),
                     lambda r:r['required_pairs'].update(cookies=['primary', 'legacy']),
                     lambda r:r['profiles']['primary']['execution_policy'].update(url='*'),
                     lambda r:r.update(core_total=16736),
                     lambda r:r['non_core_claims'].update(workflows='legacy')]
        for mutate in mutations:
            self.fixture.registry = copy.deepcopy(original); mutate(self.fixture.registry); self.fixture.save()
            with self.assertRaises(ValueError): profiles.validate_registry(self.fixture.candidate, required=True)

    def test_scope_review_binds_exact_semantics_and_independent_reviewer(self):
        self.fixture.registry['required_profile_ids'].reverse(); self.fixture.save(review=False)
        with self.assertRaisesRegex(ValueError, 'exact registry selection'):
            profiles.validate_registry(self.fixture.candidate)
        self.fixture.registry['scope_review']['reviewer'] = ' TEST-AUTHOR '
        self.fixture.save()
        with self.assertRaisesRegex(ValueError, 'independent reviewer'):
            profiles.validate_registry(self.fixture.candidate)

    def test_pair_cannot_change_devices_tls_or_other_environment(self):
        self.fixture.change_fixture('allow', 'media_devices', 'none')
        with self.assertRaisesRegex(ValueError, 'another fixture setting'):
            profiles.validate_registry(self.fixture.candidate)

    def test_primary_and_required_profile_cannot_be_substituted(self):
        self.fixture.candidate['record']['artifacts']['fixture_contract'] = self.fixture.registry['profiles']['allow']['fixture']
        with self.assertRaisesRegex(ValueError, 'primary'):
            profiles.validate_registry(self.fixture.candidate)

    def test_non_core_primary_is_bound_to_executed_standard_block_defaults(self):
        self.fixture.registry['primary_profile_id'] = 'allow'
        self.fixture.registry['non_core_claims'] = {name:'allow' for name in profiles.NON_CORE}
        self.fixture.candidate['record']['artifacts']['fixture_contract'] = self.fixture.registry['profiles']['allow']['fixture']
        self.fixture.save()
        with self.assertRaisesRegex(ValueError,'source-owned non-Core'):
            profiles.validate_registry(self.fixture.candidate)

    def test_registry_history_keeps_transitive_review_inputs(self):
        root=self.fixture.root
        # History is retained as independent immutable files before a revision.
        previous=copy.deepcopy(self.fixture.registry)
        review=write(root/'revision-one-review.json',json.loads((root/'scope-review.json').read_text()))
        previous['scope_review']['evidence']=review
        # A historical revision's own evidence record is retained, even though
        # current admission is determined by the independently reviewed head.
        prior=write(root/'revision-one.json',previous)
        self.fixture.registry.update(revision=2,previous_registry=prior);self.fixture.save()
        inputs=profiles.validate_registry(self.fixture.candidate)['inputs']
        self.assertTrue({prior['path'],review['path']} <= {record['path'] for record in inputs})
        Path(review['path']).write_text('{}')
        with self.assertRaises(ValueError):profiles.validate_registry(self.fixture.candidate)

    def test_two_envelopes_share_exact_candidate_with_distinct_profile_identity(self):
        _, primary = self.fixture.envelope(); _, other = self.fixture.envelope('allow')
        self.assertEqual(primary['candidate_sha256'], other['candidate_sha256'])
        self.assertNotEqual(primary['profile_sha256'], other['profile_sha256'])
        self.assertEqual(profiles.identity(primary, 'core')['collection_id'], 'original-core')

    def test_envelope_relabel_duplicate_collection_and_overlapping_outputs_reject(self):
        path, bound = self.fixture.envelope(); original = json.loads(path.read_text())
        mutations = [lambda r:r.update(profile_id='legacy'), lambda r:r.update(core_profile_registry_sha256='a'*64),
                     lambda r:r['collection_ids'].update(canaries=r['collection_ids']['core']),
                     lambda r:r['outputs'].update(core=r['prepared']), lambda r:r.update(attempt_id='*')]
        for mutate in mutations:
            value = copy.deepcopy(original); mutate(value); write(path, value)
            with self.assertRaises(ValueError): profiles.resolve_envelope(path, self.fixture.candidate)
        write(path, original)
        self.fixture.candidate['path'].write_text('changed candidate bytes')
        with self.assertRaises(ValueError): profiles.resolve_envelope(path, self.fixture.candidate)

    def test_recursive_inputs_preserve_asserted_hashes_and_nested_source_maps(self):
        root = self.fixture.root; checkout = root/'checkout'; checkout.mkdir()
        source = checkout/'helper.js'; source.write_text('original helper')
        leaf = root/'leaf'; leaf.write_text('original evidence')
        middle = root/'middle-no-extension'
        record = write(middle, {'evidence_artifacts': [profiles.reference(leaf)], 'source_files': {'helper.js': profiles.digest(source)}})
        inputs = profiles.record_inputs(record, checkout=checkout)
        self.assertEqual({row['path'] for row in inputs}, {str(middle), str(leaf), str(source)})
        before = profiles.digest
        changed = False
        def mutate_after_hash(path):
            nonlocal changed
            result = before(path)
            if Path(path) == middle and not changed:
                changed = True; middle.write_text('{}')
            return result
        # Hash assertion is verified by read_record, then the returned identity
        # must retain it rather than accepting newly observed bytes.
        import completion_provenance
        with patch.object(completion_provenance, 'digest', side_effect=mutate_after_hash):
            with self.assertRaisesRegex(ValueError, 'missing or changed'):
                profiles.record_inputs(record, checkout=checkout)

    def test_recursive_source_map_rejects_checkout_escape(self):
        with self.assertRaisesRegex(ValueError, 'escapes pinned checkout'):
            profiles.record_inputs({'source_files': {'../outside.js': 'a'*64}}, checkout=self.fixture.root)

    def test_registry_detects_nested_tls_change_before_return(self):
        material = self.fixture.fixture_root/'tls/ca.pem'; material.write_text('changed TLS input')
        with self.assertRaises(ValueError): profiles.validate_registry(self.fixture.candidate)

    def test_inventory_binding_cannot_replace_manifest_policy_or_coverage(self):
        file = self.fixture.root/'wrong'; file.write_text('shortened inventory')
        registry = profiles.validate_registry(self.fixture.candidate)
        with self.assertRaisesRegex(ValueError, 'inventory binding differs'):
            profiles.validate_inventory_binding(registry, file, file, file)

    def test_full_ledgers_keep_distinct_raw_and_disposition_states(self):
        _, envelope = self.fixture.envelope(); context = profiles.identity(envelope, 'core')
        rows = [{'id': str(index), 'state': 'qualified_pass', 'obligation_state': 'qualified_pass',
                 'qualified_pass': True, 'raw_pass': index < 15985} for index in range(16737)]
        summary = dict(context, schema_version=2, core_total=16737, automatic_total=15985, assessment_total=752,
            counts={'qualified_pass':16737}, complete=True, raw_pass_count=15985, qualified_pass_count=16737,
            reviewed_disposition_count=0, open_count=0)
        ids = {row['id'] for row in rows}
        profiles.validate_profile_ledger(summary, rows, ids, context)
        rows[0].update(state='satisfied_by_reviewed_disposition', obligation_state='satisfied_by_reviewed_disposition',
                       qualified_pass=False, raw_pass=False)
        summary.update(counts={'qualified_pass':16736,'satisfied_by_reviewed_disposition':1}, raw_pass_count=15984,
                       qualified_pass_count=16736,reviewed_disposition_count=1)
        result = profiles.validate_profile_ledger(summary, rows, ids, context)
        self.assertEqual(result['reviewed_disposition_count'], 1)
        rows[0]['raw_pass'] = True
        with self.assertRaisesRegex(ValueError,'relabel raw failure'):
            profiles.validate_profile_ledger(summary,rows,ids,context)

    def test_complementary_profiles_missing_and_duplicate_rows_cannot_form_union(self):
        _, envelope = self.fixture.envelope(); context = profiles.identity(envelope, 'core')
        rows = [{'id': str(index), 'state': 'qualified_pass', 'obligation_state': 'qualified_pass',
                 'qualified_pass': True, 'raw_pass': index < 15985} for index in range(16737)]
        summary = dict(context,schema_version=2,core_total=16737,automatic_total=15985,assessment_total=752,
            counts={'qualified_pass':16737},complete=True,raw_pass_count=15985,qualified_pass_count=16737,
            reviewed_disposition_count=0,open_count=0)
        ids = {row['id'] for row in rows}
        for broken in (rows[:-1], rows[:-1]+[rows[0]]):
            with self.assertRaisesRegex(ValueError,'16,737'):
                profiles.validate_profile_ledger(summary,broken,ids,context)
        for index in (0,1):
            broken = copy.deepcopy(rows); broken[index].update(state='open',obligation_state='open',qualified_pass=False,raw_pass=False)
            with self.assertRaises(ValueError): profiles.validate_profile_ledger(summary,broken,ids,context)


class CoreProfilePlatformTests(unittest.TestCase):
    def test_actual_pinned_mozinfo_apple_silicon_maps_to_registry_without_changing_raw_info(self):
        import importlib.util
        import runpy
        # Execute the installed upstream dependency with the actual host's
        # reported Darwin tuple, not a replacement metadata implementation.
        source = importlib.util.find_spec('mozinfo.mozinfo').origin
        with patch('platform.uname', return_value=('Darwin', 'test-host', '26.0.0',
                                                   'Darwin Kernel Version', 'arm64', 'arm')), \
             patch('platform.architecture', return_value=('64bit', '')), \
             patch('platform.mac_ver', return_value=('27.0', ('', '', ''), 'arm64')):
            raw = runpy.run_path(source, run_name='mozinfo._oryn_platform_control')['info']
        self.assertEqual(raw['processor'], 'arm')
        self.assertEqual(raw['bits'], 64)
        self.assertIs(raw['apple_silicon'], True)
        before = copy.deepcopy(raw)
        profiles.verify_platform(raw, {'platform': 'macOS', 'processor': 'aarch64'})
        self.assertEqual(raw, before)

    def test_generic_arm_requires_strict_64bit_apple_silicon_and_matching_platform(self):
        actual = {'os': 'mac', 'processor': 'arm', 'bits': 64, 'apple_silicon': True}
        cases = [dict(actual, bits=value) for value in (32, None, '64', 64.0, True)]
        cases += [dict(actual, apple_silicon=value) for value in (False, None, 1, 'true')]
        cases += [dict(actual, os='linux'), dict(actual, os='win'),
                  {key: value for key, value in actual.items() if key != 'bits'},
                  {key: value for key, value in actual.items() if key != 'apple_silicon'}]
        for raw in cases:
            with self.subTest(raw=raw), self.assertRaisesRegex(ValueError, 'platform differs'):
                profiles.verify_platform(raw, {'platform': 'macOS', 'processor': 'aarch64'})
        for selected in ({'platform': 'Linux', 'processor': 'aarch64'},
                         {'platform': 'macOS', 'processor': 'x86_64'}):
            with self.subTest(selected=selected), self.assertRaises(ValueError):
                profiles.verify_platform(actual, selected)

    def test_existing_explicit_processor_aliases_preserve_exact_platform_checks(self):
        for raw, selected in [({'os': 'mac', 'processor': 'arm64'},
                               {'platform': 'macOS', 'processor': 'aarch64'}),
                              ({'os': 'mac', 'processor': 'aarch64'},
                               {'platform': 'macOS', 'processor': 'aarch64'}),
                              ({'os': 'win', 'processor': 'AMD64'},
                               {'platform': 'Windows', 'processor': 'x86_64'})]:
            profiles.verify_platform(raw, selected)
            with self.assertRaisesRegex(ValueError, 'platform differs'):
                profiles.verify_platform(dict(raw, os='unknown'), selected)


if __name__ == '__main__': unittest.main()

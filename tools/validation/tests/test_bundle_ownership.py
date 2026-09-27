"""Published assembly layout binds to one exact registered build ancestor."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from oryn_validation.bundle_ownership import registered_bundle_owner
from oryn_validation.inputs import snapshot_input
from oryn_validation.manifests import manifest_digest
from test_manifests import build


class BundleOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.published = self.root / 'builds/build-1/published'
        self.bundle = self.published / 'runtime'
        (self.bundle / 'bin').mkdir(parents=True)
        (self.bundle / 'bin/oryn').write_text('original runtime')
        self.native = build()
        self.native['artifacts'] = [snapshot_input(self.bundle, 'bin/oryn', scope='bundle')]
        self.manifest = self.published / 'build.json'
        self.manifest.write_text(json.dumps(self.native))
        self.records = {'runtime': self.native['artifacts'][0],
                        'build_manifest': snapshot_input(self.root, 'builds/build-1/published/build.json')}
        self.bound = {'store_root': self.root, 'bundle_root': self.bundle,
                      'run': {'build': {'build_id': 'build-1', 'manifest_sha256': manifest_digest(self.native)},
                              'assets': copy.deepcopy(self.records)},
                      'assets': {'runtime': self.bundle / 'bin/oryn', 'build_manifest': self.manifest}}
        self.index = {'schema_version': 1, 'leases': [], 'objects': {'build:build-1': self.owner('builds/build-1')}}
        self.save()

    def owner(self, path, kind='build'):
        return {'path': path, 'kind': kind, 'dependencies': [], 'pinned': True, 'last_used_ns': 1}

    def save(self):
        (self.root / 'retention.json').write_text(json.dumps(self.index))

    def test_published_assembly_reuses_registered_build_ancestor(self):
        from oryn_validation.assembly import _pin_output
        self.assertEqual(_pin_output(self.root, 'builds/build-1/published', 'build-1', []), 'build:build-1')
        self.assertEqual(registered_bundle_owner(self.bound), 'build:build-1')
        # A newly registered publication also works without a preexisting ancestor.
        self.index['objects'] = {'published-build:build-1': self.owner('builds/build-1/published')}
        self.save()
        self.assertEqual(registered_bundle_owner(self.bound), 'published-build:build-1')

    def test_overlapping_unrelated_or_wrong_kind_owner_is_rejected(self):
        for objects in (
            {'broad': self.owner('builds')},
            {'cache': self.owner('builds/build-1', kind='cargo-cache')},
            {'one': self.owner('builds/build-1'), 'two': self.owner('builds/build-1/published')},
            {'other': self.owner('builds/another-build')},
        ):
            self.index['objects'] = objects; self.save()
            with self.subTest(objects=objects), self.assertRaises(ValueError):
                registered_bundle_owner(self.bound)

    def test_build_and_bundle_asset_substitutions_are_rejected(self):
        for mutation in ('id', 'digest', 'record', 'path', 'manifest-location'):
            bound = copy.deepcopy(self.bound)
            if mutation == 'id':
                bound['run']['build']['build_id'] = 'different'
            elif mutation == 'digest':
                bound['run']['build']['manifest_sha256'] = '0' * 64
            elif mutation == 'record':
                bound['run']['assets']['runtime']['sha256'] = '0' * 64
            elif mutation == 'path':
                bound['assets']['runtime'] = self.root / 'other-runtime'
            else:
                bound['bundle_root'] = self.root / 'builds/build-1/runtime'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                registered_bundle_owner(bound)

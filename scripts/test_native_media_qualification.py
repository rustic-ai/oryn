"""Qualification negatives use captured-output fixtures, never native execution."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import native_media_qualification as media
from native_protocol_versions import versions


class NativeMediaQualificationTests(unittest.TestCase):
    def output(self, identity=None, passed=1, ignored=0):
        constants = versions()
        build = identity or dict(media_source_sha256='a'*64, compiled_protocols=dict(
            worker_protocol_version=constants['worker'], context_service_protocol_version=constants['context'],
            agent_ipc_protocol_version=constants['agent']))
        return ('running 1 test\ntest ' + media.TEST + ' ... \nORYN_MEDIA_BUILD ' + json.dumps(build)
                + '\nok\ntest result: ok. ' + str(passed) + ' passed; 0 failed; ' + str(ignored) + ' ignored; 0 measured; 22 filtered out\n')

    def test_actual_named_single_test_output_is_required(self):
        media.validate_output(self.output(), 'a'*64)
        for text in (self.output(passed=0), self.output(ignored=1), self.output().replace(media.TEST, 'another_test'),
                     self.output().replace('ORYN_MEDIA_BUILD ', ''), self.output()+self.output()):
            with self.subTest(text=text), self.assertRaises(ValueError):
                media.validate_output(text, 'a'*64)

    def test_missing_mismatched_and_boolean_protocols_are_rejected(self):
        good = media.validate_output(self.output(), 'a'*64)
        for field in good['compiled_protocols']:
            for replacement in (None, True, 999999):
                bad = json.loads(json.dumps(good)); bad['compiled_protocols'][field] = replacement
                with self.subTest(field=field, replacement=replacement), self.assertRaises(ValueError):
                    media.validate_output(self.output(bad), 'a'*64)
        with self.assertRaisesRegex(ValueError, 'another media source'):
            media.validate_output(self.output(), 'b'*64)

    def test_generation_barrier_precedes_candidate_or_process_access(self):
        with patch.object(media, 'generation_gate', side_effect=ValueError('generation open')), \
             patch.object(media, 'load_candidate') as candidate, patch.object(media, 'run_owned') as run:
            with self.assertRaisesRegex(ValueError, 'generation open'):
                media.qualify('candidate', 'gate', 'output')
            candidate.assert_not_called(); run.assert_not_called()

    def test_fixture_revision_and_blob_mutation_are_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); contract = root/'contract.json'
            contract.write_text(json.dumps(dict(upstream_checkout=str(root))))
            bound = dict(artifacts=dict(native_controls_contract=contract))
            with patch.object(media.subprocess, 'check_output', return_value='wrong-revision\n'):
                with self.assertRaisesRegex(ValueError, 'pinned WPT'):
                    media.fixture_inputs(bound)
            for name in media.FIXTURES:
                path=root/name; path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'changed')
            with patch.object(media.subprocess, 'check_output', side_effect=[media.REVISION+'\n', b'original']):
                with self.assertRaisesRegex(ValueError, 'pinned Git blob'):
                    media.fixture_inputs(bound)

    def test_build_digest_binds_paths_and_content(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); base=root/'crates/oryn-native'; (base/'src/media_capture').mkdir(parents=True)
            for name in ('../../Cargo.toml','../../Cargo.lock','build.rs','Cargo.toml','src/media_capture.rs','src/media_capture/webm.rs'):
                (base/name).write_text(name)
            vendor=root/"vendor/libvpx"; vendor.mkdir(parents=True); (vendor/"LICENSE").write_text("license"); (vendor/"decoder.c").write_text("decoder")
            before=media.media_source_hash(root)
            (base/'src/media_capture/webm.rs').write_text('changed')
            self.assertNotEqual(before, media.media_source_hash(root))
            before_vendor=media.media_source_hash(root); (vendor/"decoder.c").write_text("changed decoder"); self.assertNotEqual(before_vendor,media.media_source_hash(root))
            after=media.media_source_hash(root)
            (base/'src/media_capture/webm.rs').rename(base/'src/media_capture/renamed.rs')
            self.assertNotEqual(after, media.media_source_hash(root))


if __name__ == '__main__':
    unittest.main()

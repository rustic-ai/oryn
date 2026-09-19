"""Exact manual source/links and platform branch custody; no observations created."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import wpt_assessment_packet as packet


class AssessmentPacketTests(unittest.TestCase):
    def test_packet_preserves_markup_and_rejects_candidate_or_source_substitution(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = root / 'manual.html'
            document.write_text('<p>Download <a href="file.txt">file</a>, then delete it.</p>')
            corpus = root / 'procedures.jsonl'
            record = {'id': 'manual:/manual.html', 'path': 'manual.html', 'source_sha256': packet.digest(document),
                      'upstream_revision': packet.REVISION, 'qualification': None,
                      'procedure': ['Download file'], 'expected_observations': ['NOT_FOUND_ERR'],
                      'required_platforms': [{'platform': 'macOS native Oryn'}], 'cleanup': ['close']}
            corpus.write_text(json.dumps(record))
            with patch.object(packet, 'DEFAULT_CORPUS', corpus):
                result = packet.create(record['id'], 'a'*64, root, 'macOS')
                self.assertIn('href="file.txt"', result['source']['text'])
                self.assertIsNone(result['observations']); self.assertFalse(result['qualified'])
                output = root / 'packet.json'; output.write_text(json.dumps(result))
                binding = {'path': str(output), 'sha256': packet.digest(output)}
                assessment = {'candidate_sha256': 'a'*64, 'assessment_packet': binding,
                              'artifacts': [binding], 'platform_selection': {'platform': 'macOS', 'api': None}}
                self.assertEqual(packet.validate(assessment, record['id'], root), result)
                assessment['candidate_sha256'] = 'b'*64
                with self.assertRaises(ValueError): packet.validate(assessment, record['id'], root)
                assessment['candidate_sha256'] = 'a'*64
                document.write_text('different source')
                with self.assertRaises(ValueError): packet.validate(assessment, record['id'], root)

    def test_foreign_api_and_missing_native_assertions_fail_before_review(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / 'a'; source.write_text('source')
            record = {'id': 'manual:/a', 'path': 'a', 'source_sha256': packet.digest(source),
                      'upstream_revision': packet.REVISION, 'qualification': None, 'atta': {'steps': []},
                      'required_platforms': [{'api': 'AXAPI', 'platform': 'macOS'}]}
            corpus = root / 'corpus'; corpus.write_text(json.dumps(record))
            for api in ('ATK', 'AXAPI'):
                with self.assertRaises(ValueError): packet.create('manual:/a', 'a'*64, root, 'macOS', api, corpus)


class ProfileAssessmentPacketTests(unittest.TestCase):
    def test_profile_and_original_envelope_are_part_of_packet_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);document=root/'manual.html';document.write_text('<p>Observe native output.</p>')
            corpus=root/'procedures.jsonl';record=dict(id='manual:/manual.html',path='manual.html',
                source_sha256=packet.digest(document),upstream_revision=packet.REVISION,qualification=None,
                procedure=['Observe'],expected_observations=['Native output'],required_platforms=[{'platform':'macOS'}],cleanup=['close'])
            corpus.write_text(json.dumps(record))
            identity=dict(candidate_sha256='a'*64,registry_sha256='b'*64,profile_id='baseline',profile_sha256='c'*64,
                envelope_sha256='d'*64,run_id='original',collection_id='core',attempt_id='attempt-1',original_run_id='original',platform='macOS')
            with patch.object(packet,'DEFAULT_CORPUS',corpus):
                result=packet.create(record['id'],'a'*64,root,'macOS',profile_context=identity)
                self.assertEqual(result['schema_version'],2);self.assertEqual(result['profile_identity'],identity)
                output=root/'packet.json';output.write_text(json.dumps(result));binding={'path':str(output),'sha256':packet.digest(output)}
                assessment=dict(candidate_sha256='a'*64,profile_identity=identity.copy(),assessment_packet=binding,
                                artifacts=[binding],platform_selection={'platform':'macOS','api':None})
                self.assertEqual(packet.validate(assessment,record['id'],root),result)
                for key in ('profile_id','profile_sha256','envelope_sha256','attempt_id','registry_sha256'):
                    changed=identity.copy();changed[key]='f'*64 if key.endswith('sha256') else 'foreign'
                    assessment['profile_identity']=changed
                    with self.subTest(key=key),self.assertRaises(ValueError):packet.validate(assessment,record['id'],root)
                with self.assertRaisesRegex(ValueError,'platform'):
                    packet.create(record['id'],'a'*64,root,'Linux',profile_context=identity)

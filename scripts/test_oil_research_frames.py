"""Actual frame fixture and durable response-fault regression source."""
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import threading
import unittest

ROOT=Path(__file__).resolve().parents[1]
def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
fixture=load('research_v3_test',ROOT/'test-harness/workflows/research_fixture_v3.py')

class FrameFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.path=Path(self.temp.name)/'research.sqlite3'
        self.fixture=fixture.Fixture(self.path);self.fixture.origin='http://127.0.0.1:41001';self.fixture.foreign_origin='http://127.0.0.1:41002'
        self.case='a'*32;self.fixture.command(dict(command='create',case_id=self.case))
    def tearDown(self):self.fixture.db.close();self.temp.cleanup()
    def test_frame_shells_use_real_distinct_origin_and_nested_native_documents(self):
        same=self.fixture.get(self.case,'frame-same',{})[1]
        foreign=self.fixture.get(self.case,'frame-foreign',{})[1]
        self.assertIn('http://127.0.0.1:41002/research/',same)
        self.assertIn('http://127.0.0.1:41001/research/',foreign)
        self.assertIn('parent.document.body',foreign)
        self.assertNotIn('srcdoc',same+foreign)
        self.assertEqual([v['path'] for v in self.fixture.command(dict(command='snapshot',case_id=self.case))['visits']],['frame-same','frame-foreign'])
    def test_unknown_case_never_receives_foreign_facts(self):
        self.assertEqual(self.fixture.get('b'*32,'frame-foreign',{})[0],404)
    def test_armed_fault_without_request_is_not_injection(self):
        self.fixture.command(dict(command='arm_response_fault',case_id=self.case,token='one',route='api',delay_ms=1))
        row=self.fixture.response_faults.snapshot(self.case)[0]
        self.assertEqual(row['terminal'],'armed');self.assertIsNone(row['started_ns'])
        self.fixture.get(self.case,'frame-same',{})
        self.assertEqual(self.fixture.response_faults.snapshot(self.case)[0]['terminal'],'armed')
    def test_delay_records_only_the_actual_matching_response_once(self):
        self.fixture.command(dict(command='arm_response_fault',case_id=self.case,token='one',route='frame-nested',delay_ms=1))
        self.fixture.get(self.case,'frame-nested',{})
        first=self.fixture.response_faults.snapshot(self.case)[0]
        self.assertEqual(first['terminal'],'released');self.assertGreater(first['released_ns'],first['started_ns'])
        self.fixture.get(self.case,'frame-nested',{})
        self.assertEqual(self.fixture.response_faults.snapshot(self.case)[0],first)
    def test_duplicate_fault_token_and_unknown_case_are_rejected(self):
        with self.assertRaises(ValueError):self.fixture.command(dict(command='arm_response_fault',case_id='b'*32,token='one',route='api',delay_ms=1))
        self.fixture.response_faults.arm(self.case,'one','api',1)
        with self.assertRaises(ValueError):self.fixture.response_faults.arm(self.case,'two','api',1)

    def test_response_fault_gate_skips_rejections_and_exact_prior_reads(self):
        faults=self.fixture.response_faults
        faults.arm(self.case,'gated','frame-nested',1,expected_status=200,skip_matches=1)
        faults.before_response(self.case,'frame-nested',404)
        faults.before_response(self.case,'frame-nested',200)
        self.assertEqual(faults.snapshot(self.case)[0]['terminal'],'armed')
        faults.before_response(self.case,'frame-nested',200)
        self.assertEqual(faults.snapshot(self.case)[0]['terminal'],'released')

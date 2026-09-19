"""Source regressions for sealed forms receipts and native upload byte fidelity."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('forms_durable_test',ROOT/'test-harness/workflows/forms_fixture_v3.py')
fixture=importlib.util.module_from_spec(spec);spec.loader.exec_module(fixture)

class DurableFormsTests(unittest.TestCase):
    def test_validation_and_receipt_view_are_persisted_before_snapshot_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            database=Path(directory)/'forms.sqlite3';server=fixture.Fixture(database);case='a'*32
            server.command(dict(command='create',case_id=case,upload=False))
            server.validate_email(case,'invalid@example.test')
            self.assertEqual(fixture.snapshot(database,case)['validations'],[dict(email='invalid@example.test',valid=False)])
            with server.lock:server.receipt_view(case)
            self.assertEqual(fixture.snapshot(database,case)['receipt_views'],1)
    def test_rejected_body_is_durable_and_does_not_create_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            database=Path(directory)/'forms.sqlite3';server=fixture.Fixture(database);case='a'*32
            server.command(dict(command='create',case_id=case,upload=False))
            status=server.receive(case,'application/x-www-form-urlencoded',b'case_id=wrong')
            data=fixture.snapshot(database,case)
            self.assertNotEqual(status,303);self.assertEqual(len(data['requests']),1);self.assertEqual(data['receipts'],[])
    def test_unknown_case_cannot_read_or_create_durable_record(self):
        with tempfile.TemporaryDirectory() as directory:
            database=Path(directory)/'forms.sqlite3';server=fixture.Fixture(database)
            self.assertEqual(server.receive('b'*32,'application/x-www-form-urlencoded',b''),404)
            with self.assertRaises(ValueError):fixture.snapshot(database,'b'*32)

"""Actual decoder, method/submitter authority and durable one-write controls."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('extended_forms_test',ROOT/'test-harness/workflows/forms_fixture_v4.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)

class ExtendedFormsTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.db=Path(self.temp.name)/'forms.sqlite3';self.fixture=module.Fixture(self.db)
        self.case='a'*32;self.fixture.command(dict(command='create',case_id=self.case,scenario='post'));self.row=self.fixture.row(self.case)
    def tearDown(self):self.temp.cleanup()
    def fields(self):return [('name','Zoë Test'),('message','Résumé ✓'),('duplicate','first'),('duplicate','second'),('nonce',self.row['nonce']),('intent','send')]
    def test_method_path_order_unicode_and_request_hash_are_durable(self):
        body=urlencode(self.fields()).encode()
        self.assertEqual(self.fixture.submit(self.case,'override','GET','application/x-www-form-urlencoded',body,self.row['session']),303)
        data=module.snapshot(self.db,self.case);request=data['requests'][0]
        self.assertEqual((request['method'],request['path']),('GET','override'))
        self.assertEqual(request['body_sha256'],hashlib.sha256(body).hexdigest())
        self.assertEqual(request['fields'][:4],[list(pair) for pair in self.fields()[:4]])
        self.assertNotIn(self.row['nonce'],str(data));self.assertNotIn(self.row['session'],str(data))
    def test_native_reentrant_or_duplicate_delivery_has_one_durable_receipt(self):
        body=urlencode(self.fields()).encode()
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','application/x-www-form-urlencoded',body,self.row['session']),303)
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','application/x-www-form-urlencoded',body,self.row['session']),409)
        data=module.snapshot(self.db,self.case)
        self.assertEqual([row['status'] for row in data['requests']],[303,409]);self.assertEqual(len(data['receipts']),1)
    def test_foreign_cookie_and_nonce_cannot_create_receipt(self):
        body=urlencode(self.fields()).encode()
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','application/x-www-form-urlencoded',body,'foreign'),403)
        changed=urlencode([(name,'foreign' if name=='nonce' else value) for name,value in self.fields()]).encode()
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','application/x-www-form-urlencoded',changed,self.row['session']),403)
        self.assertEqual(module.snapshot(self.db,self.case)['receipts'],[])
    def test_multipart_uses_real_authorized_bytes_and_omits_disabled_controls(self):
        payload=(ROOT/'test-harness/workflows/authorized-document.txt').read_bytes();boundary='native-source-regression'
        parts=[]
        for name,value in self.fields():parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="document"; filename="authorized-document.txt"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()+payload+b'\r\n')
        body=b''.join(parts)+f'--{boundary}--\r\n'.encode()
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','multipart/form-data; boundary='+boundary,body,self.row['session']),303)
        receipt=module.snapshot(self.db,self.case)['receipts'][0]
        self.assertEqual(receipt['files'][0]['sha256'],hashlib.sha256(payload).hexdigest())
        self.assertNotIn('disabled',[name for name,_ in receipt['fields']])
    def test_submitter_and_reentrant_frontends_use_native_form_algorithms(self):
        markup=module.page(self.case,dict(self.row,scenario='submitter'))
        self.assertIn('formmethod="get"',markup);self.assertIn('formaction=',markup)
        markup=module.page(self.case,dict(self.row,scenario='reentrant'))
        self.assertIn('requestSubmit',markup);self.assertIn('preventDefault',markup)
        self.assertNotIn('fetch(',markup)

    def test_face_frontend_uses_native_file_snapshot_and_durable_decoder(self):
        markup=module.page(self.case,dict(self.row,scenario='face'))
        self.assertIn('static formAssociated=true',markup)
        self.assertIn("setFormValue(entries)",markup)
        self.assertIn("entries.delete('document')",markup)
        self.assertNotIn('fetch(',markup)
        payload=(ROOT/'test-harness/workflows/authorized-document.txt').read_bytes()
        boundary='face-native-receipt'
        fields=self.fields()[:-1]+[('face_value','Résumé ✓'),('face_duplicate','first'),('face_duplicate','second')]+self.fields()[-1:]
        parts=[f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode() for name,value in fields]
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="document"; filename="authorized-document.txt"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()+payload+b'\r\n')
        body=b''.join(parts)+f'--{boundary}--\r\n'.encode()
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','multipart/form-data; boundary='+boundary,body,'foreign'),403)
        self.assertEqual(module.snapshot(self.db,self.case)['receipts'],[])
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','multipart/form-data; boundary='+boundary,body,self.row['session']),303)
        # A new Fixture reader uses the durable file, not browser-visible text.
        receipt=module.snapshot(self.db,self.case)['receipts'][0]
        self.assertEqual(receipt['fields'][5:8],[['face_value','Résumé ✓'],['face_duplicate','first'],['face_duplicate','second']])
        self.assertEqual(receipt['files'][0]['sha256'],hashlib.sha256(payload).hexdigest())
        self.assertEqual(self.fixture.submit(self.case,'submit','POST','multipart/form-data; boundary='+boundary,body,self.row['session']),409)
        self.assertEqual(len(module.snapshot(self.db,self.case)['receipts']),1)

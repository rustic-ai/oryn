"""v2 form frontend and byte-preserving native receiver controls."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('forms_frontend', ROOT/'test-harness/workflows/forms_fixture_v2.py')
frontend = importlib.util.module_from_spec(spec)
spec.loader.exec_module(frontend)


class FormsFrontendTests(unittest.TestCase):
    def test_async_validation_has_case_isolation_and_exact_history(self):
        fixture = frontend.Fixture()
        first, second = 'a'*32, 'b'*32
        fixture.command(dict(command='create', case_id=first))
        fixture.command(dict(command='create', case_id=second))
        self.assertEqual(fixture.validate_email(first, 'blocked@example.test'), (200, {'valid': False}))
        self.assertEqual(fixture.validate_email(first, 'zoe@example.test'), (200, {'valid': True}))
        self.assertEqual(fixture.command(dict(command='snapshot', case_id=second))['validations'], [])
        self.assertEqual(fixture.validate_email('c'*32, 'zoe@example.test')[0], 404)
        self.assertEqual(fixture.command(dict(command='snapshot', case_id=first))['receipts'], [])

    def test_frontend_retains_native_form_encoding_upload_and_successful_control_order(self):
        template = (ROOT/'test-harness/workflows/forms.html').read_text()
        rendered = frontend.render(template, 'a'*32)
        self.assertIn('method="post"', rendered)
        self.assertIn('enctype="__ENCODING__"', rendered)
        self.assertIn('__UPLOAD_CONTROL__', rendered)
        self.assertIn('name="disabled_field" value="must-not-be-submitted" disabled', rendered)
        self.assertLess(rendered.index('name="name"'), rendered.index('name="role"'))
        self.assertLess(rendered.index('name="role"'), rendered.index('name="message"'))
        self.assertIn('id="role-picker"', rendered)
        self.assertIn('id="details-step" hidden', rendered)

    def test_decoder_preserves_duplicate_order_unicode_and_raw_file_hash(self):
        import hashlib
        body = (b'--boundary\r\nContent-Disposition: form-data; name="skill"\r\n\r\nrust\r\n'
                b'--boundary\r\nContent-Disposition: form-data; name="skill"\r\n\r\njavascript\r\n'
                b'--boundary\r\nContent-Disposition: form-data; name="document"; filename="actual.txt"\r\n'
                b'Content-Type: application/octet-stream\r\n\r\n\x00\xffreal bytes\r\n--boundary--\r\n')
        fields, files = frontend.legacy.decode_submission('multipart/form-data; boundary=boundary', body)
        self.assertEqual(fields, [['skill', 'rust'], ['skill', 'javascript']])
        self.assertEqual(files[0]['sha256'], hashlib.sha256(b'\x00\xffreal bytes').hexdigest())
        self.assertEqual(files[0]['filename'], 'actual.txt')

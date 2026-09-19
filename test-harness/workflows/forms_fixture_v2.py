"""Multi-step/async-validation UI; actual native submissions use the v1 receiver."""
import importlib.util
import json
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('forms_backend_v1', HERE / 'forms_fixture.py')
legacy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(legacy)


class Fixture(legacy.Fixture):
    def __init__(self):
        super().__init__()
        self.validations = {}

    def command(self, message):
        result = super().command(message)
        if message['command'] == 'snapshot':
            with self.lock:
                result['validations'] = list(self.validations.get(message['case_id'], []))
        return result

    def validate_email(self, case_id, email):
        with self.lock:
            if case_id not in self.cases:
                return 404, {'valid': False}
            if len(self.validations.get(case_id, [])) >= 32:
                return 429, {'valid': False}
            valid = email == 'zoe@example.test'
            self.validations.setdefault(case_id, []).append(dict(email=email, valid=valid))
            return 200, {'valid': valid}


def render(page, case_id):
    page = page.replace('<form ', '<form id="application" data-state="editing" ', 1)
    page = page.replace('  <label for="name">', '  <section id="identity-step">\n  <label for="name">', 1)
    page = page.replace('  <label for="quantity">',
        '<button type="button" id="next-step">Review application details</button>'
        '<p id="validation-status" role="status"></p></section>'
        '<section id="details-step" hidden><button type="button" id="previous-step">Edit identity</button>'
        '  <label for="quantity">', 1)
    page = page.replace('</form>', '</section></form>', 1)
    page = page.replace('<label for="role">Primary role</label><select id="role"',
        '<button type="button" id="role-picker" aria-haspopup="listbox">Choose primary role</button>'
        '<label for="role" hidden>Primary role</label><select hidden id="role"')
    return page + f'<script type="module" src="/frontend/{case_id}/forms-client.mjs"></script>'


def extra_get(handler, fixture):
    parsed = urlsplit(handler.path)
    pieces = parsed.path.strip('/').split('/')
    if len(pieces) != 3 or pieces[0] != 'frontend':
        return False
    _, case_id, route = pieces
    with fixture.lock:
        exists = case_id in fixture.cases
    if not exists:
        handler.reply(404, 'Unknown case')
    elif route == 'forms-client.mjs':
        handler.reply(200, (HERE / 'v2/forms-client.mjs').read_text(),
                      [('Content-Type', 'text/javascript; charset=utf-8')])
    elif route == 'validate-email':
        query = parse_qs(parsed.query, keep_blank_values=True)
        if set(query) != {'email'} or len(query['email']) != 1:
            handler.reply(400, 'Invalid validation query')
        else:
            status, result = fixture.validate_email(case_id, query['email'][0])
            handler.reply(status, json.dumps(result), [('Content-Type', 'application/json')])
    else:
        handler.reply(404, 'Not found')
    return True


if __name__ == '__main__':
    legacy.serve(fixture_type=Fixture, render_page=render, extra_get=extra_get)

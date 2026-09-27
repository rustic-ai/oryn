"""Real SQLite ownership checks; no native processes or historical test reruns."""
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock
from urllib.parse import urlencode

FIXTURES = Path(__file__).resolve().parents[1] / 'test-harness/workflows'
sys.path.insert(0, str(FIXTURES))
import workflow_faults
import workflow_fixture
spec = importlib.util.spec_from_file_location('ownership_forms', FIXTURES / 'forms_fixture_v4.py')
forms = importlib.util.module_from_spec(spec)
spec.loader.exec_module(forms)
REAL_CONNECT = sqlite3.connect


@contextmanager
def connections(fail_sql=None):
    opened = []
    class Connection(sqlite3.Connection):
        closed = False
        def close(self):
            try:
                return super().close()
            finally:
                self.closed = True
        def execute(self, sql, *args, **kwargs):
            result = super().execute(sql, *args, **kwargs)
            if fail_sql and sql.startswith(fail_sql):
                raise RuntimeError('injected after write')
            return result
    def connect(*args, **kwargs):
        db = REAL_CONNECT(*args, **kwargs, factory=Connection)
        opened.append(db)
        return db
    try:
        with mock.patch.object(sqlite3, 'connect', side_effect=connect):
            yield opened
    finally:
        # Defensive test cleanup does not make the assertions pass.
        for db in opened:
            if not db.closed:
                db.close()


class SQLiteOwnershipTests(unittest.TestCase):
    def closed(self, opened):
        self.assertTrue(opened)
        self.assertTrue(all(db.closed for db in opened), 'connection escaped method ownership')

    def test_response_ledger_closed_and_committed_before_marker_and_wait(self):
        with tempfile.TemporaryDirectory() as directory, connections() as opened:
            root = Path(directory).resolve()
            faults = workflow_faults.ResponseFaults(root / 'db')
            gate = dict(token='a'*32, route='receipt', marker=str(root / 'entered'))
            faults.arm('case', gate['token'], 'receipt', 1, response_gate=gate)
            self.closed(opened)
            owner = self
            class Wait:
                def wait(self, seconds):
                    owner.closed(opened)
                    row = faults.snapshot('case')[0]
                    owner.assertEqual(row['terminal'], 'withholding')
                    owner.assertEqual(json.loads(Path(gate['marker']).read_text())['started_ns'], row['started_ns'])
                    owner.closed(opened)
            faults.pending['case'][3] = Wait()
            original_open = Path.open
            def marker_open(path, *args, **kwargs):
                if path.name == 'entered.publishing':
                    owner.closed(opened)
                    owner.assertEqual(faults.snapshot('case')[0]['terminal'], 'withholding')
                return original_open(path, *args, **kwargs)
            with mock.patch.object(Path, 'open', marker_open):
                faults.before_response('case', 'receipt')
            self.assertEqual(faults.snapshot('case')[0]['terminal'], 'released')
            self.closed(opened)

    def test_failed_insert_rolls_back_and_closes_without_arming(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / 'db'
            with connections() as opened:
                faults = workflow_faults.ResponseFaults(database)
                self.closed(opened)
            with connections('INSERT INTO workflow_faults') as opened:
                with self.assertRaisesRegex(RuntimeError, 'injected after write'):
                    faults.arm('case', 'token', 'receipt', 1)
                self.closed(opened)
            with connections() as opened:
                self.assertEqual(faults.snapshot('case'), [])
                self.assertEqual(faults.pending, {})
                self.closed(opened)

    def test_commit_gate_connections_close_and_persist_terminal(self):
        with tempfile.TemporaryDirectory() as directory, connections() as opened:
            root = Path(directory).resolve()
            gates = workflow_faults.CommitGates(root / 'db')
            gates.arm('case', 'token', 'submit', 'before_commit', root/'marker', root/'release')
            gate = gates.claim('case', 'submit', b'name=value')
            (root/'release').write_text(json.dumps(dict(schema_version=1, token='token', phase='before_commit', worker_reaped=True, interrupted_inflight=True)))
            self.closed(opened)
            self.assertEqual(gates.hold(gate), 'aborted_before_commit')
            self.assertEqual(gates.snapshot('case')[0]['terminal'], 'aborted_before_commit')
            self.closed(opened)

    def test_instrumented_forms_writes_reads_and_early_return_close(self):
        with tempfile.TemporaryDirectory() as directory, connections() as opened:
            database = Path(directory) / 'db'
            fixture = workflow_fixture.instrument(forms.Fixture, database, 'forms')(database)
            identity = 'c'*32
            fixture.command(dict(command='create', case_id=identity, scenario='post'))
            row = fixture.row(identity)
            self.assertEqual(fixture.submit('d'*32, 'submit', 'POST', 'application/x-www-form-urlencoded', b'', ''), 404)
            fixture.deny(identity, '/denied')
            body = urlencode(dict(nonce=row['nonce'], name='Owned')).encode()
            self.assertEqual(fixture.submit(identity, 'submit', 'POST', 'application/x-www-form-urlencoded', body, row['session']), 303)
            snapshot = fixture.command(dict(command='snapshot', case_id=identity))
            self.assertEqual(len(snapshot['receipts']), 1)
            self.assertEqual(len(snapshot['denials']), 1)
            self.closed(opened)


if __name__ == '__main__':
    unittest.main()

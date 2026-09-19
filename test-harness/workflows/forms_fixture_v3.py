"""Durable forms oracle over the existing native-encoding receiver and v2 UI.

Every accepted/rejected request and validation transition is committed to SQLite
before control returns to the HTTP handler. The controller seals the servers and
reopens this database; page confirmation text is never the outcome oracle.
"""
import argparse
from contextlib import closing
import importlib.util
import json
import html
from pathlib import Path
import sqlite3

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('forms_v2_durable',HERE/'forms_fixture_v2.py')
v2=importlib.util.module_from_spec(spec);spec.loader.exec_module(v2)


def snapshot(database,case_id):
    with closing(sqlite3.connect(f'file:{database}?mode=ro',uri=True)) as db:
        row=db.execute('SELECT payload FROM form_snapshots WHERE case_id=?',(case_id,)).fetchone()
        if row is None:raise ValueError('missing durable form case')
        return json.loads(row[0])


class Fixture(v2.Fixture):
    def __init__(self,database):
        super().__init__();self.database=str(database)
        with sqlite3.connect(self.database) as db:
            db.execute('CREATE TABLE form_snapshots(case_id TEXT PRIMARY KEY,payload TEXT NOT NULL)')

    def persist(self,case_id):
        # Hold the same lock as mutations so an older request cannot overwrite a
        # newer receipt. Database connections are local to the request thread.
        with self.lock, sqlite3.connect(self.database) as db:
            if case_id not in self.cases:return
            payload=dict(self.cases[case_id],validations=list(self.validations.get(case_id,[])))
            db.execute('INSERT INTO form_snapshots VALUES(?,?) ON CONFLICT(case_id) DO UPDATE SET payload=excluded.payload',
                       (case_id,json.dumps(payload,sort_keys=True)))

    def repeat_form(self,case_id):
        receipts=self.cases.get(case_id,{}).get("receipts",[])
        if not receipts or receipts[0]["files"]:return ""
        hidden="".join(f'<input type=hidden name="{html.escape(name,quote=True)}" value="{html.escape(value,quote=True)}">' for name,value in receipts[0]["fields"])
        return f'<form method=post action="/submit/{case_id}">{hidden}<button>Repeat document request</button></form>'

    def receipt_view(self,case_id):
        super().receipt_view(case_id)
        with sqlite3.connect(self.database) as db:
            payload=dict(self.cases[case_id],validations=list(self.validations.get(case_id,[])))
            db.execute("UPDATE form_snapshots SET payload=? WHERE case_id=?",(json.dumps(payload,sort_keys=True),case_id))

    def command(self,message):
        if message['command']=='snapshot':
            self.persist(message['case_id'])
            return snapshot(self.database,message['case_id'])
        result=super().command(message)
        self.persist(message['case_id']);return result

    def receive(self,case_id,content_type,body):
        result=super().receive(case_id,content_type,body)
        self.persist(case_id);return result

    def validate_email(self,case_id,email):
        result=super().validate_email(case_id,email)
        self.persist(case_id);return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--database',required=True)
    args=parser.parse_args()
    v2.legacy.serve(fixture_type=lambda:Fixture(args.database),render_page=v2.render,extra_get=v2.extra_get)

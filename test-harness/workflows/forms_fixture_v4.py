"""Native GET/POST/multipart, submitter and reentrant delivery over the form decoder.

Receipt creation uses a server-issued one-use nonce and durable SQLite transaction.
The private pipe is the only way to read raw receipt and request evidence.
"""
import argparse
from contextlib import closing
import hashlib
import html
from http.cookies import SimpleCookie
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import re
import secrets
import sqlite3
import sys
import threading
from urllib.parse import parse_qsl,urlsplit

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('extended_forms_decoder',HERE/'forms_fixture.py')
legacy=importlib.util.module_from_spec(spec);spec.loader.exec_module(legacy)
SCENARIOS={'get','post','multipart','submitter','cancel','reentrant','confinement','face'}


def snapshot(database,case_id):
    with closing(sqlite3.connect(f'file:{database}?mode=ro',uri=True)) as db:
        db.row_factory=sqlite3.Row
        case=db.execute('SELECT scenario,accepted FROM cases WHERE case_id=?',(case_id,)).fetchone()
        if case is None:raise ValueError('unknown case')
        requests=[dict(row) for row in db.execute('SELECT method,path,status,body_sha256,body_bytes,fields,files FROM requests WHERE case_id=? ORDER BY rowid',(case_id,))]
        for row in requests:row['fields']=json.loads(row['fields']);row['files']=json.loads(row['files'])
        receipts=[dict(row) for row in db.execute('SELECT method,path,fields,files FROM receipts WHERE case_id=? ORDER BY rowid',(case_id,))]
        for row in receipts:row['fields']=json.loads(row['fields']);row['files']=json.loads(row['files'])
        return dict(case=dict(case),requests=requests,receipts=receipts,
                    denials=[dict(row) for row in db.execute('SELECT path,status FROM denials WHERE case_id=? ORDER BY rowid',(case_id,))])


class Fixture:
    def __init__(self,database):
        self.database=str(database);self.lock=threading.RLock()
        with closing(sqlite3.connect(self.database)) as db, db:
            db.executescript('CREATE TABLE cases(case_id TEXT PRIMARY KEY,scenario TEXT,nonce TEXT,session TEXT,accepted INTEGER DEFAULT 0);'
                'CREATE TABLE requests(case_id TEXT,method TEXT,path TEXT,status INTEGER,body_sha256 TEXT,body_bytes INTEGER,fields TEXT,files TEXT);'
                'CREATE TABLE receipts(case_id TEXT UNIQUE,method TEXT,path TEXT,fields TEXT,files TEXT);'
                'CREATE TABLE denials(case_id TEXT,path TEXT,status INTEGER);')
    def command(self,message):
        identity=message['case_id']
        if not re.fullmatch('[0-9a-f]{32}',identity):raise ValueError('invalid case')
        if message['command']=='snapshot':return snapshot(self.database,identity)
        if message['command']!='create' or message.get('scenario') not in SCENARIOS:raise ValueError('invalid fixture command/scenario')
        with self.lock,closing(sqlite3.connect(self.database)) as db, db:
            db.execute('INSERT INTO cases(case_id,scenario,nonce,session) VALUES(?,?,?,?)',(identity,message['scenario'],secrets.token_hex(16),secrets.token_hex(32)))
        return {}
    def row(self,identity):
        with closing(sqlite3.connect(self.database)) as db, db:
            db.row_factory=sqlite3.Row
            row=db.execute('SELECT * FROM cases WHERE case_id=?',(identity,)).fetchone()
            return dict(row) if row else None
    def submit(self,identity,route,method,content_type,body,cookie):
        try:fields,files=legacy.decode_submission(content_type,body)
        except (ValueError,UnicodeError):fields,files=[],[]
        with self.lock,closing(sqlite3.connect(self.database)) as db, db:
            db.row_factory=sqlite3.Row;row=db.execute('SELECT * FROM cases WHERE case_id=?',(identity,)).fetchone()
            if row is None:return 404
            status=303
            if cookie!=row['session'] or fields.count(['nonce',row['nonce']])!=1:status=403
            elif row['accepted']:status=409
            elif not any(name=='name' and value for name,value in fields):status=422
            # Persist body fidelity without exposing the nonce in oracle snapshots.
            safe_fields=[[name,'{nonce}' if name=='nonce' else value] for name,value in fields]
            db.execute('INSERT INTO requests VALUES(?,?,?,?,?,?,?,?)',(identity,method,route,status,hashlib.sha256(body).hexdigest(),len(body),json.dumps(safe_fields),json.dumps(files)))
            if status==303:
                db.execute('INSERT INTO receipts VALUES(?,?,?,?,?)',(identity,method,route,json.dumps(safe_fields),json.dumps(files)))
                db.execute('UPDATE cases SET accepted=1 WHERE case_id=?',(identity,))
            return status
    def repeat_form(self,identity):
        row=self.row(identity);data=snapshot(self.database,identity)
        if not data["receipts"] or data["receipts"][0]["files"]:return ""
        fields=data["receipts"][0]["fields"]
        hidden="".join(f'<input type=hidden name="{html.escape(name,quote=True)}" value="{html.escape(row["nonce"] if name=="nonce" else value,quote=True)}">' for name,value in fields)
        return f'<form method=post action="/advanced/{identity}/submit">{hidden}<button>Repeat document request</button></form>'

    def deny(self,identity,path):
        with self.lock,closing(sqlite3.connect(self.database)) as db, db:db.execute('INSERT INTO denials VALUES(?,?,403)',(identity,path))


def page(identity,row):
    base='/advanced/'+identity;scenario=row['scenario']
    method='get' if scenario=='get' else 'post'
    enctype='multipart/form-data' if scenario in ('multipart','confinement','face') else 'application/x-www-form-urlencoded'
    file='<label>Authorized document<input type=file name=document required></label>' if scenario in ('multipart','confinement','face') else ''
    component=''
    if scenario=='face':
        # The selected File moves through a FACE's native submission value;
        # the picker has no name and contributes no duplicate built-in entry.
        file=file.replace('name=document','id=authorized-file')
        component='<x-document id=custom-document></x-document><fieldset disabled><x-excluded name=excluded></x-excluded></fieldset>'
    overrides=f' formaction="{base}/override" formmethod="get"' if scenario=='submitter' else ''
    script=''
    if scenario=='cancel':script='f.addEventListener("submit",e=>{e.preventDefault();document.querySelector("#status").textContent="Submission cancelled"})'
    if scenario=='reentrant':script='let entered=false;f.addEventListener("submit",e=>{if(!entered){entered=true;e.preventDefault();queueMicrotask(()=>f.requestSubmit(document.querySelector("#send")))}})'
    if scenario=='face':script += """
customElements.define('x-document',class extends HTMLElement {
  static formAssociated=true;
  constructor(){super();this.internals=this.attachInternals();this.internals.setValidity({valueMissing:true},'Choose a document');}
  set file(value){const entries=new FormData();entries.append('face_value','Résumé ✓');entries.append('face_duplicate','first');entries.append('document',value);entries.append('face_duplicate','second');this.internals.setFormValue(entries);entries.delete('document');this.internals.setValidity({});}
  formResetCallback(){this.internals.setFormValue(null);this.internals.setValidity({valueMissing:true},'Choose a document');}
});
customElements.define('x-excluded',class extends HTMLElement {static formAssociated=true;constructor(){super();this.attachInternals().setFormValue('excluded');}});
document.querySelector('#authorized-file').addEventListener('change',event=>{const file=event.target.files[0];if(file)document.querySelector('#custom-document').file=file;});
"""
    return ('<!doctype html><meta charset=utf-8><title>Controlled delivery</title>'
        f'<form id=f action="{base}/submit" method={method} enctype="{enctype}">'
        '<label>Full name<input name=name required></label><label>Message<textarea name=message></textarea></label>'
        '<input name=disabled value=excluded disabled><input type=hidden name=duplicate value=first><input type=hidden name=duplicate value=second>'
        f'<input type=hidden name=nonce value="{row["nonce"]}">{file}{component}'
        f'<button id=send name=intent value=send{overrides}>Send document</button></form><p id=status></p>'
        '<script>const f=document.querySelector("#f");'+script+'</script>')


def serve(database,fixture_type=Fixture):
    fixture=fixture_type(database)
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*_):pass
        def reply(self,status,body,headers=()):
            payload=body.encode();self.send_response(status)
            for key,value in [('Content-Type','text/html; charset=utf-8'),('Content-Length',str(len(payload))),('Cache-Control','no-store'),*headers]:self.send_header(key,value)
            self.end_headers();self.wfile.write(payload)
        def handle_request(self):
            parsed=urlsplit(self.path);match=re.fullmatch('/advanced/([0-9a-f]{32})/(start|submit|override|receipt|workflow-outcome)',parsed.path)
            if not match:self.reply(404,'Not found');return
            identity,route=match.groups();row=fixture.row(identity)
            if row is None:self.reply(404,'Unknown case');return
            cookies=SimpleCookie()
            try:cookies.load(self.headers.get('Cookie',''))
            except Exception:pass
            cookie=cookies.get('form_session');cookie=cookie.value if cookie else None
            if route=='workflow-outcome' and hasattr(fixture,'workflow_outcome'):
                from urllib.parse import parse_qs
                status,body,headers=fixture.workflow_outcome(identity,parse_qs(parsed.query).get('token',[''])[0],cookie)
                self.reply(status,body,headers);return
            if route=='start' and self.command=='GET':
                self.reply(200,page(identity,row),[('Set-Cookie',f'form_session={row["session"]}; Path=/advanced/{identity}/; HttpOnly; SameSite=Lax')]);return
            if route=='receipt':
                if cookie!=row['session']:fixture.deny(identity,route);self.reply(403,'<p id=denied>Context access denied</p>');return
                # This route is implemented by the HTTP handler, not Fixture.get;
                # publish the same actual response hold as wrapped method routes.
                faults = getattr(fixture, 'workflow_response_faults', None)
                if faults is not None:
                    faults.before_response(identity, 'receipt', 200)
                self.reply(200,'<p id=receipt>Document received</p>'+fixture.repeat_form(identity) if row['accepted'] else '<p id=empty>No receipt</p>');return
            if route not in ('submit','override'):self.reply(405,'Invalid method');return
            if self.command=='GET':body=parsed.query.encode();content_type='application/x-www-form-urlencoded'
            else:
                try:length=int(self.headers.get('Content-Length','-1'))
                except ValueError:length=-1
                if not 0<=length<=legacy.MAX_BODY or self.headers.get('Transfer-Encoding'):self.reply(413,'Body limit');return
                self.connection.settimeout(10);body=self.rfile.read(length);content_type=self.headers.get('Content-Type','')
                if len(body)!=length:self.reply(400,'Truncated body');return
            status=fixture.submit(identity,route,self.command,content_type,body,cookie)
            self.reply(status,'<p id=result>'+('Accepted' if status==303 else 'Rejected')+'</p>'+ (fixture.repeat_form(identity) if status==409 else ''),[('Location',f'/advanced/{identity}/receipt')] if status==303 else [])
        do_GET=handle_request
        do_POST=handle_request
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler);server.daemon_threads=False
    thread=threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.05});thread.start();sealed=False
    print(json.dumps(dict(event='ready',port=server.server_port)),flush=True)
    try:
        for line in sys.stdin:
            try:
                if len(line)>8192:raise ValueError('control bound')
                message=json.loads(line)
                if message.get('command')=='shutdown':break
                if message.get('command')=='seal':
                    if not sealed:server.shutdown();server.server_close();thread.join(timeout=10);sealed=True
                    result=dict(sealed=not thread.is_alive())
                else:
                    if sealed and message.get('command')!='snapshot':raise ValueError('sealed fixture')
                    result=fixture.command(message)
                print(json.dumps(dict(ok=True,result=result)),flush=True)
            except (ValueError,KeyError,TypeError,sqlite3.Error) as error:print(json.dumps(dict(ok=False,error=str(error))),flush=True)
    finally:server.shutdown();server.server_close();thread.join(timeout=10)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--database',required=True);serve(parser.parse_args().database)

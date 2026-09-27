"""Explicit workflow fixture launcher with durable post-commit response faults."""
import argparse
import importlib.util
import html
import hashlib
import json
from contextlib import closing
import sqlite3
from urllib.parse import parse_qsl
from pathlib import Path
from workflow_faults import ResponseFaults, CommitGates
HERE=Path(__file__).resolve().parent

def load(name,path):
    spec=importlib.util.spec_from_file_location(name,path);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module

def instrument(base,database,family):
    class Instrumented(base):
        def __init__(self,*args,**kwargs):
            super().__init__(*args,**kwargs)
            self.workflow_response_faults=ResponseFaults(database)
            self.workflow_commit_gates=CommitGates(database)
            self.workflow_pending={}
            with closing(sqlite3.connect(database)) as db, db:
                db.execute("CREATE TABLE IF NOT EXISTS workflow_recovery_views(case_id TEXT, token TEXT, state TEXT)")
                db.execute("CREATE TABLE IF NOT EXISTS workflow_recovery_retries(case_id TEXT, token TEXT, body_sha256 TEXT, matched INTEGER)")
        def workflow_effect(self,case):
            data=super().command(dict(command='snapshot',case_id=case))
            key={'forms':'receipts','shopping':'orders','saas':'records','booking':'reservations'}.get(family)
            if key is None:raise ValueError('commit recovery requires a mutation family')
            if key not in data:raise ValueError('durable effect table missing')
            return json.dumps(data[key],sort_keys=True,separators=(',',':'))
        def workflow_outcome(self,case,token,cookie):
            pending=self.workflow_pending.get(case)
            if pending is None or pending['gate']['token']!=token or cookie!=pending['cookie']:
                return 403,'<p id=commit-state>denied</p>',[]
            gate=pending['gate'];ledger=self.workflow_commit_gates.snapshot(case)[-1]
            if ledger['terminal'] not in ('aborted_before_commit','released_after_commit'):
                return 409,'<p id=commit-state>unresolved</p>',[]
            actual=self.workflow_effect(case)
            if actual==pending['before']:state='not_committed'
            elif pending.get('after')==actual and pending.get('status')==303:state='committed'
            else:return 409,'<p id=commit-state>conflict</p>',[]
            with closing(sqlite3.connect(database)) as db, db:
                db.execute('INSERT INTO workflow_recovery_views VALUES(?,?,?)',(case,token,state))
            markup=f'<p id=commit-state>{state}</p>'
            if state=='not_committed':
                pending['retry_authorized']=True
                if pending['content_type'].split(';')[0]!='application/x-www-form-urlencoded':
                    return 409,'<p id=commit-state>unsupported_body</p>',[]
                fields=parse_qsl(pending['body'].decode('utf-8'),keep_blank_values=True,strict_parsing=True)
                controls=''.join(f'<input type=hidden name="{html.escape(name,quote=True)}" value="{html.escape(value,quote=True)}">' for name,value in fields)
                action=('/submit/'+case) if family=='forms' and pending['method_name']=='receive' else {'forms':'/advanced/','shopping':'/shop/','booking':'/booking/','saas':'/saas/'}[family]+case+'/'+gate['route']
                markup+=f'<form method="{pending["verb"]}" action="{action}">{controls}<button>Retry pending request</button></form>'
            else:
                destination=pending.get('location')
                if not destination:return 409,'<p id=commit-state>missing_destination</p>',[]
                markup+=f'<a href="{html.escape(destination,quote=True)}">Continue committed request</a>'
            return 200,markup,[]
        def command(self,message):
            if message['command']=='arm_commit_gate':
                super().command(dict(command='snapshot',case_id=message['case_id']))
                self.workflow_commit_gates.arm(message['case_id'],message['token'],message['route'],message['phase'],message['marker'],message['release'],message.get('skip_matches',0),message.get('timeout_ms',30000))
                return dict(armed=True)
            if message['command']=='arm_response_fault':
                # Validate case authority using the backend's private snapshot.
                super().command(dict(command='snapshot',case_id=message['case_id']))
                self.workflow_response_faults.arm(message['case_id'],message['token'],message['route'],message['delay_ms'],message.get('expected_status'),message.get('skip_matches',0),message.get('response_gate'))
                return dict(armed=True)
            result=super().command(message)
            if message['command']=='snapshot':
                result['workflow_response_faults']=self.workflow_response_faults.snapshot(message['case_id'])
                result['workflow_commit_gates']=self.workflow_commit_gates.snapshot(message['case_id'])
            return result
    for method in ('get','post','submit','receive','callback','receipt_view'):
        original=getattr(base,method,None)
        if original is None:continue
        def wrap(original=original,method=method):
            def operation(self,*args,**kwargs):
                case=args[0]
                route=args[1] if method in ('get','post','submit') else 'callback' if method=='callback' else 'receipt' if method=='receipt_view' else 'submit'
                if method=='get' and route=='workflow-outcome':
                    return self.workflow_outcome(case,args[2].get('token',[''])[0],args[3])
                gate=None
                if method in ('post','submit','receive'):
                    if method=='post':body,cookie,content_type,verb=args[2],args[3],'application/x-www-form-urlencoded','POST'
                    elif method=='submit':verb,content_type,body,cookie=args[2:6]
                    else:content_type,body,cookie,verb=args[1],args[2],'','POST'
                    pending=self.workflow_pending.get(case)
                    if pending is not None and pending.get('retry_authorized') and pending['gate']['route']==route:
                        matched=(body==pending['body'] and cookie==pending['cookie'] and verb==pending['verb'])
                        with closing(sqlite3.connect(database)) as db, db:
                            db.execute('INSERT INTO workflow_recovery_retries VALUES(?,?,?,?)',(case,pending['gate']['token'],hashlib.sha256(body).hexdigest(),int(matched)))
                        pending['retry_authorized']=False
                        if not matched:return (409,'Original request mismatch',[]) if method=='post' else 409
                    gate=self.workflow_commit_gates.claim(case,route,body,cookie,content_type,verb)
                if gate is not None:
                    self.workflow_pending[case]=dict(gate=gate,body=body,cookie=cookie,content_type=content_type,verb=verb,method_name=method,before=self.workflow_effect(case))
                if gate is not None and gate['phase']=='before_commit':
                    self.workflow_commit_gates.hold(gate)
                    # The original request is permanently aborted before entering
                    # the application's mutation method, even if release times out.
                    return (499,'Qualification request interrupted',[]) if method=='post' else 499
                result=original(self,*args,**kwargs)
                if gate is not None:
                    status=result[0] if isinstance(result,tuple) else result
                    pending=self.workflow_pending[case];pending['after']=self.workflow_effect(case);pending['status']=status
                    pending['location']=next((value for key,value in result[2] if key.lower()=='location'),None) if isinstance(result,tuple) else '/advanced/'+case+'/receipt' if method=='submit' else '/receipt/'+case
                    self.workflow_commit_gates.hold(gate,status)
                # Delay after the backend has returned/committed, before the HTTP
                # handler writes headers. Cancellation therefore loses a real
                # response and must not duplicate its already committed mutation.
                status=result[0] if isinstance(result,tuple) else result if isinstance(result,int) else 200 if method=='receipt_view' else None
                self.workflow_response_faults.before_response(case,route,status)
                return result
            return operation
        setattr(Instrumented,method,wrap())
    return Instrumented


def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--family',choices=('research','forms','shopping','saas','booking'),required=True)
    parser.add_argument('--version',type=int,required=True);parser.add_argument('--database',required=True)
    args=parser.parse_args();suffix='' if args.version==1 else f'_v{args.version}'
    module=load('workflow_application',HERE/f'{args.family}_fixture{suffix}.py')
    wrapped=instrument(module.Fixture,args.database,args.family)
    if args.family=='research':
        backend=module if args.version==1 else module.legacy if args.version==2 else module.v2.legacy
        backend.serve(args.database,fixture_type=wrapped,
                      extra_paths=() if args.version==1 else tuple(module.ASSETS) if args.version==2 else (*module.v2.ASSETS,*module.ROUTES),
                      configure_server=module.configure_server if args.version==3 else None)
    elif args.family=='shopping':
        if args.version==1:module.serve(args.database,fixture_type=wrapped)
        else:module.legacy.serve(args.database,fixture_type=wrapped,extra_paths=('shopping-client.mjs','workflow-outcome'),configure_server=module.configure_server)
    elif args.family=='forms':
        if args.version==4:module.serve(args.database,fixture_type=wrapped)
        elif args.version==3:
            def recovery_get(handler,fixture):
                import re
                from urllib.parse import urlsplit,parse_qs
                parsed=urlsplit(handler.path);match=re.fullmatch('/form/([0-9a-f]{32})/workflow-outcome',parsed.path)
                if not match:return module.v2.extra_get(handler,fixture)
                status,body,headers=fixture.workflow_outcome(match[1],parse_qs(parsed.query).get('token',[''])[0],'')
                handler.reply(status,body,headers);return True
            module.v2.legacy.serve(fixture_type=lambda:wrapped(args.database),render_page=module.v2.render,extra_get=recovery_get)
        else:raise ValueError('workflow forms require durable version 3 or 4')
    elif args.family=='saas':
        if args.version==4:module.v2.serve(args.database,fixture_type=wrapped,extra_routes=('negative','probe','workflow-outcome'))
        else:module.serve(args.database,fixture_type=wrapped,extra_routes=('workflow-outcome',))
    else:module.serve(args.database,fixture_type=wrapped)

if __name__=='__main__':main()

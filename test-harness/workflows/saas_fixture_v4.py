"""Browser-driven adversarial public-client exchanges against the real OAuthlib IdP.

Native OIL selects the attack in the application, authenticates at the distinct
IdP, and returns with a freshly issued code. Only the selected exchange parameter
is changed. The application never creates a session for these rejected flows.
"""
import argparse
import html
import importlib.util
import json
from pathlib import Path
import secrets
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import Request,build_opener,ProxyHandler
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('saas_adversarial_v2',HERE/'saas_fixture_v2.py')
v2=importlib.util.module_from_spec(spec);spec.loader.exec_module(v2)
snapshot=v2.snapshot
MODES=('state','redirect','verifier','replay','expiry')


class Fixture(v2.Fixture):
    def __init__(self,database):
        super().__init__(database)
        self.db.execute('CREATE TABLE attack_modes(case_id TEXT PRIMARY KEY,mode TEXT)');self.db.commit()
    def get(self,case,path,query,cookie):
        if path=='negative':
            with self.lock:
                if not self.db.execute('SELECT 1 FROM cases WHERE case_id=?',(case,)).fetchone():return 404,'Unknown case',[]
            body=''.join(f'<a href="/saas/{case}/sso?probe={mode}">Test {mode} rejection</a>' for mode in MODES)
            return 200,self.page('Controlled protocol rejection',body),[]
        if path=='sso' and query.get('probe'):
            if set(query)!={'probe'} or len(query['probe'])!=1 or query['probe'][0] not in MODES:return 400,'Invalid probe',[]
            with self.lock,self.db:self.db.execute('INSERT INTO attack_modes VALUES(?,?) ON CONFLICT(case_id) DO UPDATE SET mode=excluded.mode',(case,query['probe'][0]))
        if path=='probe':
            # Public attack form intentionally has no authenticated CSRF value.
            # The real persisted record backend must reject the submitted write.
            body=(f'<form method=post action="/saas/{case}/update"><input name=csrf value=foreign>'
                  '<input name=version value=1><input name=title value=Unauthorized><button>Attempt unauthorized save</button></form>')
            return 200,self.page('Isolated writer probe',body),[]
        return super().get(case,path,query,cookie)
    def callback(self,case,query,cookie):
        with self.lock:
            mode=self.db.execute('SELECT mode FROM attack_modes WHERE case_id=?',(case,)).fetchone()
        if mode is None:return super().callback(case,query,cookie)
        mode=mode[0]
        if mode=='state' and query.get('state')!=['invalid-state']:
            altered=urlencode(dict(code=query.get('code',[''])[0],state='invalid-state'))
            return 200,self.page('Adversarial callback',f'<a href="/saas/{case}/callback?{html.escape(altered,quote=True)}">Continue with altered state</a>'),[]
        if mode=='state':
            response=super().callback(case,query,cookie)
            return response[0],response[1]+'<p id=protocol>state_rejected</p>',response[2]
        state=query.get('state',[''])[0]
        with self.lock,self.db:
            row=self.db.execute('SELECT * FROM login_flows WHERE state_digest=? AND case_id=?',(v2.oauth.fingerprint(state),case)).fetchone()
            epoch=self.db.execute('SELECT epoch FROM login_epochs WHERE case_id=?',(case,)).fetchone()
            if (set(query)!={'state','code'} or any(len(values)!=1 for values in query.values()) or not row or row['consumed']
                    or row['expires']<=self.clock() or not epoch or epoch[0]!=row['epoch']
                    or not secrets.compare_digest(row['cookie_digest'],v2.oauth.fingerprint(cookie))):
                self.event(case,'callback_rejected',400);return 400,'Invalid callback',[]
            self.db.execute("UPDATE login_flows SET consumed=1,verifier='' WHERE state_digest=?",(row['state_digest'],))
        fields=dict(grant_type='authorization_code',client_id=case,code=query['code'][0],
                    redirect_uri=f'{self.origin}/saas/{case}/callback',code_verifier=row['verifier'])
        if mode=='redirect':fields['redirect_uri']+='/unregistered'
        if mode=='verifier':fields['code_verifier']='B'*43
        if mode=='expiry':
            # Expire this actual grant only; other cases' clocks and grants are untouched.
            with self.provider.lock,self.provider.db:
                self.provider.db.execute('UPDATE oauth_codes SET expires=0 WHERE digest=?',(v2.oauth.fingerprint(fields['code']),))
                self.provider.audit(case,'grant_expired',200)
        opener=build_opener(ProxyHandler({}),v2.NoRedirect())
        response={};status=0
        for _ in range(2 if mode=='replay' else 1):
            request=Request(self.issuer+'/token',data=urlencode(fields).encode(),headers={'Content-Type':'application/x-www-form-urlencoded'})
            try:
                with opener.open(request,timeout=10) as stream:status=stream.status;response=json.loads(stream.read(16384))
            except HTTPError as error:
                status=error.code;response=json.loads(error.read(16384))
        error=response.get('error')
        with self.lock,self.db:self.event(case,'exchange_rejected' if status>=400 and error else 'unexpected_exchange',status)
        # Never return or persist bearer token bytes in application/oracle artifacts.
        return status,self.page('Protocol outcome','<p id=protocol>'+html.escape(error or 'unexpected_token')+'</p>'),[]


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--database',required=True)
    v2.serve(parser.parse_args().database,fixture_type=Fixture,extra_routes=('negative','probe'))

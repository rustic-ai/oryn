"""Real same-origin, cross-origin and nested research frames with durable visits."""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import re
import threading
from urllib.parse import parse_qs

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('research_v2', HERE/'research_fixture_v2.py')
v2 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v2)
fault_spec=importlib.util.spec_from_file_location("research_response_faults",HERE/"workflow_faults.py")
fault_module=importlib.util.module_from_spec(fault_spec);fault_spec.loader.exec_module(fault_module)
ResponseFaults=fault_module.ResponseFaults

ROUTES = ('frames', 'frame-same', 'frame-foreign', 'frame-nested')


class Fixture(v2.Fixture):
    def __init__(self, database):
        super().__init__(database)
        self.response_faults = ResponseFaults(database)
        self.origin = self.foreign_origin = None

    def command(self, message):
        if message['command'] == 'arm_response_fault':
            case_id = message['case_id']
            with self.lock:
                if not self.db.execute('SELECT 1 FROM cases WHERE case_id=?', (case_id,)).fetchone():
                    raise ValueError('unknown fault case')
            self.response_faults.arm(case_id, message['token'], message['route'], message['delay_ms'])
            return {'armed': True}
        result = super().command(message)
        if message['command'] == 'snapshot':
            result['response_faults'] = self.response_faults.snapshot(message['case_id'])
        return result

    def get(self, case_id, path, query):
        self.response_faults.before_response(case_id, path)
        if path not in ROUTES:
            return super().get(case_id, path, query)
        with self.lock, self.db:
            if not self.db.execute('SELECT 1 FROM cases WHERE case_id=?', (case_id,)).fetchone():
                return 404, 'Unknown case', []
            base = f'/research/{case_id}'
            if path == 'frames':
                body = ('<p id=facts>Root research index only.</p>'
                        f'<iframe id=same src="{base}/frame-same"></iframe>'
                        '<button id=remove>Remove source tree</button>'
                        '<script>document.querySelector("#remove").onclick=()=>{document.querySelector("#same").remove();}</script>')
            elif path == 'frame-same':
                foreign = json.dumps(self.foreign_origin+base+'/frame-foreign')
                body = ('<p id=facts>Same-origin Alpha: 11 MWh.</p>'
                        '<button id=load>Load foreign source</button><div id=children></div>'
                        '<script>document.querySelector("#load").onclick=()=>{const f=document.createElement("iframe");'
                        f'f.id="foreign";f.src={foreign};document.querySelector("#children").replaceChildren(f);}};</script>')
            elif path == 'frame-foreign':
                nested = json.dumps(self.origin+base+'/frame-nested')
                body = ('<p id=facts>Cross-origin Beta: 22 MWh.</p><p id=isolation>unchecked</p>'
                        '<button id=load>Load nested source</button><div id=children></div><script>'
                        'try{parent.document.body;document.querySelector("#isolation").textContent="LEAK"}'
                        'catch(e){document.querySelector("#isolation").textContent=e.name==="SecurityError"?"Parent access denied":"Unexpected error"}'
                        'document.querySelector("#load").onclick=()=>{const f=document.createElement("iframe");'
                        f'f.id="nested";f.src={nested};document.querySelector("#children").replaceChildren(f);}};</script>')
            else:
                body = '<p id=facts>Nested Gamma: 33 MWh.</p>'
            body = self.page(body)
            self.db.execute('INSERT INTO visits VALUES(?,?,?,?,?)', (case_id,path,json.dumps(query,sort_keys=True),200,hashlib.sha256(body.encode()).hexdigest()))
            return 200, body, []


def configure_server(fixture, server):
    fixture.origin = f'http://127.0.0.1:{server.server_port}'
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            path, _, query = self.path.partition('?')
            match = re.fullmatch(r'/research/([0-9a-f]{32})/(frame-foreign)', path)
            status, body, headers = fixture.get(*match.groups(),parse_qs(query)) if match else (404,'Not found',[])
            payload = body.encode()
            self.send_response(status)
            for name, value in [('Content-Type','text/html; charset=utf-8'),('Content-Length',str(len(payload))),('Cache-Control','no-store'),*headers]: self.send_header(name,value)
            self.end_headers();self.wfile.write(payload)
    foreign = ThreadingHTTPServer(('127.0.0.1',0),Handler)
    foreign.daemon_threads = False
    fixture.foreign_origin = f'http://127.0.0.1:{foreign.server_port}'
    thread = threading.Thread(target=foreign.serve_forever, kwargs={'poll_interval': 0.05});thread.start()
    def close():
        foreign.shutdown();foreign.server_close();thread.join(timeout=10)
        if thread.is_alive(): raise ValueError('foreign research server did not retire')
    return close


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--database',required=True)
    v2.legacy.serve(parser.parse_args().database,fixture_type=Fixture,
                    extra_paths=(*v2.ASSETS,*ROUTES),configure_server=configure_server)

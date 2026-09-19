"""Failure-evidence regressions; subprocess fixtures are mocked, Node uses no browser."""
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


def fixture_module():
    spec = importlib.util.spec_from_file_location('chromium_fixture_diagnostics', ROOT / 'scripts/tests/test_wpt_direct_fixture.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def server_module():
    spec = importlib.util.spec_from_file_location('wpt_server_diagnostics', ROOT / 'scripts/wptserve-helper.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@unittest.skipUnless(shutil.which('node'), 'requires Node for Chromium diagnostic protocol tests')
class ChromiumDiagnosticProtocolTests(unittest.TestCase):
    def evaluate(self, body, node_flags=()):
        script = 'import {createDiagnostics,safeLocation} from ' + json.dumps((ROOT / 'scripts/wpt-chromium-diagnostics.mjs').as_uri()) + ';\n' + body
        result = subprocess.run([shutil.which('node'), *node_flags, '--input-type=module', '-e', script],
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_actual_canary_launch_uses_mock_keychain_only_for_macos_fresh_profile(self):
        record = self.evaluate('''
import {readFile} from 'node:fs/promises';
import {SourceTextModule,SyntheticModule,createContext} from 'node:vm';
import {EventEmitter} from 'node:events';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
const scriptURL=new URL('./wpt-chromium-canary.mjs', import.meta.url);
const source=await readFile(scriptURL,'utf8');
const records=[];
for (const platform of ['darwin','linux','win32']) {
  const calls=[],profiles=[];
  const stop=new Error('browser spawn observed');
  const context=createContext({URL,setTimeout,clearTimeout,
    process:{platform,argv:['node','canary','/fixture/CfT','http://localhost:8123/target.html','10000'],
      env:{ORYN_WPT_PYTHON:'/fixture/python'},once(){}}});
  const spawn=(binary,args,options)=>{
    calls.push({binary,args,options});
    if(calls.length===1){
      const lease=new EventEmitter();lease.stdout=new EventEmitter();lease.kill=()=>{};
      queueMicrotask(()=>lease.stdout.emit('data',Buffer.from('ready')));
      return lease;
    }
    throw stop;
  };
  const unused=()=>{throw new Error('unexpected operation beyond browser launch')};
  const modules={
    'node:child_process':{spawn},
    'node:fs/promises':{mkdtemp:async prefix=>{profiles.push(prefix);return prefix+'fresh-token'},readFile:unused,rm:unused},
    'node:os':{tmpdir:()=>'/fixture/tmp'},'node:path':{join},'node:url':{fileURLToPath},
    'node:timers/promises':{setTimeout:unused},
    './wpt-chromium-driver.mjs':{dispatchDriver:unused,settleAutomation:unused},
    './wpt-chromium-diagnostics.mjs':{createDiagnostics}
  };
  const module=new SourceTextModule(source,{context,identifier:scriptURL.href,
    initializeImportMeta(meta){meta.url=scriptURL.href}});
  await module.link(specifier=>{
    const values=modules[specifier];if(!values)throw new Error('unexpected import '+specifier);
    return new SyntheticModule(Object.keys(values),function(){
      for(const [name,value] of Object.entries(values))this.setExport(name,value);
    },{context});
  });
  try {await module.evaluate();throw new Error('browser launch was not observed')}
  catch(error){if(error!==stop)throw error}
  records.push({platform,calls,profiles});
}
console.log(JSON.stringify(records));
'''.replace("new URL('./wpt-chromium-canary.mjs', import.meta.url)",
               'new URL(' + json.dumps((ROOT / 'scripts/wpt-chromium-canary.mjs').as_uri()) + ')'),
            node_flags=('--experimental-vm-modules',))
        baseline = ['--headless=new', '--remote-debugging-port=0',
                    '--user-data-dir=/fixture/tmp/oryn-wpt-chromium-fresh-token',
                    '--no-first-run', '--no-default-browser-check',
                    '--disable-background-networking', '--ignore-certificate-errors']
        self.assertEqual([item['platform'] for item in record], ['darwin', 'linux', 'win32'])
        for item in record:
            with self.subTest(platform=item['platform']):
                self.assertEqual(item['profiles'], ['/fixture/tmp/oryn-wpt-chromium-'])
                self.assertEqual(len(item['calls']), 2)
                self.assertEqual(item['calls'][0]['binary'], '/fixture/python')
                self.assertTrue(item['calls'][0]['args'][0].endswith('/wpt-reference-lock.py'))
                self.assertEqual(item['calls'][1]['binary'], '/fixture/CfT')
                platform_args = ['--use-mock-keychain'] if item['platform'] == 'darwin' else []
                self.assertEqual(item['calls'][1]['args'], baseline + platform_args + ['about:blank'])
                self.assertEqual(item['calls'][1]['options'], {'stdio': ['ignore', 'ignore', 'pipe']})

    def test_navigation_timeout_retains_exact_request_phase_and_cleanup(self):
        record = self.evaluate('''
let now=0; const d=createDiagnostics('http://localhost:8123/target.html?secret=hidden',{clock:()=>now});
d.phase('navigate');d.request(7,'Page.navigate');
d.event({method:'Target.attachedToTarget',params:{sessionId:'child',waitingForDebugger:true,targetInfo:{type:'iframe',targetId:'frame',url:'https://alt.localhost/child'}}});
d.request(8,'Runtime.runIfWaitingForDebugger','child');now=3;d.response(8);
now=10000;d.timeout(7,10000);d.failure(new Error('CDP timeout: Page.navigate'));
d.phase('cleanup');d.browser('exit',{pid:123,code:0});console.log(JSON.stringify(d.snapshot(new Error('ignored'))));
''')
        self.assertEqual(record['failure_phase'], 'navigate')
        self.assertEqual(record['phase'], 'cleanup')
        self.assertEqual(record['last_timeout']['method'], 'Page.navigate')
        self.assertEqual(record['last_timeout']['timeout_ms'], 10000)
        self.assertEqual(record['last_timeout']['elapsed_ms'], 10000)
        self.assertEqual(record['pending_cdp'], [])
        resumed = [event for event in record['timeline'] if event['kind'] == 'cdp_response']
        self.assertEqual(resumed[0]['session_id'], 'child')
        self.assertEqual(resumed[0]['method'], 'Runtime.runIfWaitingForDebugger')
        self.assertEqual(record['timeline'][-1]['event'], 'exit')

    def test_whitelisted_network_fields_exclude_credentials_headers_bodies_and_bindings(self):
        record = self.evaluate('''
const d=createDiagnostics('https://user:password@localhost/test.html?token=private#fragment');
d.event({method:'Network.requestWillBeSent',params:{type:'Document',requestId:'request',frameId:'frame',request:{method:'GET',url:'https://u:p@localhost/path?secret=value',headers:{Authorization:'Bearer-secret'},postData:'body-secret'}}});
d.event({method:'Network.responseReceived',params:{type:'Document',requestId:'request',response:{status:200,protocol:'h2',headers:{'Set-Cookie':'cookie-secret'}}}});
d.event({method:'Target.attachedToTarget',params:{targetInfo:{url:'data:text/html,inline-secret'}}});
d.event({method:'Page.frameNavigated',params:{frame:{url:'javascript:script-secret'}}});
d.event({method:'Runtime.bindingCalled',params:{name:'token',payload:'binding-secret'}});
d.event({method:'Runtime.consoleAPICalled',params:{args:[{value:'console-secret'}]}});
d.event({method:'Network.loadingFinished',params:{requestId:'request'}});
console.log(JSON.stringify(d.snapshot(new Error('error-secret'))));
''')
        serialized = json.dumps(record)
        for secret in ('password', 'private', 'fragment', 'value', 'Bearer-secret', 'body-secret', 'cookie-secret', 'binding-secret', 'console-secret', 'error-secret', 'inline-secret', 'script-secret'):
            if secret == 'fragment':
                self.assertNotIn('#fragment', serialized)
            else:
                self.assertNotIn(secret, serialized)
        self.assertEqual(record['requested_location']['origin'], 'https://localhost')
        self.assertEqual(record['requested_location']['path'], '/test.html')
        self.assertTrue(record['requested_location']['query_present'])
        self.assertEqual([item['kind'] for item in record['timeline']], ['document_request','document_response','target_attached','frame_navigated','document_terminal'])

    def test_timeline_pending_and_document_tracking_are_bounded_with_explicit_overflow(self):
        record = self.evaluate('''
const d=createDiagnostics('http://localhost/',{limit:3,clock:()=>0});
for(let i=0;i<10;i++) {d.request(i,'Runtime.enable');d.event({method:'Network.requestWillBeSent',params:{type:'Document',requestId:'r'+i,request:{url:'http://localhost/'+i,method:'GET'}}});}
console.log(JSON.stringify(d.snapshot(new Error('failure'))));
''')
        self.assertEqual(len(record['timeline']), 3)
        self.assertEqual(record['timeline_dropped'], 17)
        self.assertEqual(len(record['pending_cdp']), 3)
        self.assertEqual(record['request_tracking_overflow'], 7)
        self.assertEqual(record['document_tracking_overflow'], 7)
        self.assertEqual(record['pending_cdp'][-1]['id'], 9)

    def test_evicted_navigation_still_retains_definitive_timeout_and_serialization_budget(self):
        record = self.evaluate('''let now=0;const d=createDiagnostics('http://localhost/',{limit:3,clock:()=>now});
d.phase('navigate');const original=d.request(1,'Page.navigate','root');
for(let i=2;i<7;i++)d.request(i,'Runtime.evaluate','child');
now=10000;d.timeout(1,10000,original);d.failure(new Error('CDP timeout: Page.navigate'));
console.log(d.serialize(new Error('failure'),'failure',8192));''')
        self.assertEqual(record['last_timeout']['id'],1)
        self.assertEqual(record['last_timeout']['method'],'Page.navigate')
        self.assertEqual(record['last_timeout']['session_id'],'root')
        self.assertEqual(record['last_timeout']['timeout_ms'],10000)
        self.assertEqual(record['last_timeout']['elapsed_ms'],10000)
        self.assertGreater(record['request_tracking_overflow'],0)
        self.assertLessEqual(len(json.dumps(record).encode()),8192)

    def test_document_terminals_are_scoped_to_session_and_snapshot_budget_is_explicit(self):
        record = self.evaluate('''const d=createDiagnostics('http://localhost/',{limit:256});
for(const sessionId of ['first','second']) d.event({sessionId,method:'Network.requestWillBeSent',params:{type:'Document',requestId:'same',request:{url:'http://localhost/path',method:'GET'}}});
for(const sessionId of ['first','second']) d.event({sessionId,method:'Network.loadingFinished',params:{requestId:'same'}});
for(let i=0;i<100;i++) d.event({method:'Page.frameNavigated',params:{frame:{url:'http://localhost/'+('x'.repeat(2048))}}});
const output=d.serialize(new Error('failure'),'failure',8192);const parsed=JSON.parse(output);
console.log(JSON.stringify({bytes:Buffer.byteLength(output),snapshot:parsed}));''')
        self.assertLessEqual(record['bytes'],8192)
        self.assertGreater(record['snapshot']['serialization_omitted_events'],0)
        # Independent short trace verifies both same-ID sessions are terminated.
        short = self.evaluate('''const d=createDiagnostics('http://localhost/');
for(const sessionId of ['first','second']) d.event({sessionId,method:'Network.requestWillBeSent',params:{type:'Document',requestId:'same',request:{url:'http://localhost/',method:'GET'}}});
for(const sessionId of ['first','second']) d.event({sessionId,method:'Network.loadingFinished',params:{requestId:'same'}});
console.log(JSON.stringify(d.snapshot(new Error('failure'))));''')
        self.assertEqual([row['session_id'] for row in short['timeline'] if row['kind']=='document_terminal'],['first','second'])

    def test_native_browser_stderr_preserves_only_codes_across_chunk_boundaries(self):
        record = self.evaluate('''const d=createDiagnostics('http://localhost/',{clock:()=>0});
const line='[123:456:0918/120000.000:ERROR:content/browser/network_service_instance_impl.cc(99)] Network service crashed net::ERR_CONNECTION_RESET errno=54 https://u:password@localhost/private?token=secret body-private\\n';
d.browserStderr(Buffer.from(line.slice(0,41)));d.browserStderr(Buffer.from(line.slice(41)));
d.browserStderr(Buffer.from('authored-header Authorization: private-token\\n'));d.browserStderrEnd();
console.log(JSON.stringify(d.snapshot(new Error('failure'))));''')
        native=record['browser_stderr']
        self.assertTrue(native['stream_ended'])
        self.assertEqual(native['records'],[dict(at_ms=0,pid=123,severity='ERROR',component='network_service',category='network_service_crashed',net_error='net::ERR_CONNECTION_RESET',os_error=54)])
        self.assertEqual(native['unclassified_lines'],1)
        self.assertEqual(native['bytes_seen'],native['bytes_inspected'])
        for secret in ('password','secret','body-private','private-token','network_service_instance_impl.cc'):
            self.assertNotIn(secret,json.dumps(record))

    def test_native_stderr_budget_keeps_draining_and_reports_omitted_records(self):
        record = self.evaluate('''const d=createDiagnostics('http://localhost/');
const line='[10:20:0918/120000:ERROR:child_process_launcher_helper.cc(1)] launch failed os_error=13\\n';
for(let i=0;i<40;i++)d.browserStderr(Buffer.from(line));
d.browserStderr(Buffer.alloc(80000,120));d.browserStderr(Buffer.from('not retained after cap\\n'));d.browserStderrEnd();
console.log(JSON.stringify(d.snapshot(new Error('failure'))));''')
        native=record['browser_stderr']
        self.assertEqual(len(native['records']),32)
        self.assertEqual(native['records_omitted'],8)
        self.assertEqual(native['bytes_inspected'],65536)
        self.assertEqual(native['bytes_omitted'],native['bytes_seen']-65536)
        self.assertGreater(native['oversized_or_partial_lines'],0)
        self.assertTrue(native['stream_ended'])
        self.assertNotIn('not retained',json.dumps(native))

    def test_native_stderr_snapshot_keeps_navigation_failure_and_serialization_bounds(self):
        record = self.evaluate('''let now=0;const d=createDiagnostics('http://localhost/',{clock:()=>now});
d.phase('navigate');d.request(11,'Page.navigate');
for(let i=0;i<40;i++)d.browserStderr(Buffer.from('[11:22:0918/120000:ERROR:sandbox.cc(5)] Operation not permitted errno=1 private-payload\\n'));
now=10000;d.timeout(11,10000);d.failure(new Error('original'));
for(let i=0;i<100;i++)d.event({method:'Page.frameNavigated',params:{frame:{url:'http://localhost/'+('x'.repeat(2048))}}});
d.phase('cleanup');d.browserStderrEnd();const value=d.serialize(new Error('other'),'cleanup',8192);
console.log(JSON.stringify({bytes:Buffer.byteLength(value),record:JSON.parse(value)}));''')
        self.assertLessEqual(record['bytes'],8192)
        self.assertEqual(record['record']['last_timeout']['timeout_ms'],10000)
        self.assertEqual(record['record']['failure_phase'],'navigate')
        self.assertEqual(record['record']['phase'],'cleanup')
        self.assertTrue(record['record']['browser_stderr']['stream_ended'])
        self.assertNotIn('private-payload',json.dumps(record))


class HttpBoundaryDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.module=server_module()

    def test_accept_parse_dispatch_preserves_handler_metadata_result_and_status(self):
        events=[]; sock=object(); calls=[]
        class Server:
            server_port=8123
            def get_request(self): return sock,('127.0.0.1',4567)
        class Handler:
            def get_request_line(self): return True
            def parse_request(self):
                self.command='GET';self.path='/mounted/test.html?secret=value';return True
            def finish_handling(self,request,response,handler):
                calls.append((handler.base_path,handler.url_base))
                return handler(request,response)
        Server,Handler=self.module.http_diagnostic_classes(Server,Handler,lambda event,**fields:events.append(dict(event=event,**fields)))
        instance=Handler();instance.server=Server();instance.client_address=('127.0.0.1',4567);instance.close_connection=False
        accepted=instance.server.get_request()
        self.assertIs(accepted[0],sock)
        self.assertTrue(instance.get_request_line());self.assertTrue(instance.parse_request())
        request=SimpleNamespace(method='GET',request_path=instance.path)
        response=SimpleNamespace(status=(200,'OK'))
        result=object()
        def actual_handler(req,resp):
            self.assertIs(req,request);self.assertIs(resp,response)
            resp.status=(201,'Created');return result
        actual_handler.base_path='/pinned/mounted';actual_handler.url_base='/mounted'
        self.assertIs(instance.finish_handling(request,response,actual_handler),result)
        self.assertEqual(calls,[('/pinned/mounted','/mounted')])
        self.assertEqual([row['event'] for row in events],['accepted','request_line_wait','request_line_complete','headers_complete','handler_enter','response_ready','dispatch_return'])
        self.assertEqual([row['status'] for row in events if 'status' in row],[201,201])
        self.assertTrue(all(row['peer_port']==4567 for row in events))
        self.assertTrue(all(row['request_sequence']==1 for row in events[1:]))

    def test_http_observer_failure_preserves_original_handler_exception(self):
        original=RuntimeError('original handler failure')
        class Handler:
            def finish_handling(self,request,response,handler): return handler(request,response)
        _,Observed=self.module.http_diagnostic_classes(object,Handler,mock.Mock(side_effect=OSError('diagnostic write failed')))
        instance=Observed();instance.server=SimpleNamespace(server_port=8123)
        instance.client_address=('127.0.0.1',4567);instance.close_connection=False
        def fail(_request,_response): raise original
        with self.assertRaises(RuntimeError) as captured:
            instance.finish_handling(SimpleNamespace(method='GET',request_path='/test'),SimpleNamespace(status=(200,'OK')),fail)
        self.assertIs(captured.exception,original)

    def test_http_writer_bounds_redacts_and_emits_one_truncation_marker(self):
        stream=io.StringIO();emit=self.module.http_diagnostic_writer(stream,limit=2)
        emit('headers_complete',server_port=8123,peer_port=4567,method='GET',path='http://u:password@localhost/test?token=secret#fragment',headers={'Authorization':'private'},body='body-secret')
        emit('headers_complete',path='data:opaque-secret')
        for _ in range(5):emit('accepted',peer_port=4567)
        records=[json.loads(line.split(' ',1)[1]) for line in stream.getvalue().splitlines()]
        self.assertEqual(len(records),3)
        self.assertEqual(records[0]['path'],'/test')
        self.assertTrue(records[0]['query_present']);self.assertTrue(records[0]['fragment_present'])
        self.assertTrue(records[1]['opaque_target'])
        self.assertEqual(records[2]['event'],'records_truncated')
        self.assertEqual(records[2]['limit'],2)
        for secret in ('password','secret','private','Authorization','data:'):
            self.assertNotIn(secret,stream.getvalue())
        broken=self.module.http_diagnostic_writer(mock.Mock(write=mock.Mock(side_effect=OSError('full'))))
        broken('accepted',peer_port=4567)
        for character in ('\u2603','\U0001f9ed','\x01'):
            bounded=io.StringIO();self.module.http_diagnostic_writer(bounded)('headers_complete',path='/'+(character*2048))
            self.assertLess(len(bounded.getvalue().encode()),8192)
            parsed=fixture_module().select_http_diagnostics(bounded.getvalue())
            self.assertEqual(len(parsed),1);self.assertTrue(parsed[0]['path_truncated'])
            self.assertEqual(parsed[0]['path'],'/'+character*511)

    def test_missing_handler_keeps_upstream_404_result(self):
        events=[]
        class Handler:
            def finish_handling(self,request,response,handler):
                self.asserted_none=handler is None;response.status=(404,'Not Found');return 'original'
        _,Observed=self.module.http_diagnostic_classes(object,Handler,lambda event,**fields:events.append(dict(event=event,**fields)))
        instance=Observed();instance.server=SimpleNamespace(server_port=8123)
        instance.client_address=('127.0.0.1',4567);instance.close_connection=True
        response=SimpleNamespace(status=(200,'OK'))
        self.assertEqual(instance.finish_handling(object(),response,None),'original')
        self.assertTrue(instance.asserted_none)
        self.assertEqual(events,[dict(event='dispatch_return',status=404,close_connection=True,server_port=8123,peer_port=4567,request_sequence=0)])


class ChromiumFixtureFailureTests(unittest.TestCase):
    def setUp(self):
        self.environment = mock.patch.dict(os.environ, {'ORYN_WPT_CHECKOUT':'/fixture/wpt',
            'ORYN_WPT_NODE':'/fixture/node','ORYN_WPT_CHROMIUM':'/fixture/CfT'})
        self.environment.start(); self.addCleanup(self.environment.stop)
        self.module = fixture_module()
        self.log = tempfile.TemporaryFile(mode='w+')
        self.addCleanup(self.log.close)
        self.log.write('server ready\n127.0.0.1 - - [now] "GET /target.html?token=secret HTTP/1.1" 200 -\n')
        self.log.flush()
        self.fixture = mock.Mock(ports=[8123,8443,8124,8444,9000,8888,8889],
            generations=[{'id':'generation','backend':'chromium'}],timings={'startup_ms':1,'teardown_ms':0},
            shutdown_complete=False,failure=None,log=self.log)
        self.fixture.process.pid=321;self.fixture.process.poll.return_value=None

    def test_server_log_snapshot_is_bounded_sanitized_and_does_not_move_writer_offset(self):
        position = self.log.tell()
        record = self.module.server_snapshot(self.fixture)
        self.assertEqual(self.log.tell(), position)
        self.assertEqual(record['pid'],321)
        self.assertTrue(record['request_log_available'])
        self.assertEqual(record['request_log'],[{'method':'GET','path':'/target.html','path_truncated':False,'query_present':True,'status':200}])
        self.assertNotIn('secret',json.dumps(record))
        self.assertNotIn('server ready',json.dumps(record))

    def test_actual_upstream_single_quote_logs_and_structured_boundaries_are_retained(self):
        stream=io.StringIO();emit=server_module().http_diagnostic_writer(stream)
        emit('accepted',server_port=8123,peer_port=4567)
        emit('headers_complete',server_port=8123,peer_port=4567,request_sequence=1,method='GET',path='/actual.html?secret=hidden',valid=True)
        emit('response_ready',server_port=8123,peer_port=4567,request_sequence=1,status=200)
        self.log.write("127.0.0.1 - - [now] 'GET /upstream.html?secret=hidden HTTP/1.1' 200 -\n")
        self.log.write(stream.getvalue());self.log.flush()
        position=self.log.tell()
        result=self.module.server_snapshot(self.fixture)
        self.assertEqual(self.log.tell(),position)
        self.assertEqual(result['request_log'][-1],dict(method='GET',path='/upstream.html',path_truncated=False,query_present=True,status=200))
        self.assertTrue(result['http_boundary_log_available'])
        self.assertEqual([row['event'] for row in result['http_boundaries']],['accepted','headers_complete','response_ready'])
        self.assertEqual(result['http_boundaries'][1]['path'],'/actual.html')
        self.assertEqual(result['http_boundaries'][-1]['status'],200)
        self.assertNotIn('hidden',json.dumps(result))

    def test_boundary_parser_drops_unknown_payloads_and_retains_truncation(self):
        lines=[]
        for value in (dict(schema_version=1,kind='wpt_http_boundary',event='headers_complete',method='GET',path='/page?token=secret',headers={'Authorization':'private'},body='payload'),
                      dict(schema_version=1,kind='wpt_http_boundary',event='records_truncated',limit=256),
                      dict(schema_version=1,kind='wpt_http_boundary',event='authored-secret',path='/ignored')):
            lines.append('ORYN_WPT_HTTP_DIAGNOSTIC '+json.dumps(value))
        result=self.module.select_http_diagnostics('\n'.join(lines))
        self.assertEqual([row['event'] for row in result],['headers_complete','records_truncated'])
        self.assertEqual(result[0]['path'],'/page');self.assertTrue(result[0]['query_present'])
        self.assertEqual(result[1]['limit'],256)
        for value in ('secret','private','payload','ignored','Authorization'):
            self.assertNotIn(value,json.dumps(result))

    def test_fixture_enables_boundary_records_only_for_chromium_backend(self):
        fixture_class=self.module.DirectFixture
        owner=sys.modules[fixture_class.__module__]
        commands=[]
        for backend in ('oryn','chromium'):
            fixture=fixture_class.__new__(fixture_class)
            fixture.process=None;fixture.failure=None;fixture.reservations=[]
            fixture.checkout=Path('/fixture/pinned-wpt');fixture.ports=[8123,8443,8124,8444,9000,8888,8889]
            fixture.timings={'startup_ms':0,'teardown_ms':0};fixture.generations=[]
            process=mock.Mock();process.stdout.readline.return_value=json.dumps({'port':8123,'https_port':8443})
            with mock.patch.object(owner,'fixture_certificate',return_value={'certificate':'cert','key':'key'}), \
                 mock.patch.object(owner.subprocess,'Popen',return_value=process) as popen, \
                 mock.patch.object(owner.select,'select',return_value=([process.stdout],[],[])):
                fixture.start(backend)
                commands.append(popen.call_args.args[0])
            fixture.log.close()
        self.assertNotIn('--http-diagnostics',commands[0])
        self.assertEqual(commands[1].count('--http-diagnostics'),1)
        self.assertEqual(commands[0][commands[0].index('--public-port')+1],commands[1][commands[1].index('--public-port')+1])

    def test_selected_native_codes_survive_without_raw_browser_stderr(self):
        record=dict(schema_version=1,kind='chromium_reference_failure',phase='navigate',requested_location={'origin':'http://localhost','path':'/'},
                    browser_stderr=dict(bytes_seen=200,bytes_inspected=200,bytes_omitted=0,stream_ended=True,raw='private stderr',records=[
                        dict(pid=123,component='network_service',severity='ERROR',category='network_service_crashed',net_error='net::ERR_FAILED',os_error=13,message='private-message',headers={'Authorization':'secret'})]),
                    timeline=[],pending_cdp=[])
        result=self.module.select_canary_diagnostics('ORYN_CHROMIUM_DIAGNOSTIC '+json.dumps(record)+'\n')
        self.assertEqual(result[0]['browser_stderr']['records'],[dict(pid=123,component='network_service',severity='ERROR',category='network_service_crashed',net_error='net::ERR_FAILED',os_error=13)])
        self.assertTrue(result[0]['browser_stderr']['stream_ended'])
        for secret in ('private','secret','Authorization','headers'):
            self.assertNotIn(secret,json.dumps(result))

    def test_failure_retains_actual_later_path_and_original_nonzero_assertion(self):
        passed = dict(completion={'harness':0,'tests':[{'status':0}]},
                      automation_events=[{'outcome':'started'}],automation_bridge_errors=[])
        results = [subprocess.CompletedProcess([],0,json.dumps(passed),'') for _ in range(3)]
        results.append(subprocess.CompletedProcess([],1,'','CDP timeout: Page.navigate'))
        case=self.module.DirectFixtureTests('test_chromium_click_and_send_keys_reference');case.fixture=self.fixture
        stream=io.StringIO()
        with mock.patch.object(self.module.subprocess,'run',side_effect=results) as run, mock.patch.object(self.module.sys,'stderr',stream):
            with self.assertRaises(AssertionError):case.test_chromium_click_and_send_keys_reference()
        self.assertEqual(run.call_count,4)
        record=json.loads(stream.getvalue().split('ORYN_CHROMIUM_FIXTURE_DIAGNOSTIC ',1)[1])
        self.assertEqual(record['fixture_path'],'infrastructure/testdriver/actions/actionsWithKeyPressed.html')
        self.assertIsNone(record['sample']);self.assertEqual(record['returncode'],1)
        self.assertEqual(record['stderr_bytes'],len('CDP timeout: Page.navigate'))
        self.assertNotIn('stderr',record)
        self.assertEqual(run.call_args.args[0][-1],'10000')
        self.assertEqual(run.call_args.kwargs['timeout'],35)

    def test_failure_retains_later_reentrant_sample_without_changing_18_event_assertions(self):
        events=[{'id':identifier,'outcome':phase} for identifier in range(1,10) for phase in ('started','completed')]
        passed=dict(completion={'harness':0,'tests':[{'status':0}],'testdriver':events},automation_events=events)
        results=[subprocess.CompletedProcess([],0,json.dumps(passed),'') for _ in range(2)]
        results.append(subprocess.CompletedProcess([],1,'','CDP timeout: Page.navigate'))
        case=self.module.DirectFixtureTests('test_chromium_completion_drains_reentrant_send_keys');case.fixture=self.fixture
        stream=io.StringIO()
        with mock.patch.object(self.module.subprocess,'run',side_effect=results), mock.patch.object(self.module.sys,'stderr',stream):
            with self.assertRaises(AssertionError):case.test_chromium_completion_drains_reentrant_send_keys()
        record=json.loads(stream.getvalue().split('ORYN_CHROMIUM_FIXTURE_DIAGNOSTIC ',1)[1])
        self.assertEqual(record['sample'],2)
        self.assertTrue(record['fixture_path'].endswith('/focus-tabindex-order.html'))
        self.assertEqual(record['returncode'],1)

    def test_outer_timeout_preserves_original_exception_and_partial_stderr(self):
        error=subprocess.TimeoutExpired(['canary'],35,stderr=b'partial navigation diagnostics')
        stream=io.StringIO()
        with mock.patch.object(self.module.subprocess,'run',side_effect=error), mock.patch.object(self.module.sys,'stderr',stream):
            with self.assertRaises(subprocess.TimeoutExpired) as observed:
                self.module.run_chromium_reference(self.fixture,'explicit.html','http://localhost:8123/explicit.html')
        self.assertIs(observed.exception,error)
        record=json.loads(stream.getvalue().split('ORYN_CHROMIUM_FIXTURE_DIAGNOSTIC ',1)[1])
        self.assertEqual(record['fixture_path'],'explicit.html')
        self.assertEqual(record['exception'],'TimeoutExpired')
        self.assertEqual(record['stderr_bytes'],len('partial navigation diagnostics'))
        self.assertNotIn('stderr',record)


    def test_diagnostic_io_failure_does_not_replace_original_timeout(self):
        error=subprocess.TimeoutExpired(['canary'],35,stderr=b'partial')
        with mock.patch.object(self.module.subprocess,'run',side_effect=error), \
             mock.patch.object(self.module.sys,'stderr',mock.Mock(write=mock.Mock(side_effect=OSError('unavailable')))):
            with self.assertRaises(subprocess.TimeoutExpired) as observed:
                self.module.run_chromium_reference(self.fixture,'explicit.html','http://localhost:8123/explicit.html')
        self.assertIs(observed.exception,error)

    def test_retained_structured_stderr_drops_unknown_payload_fields(self):
        raw='authored raw-stack secret\nORYN_CHROMIUM_DIAGNOSTIC '+json.dumps(dict(
            schema_version=1,kind='chromium_reference_failure',phase='navigate',
            requested_location={'scheme':'data:','path':'inline-secret'},error={'name':'Error','message':'message-secret'},
            timeline=[{'kind':'frame_navigated','location':{'scheme':'javascript:','path':'script-secret'},'payload':'body-secret'}],
            pending_cdp=[],headers={'Authorization':'header-secret'}))+'\n'
        record=self.module.select_canary_diagnostics(raw)
        self.assertEqual(len(record),1)
        for secret in ('raw-stack','inline-secret','message-secret','script-secret','body-secret','header-secret'):
            self.assertNotIn(secret,json.dumps(record))
        self.assertTrue(record[0]['requested_location']['opaque_body_redacted'])

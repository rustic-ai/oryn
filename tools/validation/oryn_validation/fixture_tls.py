"""Pinned WPT H1 normal-response TLS close_notify, without changing HTTP bytes."""
import hashlib,importlib.abc,importlib.machinery,json,os,re,ssl,sys,threading,time
from pathlib import Path
CONTRACT='ORYN_FIXTURE_TLS_CONTRACT'
DIGEST='ORYN_FIXTURE_TLS_CONTRACT_SHA256'
RECEIPTS='ORYN_FIXTURE_TLS_RECEIPTS'
ROOT=Path(__file__).resolve().parents[3]
_INSTALLED=False

def record(path):
    p=Path(path).resolve()
    return {'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
def checked(item):
    if record(item['path'])!=item:raise ValueError('fixture bootstrap input drift: '+item['path'])
def bootstrap_inputs(path,digest):
    """Verify only the finite child bootstrap roles; the owner leases all inputs.

    The coordinator passes the exact admitted run digest. We parse that strict
    manifest, but never hash or rediscover its dataset/build directory closures
    in a Python multiprocessing child.
    """
    from .manifests import load_manifest
    from .inputs import resolve_input, verify_inputs
    from .admission import read_object
    run_path=os.environ.get('ORYN_WPT_RUN_CONTRACT')
    run_digest=os.environ.get('ORYN_WPT_RUN_CONTRACT_SHA256')
    store=os.environ.get('ORYN_VALIDATION_STORE_ROOT')
    bundle=os.environ.get('ORYN_VALIDATION_BUNDLE_ROOT')
    if not run_path or not run_digest or not store or not bundle:
        raise ValueError('fixture TLS bootstrap needs the admitted typed run and digest')
    if record(run_path)['sha256']!=run_digest:
        raise ValueError('fixture bootstrap run manifest differs from admitted identity')
    run=load_manifest(run_path,expected_kind='run')
    records=run['assets'];assets={}
    def bind(name,kind='file'):
        item=records[name]
        if item['kind']!=kind:
            raise ValueError('bootstrap asset kind differs: '+name)
        root=bundle if item['scope']=='bundle' else store
        target=resolve_input(root,item['path'])
        if kind=='file':verify_inputs([item],store_root=store,bundle_root=bundle)
        assets[name]=target
        return target
    policy_path=bind('fixture_tls_policy')
    if Path(path).resolve()!=policy_path or record(policy_path)['sha256']!=digest:
        raise ValueError('fixture TLS policy differs from admitted identity')
    policy=read_object(policy_path)
    registry=read_object(bind('core_profile_registry'))
    fixture_name=registry['profiles'][run['profile']]['fixture']
    bind(fixture_name)
    for name in ('python','fixture_tls_module','fixture_tls_bootstrap',
                 policy['certificate'],policy['response_module'],policy['handlers_module'],policy['pipes_module'],
                 *policy['modules'].values()):
        bind(name)
    bind('wpt_checkout','directory')
    if bind('runner_source','directory')!=ROOT:
        raise ValueError('child bootstrap differs from admitted runner export')
    if record(run_path)['sha256']!=run_digest:
        raise ValueError('fixture bootstrap run changed while loading')
    return {'run':run,'assets':assets,'path':Path(run_path),'sha256':run_digest,
            'store_root':Path(store),'bundle_root':Path(bundle)}


def load_contract(path,digest,bound=None):
    """Resolve one explicit current policy; no generation or historical graph."""
    from .admission import read_object
    if bound is None:
        bound=bootstrap_inputs(path,digest)
    assets=bound['assets'];run=bound['run']
    if Path(path).resolve()!=assets.get('fixture_tls_policy') or record(path)['sha256']!=digest:
        raise ValueError('fixture TLS policy differs from typed run asset')
    value=read_object(path)
    fields={'schema_version','kind','unwrap_timeout_seconds','modules','response_module','handlers_module','pipes_module','certificate','bootstrap_inputs'}
    if (set(value)!=fields or value['schema_version']!=2
            or value['kind']!='wpt_h1_normal_completion_tls_shutdown' or value['unwrap_timeout_seconds']!=1
            or set(value['modules'])!={'wptserve.server','wptrunner.environment'}
            or value['bootstrap_inputs']!=['fixture_tls_module','fixture_tls_bootstrap']):
        raise ValueError('unsupported typed fixture TLS policy')
    if record(sys.executable)['sha256']!=record(assets['python'])['sha256']:
        raise ValueError('fixture bootstrap interpreter differs')
    registry=read_object(assets['core_profile_registry'])
    fixture_path=assets[registry['profiles'][run['profile']]['fixture']]
    fixture=read_object(fixture_path)
    if assets[value['certificate']]!=(fixture_path.parent/fixture['tls']['certificate']['path']).resolve():
        raise ValueError('fixture bootstrap certificate differs from selected profile')
    expected_modules={'wptserve.server':'tools/wptserve/wptserve/server.py',
                      'wptrunner.environment':'tools/wptrunner/wptrunner/environment.py'}
    modules={name:record(assets[asset]) for name,asset in value['modules'].items()}
    for name,relative in expected_modules.items():
        if Path(modules[name]['path'])!=assets['wpt_checkout']/relative:
            raise ValueError('fixture bootstrap target module differs from pinned upstream')
    for key,relative in {'response_module':'response.py','handlers_module':'handlers.py','pipes_module':'pipes.py'}.items():
        if assets[value[key]]!=assets['wpt_checkout']/'tools/wptserve/wptserve'/relative:
            raise ValueError('fixture bootstrap dependency differs from pinned upstream')
    if (assets['fixture_tls_module']!=ROOT/'tools/validation/oryn_validation/fixture_tls.py'
            or assets['fixture_tls_bootstrap']!=ROOT/'tools/validation/bootstrap/sitecustomize.py'):
        raise ValueError('fixture bootstrap code differs from committed runner source')
    receipt_inputs={name:run['assets'][asset] for name,asset in {
        'contract':'fixture_tls_policy','server_module':value['modules']['wptserve.server'],
        'response_module':value['response_module'],'certificate':value['certificate'],
        'bootstrap_module':'fixture_tls_module','bootstrap_site':'fixture_tls_bootstrap'}.items()}
    return dict(value, receipt_inputs=receipt_inputs, modules=modules, certificate=record(assets[value['certificate']]),
                **{key:record(assets[value[key]]) for key in ('response_module','handlers_module','pipes_module')},
                bootstrap_inputs=[record(assets[name]) for name in value['bootstrap_inputs']],
                bootstrap_directory=str(ROOT/'tools/validation/bootstrap'))

def identity(pid):
    if str(ROOT/'scripts') not in sys.path:sys.path.insert(0,str(ROOT/'scripts'))
    from completion_process_supervisor import process_identity
    return process_identity(pid)

def prepare_environment(base,contract_path,digest,receipt_dir):
    value=load_contract(contract_path,digest)
    directory=Path(receipt_dir).resolve()
    if not directory.is_dir():raise ValueError('fresh owner-created receipt directory required')
    env=dict(base);env[CONTRACT]=str(Path(contract_path).resolve());env[DIGEST]=digest;env[RECEIPTS]=str(directory)
    parts=[value['bootstrap_directory']]+[p for p in env.get('PYTHONPATH','').split(os.pathsep) if p and p!=value['bootstrap_directory']]
    env['PYTHONPATH']=os.pathsep.join(parts);env['PYTHONDONTWRITEBYTECODE']='1'
    return env

def verify_receipts(directory,contract_path,digest,expected_ports,require_live=True,expected_processes=None,bound=None):
    value=load_contract(contract_path,digest,bound=bound);found={}
    # Startup listens before __init__ has published its receipt. Wait only for
    # missing expected ports; malformed, duplicate or mismatched receipts fail.
    deadline=time.monotonic()+10
    while require_live:
        present={json.loads(p.read_text()).get('port') for p in Path(directory).glob('server-*.json')}
        if set(expected_ports)<=present:break
        if time.monotonic()>=deadline:raise ValueError('required actual TLS server receipt missing after bounded readiness wait')
        time.sleep(.05)
    for path in sorted(Path(directory).glob('server-*.json')):
        receipt=json.loads(path.read_text())
        if receipt.get('schema_version')!=2 or receipt.get('input_identities')!=value['receipt_inputs']:
            raise ValueError('server receipt typed input identities mismatch')
        expected={'contract':record(contract_path),'certificate':value['certificate'],
                  'server_module':value['modules']['wptserve.server'],'response_module':value['response_module']}
        for name,item in expected.items():
            observed=receipt.get(name)
            if (not isinstance(observed,dict) or set(observed)!={'path','sha256'}
                    or observed['sha256']!=item['sha256'] or not Path(observed['path']).is_absolute()
                    or (require_live and observed!=item)):
                raise ValueError('server receipt contract/source mismatch: '+name)
        observed=receipt.get('bootstrap_inputs')
        if (not isinstance(observed,list) or len(observed)!=len(value['bootstrap_inputs'])
                or any(not isinstance(old,dict) or set(old)!={'path','sha256'}
                       or old['sha256']!=current['sha256'] or not Path(old['path']).is_absolute()
                       or (require_live and old!=current)
                       for old,current in zip(observed,value['bootstrap_inputs']))):
            raise ValueError('server receipt bootstrap source mismatch')
        port=receipt['port']
        if port in found:raise ValueError('duplicate server receipt for port')
        if receipt.get('mode') not in ('normal_h1_tls_shutdown','http2_unchanged','connect_tls_unchanged') or not receipt.get('birth_identity'):
            raise ValueError('invalid TLS server receipt')
        if require_live:
            observed=identity(receipt['pid'])
            if observed is None or list(observed[0])!=receipt['birth_identity']:raise ValueError('server receipt process no longer current')
        if expected_processes is not None and port in expected_processes and receipt['pid']!=expected_processes[port]:
            raise ValueError('receipt does not belong to actual upstream child')
        if receipt.get('kind')!='actual_tls_server_bootstrap':
            raise ValueError('server receipt upstream module mismatch')
        found[port]=receipt
    if not set(expected_ports)<=set(found):raise ValueError('required actual TLS server receipt missing')
    for port in expected_ports:
        if found[port]['mode']!='normal_h1_tls_shutdown':raise ValueError('required H1 TLS port not hooked')
    return [found[p] for p in expected_ports]

class _Writes:
    """Observe successful high-level writes, retaining only bounded headers."""
    def __init__(self,target):
        self.target=target;self.prefix=bytearray();self.total=0;self.failed=False;self.complete=False;self.tainted=False;self.active=True
    def write(self,data):
        try:
            count=self.target.write(data)
            if count is not None and count!=len(data):self.failed=True
            self.total+=len(data)
            if len(self.prefix)<65536:self.prefix.extend(data[:65536-len(self.prefix)])
            return count
        except BaseException:
            self.failed=True;raise
    def __getattr__(self,name):return getattr(self.target,name)
    def normal(self,method):
        if self.failed or self.tainted or not self.complete:return False
        head,separator,_=bytes(self.prefix).partition(b'\r\n\r\n')
        if not separator:return False
        lines=head.split(b'\r\n')
        match=re.fullmatch(rb'HTTP/1\.[01] ([2-5][0-9][0-9]) [^\r\n]*',lines[0])
        if match is None:return False
        fields={}
        for line in lines[1:]:
            name,colon,val=line.partition(b':')
            if not colon or re.fullmatch(rb"[!#$%&'*+.^_`|~0-9A-Za-z-]+",name) is None:return False
            val=val.strip(b' \t')
            if any(c<32 and c!=9 or c==127 for c in val):return False
            fields.setdefault(name.lower(),[]).append(val)
        if b'transfer-encoding' in fields or b'upgrade' in fields:return False
        length=fields.get(b'content-length',[])
        if len(length)>1 or length and (len(length[0])>20 or re.fullmatch(rb'[0-9]+',length[0]) is None):return False
        body=self.total-len(head)-4;status=int(match[1])
        if method=='HEAD' or status in (204,304):return body==0
        return not length or int(length[0])==body

def _patch_server(module,value,path,digest,directory):
    import wptserve.response as response_module
    import wptserve.handlers as handlers_module
    import wptserve.pipes as pipes_module
    for module_,key in ((handlers_module,'handlers_module'),(pipes_module,'pipes_module')):
        checked(value[key])
        if Path(module_.__file__).resolve()!=Path(value[key]['path']):raise ValueError('static handler dependency path mismatch')
    FileHandler=handlers_module.FileHandler
    file_methods={name:getattr(FileHandler,name) for name in ('__call__','get_headers','get_data')}
    pinned_pipes=dict(pipes_module.Pipeline.pipes)
    checked(value['response_module'])
    if Path(response_module.__file__).resolve()!=Path(value['response_module']['path']):raise ValueError('response module path mismatch')
    Response=response_module.Response;Writer=response_module.ResponseWriter
    original_write=Response.write;original_writer_write=Writer.write;original_file_write=Writer.write_content_file
    original_status=Response.write_status_headers;original_content=Response.write_content
    original_finish=module.BaseWebTestRequestHandler.finish_handling
    original_one=module.Http1WebTestRequestHandler.handle_one_request
    original_shutdown=module.WebTestServer.shutdown_request
    original_init=module.WebTestServer.__init__
    def writer_write(self,data):
        state=getattr(self,'_oryn_normal_write',None)
        if state is not None and not state.active:state.tainted=True
        return original_writer_write(self,data)
    def writer_file(self,data):
        state=getattr(self,'_oryn_normal_write',None)
        if state is not None and not state.active:state.tainted=True
        return original_file_write(self,data)
    def write(self):
        handler=self.writer._handler
        if (not getattr(handler,'_oryn_auto_response',False) or type(self) is not Response or type(self.writer) is not Writer or self.writer.content_written
                or getattr(handler.server,'scheme',None)!='https' or not isinstance(handler.connection,ssl.SSLSocket)
                or handler.server.encrypt_after_connect or getattr(handler,'wbufsize',None)!=0
                or getattr(self.write_status_headers,'__func__',None) is not original_status
                or getattr(self.write_content,'__func__',None) is not original_content):
            return original_write(self)
        state=_Writes(self.writer._wfile);self.writer._oryn_normal_write=state;self.writer._wfile=state
        try:
            result=original_write(self);state.complete=True;return result
        finally:
            state.active=False;self.writer._wfile=state.target
    class ReturnedHandler:
        def __init__(self,original,owner):self.original=original;self.owner=owner
        def __getattr__(self,name):return getattr(self.original,name)
        def __call__(self,*args,**kwargs):
            result=self.original(*args,**kwargs)
            self.owner._oryn_auto_response=(type(self.original) is FileHandler
                and all(getattr(getattr(self.original,name,None),'__func__',None) is function for name,function in file_methods.items())
                and pipes_module.Pipeline.pipes==pinned_pipes)
            return result
    def finish(self,request,response,handler):
        # Only exact pinned FileHandler automatic output is eligible.
        # Handler-managed writes, raw socket APIs and deliberate manual framing
        # retain their original abrupt-close behavior, even if bytes look valid.
        self._oryn_auto_response=False
        selected=ReturnedHandler(handler,self) if handler is not None else None
        result=original_finish(self,request,response,selected)
        state=getattr(response.writer,'_oryn_normal_write',None)
        if state is not None and state.normal(request.method) and self.close_connection:
            self.connection._oryn_normal_tls_complete=True
        return result
    def one(self):
        tls=self.server.scheme=='https' and isinstance(self.connection,ssl.SSLSocket)
        if tls:self.connection._oryn_normal_tls_complete=False
        self._oryn_auto_response=False
        try:return original_one(self)
        except BaseException:
            if tls:self.connection._oryn_normal_tls_complete=False
            raise
    def shutdown(self,request):
        timeout=None;changed=False
        try:
            if (self.scheme=='https' and not self.encrypt_after_connect and isinstance(request,ssl.SSLSocket)
                    and getattr(request,'_oryn_normal_tls_complete',False)):
                timeout=request.gettimeout();changed=True
                try:
                    request.settimeout(value['unwrap_timeout_seconds']);request=request.unwrap()
                except (OSError,ValueError):pass
        finally:
            try:
                if changed:request.settimeout(timeout)
            except (OSError,ValueError):pass
            finally:original_shutdown(self,request)
    def init(self,*args,**kwargs):
        original_init(self,*args,**kwargs)
        if self.scheme not in ('https','http2'):return
        try:
            birth=identity(os.getpid())
            if birth is None:raise ValueError('TLS server birth identity absent')
            receipt={'schema_version':2,'input_identities':value['receipt_inputs'],'kind':'actual_tls_server_bootstrap','pid':os.getpid(),'ppid':os.getppid(),'birth_identity':list(birth[0]),'address':list(self.socket.getsockname()),'port':self.socket.getsockname()[1],'mode':('normal_h1_tls_shutdown' if not self.encrypt_after_connect else 'connect_tls_unchanged') if self.scheme=='https' else 'http2_unchanged','contract':record(path),'bootstrap_inputs':value['bootstrap_inputs'],'server_module':value['modules']['wptserve.server'],'response_module':value['response_module'],'certificate':record(self.certificate),'qualified':False}
            if receipt['certificate']!=value['certificate']:raise ValueError('TLS server certificate differs from contract')
            name=f"server-{os.getpid()}-{receipt['port']}-{birth[0][0]}-{birth[0][1]}.json"
            temporary=directory/(name+'.publishing')
            try:
                with temporary.open('x') as f:json.dump(receipt,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
                os.link(temporary,directory/name)
            finally:
                if temporary.exists():temporary.unlink()
        except BaseException:
            self.server_close();raise
    Writer.write=writer_write;Writer.write_content_file=writer_file;Response.write=write
    module.BaseWebTestRequestHandler.finish_handling=finish
    module.Http1WebTestRequestHandler.handle_one_request=one
    module.WebTestServer.shutdown_request=shutdown;module.WebTestServer.__init__=init

def _patch_environment(module,value,path,digest,directory):
    original=module.TestEnvironment.ensure_started
    def ensure(self):
        result=original(self);processes={}
        for scheme,servers in self.servers.items():
            if scheme.startswith('https'):
                for port,server in servers:processes[port]=server.proc.pid
        if not processes:raise ValueError('official environment lacks expected HTTPS servers')
        verify_receipts(directory,path,digest,sorted(processes),require_live=True,expected_processes=processes)
        return result
    module.TestEnvironment.ensure_started=ensure

class _Loader(importlib.abc.Loader):
    def __init__(self,loader,fullname,value,path,digest,directory):self.loader=loader;self.fullname=fullname;self.value=value;self.path=path;self.digest=digest;self.directory=directory
    def create_module(self,spec):return self.loader.create_module(spec) if hasattr(self.loader,'create_module') else None
    def exec_module(self,module):
        checked(self.value['modules'][self.fullname]);self.loader.exec_module(module)
        if self.fullname=='wptserve.server':_patch_server(module,self.value,self.path,self.digest,self.directory)
        else:_patch_environment(module,self.value,self.path,self.digest,self.directory)
class _Finder(importlib.abc.MetaPathFinder):
    def __init__(self,value,path,digest,directory):self.value=value;self.path=path;self.digest=digest;self.directory=directory
    def find_spec(self,fullname,path=None,target=None):
        if fullname not in self.value['modules']:return None
        spec=importlib.machinery.PathFinder.find_spec(fullname,path)
        if spec is None or spec.loader is None or str(Path(spec.origin).resolve())!=self.value['modules'][fullname]['path']:
            raise ImportError('unexpected fixture module origin: '+fullname)
        spec.loader=_Loader(spec.loader,fullname,self.value,self.path,self.digest,self.directory);return spec

def install():
    global _INSTALLED
    if _INSTALLED:return
    path=os.environ[CONTRACT];digest=os.environ[DIGEST];directory=Path(os.environ[RECEIPTS]).resolve()
    if not directory.is_dir():raise ValueError('fixture receipt directory missing')
    value=load_contract(path,digest)
    if any(name in sys.modules for name in value['modules']):raise ValueError('fixture bootstrap must precede target module import')
    sys.meta_path.insert(0,_Finder(value,path,digest,directory));_INSTALLED=True

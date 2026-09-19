"""Concrete shared-parent adapters using existing family tasks and oracle validators.

Native activity is read from the native executor. Python wall times, worker
counts, and fixture-request times are never substituted for native operation spans.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
from generation_gate import check_execution_generation
import subprocess
import threading

from workflow_failures import attribute_native_failure
from workflow_frontend_evidence import WITNESSES, source_records
from workflow_resources import ProcessSampler, reconcile_resources
from workflow_fault_recipes import PREFERRED, decorate as decorate_fault
from workflow_family_recipes import RECIPES, DIMENSIONS, observed as project_observed, expected as project_expected, forms_expected

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT/'test-harness/workflows'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


support = load('native_adapter_support', ROOT/'scripts/run-oil-forms-outcomes.py')
research = load('native_adapter_research', ROOT/'scripts/run-oil-research-outcomes.py')
RESEARCH_RECIPES = {
    'research.stale': [(3,'generation_bound_stale_reference')],
    'research.compare': [(1, 'search_paginate_compare')],
    'research.pagination': [(1, 'dynamic_results'), (1, 'refreshed_results'), (2, 'hydrated_pagination')],
    'research.frames': [(3, 'nested_cross_origin_extraction')],
    'research.missing': [(1, 'empty_results'), (1, 'missing_source_recovery'), (1, 'redirected_source')],
}


def artifact(root, path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2)+'\n')
    return dict(path=str(path.relative_to(root)), sha256=hashlib.sha256(path.read_bytes()).hexdigest())


def native_activity(evidence, expected_contexts):
    audit = evidence.get('native_activity', {})
    all_sessions = evidence.get('sessions', [])
    sessions=[row for row in all_sessions if row.get('adversarial_for') is None]
    owners={row.get('id') for row in sessions}
    if any(row.get('adversarial_for') is not None and row['adversarial_for'] not in owners for row in all_sessions):
        raise ValueError('unattributed adversarial context')
    if len({row.get('context_id') for row in all_sessions})!=len(all_sessions):
        raise ValueError('duplicate context identity across cohort and probes')
    if audit.get('schema_version') != 1 or audit.get('overflow') or len(sessions) != expected_contexts:
        raise ValueError('missing complete native activity evidence')
    identities = {row.get('context_id') for row in sessions}
    if len(identities) != expected_contexts or any(type(value) is not int for value in identities):
        raise ValueError('duplicate or malformed native context identity')
    spans = []
    for span in audit.get('operations', []):
        if (span.get('context_id') not in identities or type(span.get('start_ns')) is not int
                or type(span.get('end_ns')) is not int or span['end_ns'] <= span['start_ns']
                or span.get('terminal') != 'completed'):
            continue
        spans.append(span)
    # Find an actual simultaneous set of completed native operations. Envelopes
    # spanning idle gaps would manufacture overlap, so use one real span/context.
    for boundary in sorted({span['start_ns'] for span in spans}):
        active = {}
        for span in spans:
            if span['start_ns'] <= boundary < span['end_ns']:
                active.setdefault(span['context_id'], span)
        if set(active) == identities:
            return [dict(context_id=str(identity),start_ns=active[identity]['start_ns'],
                         end_ns=active[identity]['end_ns'],operation_id=active[identity]['operation_id'])
                    for identity in sorted(identities)]
    raise ValueError('required contexts had no measured simultaneous native operation')


def execute_native(binary, worker, plan, directory, stop):
    check_execution_generation()
    if stop.is_set():
        raise RuntimeError('cancelled before native execution')
    task = directory/'native-plan.json'
    task.write_text(json.dumps(plan, indent=2)+'\n')
    stdout_path, stderr_path = directory/'native.stdout', directory/'native.stderr'
    with stdout_path.open('xb') as stdout, stderr_path.open('xb') as stderr:
        check_execution_generation()
        process = subprocess.Popen([str(binary),'--input',str(task),'--output',str(directory/'native-evidence.json')],
                                   stdout=stdout,stderr=stderr,start_new_session=True,
                                   env=dict(os.environ,ORYN_PAGE_WORKER=str(worker)))
        sampler=None;watcher=None
        cleanup_errors=[]
        resource_samples=dict(schema_version=1,parent_pid=process.pid,samples=[],errors=['sampler did not start'])
        cancelled = threading.Event()
        monitor_done = threading.Event()
        def monitor():
            while not monitor_done.is_set() and process.poll() is None:
                if stop.wait(.1):
                    cancelled.set()
                    try: os.killpg(process.pid,signal.SIGKILL)
                    except ProcessLookupError: pass
                    return
        try:
            sampler=ProcessSampler(process.pid);sampler.start()
            watcher=threading.Thread(target=monitor,daemon=True);watcher.start()
            returncode=process.wait(timeout=180)
        except subprocess.TimeoutExpired:
            raise RuntimeError('native executor timed out; forced cleanup retained')
        finally:
            monitor_done.set()
            try:
                if process.poll() is None:
                    try:os.killpg(process.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
                    process.wait(timeout=15)
                try:os.killpg(process.pid,0)
                except ProcessLookupError:pass
                else:
                    cleanup_errors.append('native executor left process-group survivors')
                    try:os.killpg(process.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
            except Exception as error:
                cleanup_errors.append('native process cleanup: '+str(error))
            if watcher is not None:
                try:
                    watcher.join(timeout=1)
                    if watcher.is_alive():cleanup_errors.append('native cancellation monitor did not stop')
                except RuntimeError as error:
                    cleanup_errors.append('native cancellation monitor cleanup: '+str(error))
            if sampler is not None:
                try:resource_samples=sampler.finish()
                except Exception as error:
                    cleanup_errors.append('native sampler cleanup: '+str(error))
            (directory/'native-resource-samples.json').write_text(json.dumps(resource_samples,separators=(',',':'))+'\n')
            (directory/'native-process-cleanup.json').write_text(json.dumps(dict(
                parent_pid=process.pid,returncode=process.returncode,errors=cleanup_errors),indent=2)+'\n')
    if cleanup_errors:
        raise RuntimeError('; '.join(cleanup_errors))
    if cancelled.is_set() or returncode != 0:
        raise RuntimeError('native executor cancelled or failed')
    evidence=json.loads((directory/'native-evidence.json').read_text())
    (directory/'native-failure-attribution.json').write_text(json.dumps(attribute_native_failure(evidence),indent=2)+'\n')
    resource_result=reconcile_resources(resource_samples,evidence)
    (directory/'native-resource-accounting.json').write_text(json.dumps(resource_result,indent=2)+'\n')
    evidence['resource_accounting']=resource_result
    runtime=evidence.get('runtime',{})
    if runtime.get('sandboxed') is not True or runtime.get('worker_binary_sha256')!=support.digest(worker):
        raise ValueError('native worker attestation does not match pinned candidate')
    if evidence.get('attempt_id') != plan['attempt_id'] or evidence.get('infrastructure_errors'):
        raise ValueError('native executor attempt identity or infrastructure failure')
    return evidence


def read_case_contract(family,version,name):
    suffix='' if version==1 else f'-v{version}'
    path=FIXTURES/'research-reference-cases-v1.json' if family=='research' and name=='generation_bound_stale_reference' else FIXTURES/f'{family}-cases{suffix}.json'
    return json.loads(path.read_text())


def validate_forms(case,contract,identity,origin,frames,data):
    if not case.get('extended'):
        clean=dict(data);extra=case.get('extra_request_statuses',[])
        problems=[]
        if extra:
            tail=data['requests'][-len(extra):]
            if [row['status'] for row in tail]!=extra:problems.append('exact duplicate form request status mismatch')
            if any(row['sha256']!=data['requests'][0]['sha256'] for row in tail):problems.append('duplicate form request body changed')
            clean['requests']=data['requests'][:-len(extra)]
            # The baseline validator checks baseline action count; the extra real
            # action is independently present in native/fault evidence.
            filtered=[];remaining=case.get('extra_native_actions',0)
            for frame in reversed(frames):
                if frame.get('kind')=='action' and remaining:remaining-=1;continue
                filtered.append(frame)
            frames=list(reversed(filtered))
        return problems+support.validate_outcome(case,contract,identity,origin,frames,clean,upload_metadata())
    errors=[]
    if [frame['value'] for frame in frames if frame.get('kind')=='structured']!=case['expected_text']:
        errors.append('extended forms visible outcome mismatch')
    if project_observed('forms',case,data)!=forms_expected(contract,case,upload_metadata()):
        errors.append('extended forms durable method/body/receipt outcome mismatch')
    for row in data['requests']:
        if len(row.get('body_sha256',''))!=64 or row.get('body_bytes',0)<=0:errors.append('missing actual form body evidence')
    actions=[frame['result'] for frame in frames if frame.get('kind')=='action']
    expected=sum(line.split()[0] in ('click','type','upload') for line in case['steps'])+case.get('extra_native_actions',0)
    if len(actions)!=expected or any(action.get('execution_domain')!='native' for action in actions):errors.append('extended forms native action mismatch')
    return errors


def upload_metadata():
    path=FIXTURES/'authorized-document.txt'
    return dict(name='document',filename=path.name,bytes=path.stat().st_size,sha256=support.digest(path))


def authorized_uploads(family,case):
    return [dict(label='authorized',path=str(FIXTURES/'authorized-document.txt'),mime='application/octet-stream')] if family=='forms' and case['upload'] else []


def family_lines(family,runner,contract,case,base):
    if family!='forms' or case.get('extended'): return [line.replace('{base}',base) for line in case['steps']] if case.get('extended') else runner.task_lines(contract,case,base)
    lines=[f'goto "{base}"','observe']+case['before_fill']
    if case['fill']:lines+=contract['fill']
    lines+=case['after_fill']
    if case['submit']:lines+=['click "Submit application"']
    if case.get('expected_receipts',1):lines+=['wait visible css(#receipt)','click "Reload receipt"','wait visible css(#receipt)']
    return lines+['observe']


class DatabaseCellAdapter:
    """Real OIL behavior recipes over the existing sealed/reopened SQLite fixtures."""
    def __init__(self, cell, binary, worker):
        if cell not in RESEARCH_RECIPES and cell not in RECIPES:
            raise ValueError('no concrete behavior recipe for '+cell)
        self.cell,self.binary,self.worker=cell,Path(binary),Path(worker)

    def recipes(self):
        return [("research",version,name) for version,name in RESEARCH_RECIPES[self.cell]] if self.cell in RESEARCH_RECIPES else RECIPES[self.cell]

    def __call__(self, attempt, directory, stop):
        if attempt['cell'] != self.cell:
            raise ValueError('adapter cell identity changed')
        # Recovery/fault cells have separate recipes. Do not silently run a normal
        # task when a contract requested an interruption or stale reference.
        if attempt['fault'] is not None:
            raise ValueError('behavior-cell interruption/fault recovery recipe not implemented')
        root=directory.parents[1]
        errors,actions,outcomes,activity=[],[],[],None
        cleanup=dict(returncode=0,forced=False)
        if attempt["mode"]=="mixed_family":
            errors.append("heterogeneous mixed-family session recipes pending; same-cell concurrency is diagnostic only")
        for ordinal,(family,version,case_name) in enumerate(self.recipes()):
            runner=research if family=="research" else load("native_adapter_"+family,ROOT/f"scripts/run-oil-{family}-outcomes.py")
            wave=directory/f'case-{ordinal}';wave.mkdir()
            suffix='' if version==1 else f'-v{version}'
            contract=read_case_contract(family,version,case_name)
            matches=[case for case in contract['cases'] if case['id']==case_name]
            if len(matches)!=1: raise ValueError('missing or duplicate case identity')
            case=matches[0]
            fixture=FIXTURES/f"{family}_fixture{suffix.replace('-','_')}.py"
            receiver=load("native_snapshot_"+family+str(version),fixture)
            database=wave/(family+'.sqlite3')
            oracle=support.Oracle(wave,fixture,['--database',database])
            contexts=[]
            oracle_origin=oracle.origin
            try:
                for index in range(attempt['context_count']):
                    identity=secrets.token_hex(16)
                    oracle.request('create',identity,**({'upload':case['upload']} if family=='forms' and not case.get('extended') else {'scenario':case.get('scenario','normal')}))
                    prefix='advanced' if family=='forms' and case.get('extended') else {'research':'research','shopping':'shop','saas':'saas','booking':'booking','forms':'form'}[family]
                    base=f'{oracle.origin}/{prefix}/{identity}'
                    lines=family_lines(family,runner,contract,case,base)
                    contexts.append(dict(id=f'context-{index}',case_id=identity,base=base,lines=lines))
                plan=dict(schema_version=1,attempt_id=attempt['attempt_id'],sessions=[dict(
                    id=item['id'],steps=[dict(oil=line,**case.get('step_options',{}).get(str(index),{})) for index,line in enumerate(item['lines'])],uploads=authorized_uploads(family,case),fault=None) for item in contexts])
                evidence=execute_native(self.binary,self.worker,plan,wave,stop)
                measured=native_activity(evidence,attempt['context_count'])
                if activity is None: activity=measured
                oracle.request('seal','0'*32)
                cleanup_result=oracle.close();oracle=None
                if cleanup_result != cleanup:
                    cleanup=cleanup_result;errors.append('fixture did not cleanly retire')
                sessions={session['id']:session for session in evidence['sessions']}
                observed=[]
                for item in contexts:
                    session=sessions[item['id']]
                    if session['errors'] or session['cleanup_errors'] or not session['workers_retired']:
                        errors.append('native operation/worker cleanup failed')
                    if len(session['steps']) != len(item['lines']): errors.append('native task truncated')
                    frames=[frame for step in session['steps'] for frame in step['outputs']]
                    data=(research.receiver if family=='research' else receiver).snapshot(database,item['case_id'])
                    (wave/(item['id']+'-sealed-oracle.json')).write_text(json.dumps(data,indent=2)+'\n')
                    if family=='forms':
                        errors.extend(validate_forms(case,contract,item['case_id'],oracle_origin,frames,data))
                    else:
                        errors.extend(runner.validate(case,item['lines'],item['base'],frames,data))
                    for frame in frames:
                        if frame.get('kind')=='action':
                            action=dict(frame['result']);action['context_id']=str(session['context_id']);action['recipe']=case_name
                            actions.append(action)
                    # Actual extracted values and independent durable request facts,
                    # normalized only for the per-run origin/case namespace.
                    if family=='research':
                        facts=[frame['value'] for frame in frames if frame.get('kind')=='structured']
                        facts=json.loads(json.dumps(facts).replace('/research/'+item['case_id'],'{base}'))
                        visits=[dict(path=row['path'],query=json.loads(row['query']),status=row['status']) for row in data['visits']]
                        observed.append(dict(facts=facts,visits=visits))
                    else:
                        projection=project_observed(family,case,data)
                        if family=='forms': projection=json.loads(json.dumps(projection).replace(item['case_id'],'{case_id}'))
                        observed.append(projection)
                outcomes.append(dict(case=case_name,contexts=observed))
            finally:
                if oracle is not None:
                    result=oracle.close()
                    if result != dict(returncode=0,forced=False): cleanup=result
        return dict(passed=not errors,errors=errors,timed_out=False,cleanup=cleanup,
                    actions=artifact(root,directory/'actions.json',actions),
                    oracle=artifact(root,directory/'oracle.json',outcomes),
                    context_activity=artifact(root,directory/'context-activity.json',activity or []),
                    frontend_dimensions=['cross_origin_frames'] if self.cell=='research.frames' else
                        ['ssr_hydration','routing_chunks','client_rendering'] if self.cell=='research.pagination' else DIMENSIONS.get(self.cell.split('.')[0],[]))

    def expected(self, context_count):
        outcomes=[]
        for family,version,case_name in self.recipes():
            suffix='' if version==1 else f'-v{version}'
            contract=read_case_contract(family,version,case_name)
            case=next(case for case in contract['cases'] if case['id']==case_name)
            receiver=None if family=='research' else load('expected_'+family+str(version),FIXTURES/f"{family}_fixture{suffix.replace('-','_')}.py")
            expectation=dict(facts=case['expected'],visits=case['visits']) if family=='research' else forms_expected(contract,case,upload_metadata()) if family=='forms' else project_expected(family,case,receiver)
            outcomes.append(dict(case=case_name,contexts=[expectation for _ in range(context_count)]))
        return outcomes

# Kept for callers that explicitly select the research subset.
ResearchCellAdapter = DatabaseCellAdapter

# Each mixed wave has the requested behavior plus real companion application
# tasks. Counts 5/20 cover all five families in one Browser; count 1 is the frozen
# single-context control. Companion outcomes are independently checked as well.
COMPANIONS = [('research',3,'nested_cross_origin_extraction'),
              ('forms',3,'multistep_custom_role'),
              ('shopping',2,'framed_persisted_checkout'),
              ('saas',2,'pkce_create_update_reload_logout'),
              ('booking',2,'client_phone_option')]


def mixed_recipes(primary,count):
    if count not in (1,5,20): raise ValueError('unsupported frozen context count')
    other=[recipe for recipe in COMPANIONS if recipe[0]!=primary[0]]
    cycle=[primary,*other]
    return [cycle[index%len(cycle)] for index in range(count)]


class PreparedCase:
    def __init__(self,recipe,directory):
        self.family,self.version,self.name=recipe
        self.directory=directory;directory.mkdir()
        suffix='' if self.version==1 else f'-v{self.version}'
        self.contract=read_case_contract(self.family,self.version,self.name)
        matches=[case for case in self.contract['cases'] if case['id']==self.name]
        if len(matches)!=1:raise ValueError('missing or duplicate concrete case')
        self.case=matches[0]
        self.runner=research if self.family=='research' else load('mixed_runner_'+self.family,ROOT/f'scripts/run-oil-{self.family}-outcomes.py')
        fixture=FIXTURES/f"{self.family}_fixture{suffix.replace('-','_')}.py"
        self.receiver=research.receiver if self.family=='research' else load('mixed_receiver_'+self.family,fixture)
        self.database=directory/'oracle.sqlite3'
        self.oracle=support.Oracle(directory,FIXTURES/'workflow_fixture.py',['--family',self.family,'--version',self.version,'--database',self.database])
        self.identity=secrets.token_hex(16);self.origin=self.oracle.origin
        try:
            self.oracle.request('create',self.identity,**({'upload':self.case['upload']} if self.family=='forms' and not self.case.get('extended') else {'scenario':self.case.get('scenario','normal')}))
        except BaseException:
            self.oracle.close();self.oracle=None;raise
        prefix='advanced' if self.family=='forms' and self.case.get('extended') else {'research':'research','forms':'form','shopping':'shop','saas':'saas','booking':'booking'}[self.family]
        self.base=f'{self.origin}/{prefix}/{self.identity}'
        self.lines=family_lines(self.family,self.runner,self.contract,self.case,self.base)
        self.fault=None;self.recovery_steps=[]

    def session(self,identity):
        return dict(id=identity,steps=[dict(oil=line,**self.case.get("step_options",{}).get(str(index),{})) for index,line in enumerate(self.lines)],
                    uploads=authorized_uploads(self.family,self.case),fault=self.fault,recovery_steps=self.recovery_steps)

    def configure_fault(self,kind):
        description=decorate_fault(self.family,self.case,self.lines,kind,self.base)
        self.case=description['case'];self.lines=description['lines'];self.fault=description['fault'];self.recovery_steps=description['recovery_steps']
        if description['arm']:
            self.oracle.request('arm_response_fault',self.identity,token=secrets.token_hex(16),**description['arm'])

    def seal(self):
        self.oracle.request('seal','0'*32)
        result=self.oracle.close();self.oracle=None
        return result

    def outcome(self,session):
        data=self.receiver.snapshot(self.database,self.identity)
        (self.directory/'sealed-oracle.json').write_text(json.dumps(data,indent=2)+'\n')
        return replay_case_outcome(self,session,data)


def replay_case_outcome(self,session,data):
    errors=[]
    if session['errors'] or session['cleanup_errors'] or not session['workers_retired']:
        errors.append('native operation/worker cleanup failed')
    if len(session['steps'])!=len(self.lines):errors.append('native task truncated')
    frames=[frame for step in [*session['steps'],*session.get('recovery',[])] for frame in step['outputs']]
    if self.family=='forms':
        errors.extend(validate_forms(self.case,self.contract,self.identity,self.origin,frames,data))
    else:
        errors.extend(self.runner.validate(self.case,self.lines,self.base,frames,data))
    if self.family=='research':
        facts=[frame['value'] for frame in frames if frame.get('kind')=='structured']
        facts=json.loads(json.dumps(facts).replace('/research/'+self.identity,'{base}'))
        projection=dict(facts=facts,visits=[dict(path=row['path'],query=json.loads(row['query']),status=row['status']) for row in data['visits']])
    else:
        projection=project_observed(self.family,self.case,data)
        if self.family=='forms':projection=json.loads(json.dumps(projection).replace(self.identity,'{case_id}'))
    actions=[dict(frame['result'],context_id=str(session['context_id']),recipe=self.name)
             for frame in frames if frame.get('kind')=='action']
    replay=session.get('fault_evidence',{}).get('replay',{}) if isinstance(session.get('fault_evidence'),dict) else {}
    actions.extend(dict(frame['result'],context_id=str(session['context_id']),recipe=self.name) for frame in replay.get('outputs',[]) if frame.get('kind')=='action')
    return dict(family=self.family,case=self.name,outcome=projection),actions,errors

def expected_recipe(recipe,fault=None):
    family,version,name=recipe;suffix='' if version==1 else f'-v{version}'
    contract=read_case_contract(family,version,name)
    matches=[case for case in contract['cases'] if case['id']==name]
    if len(matches)!=1:raise ValueError('missing or duplicate expected recipe')
    case=matches[0]
    if fault:
        runner=research if family=='research' else load('fault_expected_'+family,ROOT/f'scripts/run-oil-{family}-outcomes.py')
        case=decorate_fault(family,case,family_lines(family,runner,contract,case,'{base}'),fault,'{base}')['case']
    receiver=None if family=='research' else load('mixed_expected_'+family,FIXTURES/f"{family}_fixture{suffix.replace('-','_')}.py")
    expectation=dict(facts=case['expected'],visits=case['visits']) if family=='research' else forms_expected(contract,case,upload_metadata()) if family=='forms' else project_expected(family,case,receiver)
    return dict(family=family,case=name,outcome=expectation)


class MixedCellAdapter(DatabaseCellAdapter):
    def expected(self,count,mode=None,fault=None):
        return [dict(case=primary[2],contexts=[expected_recipe(recipe,fault if recipe[2]==PREFERRED.get(self.cell) else None) for recipe in (mixed_recipes(primary,count) if mode=='mixed_family' else [primary]*count)]) for primary in self.recipes()]

    def __call__(self,attempt,directory,stop):
        if attempt['cell']!=self.cell:raise ValueError('attempt cell mismatch')
        if attempt['fault'] and self.cell not in PREFERRED:raise ValueError('missing explicit fault boundary')
        root=directory.parents[1];errors=[];actions=[];outcomes=[];activity=None;dimensions=set();topologies=[];probe_results=[];fault_records=[];resource_records=[]
        cleanup=dict(returncode=0,forced=False);frontend_witnesses=[];raw_waves=[]
        for ordinal,primary in enumerate(self.recipes()):
            wave=directory/f'mixed-{ordinal}';wave.mkdir();prepared=[];probes=[]
            try:
                for index,recipe in enumerate(mixed_recipes(primary,attempt['context_count']) if attempt['mode']=='mixed_family' else [primary]*attempt['context_count']):
                    prepared.append(PreparedCase(recipe,wave/f'context-{index}'))
                    if attempt['fault'] and recipe[2]==PREFERRED.get(self.cell):prepared[-1].configure_fault(attempt['fault'])
                plan=dict(schema_version=1,attempt_id=attempt['attempt_id'],sessions=[case.session(f'context-{index}') for index,case in enumerate(prepared)])
                probes=[]
                for index,case in enumerate(prepared):
                    if case.family=='forms' and case.case.get('scenario')=='confinement':
                        probes.append(prepare_form_probe(case,plan,index,wave))
                    elif case.family=='saas' and case.case.get('isolation_probe'):
                        probes.append(prepare_saas_probe(case,plan,index))
                evidence=execute_native(self.binary,self.worker,plan,wave,stop)
                resource_records.append(dict(wave=ordinal,accounting=artifact(root,wave/'native-resource-accounting.json',evidence['resource_accounting']),samples=dict(path=str((wave/'native-resource-samples.json').relative_to(root)),sha256=support.digest(wave/'native-resource-samples.json')),process_cleanup=dict(path=str((wave/'native-process-cleanup.json').relative_to(root)),sha256=support.digest(wave/'native-process-cleanup.json'))))
                resource_records[-1]['native_evidence']=dict(path=str((wave/'native-evidence.json').relative_to(root)),sha256=support.digest(wave/'native-evidence.json'))
                measured=native_activity(evidence,attempt['context_count'])
                if activity is None:activity=measured
                sessions={session['id']:session for session in evidence['sessions']}
                topologies.append(context_topology(evidence,attempt['context_count']))
                for probe in probes:
                    result,problems=validate_form_probe(probe,sessions) if probe['kind']=='forms' else validate_saas_probe(probe,sessions)
                    probe_results.append(result);errors.extend(problems)
                wave_outcomes=[];raw_cases=[]
                for index,case in enumerate(prepared):
                    result=case.seal()
                    if case.fault:
                        fault_records.append(read_fault_evidence(case,sessions[f'context-{index}']))
                    if result!=dict(returncode=0,forced=False):cleanup=result;errors.append('mixed fixture cleanup failed')
                    outcome,case_actions,case_errors=case.outcome(sessions[f'context-{index}'])
                    for action in case_actions:action['recipe']=f'{ordinal}:{index}:'+action['recipe']
                    wave_outcomes.append(outcome);actions.extend(case_actions);errors.extend(case_errors)
                    recipe=(case.family,case.version,case.name)
                    def preserved(path):
                        return dict(path=str(path.relative_to(root)),sha256=support.digest(path))
                    raw_cases.append(dict(recipe=list(recipe),session=f'context-{index}',base=case.base,
                        database=preserved(case.database),sealed_oracle=preserved(case.directory/'sealed-oracle.json'),fixture_cleanup=result))
                    matching_probes=[probe for probe in probes if probe['owner']==f'context-{index}']
                    if matching_probes:
                        probe=matching_probes[0]
                        raw_cases[-1]['probe']=dict(kind=probe['kind'],id=probe['id'],owner=probe['owner'])
                        if probe['kind']=='forms':
                            raw_cases[-1]['probe'].update(case_id=probe['case_id'],secret_sha256=hashlib.sha256(probe['secret_value']).hexdigest())
                    if recipe in WITNESSES and attempt['fault'] is None and not case_errors:
                        def preserved(path):
                            return dict(path=str(path.relative_to(root)),sha256=support.digest(path))
                        frontend_witnesses.append(dict(recipe=list(recipe),session=f'context-{index}',base=case.base,
                            sources=source_records(recipe),native_plan=preserved(wave/'native-plan.json'),
                            native_evidence=preserved(wave/'native-evidence.json'),
                            sealed_oracle=preserved(case.directory/'sealed-oracle.json')))
                        dimensions.update(WITNESSES[recipe])
                raw_waves.append(dict(primary=list(primary),native_plan=preserved(wave/'native-plan.json'),
                    native_evidence=preserved(wave/'native-evidence.json'),cases=raw_cases,
                    samples=preserved(wave/'native-resource-samples.json'),accounting=preserved(wave/'native-resource-accounting.json'),process_cleanup=preserved(wave/'native-process-cleanup.json')))
                outcomes.append(dict(case=primary[2],contexts=wave_outcomes))
            finally:
                for probe in probes:
                    if probe.get("secret_path"):probe["secret_path"].unlink(missing_ok=True)
                for case in prepared:
                    if case.oracle is not None:
                        result=case.oracle.close();case.oracle=None
                        if result!=dict(returncode=0,forced=False):cleanup=result
        supplements=run_commit_supplement(self,attempt,directory,stop)
        fault_records.extend(supplements)
        if attempt['fault'] and (not fault_records or any(record.get('injected') is not True for record in fault_records)):errors.append('required actual fault injection missing')
        return dict(passed=not errors,errors=errors,timed_out=False,cleanup=cleanup,
                    actions=artifact(root,directory/'actions.json',actions),
                    oracle=artifact(root,directory/'oracle.json',outcomes),
                    context_activity=artifact(root,directory/'context-activity.json',activity or []),
                    fault_evidence=artifact(root,directory/'fault-evidence.json',dict(fault=attempt['fault'],injected=bool(fault_records) and all(record.get('injected') is True for record in fault_records),records=fault_records)),
                    frontend_dimensions=sorted(dimensions),
                    frontend_evidence=artifact(root,directory/'frontend-evidence.json',frontend_witnesses),
                    context_topology=artifact(root,directory/'context-topology.json',topologies),
                    adversarial_evidence=artifact(root,directory/'adversarial-evidence.json',probe_results),
                    resource_evidence=artifact(root,directory/'resource-evidence.json',resource_records),
                    raw_evidence=artifact(root,directory/'raw-evidence.json',dict(schema_version=1,attempt_id=attempt['attempt_id'],waves=raw_waves,supplements=[record['raw_wave'] for record in supplements])))



def context_topology(evidence,cohort_count):
    sessions=evidence['sessions']
    return dict(cohort_count=cohort_count,probe_count=sum(row.get('adversarial_for') is not None for row in sessions),
                total_live_contexts=len(sessions),contexts=[dict(context_id=str(row['context_id']),id=row['id'],adversarial_for=row.get('adversarial_for')) for row in sessions])


def prepare_form_probe(case,plan,index,wave):
    owner=f'context-{index}';identity=secrets.token_hex(16)
    case.oracle.request('create',identity,scenario='confinement')
    secret_value=secrets.token_hex(32).encode()
    path=wave/f'synthetic-secret-{index}'
    descriptor=os.open(path,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600)
    with os.fdopen(descriptor,'wb') as stream:stream.write(secret_value)
    plan['sessions'][index]['secrets']=[dict(label='confined',path=str(path),origin=case.origin)]
    probe_id=f'adversary-{index}'
    plan['sessions'].append(dict(id=probe_id,adversarial_for=owner,uploads=[],fault=None,steps=[
        dict(oil=f'goto "{case.base}/receipt"'),dict(oil='extract text --selector "#denied"'),
        dict(oil=f'goto "{case.origin}/advanced/{identity}/start"'),
        dict(oil='upload "Authorized document" $file:authorized',expected_error='upload label is unknown'),
        dict(oil='type "Full name" $secret:confined',expected_error='secret label is unknown'),dict(oil='observe')]))
    return dict(kind='forms',owner=owner,id=probe_id,case_id=identity,case=case,secret_path=path,secret_value=secret_value)


def validate_form_probe(probe,sessions):
    session=sessions[probe['id']];errors=[]
    if session['errors'] or session['cleanup_errors'] or not session['workers_retired']:errors.append('adversarial context did not finish native denials/cleanup')
    if len(session['steps'])!=6 or any(not row['expected_error_matched'] for row in session['steps']):errors.append('native confinement denial steps missing')
    frames=[frame for row in session['steps'] for frame in row['outputs']]
    if [frame['value'] for frame in frames if frame.get('kind')=='structured']!=['Context access denied']:errors.append('foreign-context receipt became visible')
    if session.get('secret_labels') or sessions[probe['owner']].get('secret_labels')!=['confined']:errors.append('secret context ownership mismatch')
    if probe['secret_value'].decode() in json.dumps(sessions):errors.append('secret value disclosed in native evidence')
    data=probe['case'].oracle.request('snapshot',probe['case_id'])
    if data['requests'] or data['receipts']:errors.append('unauthorized upload/secret probe caused a mutation')
    probe['secret_path'].unlink(missing_ok=True)
    return dict(kind='forms_confinement',owner=probe['owner'],context_id=str(session['context_id']),
                steps=session['steps'],requests=data['requests'],receipts=data['receipts'],passed=not errors),errors



def prepare_saas_probe(case,plan,index):
    owner=f'context-{index}';probe_id=f'adversary-{index}'
    plan['sessions'].append(dict(id=probe_id,adversarial_for=owner,uploads=[],fault=None,steps=[
        dict(oil=f'goto "{case.base}/records"'),dict(oil='extract text --selector "h1"'),
        dict(oil=f'goto "{case.base}/probe"'),dict(oil='click "Attempt unauthorized save"'),
        dict(oil='extract text --selector "h1"'),dict(oil='observe')]))
    return dict(kind='saas',owner=owner,id=probe_id,case=case)


def validate_saas_probe(probe,sessions):
    session=sessions[probe['id']];errors=[]
    if session['errors'] or session['cleanup_errors'] or not session['workers_retired']:errors.append('SaaS adversarial context failed or leaked worker')
    frames=[frame for row in session['steps'] for frame in row['outputs']]
    if [frame['value'] for frame in frames if frame.get('kind')=='structured']!=['Session required','Session expired']:
        errors.append('unauthenticated context read/write was not denied')
    return dict(kind='saas_isolation',owner=probe['owner'],context_id=str(session['context_id']),steps=session['steps'],passed=not errors),errors



def read_fault_evidence(case,session):
    result=dict(session['fault_evidence'])
    with sqlite3.connect(f'file:{case.database}?mode=ro',uri=True) as database:
        database.row_factory=sqlite3.Row
        responses=[dict(row) for row in database.execute('SELECT * FROM workflow_faults WHERE case_id=? ORDER BY rowid',(case.identity,))]
    result['responses']=responses
    if case.fault['kind']=='delayed_response':
        result['injected']=len(responses)==1 and responses[0]['terminal']=='released' and responses[0]['released_ns']-responses[0]['started_ns']>=responses[0]['delay_ms']*1_000_000
    if case.fault['kind']=='cancellation':
        result['injected']=result.get('injected') is True and result.get('native',{}).get('interrupted_inflight') is True and bool(session.get('recovery')) and len(responses)==1 and responses[0]['terminal']=='released'
    return result


def commit_recipe(case):
    """Last successful mutation in the pinned recovery task, before logout/readback.

    Earlier rejected save/quote/reservation attempts remain real application
    traffic. The ordinal is fixed by the source task, never inferred from a run.
    """
    label={'forms':'Send document' if case.case.get('extended') else 'Submit application',
           'shopping':'Place test order','saas':'Save changes','booking':'Reserve test appointment'}[case.family]
    indexes=[index for index,line in enumerate(case.lines) if line==f'click "{label}"']
    if not indexes:raise ValueError('recovery recipe has no exact native mutation command')
    route={'forms':'submit','shopping':'checkout','saas':'update','booking':'reserve'}[case.family]
    return dict(route=route,skip_matches=len(indexes)-1,before_step=indexes[-1],resume_after=indexes[-1])


def run_commit_supplement(adapter,attempt,directory,stop):
    """Both commit phases for actual worker loss/cancellation, beside baseline.

    These additional waves preserve the frozen baseline oracle. Their own durable
    projection must equal the source baseline too, and their recovery decisions
    must match the sealed commit ledger. No execution result becomes an expected
    value, and no failed supplemental wave can qualify its enclosing attempt.
    """
    if attempt['fault'] not in ('worker_failure','cancellation') or adapter.cell.startswith('research.'):
        return []
    recipe=next(recipe for recipe in adapter.recipes() if recipe[2]==PREFERRED[adapter.cell])
    records=[]
    for phase in ('before_commit','after_commit'):
        wave=directory/('commit-'+phase);wave.mkdir();prepared=[];problems=[]
        try:
            sessions=[]
            for index in range(attempt['context_count']):
                case=PreparedCase(recipe,wave/f'context-{index}');prepared.append(case)
                boundary=commit_recipe(case);token=secrets.token_hex(16)
                gate=dict(token=token,phase=phase,marker=str((case.directory/'gate.json').resolve()),release=str((case.directory/'release.json').resolve()))
                case.oracle.request('arm_commit_gate',case.identity,route=boundary['route'],skip_matches=boundary['skip_matches'],**gate)
                session=case.session(f'context-{index}')
                session['fault']=dict(kind=attempt['fault'],before_step=boundary['before_step'],delay_ms=300,commit_gate=gate)
                session['commit_recovery']=dict(outcome_url=case.base+'/workflow-outcome?token='+token,resume_after=boundary['resume_after'])
                sessions.append(session)
            plan=dict(schema_version=1,attempt_id=attempt['attempt_id']+':'+phase,sessions=sessions)
            native=execute_native(adapter.binary,adapter.worker,plan,wave,stop)
            measured=native_activity(native,attempt['context_count'])
            by_id={session['id']:session for session in native['sessions']};outcomes=[];raw_cases=[]
            for index,case in enumerate(prepared):
                session=by_id[f'context-{index}'];closed=case.seal()
                if closed!=dict(returncode=0,forced=False):problems.append('commit fixture cleanup failed')
                if session['errors'] or session['cleanup_errors'] or not session['workers_retired']:problems.append('commit native session failed')
                fault=session['fault_evidence']
                if fault.get('injected') is not True or fault.get('native',{}).get('interrupted_inflight') is not True or fault.get('native',{}).get('reaped') is not True:problems.append('commit gate lacks actual inflight worker exit')
                data=case.receiver.snapshot(case.database,case.identity)
                with sqlite3.connect(f'file:{case.database}?mode=ro',uri=True) as db:
                    db.row_factory=sqlite3.Row
                    ledger=[dict(row) for row in db.execute('SELECT token,phase,body_sha256,started_ns,released_ns,terminal,response_status FROM workflow_commit_gates WHERE case_id=?',(case.identity,))]
                    reads=[dict(row) for row in db.execute('SELECT token,state FROM workflow_recovery_views WHERE case_id=?',(case.identity,))]
                    retries=[dict(row) for row in db.execute('SELECT token,body_sha256,matched FROM workflow_recovery_retries WHERE case_id=?',(case.identity,))]
                outcome,outcome_errors=replay_commit_outcome(case,session,data,ledger,reads,retries,phase)
                outcomes.append(outcome);problems.extend(outcome_errors)
                root=directory.parents[1]
                def preserved(path):
                    return dict(path=str(path.relative_to(root)),sha256=support.digest(path))
                raw_cases.append(dict(recipe=list(recipe),session=f'context-{index}',base=case.base,database=preserved(case.database),fixture_cleanup=closed))
            records.append(dict(phase=phase,fault=attempt['fault'],injected=not problems,errors=problems,context_topology=context_topology(native,attempt['context_count']),native_activity=measured,outcomes=outcomes,raw_wave=dict(phase=phase,cases=raw_cases,native_plan=preserved(wave/'native-plan.json'),native_evidence=preserved(wave/'native-evidence.json'),samples=preserved(wave/'native-resource-samples.json'),accounting=preserved(wave/'native-resource-accounting.json'),process_cleanup=preserved(wave/'native-process-cleanup.json'))))
        finally:
            for case in prepared:
                if case.oracle is not None:case.oracle.close();case.oracle=None
    return records


def replay_commit_outcome(case,session,data,ledger,reads,retries,phase):
    problems=[]
    fault=session['fault_evidence']
    actual=project_observed(case.family,case.case,data)
    expected=forms_expected(case.contract,case.case,upload_metadata()) if case.family=='forms' else project_expected(case.family,case.case,case.receiver)
    if case.family=='forms':actual=json.loads(json.dumps(actual).replace(case.identity,'{case_id}'))
    if json.dumps(actual,sort_keys=True)!=json.dumps(expected,sort_keys=True):problems.append('commit recovery changed durable baseline outcome or request history')
    state='not_committed' if phase=='before_commit' else 'committed'
    terminal='aborted_before_commit' if phase=='before_commit' else 'released_after_commit'
    if (len(ledger)!=1 or ledger[0]['terminal']!=terminal or ledger[0]['phase']!=phase
            or type(ledger[0]['started_ns']) is not int or type(ledger[0]['released_ns']) is not int
            or ledger[0]['started_ns']<=0 or ledger[0]['released_ns']<ledger[0]['started_ns']
            or not isinstance(ledger[0]['body_sha256'],str) or len(ledger[0]['body_sha256'])!=64):
        problems.append('commit ledger missing actual phase release')
    if len(reads)!=1 or reads[0]['state']!=state or not ledger or reads[0]['token']!=ledger[0]['token']:problems.append('recovery did not inspect exact persisted outcome once')
    if phase=='before_commit':
        if len(retries)!=1 or type(retries[0]['matched']) is not int or retries[0]['matched']!=1 or not ledger or retries[0]['token']!=ledger[0]['token'] or retries[0]['body_sha256']!=ledger[0]['body_sha256']:problems.append('retry did not preserve exact original request bytes')
    elif retries:problems.append('committed operation was retried')
    resolutions=[step for step in session.get('recovery',[]) if step.get('stage')=='resolve_commit']
    if len(resolutions)!=1 or resolutions[0].get('state')!=state:problems.append('native conditional retry decision differs from durable outcome')
    # Native evidence keeps all interrupted operations and recovery
    # commands, even when a baseline action never received a reply.
    return dict(context_id=str(session['context_id']),expected=expected,actual=actual,ledger=ledger,recovery_reads=reads,recovery_retries=retries,native_fault=fault,recovery=session.get('recovery',[])),problems

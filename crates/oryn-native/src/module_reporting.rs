// Included in raw_v8. Admission precedes compilation/evaluation. Successful
// evaluation never loses its error observer to a later allocation failure.
#[derive(Default)]
struct ModuleReportQueue{jobs:std::collections::VecDeque<ModuleReport>,admitted:Rc<std::cell::Cell<usize>>}
struct ModuleReport{
    environment:EnvironmentDescriptor,epoch:u64,handler:v8::Global<v8::Function>,
    resolver:v8::Global<v8::PromiseResolver>,promise:Option<v8::Global<v8::Promise>>,
    rejection:Option<v8::Global<v8::Value>>,retry:Instant,deadline:Instant,admitted:Rc<std::cell::Cell<usize>>,
}
impl Drop for ModuleReport{fn drop(&mut self){self.admitted.set(self.admitted.get()-1);}}
impl ModuleReport{
 fn prepare(scope:&mut v8::PinScope)->Result<Self,HostError>{
    let environment=current_environment(scope).map_err(HostError::Execution)?;
    let epoch=scope.get_current_context().get_slot::<V8BindingState>().map_or(0,|state|state.document_authority_epoch());
    let queue=scope.get_slot_mut::<ModuleReportQueue>().ok_or_else(||HostError::Execution("module report queue absent".into()))?;
    if queue.admitted.get()>=256{return Err(HostError::LimitExceeded{resource:"module reporting admission".into()});}
    queue.jobs.try_reserve(1).map_err(|_|HostError::Execution("module report queue allocation".into()))?;
    let admitted=queue.admitted.clone();
    let handler=v8::Function::new(scope,module_error_report_callback).ok_or_else(||HostError::Execution("module report callback allocation".into()))?;
    let handler=v8::Global::new(scope,handler);
    let resolver=v8::PromiseResolver::new(scope).ok_or_else(||HostError::Execution("module report resolver allocation".into()))?;
    let resolver=v8::Global::new(scope,resolver);
    admitted.set(admitted.get()+1);
    Ok(Self{environment,epoch,handler,resolver,promise:None,rejection:None,retry:Instant::now(),deadline:Instant::now()+Duration::from_secs(10),admitted})
 }
 fn current(&self,scope:&mut v8::PinScope)->bool{
    current_environment(scope).is_ok_and(|environment|environment==self.environment)
      &&scope.get_current_context().get_slot::<V8BindingState>().is_none_or(|state|!state.execution_retired()&&state.document_authority_epoch()==self.epoch)
 }
 fn poll(&mut self,scope:&mut v8::PinScope)->bool{
    if !self.current(scope){return true;}
    if scope.is_execution_terminating()||scope.get_current_context().get_slot::<V8BindingState>().is_some_and(|state|state.execution_suspended()){return false;}
    if let Some(error)=&self.rejection{
        let resolver=v8::Local::new(scope,&self.resolver);let error=v8::Local::new(scope,error);
        if resolver.reject(scope,error)!=Some(true){return false;}
        self.rejection=None;
    }
    let promise=match &self.promise{Some(promise)=>v8::Local::new(scope,promise),None=>v8::Local::new(scope,&self.resolver).get_promise(scope)};
    promise.mark_as_handled();
    #[cfg(test)]if FAIL_MODULE_REPORT_INSTALL.with(|fail|fail.replace(false)){return false;}
    let handler=v8::Local::new(scope,&self.handler);
    promise.catch(scope,handler).is_some()
 }
 fn submit(mut self,scope:&mut v8::PinScope,promise:Option<v8::Local<v8::Promise>>,error:Option<v8::Local<v8::Value>>){
    self.promise=promise.map(|promise|v8::Global::new(scope,promise));self.rejection=error.map(|error|v8::Global::new(scope,error));
    if !self.poll(scope){self.retry=Instant::now()+Duration::from_millis(10);scope.get_slot_mut::<ModuleReportQueue>().expect("admitted report registry").jobs.push_back(self);}
 }
}
#[cfg(test)]thread_local!{static FAIL_MODULE_REPORT_INSTALL:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
impl RawV8Host{
 fn module_report_delay(&self)->Option<Duration>{
    let now=Instant::now();self.isolate.get_slot::<ModuleReportQueue>()?.jobs.iter().filter_map(|job|{
      if !self.isolate.get_slot::<AgentEnvironments>().is_some_and(|environments|environments.is_live(&job.environment)){return Some(Duration::ZERO);}
      if self.realm_is_parked(job.environment.realm){return Some(job.deadline.saturating_duration_since(now));}
      Some(job.retry.min(job.deadline).saturating_duration_since(now))
    }).min()
 }
 pub(crate) fn has_pending_script_reporting(&self)->bool{
    self.isolate.get_slot::<ModuleReportQueue>().is_some_and(|queue|queue.jobs.iter().any(|job|job.environment.realm==self.active_realm))
     ||self.binding_states.get(&self.active_realm).is_some_and(|state|state.has_pending_script_error_reports())
 }
 fn poll_module_reports(&mut self)->Result<usize,HostError>{
    let count=self.isolate.get_slot::<ModuleReportQueue>().map_or(0,|queue|queue.jobs.len());let mut completed=0;
    for _ in 0..count.min(32){
      let Some(mut job)=self.isolate.get_slot_mut::<ModuleReportQueue>().and_then(|queue|queue.jobs.pop_front())else{break};
      if !self.isolate.get_slot::<AgentEnvironments>().is_some_and(|environments|environments.is_live(&job.environment)){completed+=1;continue;}
      if Instant::now()>=job.deadline{return Err(HostError::Execution("module error observer installation deadline exceeded".into()));}
      if self.realm_is_parked(job.environment.realm)||job.retry>Instant::now(){self.isolate.get_slot_mut::<ModuleReportQueue>().unwrap().jobs.push_back(job);continue;}
      let Some(context)=self.contexts.get(&job.environment.realm).cloned()else{continue};
      v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);
      if job.poll(scope){completed+=1;scope.perform_microtask_checkpoint();}else{job.retry=Instant::now()+Duration::from_millis(10);scope.get_slot_mut::<ModuleReportQueue>().unwrap().jobs.push_back(job);}
    }
    Ok(completed)
 }
}

#[cfg(test)]mod module_reporting_tests{
 use super::*;
 fn host()->RawV8Host{
    let mut host=RawV8Host::new();dynamic_import_continuation_tests::install_document(&mut host);host
 }
 #[test]fn interrupted_report_install_keeps_evaluated_body_and_original_rejection(){
    let mut host=host();host.execute("globalThis.trace=[];globalThis.original=new TypeError('original');onerror=(m,u,l,c,e)=>{trace.push(e===original?'error':'wrong');return true;}").unwrap();
    FAIL_MODULE_REPORT_INSTALL.with(|fail|fail.set(true));
    host.execute_module_script_for_request("https://example.test/report.js","trace.push('body');throw original;",&BTreeMap::new(),Some("https://example.test/report.js")).unwrap();
    assert!(host.has_pending_script_reporting());assert_eq!(host.evaluate_string("trace.join()").unwrap(),"body");
    host.isolate.get_slot_mut::<ModuleReportQueue>().unwrap().jobs.front_mut().unwrap().retry=Instant::now();
    assert_eq!(host.poll_module_reports().unwrap(),1);assert!(!host.has_pending_script_reporting());assert_eq!(host.evaluate_string("trace.join()").unwrap(),"body,error");assert_eq!(host.poll_module_reports().unwrap(),0);
 }
 #[test]fn original_tla_promise_is_retained_without_forwarding_reactions(){
    let mut host=host();host.execute_module("https://example.test/tla.js","globalThis.runs=(globalThis.runs||0)+1;await new Promise(resolve=>globalThis.finish=resolve);",&BTreeMap::new()).unwrap();
    let promise=host.isolate.get_slot::<ModuleEvaluations>().unwrap().0.values().next().unwrap().1.clone();
    host.execute_module("https://example.test/tla.js","throw Error('must reuse original record');",&BTreeMap::new()).unwrap();
    let same=host.isolate.get_slot::<ModuleEvaluations>().unwrap().0.values().next().unwrap().1.clone();assert!(promise==same);assert_eq!(host.evaluate_string("String(runs)").unwrap(),"1");
    host.execute("finish()").unwrap();host.run_microtask_checkpoint().unwrap();v8::scope!(let scope,&mut host.isolate);let promise=v8::Local::new(scope,promise);assert_eq!(promise.state(),v8::PromiseState::Fulfilled);
 }
 #[test]fn retiring_pending_report_releases_admission_without_publication(){
    let mut host=host();let frame=host.create_realm("https://example.test",7,Some(1)).unwrap();host.activate_realm(frame).unwrap();dynamic_import_continuation_tests::install_document(&mut host);
    FAIL_MODULE_REPORT_INSTALL.with(|fail|fail.set(true));host.execute_module_script_for_request("https://example.test/bad.js","export const bad = ;",&BTreeMap::new(),None).unwrap();
    assert_eq!(host.isolate.get_slot::<ModuleReportQueue>().unwrap().admitted.get(),1);host.remove_realm(frame).unwrap();assert_eq!(host.isolate.get_slot::<ModuleReportQueue>().unwrap().admitted.get(),0);assert!(host.module_report_delay().is_none());assert_eq!(host.poll_module_reports().unwrap(),0);
 }
 #[test]fn report_install_deadline_is_terminal_without_replaying_body_or_error(){
    let mut host=host();host.execute("globalThis.runs=0;globalThis.errors=0;onerror=()=>{errors++;return true}").unwrap();
    FAIL_MODULE_REPORT_INSTALL.with(|fail|fail.set(true));
    host.execute_module_script_for_request("https://example.test/expired.js","runs++;throw new Error('original');",&BTreeMap::new(),None).unwrap();
    host.isolate.get_slot_mut::<ModuleReportQueue>().unwrap().jobs.front_mut().unwrap().deadline=Instant::now();
    assert_eq!(host.module_report_delay(),Some(Duration::ZERO));assert!(host.poll_module_reports().is_err());
    assert_eq!(host.isolate.get_slot::<ModuleReportQueue>().unwrap().admitted.get(),0);assert_eq!(host.poll_module_reports().unwrap(),0);
    assert_eq!(host.evaluate_string("runs+'|'+errors").unwrap(),"1|0");
 }

}

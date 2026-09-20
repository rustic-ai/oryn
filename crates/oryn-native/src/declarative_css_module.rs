// Included in raw_v8. Declarative sheets are consumers of the ordinary typed
// module map and modulepreload graph, never a second CSS fetch/cache.
fn declarative_css_value<'s>(scope:&mut v8::PinScope<'s,'_>,environment:EnvironmentId,url:&str)->Option<v8::Local<'s,v8::Value>>{
    let typed=scope.get_slot::<TypedModules>()?;
    let module=match typed.entries.get(&(environment,url.into(),crate::ModuleType::Css))?{Some(Ok(module))=>module,_=>return None};
    let value=typed.values.iter().find(|(owner,record,_)|*owner==environment&&record==module)?.2.clone();
    Some(v8::Local::new(scope,value))
}
fn publish_declarative_css_module(scope:&mut v8::PinScope,url:&str,module:Option<v8::Local<v8::Module>>)->Result<(),oryn_js::v8_bindings::V8BindingError>{
    let context=scope.get_current_context();let Some(state)=context.get_slot::<V8BindingState>()else{return Ok(())};
    let sheet=if module.is_some(){let environment=current_environment(scope).map_err(oryn_js::v8_bindings::V8BindingError::EventCallback)?;declarative_css_value(scope,environment.id,url)}else{None};
    oryn_js::v8_bindings::complete_declarative_stylesheet(scope,&state,url,sheet)
}
fn prepare_declarative_css_module<'s>(scope:&mut v8::PinScope<'s,'_>,state:&Rc<V8BindingState>,specifier:&str)->Result<oryn_js::v8_bindings::DeclarativeStylesheetResult<'s>,oryn_js::v8_bindings::V8BindingError>{
    use oryn_js::v8_bindings::{DeclarativeStylesheetResult as Outcome,V8BindingError};
    if state.execution_retired(){return Err(V8BindingError::Allocation("declarative CSS document retired"));}
    let base=oryn_js::v8_bindings::document_base_url(scope,state,true)?;
    let base_url=Url::parse(&base).map_err(|error|V8BindingError::EventCallback(error.to_string()))?;
    let environment=current_environment(scope).map_err(V8BindingError::EventCallback)?;
    let map=module_import_map(scope);
    let url=match map.borrow().resolve(&base_url,specifier){Ok(Some(url))=>url,_=>return Ok(Outcome::Invalid)};
    map.borrow_mut().record_resolution(&base_url,specifier).map_err(V8BindingError::EventCallback)?;
    if let Some(sheet)=declarative_css_value(scope,environment.id,&url){return Ok(Outcome::Ready{url,sheet});}
    let failed=scope.get_slot::<TypedModules>().is_some_and(|typed|matches!(typed.entries.get(&(environment.id,url.clone(),crate::ModuleType::Css)),Some(Some(Err(_)))));
    let results=typed_module_fetch_results(scope);
    if failed||matches!(results.borrow().0.get(&(url.clone(),crate::ModuleType::Css)),Some(None)){return Ok(Outcome::Failed{url});}
    let integrity=map.borrow().integrity_for(&url).map(str::to_owned);let policy=state.document_referrer_policy();
    let cached=results.borrow().0.contains_key(&(url.clone(),crate::ModuleType::Css));
    if !cached&&Url::parse(&url).is_ok_and(|url|url.scheme()=="data"){
        let backend=module_fetch_backend_in_scope(scope).map_err(|error|V8BindingError::EventCallback(error.to_string()))?;
        #[cfg(unix)]
        let creator=if current_context_endpoint(scope).is_some(){state.native_worker_creator()?}else{state.native_local_creator_identity()?};
        #[cfg(not(unix))]
        let creator=state.native_local_creator_identity()?;
        match backend.fetch_local_css_module(&url,&base,&policy,integrity.clone(),creator){
            Ok((final_url,source))=>results.borrow_mut().retain(&url,crate::ModuleType::Css,Some((&final_url,&source)),true).map_err(|error|V8BindingError::EventCallback(error.to_string()))?,
            Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Fetch(_)|crate::script_load_failure::ScriptLoadFailure::Integrity(_)))=>{
                results.borrow_mut().retain(&url,crate::ModuleType::Css,None,true).map_err(|error|V8BindingError::EventCallback(error.to_string()))?;return Ok(Outcome::Failed{url});
            },
            Err(error)=>return Err(V8BindingError::EventCallback(error.to_string())),
        }
    }
    if results.borrow().0.contains_key(&(url.clone(),crate::ModuleType::Css)){
        let context=scope.get_current_context();let previous_link=context.get_slot::<PreparedImportLink>();context.set_slot(Rc::new(PreparedImportLink{root:RefCell::new(Some(url.clone())),error:None}));
        let _=take_module_preparation_failure(scope);
        v8::tc_scope!(let caught,scope);
        let module=load_typed_module_record(caught,&base,&url,crate::ModuleType::Css,false,&policy);
        context.remove_slot::<PreparedImportLink>();if let Some(previous)=previous_link{context.set_slot(previous);}
        if let Some(failure)=take_module_preparation_failure(caught){if caught.has_caught(){caught.reset();}return Err(V8BindingError::EventCallback(failure.to_string()));}
        if module.is_none(){if caught.has_caught(){caught.reset();}return Ok(Outcome::Failed{url});}
        let sheet=declarative_css_value(caught,environment.id,&url).ok_or(V8BindingError::Allocation("CSS module default identity unavailable"))?;
        return Ok(Outcome::Ready{url,sheet});
    }
    let queue=module_preloads(scope);
    let pending=queue.borrow().jobs.iter().any(|job|job.environment.id==environment.id&&job.key.url==url&&job.key.kind==Some(crate::ModuleType::Css));
    if !pending{
        admit_module_preload(scope,state,None,ModulePreloadKey{url:url.clone(),kind:Some(crate::ModuleType::Css),include:false,policy,integrity,base,media:String::new(),nonce:None,parser_inserted:true})
            .map_err(|error|V8BindingError::EventCallback(error.to_string()))?;
    }
    Ok(Outcome::Pending{url})
}
impl RawV8Host{
    fn publish_declarative_css_preload(&mut self,key:&ModulePreloadKey,loaded:bool)->Result<(),HostError>{
        if key.kind!=Some(crate::ModuleType::Css){return Ok(())}
        let context=self.contexts.get(&self.active_realm).cloned().ok_or_else(||HostError::Execution("declarative CSS realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);
        let environment=current_environment(scope).map_err(HostError::Execution)?;
        let sheet=if loaded{declarative_css_value(scope,environment.id,&key.url)}else{None};
        let state=context.get_slot::<V8BindingState>().ok_or_else(||HostError::Execution("declarative CSS owner absent".into()))?;
        oryn_js::v8_bindings::complete_declarative_stylesheet(scope,&state,&key.url,sheet).map_err(|error|HostError::Execution(error.to_string()))
    }
}

#[cfg(test)] mod declarative_css_native_tests {
 use super::*;
 fn host()->RawV8Host{let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<!doctype html><head></head><body></body>").into_runtime().unwrap())),1).unwrap();host.commit_location("https://example.test/").unwrap();host}
 fn network_host()->(RawV8Host,Arc<std::sync::atomic::AtomicUsize>){
  struct Css(Arc<std::sync::atomic::AtomicUsize>);
  impl crate::network::NetworkBroker for Css{fn send(&self,request:crate::network::NetworkRequest)->Result<crate::network::NetworkResponse,crate::network::NetworkError>{
   self.0.fetch_add(1,std::sync::atomic::Ordering::SeqCst);let found=request.url=="https://example.test/sheet.css";
   Ok(crate::network::NetworkResponse{response_policy: None,timing:None,request_id:request.request_id,status:if found{200}else{404},headers:vec![("content-type".into(),"text/css".into())],body:if found{b"div{color:red}".to_vec()}else{Vec::new()}})
  }}
  let requests=Arc::new(std::sync::atomic::AtomicUsize::new(0));let mut host=RawV8Host::new_with_network(SharedNetworkBroker::new(Css(requests.clone())),Url::parse("https://example.test/").unwrap());dynamic_import_continuation_tests::install_document(&mut host);(host,requests)
 }
 #[test] fn data_css_is_synchronous_and_reuses_native_module_default_identity(){
  let mut host=host();host.execute(r#"globalThis.target=document.createElement('div');document.body.append(target);target.setHTMLUnsafe('<template shadowrootmode="open" shadowrootadoptedstylesheets="data:text/css,div%7Bcolor:red%7D data:text/css,div%7Bcolor:red%7D"></template>');globalThis.sheet=target.shadowRoot.adoptedStyleSheets[0]"#).unwrap();
  assert_eq!(host.evaluate_string("String(sheet.cssRules.length===1&&sheet===target.shadowRoot.adoptedStyleSheets[1])").unwrap(),"true");
  let context=host.contexts.get(&host.active_realm).cloned().unwrap();v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);
  let state=context.get_slot::<V8BindingState>().unwrap();let url="data:text/css,div%7Bcolor:red%7D";
  let oryn_js::v8_bindings::DeclarativeStylesheetResult::Ready{sheet:first,..}=prepare_declarative_css_module(scope,&state,url).unwrap()else{panic!("data CSS must be ready")};
  let module=load_typed_module_record(scope,"https://example.test/",url,crate::ModuleType::Css,false,"strict-origin-when-cross-origin").unwrap();
  let environment=current_environment(scope).unwrap();let second=declarative_css_value(scope,environment.id,url).unwrap();assert!(first.strict_equals(second));let _=module;
 }
 #[test] fn sandboxed_declarative_css_prepares_with_script_execution_forbidden(){
  let mut host=host();host.set_standalone_resource_policy(crate::csp::CspPolicySet::from_headers("https://example.test/",&[("content-security-policy".into(),"sandbox allow-same-origin; script-src 'none'; style-src data:".into())])).unwrap();
  let context=host.contexts.get(&host.active_realm).cloned().unwrap();v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);let state=context.get_slot::<V8BindingState>().unwrap();assert!(!state.automatic_features_allowed());
  assert!(matches!(prepare_declarative_css_module(scope,&state,"data:text/css,div%7Bcolor:red%7D").unwrap(),oryn_js::v8_bindings::DeclarativeStylesheetResult::Ready{..}));
  let policy=context.get_slot::<RefCell<crate::csp::CspPolicySet>>().unwrap();assert!(!policy.borrow().authorize(&crate::csp::CspRequest{destination:crate::csp::CspDestination::Script,url:"https://example.test/code.js".into(),nonce:None,inline_source:None,parser_inserted:true,initiated_by_trusted_script:false}).allowed);
 }
 #[test] fn typed_preparation_failure_retries_cached_bytes_without_poisoning_sheet(){
  let mut host=host();let url="https://example.test/sheet.css";host.retain_typed_module_fetch(url,crate::ModuleType::Css,Some((url.into(),"div{color:red}".into()))).unwrap();
  let context=host.contexts.get(&host.active_realm).cloned().unwrap();v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);let state=context.get_slot::<V8BindingState>().unwrap();
  FAIL_TYPED_PREPARATION.with(|failure|*failure.borrow_mut()=Some(crate::script_load_failure::ScriptLoadFailure::Infrastructure("injected CSS preparation failure".into())));
  assert!(prepare_declarative_css_module(scope,&state,url).is_err());
  let oryn_js::v8_bindings::DeclarativeStylesheetResult::Ready{sheet:first,..}=prepare_declarative_css_module(scope,&state,url).unwrap()else{panic!("retry must use retained CSS")};
  let oryn_js::v8_bindings::DeclarativeStylesheetResult::Ready{sheet:second,..}=prepare_declarative_css_module(scope,&state,url).unwrap()else{panic!("cached CSS")};assert!(first.strict_equals(second));
 }
 fn drain(host:&mut RawV8Host){let deadline=Instant::now()+Duration::from_secs(5);loop{let queue=host.isolate.get_slot::<Rc<RefCell<ModulePreloads>>>().unwrap().clone();if queue.borrow().jobs.is_empty(){break}for job in &mut queue.borrow_mut().jobs{job.retry=Instant::now()}host.poll_module_preloads().unwrap();assert!(Instant::now()<deadline);std::thread::yield_now();}}
 #[test] fn async_css_uses_preload_cache_and_failure_keeps_original_placeholder(){
  let (mut host,requests)=network_host();
  host.execute(r#"globalThis.target=document.createElement('div');document.body.append(target);target.setHTMLUnsafe('<template shadowrootmode="open" shadowrootadoptedstylesheets="/sheet.css /sheet.css /missing.css"></template>');globalThis.before=Array.from(target.shadowRoot.adoptedStyleSheets);target.shadowRoot.adoptedStyleSheets=[before[1],before[2],before[0]]"#).unwrap();
  let queue=host.isolate.get_slot::<Rc<RefCell<ModulePreloads>>>().unwrap().clone();assert_eq!(queue.borrow().jobs.len(),2,"duplicate CSS URLs share admission");
  drain(&mut host);assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst),2,"one successful CSS fetch plus one missing response");
  assert_eq!(host.evaluate_string("String(target.shadowRoot.adoptedStyleSheets[0]===target.shadowRoot.adoptedStyleSheets[2]&&target.shadowRoot.adoptedStyleSheets[0]!==before[0]&&target.shadowRoot.adoptedStyleSheets[1]===before[2]&&target.shadowRoot.adoptedStyleSheets[0].cssRules.length===1)").unwrap(),"true");
 }
 #[cfg(unix)] #[test] fn parked_css_completion_retains_job_without_runnable_deadline(){
  let (mut host,requests)=network_host();
  host.execute(r#"globalThis.target=document.createElement('div');document.body.append(target);target.setHTMLUnsafe('<template shadowrootmode="open" shadowrootadoptedstylesheets="/sheet.css"></template>');globalThis.before=target.shadowRoot.adoptedStyleSheets[0]"#).unwrap();
  let state=host.binding_states[&host.active_realm].clone();state.set_execution_suspended(true);assert!(state.execution_suspended());assert_eq!(host.poll_module_preloads().unwrap(),0);
  let queue=host.isolate.get_slot::<Rc<RefCell<ModulePreloads>>>().unwrap().clone();assert_eq!(queue.borrow().jobs.len(),1);assert!(host.module_preload_delay().is_none());state.set_execution_suspended(false);assert!(!state.execution_suspended());assert!(host.module_preload_delay().is_some());drain(&mut host);assert!(queue.borrow().jobs.is_empty());assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst),1);
 }

}

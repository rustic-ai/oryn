// External declarative patches: native task admission, bounded response custody,
// exact document cancellation, and retained parser/event publication.
include!("external_template_transport.rs");
#[derive(Default)]
struct ExternalTemplates { jobs:std::collections::VecDeque<ExternalTemplate>, seen:BTreeMap<(EnvironmentId,NodeId),(u64,u64)> }
struct ExternalTemplate {
 environment:EnvironmentDescriptor, owner:Rc<V8BindingState>, epoch:u64, cancellation:u64,
 target:oryn_dom::TrackedNodeLease, options:oryn_dom::html::ExternalTemplateOptions, transport:ExternalTemplateTransport, request_id:u64,
 pending:Option<TemplatePending>,
 stream:Option<u64>, decoder:Option<oryn_net::IncrementalTextDecoder>, parser:Option<oryn_dom::html::StreamingParser>,
 input:String, buffered:bool, eof:bool, parsed_eof:bool, bytes:usize,
 events:std::collections::VecDeque<oryn_dom::html::ParserEvent>, deadline:Instant, suspended_at:Option<Instant>, terminal_failure:bool,
}

fn external_templates(scope:&mut v8::PinScope)->Rc<RefCell<ExternalTemplates>>{
 if let Some(queue)=scope.get_slot::<Rc<RefCell<ExternalTemplates>>>(){return queue.clone()}
 let queue=Rc::new(RefCell::new(ExternalTemplates::default()));scope.set_slot(queue.clone());queue
}
fn capture_native_resource_preloads(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,node:NodeId)->Result<(),oryn_js::v8_bindings::V8BindingError>{
 capture_module_preloads(scope,state,node)?;capture_external_templates(scope,state,node)
}
fn capture_external_templates(scope:&mut v8::PinScope,state:&Rc<V8BindingState>,root:NodeId)->Result<(),oryn_js::v8_bindings::V8BindingError>{
 {
 use oryn_js::v8_bindings::V8BindingError as E;
 if state.execution_retired()||state.document_load_stopped(){return Ok(())}
 let dom=state.dom();let nodes=dom.borrow().arena().descendants_inclusive(root)?;
 let candidates=nodes.into_iter().filter(|node|{let dom=dom.borrow();dom.is_connected(*node).unwrap_or(false)&&dom.is_element(*node,"http://www.w3.org/1999/xhtml","template")&&dom.attribute_in_null_namespace(*node,"src").ok().flatten().is_some()}).collect::<Vec<_>>();
 if candidates.is_empty(){return Ok(())}
 let backend=module_fetch_backend_in_scope(scope).map_err(|e|E::EventCallback(e.to_string()))?;
 let initiator=match &backend{#[cfg(unix)]ModuleGraphBackend::Window(_,_,Some((_,initiator)),_)=>initiator.clone(),ModuleGraphBackend::Broker{initiator,..}=>initiator.clone(),ModuleGraphBackend::Sources(..)=>state.request_initiator_url().map_err(|e|E::EventCallback(e.to_string()))?,_=>return Err(E::EventCallback("external template requires document resource authority".into()))};
 let environment=current_environment(scope).map_err(E::EventCallback)?;let queue=external_templates(scope);
 for node in candidates{
  queue.borrow_mut().seen.retain(|(id,_),identity|*id!=environment.id||*identity==(state.document_authority_epoch(),state.document_load_cancellation_epoch()));
  if queue.borrow().seen.contains_key(&(environment.id,node)){continue}
  if queue.borrow().jobs.len()>=64||queue.borrow().seen.len()>=4096{return Err(E::Allocation("external template admission"))}
  let (src,nonce,integrity,policy,buffered)={let dom=dom.borrow();let attr=|name|dom.attribute_in_null_namespace(node,name).map(|v|v.unwrap_or("").to_owned());(attr("src")?,dom.element_nonce_for_csp(node)?.map(str::to_owned),attr("integrity")?,attr("referrerpolicy")?,dom.attribute_in_null_namespace(node,"buffer")?.is_some())};
  if src.len()+integrity.len()+policy.len()+nonce.as_ref().map_or(0,String::len)>192*1024{return Err(E::Allocation("external template metadata"))}
  let url=state.native_document_resource_url(&src).map_err(|e|E::EventCallback(e.to_string()))?;
  let request_id=next_external_template_request().ok_or(E::Allocation("external template request identity"))?;
  let transport=ExternalTemplateTransport::new(&backend,request_id).map_err(E::EventCallback)?;
  let target=oryn_dom::TrackedNodeLease::new(&dom,node)?;let options=oryn_dom::html::ExternalTemplateOptions::capture(&dom.borrow(),node)?;if options.name.len()>192*1024{return Err(E::Allocation("external template target metadata"))}
  let request=TemplateRequest{request_id,url:url.to_string(),initiator:initiator.clone(),nonce,integrity:integrity.clone(),referrer_policy:crate::referrer::ReferrerPolicy::parse_attribute(&policy).map(|v|v.as_str().to_owned()).unwrap_or_else(||state.document_referrer_policy())};
  queue.borrow_mut().jobs.try_reserve(1).map_err(|_|E::Allocation("external template queue"))?;
  let pending=transport.start(request).map_err(E::EventCallback)?;
  queue.borrow_mut().seen.insert((environment.id,node),(state.document_authority_epoch(),state.document_load_cancellation_epoch()));
  queue.borrow_mut().jobs.push_back(ExternalTemplate{environment:environment.clone(),owner:state.clone(),epoch:state.document_authority_epoch(),cancellation:state.document_load_cancellation_epoch(),target,options,transport,request_id,pending:Some(pending),stream:None,decoder:None,parser:None,input:String::new(),buffered:buffered||!integrity.is_empty(),eof:false,parsed_eof:false,bytes:0,events:Default::default(),deadline:Instant::now()+Duration::from_secs(60),suspended_at:None,terminal_failure:false});
 }Ok(())
 }
}
fn next_external_template_request()->Option<u64>{static NEXT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(1<<60);NEXT.fetch_update(std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire,|id|id.checked_add(1)).ok()}
impl RawV8Host{
 pub(crate) fn prepare_external_template(&mut self,node:NodeId)->Result<(),HostError>{
  let state=self.binding_states.get(&self.active_realm).cloned().ok_or_else(||HostError::Execution("external template realm absent".into()))?;
  let context=self.contexts.get(&self.active_realm).cloned().ok_or_else(||HostError::Execution("external template context absent".into()))?;
  v8::scope!(let scope,&mut self.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);capture_external_templates(scope,&state,node).map_err(|e|HostError::Execution(e.to_string()))
 }
 fn external_template_delay(&self)->Option<Duration>{
  {self.isolate.get_slot::<Rc<RefCell<ExternalTemplates>>>().filter(|q|!q.borrow().jobs.is_empty()).map(|_|Duration::from_millis(10))}
 }
 fn poll_external_templates(&mut self)->Result<usize,HostError>{
  {
   let Some(queue)=self.isolate.get_slot::<Rc<RefCell<ExternalTemplates>>>().cloned()else{return Ok(0)};
   let count=queue.borrow().jobs.len().min(16);let previous=self.active_realm;let mut completed=0;
   for _ in 0..count{let Some(mut job)=queue.borrow_mut().jobs.pop_front()else{break};
    if !job.live(){continue}
    if job.owner.execution_suspended(){job.suspended_at.get_or_insert_with(Instant::now);queue.borrow_mut().jobs.push_back(job);continue}
    if let Some(suspended)=job.suspended_at.take(){job.deadline=job.deadline.checked_add(suspended.elapsed()).unwrap_or_else(Instant::now);}
    if Instant::now()>=job.deadline{if self.contexts.contains_key(&previous){self.active_realm=previous}return Err(HostError::Execution("external template response/publication deadline exceeded".into()))}
    self.active_realm=job.environment.realm;
    let outcome=job.poll(self);
    match outcome{Ok(true)=>completed+=1,Ok(false)=>queue.borrow_mut().jobs.push_back(job),Err(error)=>{
     // Transport/policy failures are terminal resource failures. Retained parser
     // publication failures keep their exact input/event suffix for retry.
     if job.parser.is_some()&&!job.terminal_failure{queue.borrow_mut().jobs.push_back(job);if self.contexts.contains_key(&previous){self.active_realm=previous}return Err(error)}
    }}
   }
   if self.contexts.contains_key(&previous){self.active_realm=previous}Ok(completed)
  }
 }
}
impl ExternalTemplate{
 fn live(&self)->bool{let(dom,node)=self.target.current();!self.owner.execution_retired()&&self.epoch==self.owner.document_authority_epoch()&&self.cancellation==self.owner.document_load_cancellation_epoch()&&Rc::ptr_eq(&dom,&self.owner.dom())&&dom.borrow().node_document(node).ok().flatten()==Some(self.owner.document_root())&&(self.parsed_eof||self.parser.as_ref().is_some_and(|parser|parser.external_template_cleanup_started())||dom.borrow().is_connected(node).unwrap_or(false))}
 fn poll(&mut self,host:&mut RawV8Host)->Result<bool,HostError>{

  if let Some(receiver)=self.pending.as_ref(){match receiver.try_recv(){
   Err(std::sync::mpsc::TryRecvError::Empty)=>return Ok(false),
   Err(_)=>{self.terminal_failure=true;return Err(HostError::Execution("external template response disconnected".into()))},
   Ok(result)=>{self.pending=None;self.terminal_failure=true;match result.map_err(HostError::Execution)?{
    TemplateResponse::Started{request_id,stream_id,status,headers}if request_id==self.request_id&&self.stream.is_none()=>{if !(200..300).contains(&status){return Err(HostError::Execution("external template HTTP status".into()))}self.transport.request_finished();self.stream=Some(stream_id);let content_type=headers.iter().find(|(n,_)|n.eq_ignore_ascii_case("content-type")).map(|(_,v)|v.as_str());self.decoder=Some(oryn_net::IncrementalTextDecoder::with_fallback(content_type,crate::network::navigation_text_kind(content_type),None));},
    TemplateResponse::Chunk{stream_id,bytes,done}if Some(stream_id)==self.stream=>{
     if bytes.len()>64*1024||bytes.is_empty()&&!done{return Err(HostError::Execution("external template chunk shape".into()))}self.bytes=self.bytes.checked_add(bytes.len()).filter(|n|*n<=8*1024*1024).ok_or_else(||HostError::LimitExceeded{resource:"external template body".into()})?;
     let decoder=self.decoder.as_mut().ok_or_else(||HostError::Execution("external template decoder absent".into()))?;self.input.push_str(&decoder.push(&bytes));if done{self.input.push_str(&decoder.finish());self.eof=true;self.transport.stream_finished();}
    },_=>return Err(HostError::Execution("external template response identity".into()))
   }self.terminal_failure=false;}
  }}
  if (!self.buffered||self.eof)&&(!self.input.is_empty()||self.eof||self.parser.as_ref().is_some_and(|parser|parser.has_pending_mutations())){
   if self.parser.is_none(){let(dom,node)=self.target.current();self.parser=(if self.buffered{oryn_dom::html::StreamingParser::with_external_template_buffered_captured(dom,node,self.options.clone(),None,true)}else{oryn_dom::html::StreamingParser::with_external_template_captured(dom,node,self.options.clone(),None,true)}).map_err(|e|HostError::Execution(e.to_string()))?;if self.parser.is_none(){return Ok(true)}}
   if !self.input.is_empty()||self.parser.as_ref().is_some_and(|parser|parser.has_pending_mutations()){let parser=self.parser.as_mut().unwrap();let(progress,construction_error)=host.with_parser_construction(||parser.push_until_resource_or_script(&self.input))?;self.input.drain(..progress.consumed_bytes);self.events.extend(parser.take_events());if let Some(error)=construction_error{return Err(error)}}
   if self.eof&&self.input.is_empty()&&!self.parsed_eof{self.parsed_eof=self.parser.as_mut().unwrap().prepare_finish();self.events.extend(self.parser.as_mut().unwrap().take_events());}
  }
  if !self.buffered||self.parsed_eof{
   while let Some(event)=self.events.front(){
    if let oryn_dom::html::ParserEvent::EventHandlerAttributeChanged{node,name}=event{host.prepare_event_handler_attribute(*node,name)?;self.events.pop_front();continue;}
    let node=match event{oryn_dom::html::ParserEvent::ScriptReady{node,..}|oryn_dom::html::ParserEvent::ExternalTemplateDiscovered{node}|oryn_dom::html::ParserEvent::ModulePreloadDiscovered{node}|oryn_dom::html::ParserEvent::StylesheetDiscovered{node,..}|oryn_dom::html::ParserEvent::FrameDiscovered{node,..}|oryn_dom::html::ParserEvent::FrameLocalDocument{node}|oryn_dom::html::ParserEvent::EventHandlerAttributeChanged{node,..}|oryn_dom::html::ParserEvent::InlineStyleReady{node}=>Some(*node),_=>None};
    if let Some(node)=node{let context=host.contexts.get(&host.active_realm).cloned().ok_or_else(||HostError::Execution("external template publication realm".into()))?;v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,context);let scope=&mut v8::ContextScope::new(scope,context);oryn_js::v8_bindings::prepare_native_fragment_nodes(scope,&self.owner,&[node]).map_err(|e|HostError::Execution(e.to_string()))?;}
    self.events.pop_front();if !self.live(){return Ok(true)}
   }
  }
  if self.parsed_eof&&self.events.is_empty(){return Ok(true)}
  if self.pending.is_none()&&!self.eof&&(self.buffered||self.input.is_empty())&&self.parser.as_ref().is_none_or(|parser|!parser.has_pending_mutations()){if let Some(id)=self.stream{self.pending=Some(self.transport.read(id).map_err(HostError::Execution)?);}}
  Ok(false)
 }
}
fn external_template_chunk(encoded:&str,done:bool)->Result<Vec<u8>,String>{
 if encoded.len()>((64*1024+2)/3)*4{return Err("external template encoded chunk limit".into())}
 let bytes=base64::Engine::decode(&base64::engine::general_purpose::STANDARD,encoded).map_err(|e|e.to_string())?;
 if bytes.len()>64*1024||bytes.is_empty()&&!done{return Err("external template chunk shape".into())}Ok(bytes)
}
#[cfg(test)]mod external_template_tests{
 #[test]fn chunk_admission_rejects_empty_nonterminal_and_oversize(){assert!(super::external_template_chunk("",false).is_err());assert!(super::external_template_chunk("",true).unwrap().is_empty());let encoded=base64::Engine::encode(&base64::engine::general_purpose::STANDARD,vec![0;65537]);assert!(super::external_template_chunk(&encoded,true).is_err());}
}

#[cfg(test)]
mod external_template_host_regressions {
 use super::*;
 fn fixture(buffered:bool)->(RawV8Host,oryn_dom::SharedDomRuntime,NodeId){
  let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);
  let source=format!("<!doctype html><body><section id=target><?start name=patch>OLD<?end></section><template id=patch for=patch src='/fragment' {}></template></body>",if buffered{"buffer"}else{""});
  let dom=Rc::new(RefCell::new(oryn_dom::html::parse(&source).into_runtime().unwrap()));let node=dom.borrow().find_by_id("patch").unwrap();host.install_dom(dom.clone(),1).unwrap();
  host.commit_location("https://example.test/").unwrap();
  host.set_module_sources(&BTreeMap::from([("https://example.test/fragment".into(),"<b id=published>NEW</b>".into())]));(host,dom,node)
 }
 fn settle(host:&mut RawV8Host){let deadline=Instant::now()+Duration::from_secs(5);loop{host.poll_external_templates().unwrap();if host.isolate.get_slot::<Rc<RefCell<ExternalTemplates>>>().is_none_or(|queue|queue.borrow().jobs.is_empty()){break}assert!(Instant::now()<deadline,"external template did not settle");std::thread::yield_now();}}
 #[test]fn source_backend_updates_target_and_removes_template_in_both_modes(){
  for buffered in [false,true]{let(mut host,dom,node)=fixture(buffered);host.prepare_external_template(node).unwrap();let target=dom.borrow().find_by_id("target").unwrap();assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"OLD");settle(&mut host);assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"NEW");assert!(dom.borrow().find_by_id("published").is_some());assert!(!dom.borrow().is_connected(node).unwrap());}
 }
 #[test]fn removal_before_response_publication_preserves_old_target(){
  let(mut host,dom,node)=fixture(true);host.prepare_external_template(node).unwrap();dom.borrow_mut().remove(node).unwrap();settle(&mut host);let target=dom.borrow().find_by_id("target").unwrap();assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"OLD");assert!(dom.borrow().find_by_id("published").is_none());
 }
 #[test]fn stopped_document_cannot_accept_late_template_response(){
  let(mut host,dom,node)=fixture(false);host.prepare_external_template(node).unwrap();host.stop_pending_document_load(host.active_realm()).unwrap();settle(&mut host);let target=dom.borrow().find_by_id("target").unwrap();assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"OLD");
 }
 #[test]fn buffered_scripts_observe_terminal_target_publication(){
  let(mut host,dom,node)=fixture(true);host.set_module_sources(&BTreeMap::from([("https://example.test/fragment".into(),"<script>globalThis.templateObserved=!!document.getElementById('tail')</script><b id=tail>TAIL</b>".into())]));
  host.prepare_external_template(node).unwrap();settle(&mut host);host.drain_scheduled_timers(false).unwrap();
  assert!(dom.borrow().find_by_id("tail").is_some());assert_eq!(host.evaluate_string("String(globalThis.templateObserved)").unwrap(),"true");
 }
 #[test]fn response_uses_admitted_marker_and_sanitize_attributes(){
  for buffered in [false,true]{let(mut host,dom,node)=fixture(buffered);let _retained=oryn_dom::TrackedNodeLease::new(&dom,node).unwrap();dom.borrow_mut().set_attribute(node,"sanitize","").unwrap();host.set_module_sources(&BTreeMap::from([("https://example.test/fragment".into(),"<b id=published onclick='bad()'>NEW</b><script>globalThis.badTemplate=true</script>".into())]));
   host.prepare_external_template(node).unwrap();dom.borrow_mut().set_attribute(node,"for","changed-after-admission").unwrap();dom.borrow_mut().set_attribute(node,"sanitize","unsafe").unwrap();settle(&mut host);
   // The default sanitizer also removes id. Locate the admitted target,
   // then require its entire published structure, not an arbitrary survivor.
   let target=dom.borrow().find_by_id("target").unwrap();
   let published={let dom=dom.borrow();let children=&dom.arena().get(target).unwrap().children;let elements=children.iter().copied().filter(|child|matches!(&dom.arena().get(*child).unwrap().kind,oryn_dom::NodeKind::Element{..})).collect::<Vec<_>>();assert_eq!(elements.len(),1);assert!(dom.is_element(elements[0],"http://www.w3.org/1999/xhtml","b"));assert_eq!(dom.serialize(elements[0]).unwrap(),"<b>NEW</b>");elements[0]};
   assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"NEW");assert!(dom.borrow().find_by_id("published").is_none());
   assert!(dom.borrow().attribute_in_null_namespace(published,"onclick").unwrap().is_none());assert!(dom.borrow().attribute_in_null_namespace(published,"id").unwrap().is_none());assert_eq!(dom.borrow().attribute_in_null_namespace(node,"for").unwrap(),Some("changed-after-admission"));
   assert_eq!(host.evaluate_string("typeof globalThis.badTemplate").unwrap(),"undefined");
  }
 }
 #[test]fn parked_document_retains_response_without_publishing(){
  let(mut host,dom,node)=fixture(true);host.prepare_external_template(node).unwrap();let state=host.binding_states[&host.active_realm()].clone();state.set_execution_suspended(true);host.poll_external_templates().unwrap();
  let target=dom.borrow().find_by_id("target").unwrap();assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"OLD");let queue=host.isolate.get_slot::<Rc<RefCell<ExternalTemplates>>>().unwrap();assert_eq!(queue.borrow().jobs.len(),1);assert!(queue.borrow().jobs.front().unwrap().suspended_at.is_some());
  state.set_execution_suspended(false);settle(&mut host);assert_eq!(dom.borrow().arena().text_content(target).unwrap(),"NEW");
 }
 #[test]fn repeated_discovery_does_not_admit_duplicate_response(){
  let(mut host,_,node)=fixture(false);host.prepare_external_template(node).unwrap();host.prepare_external_template(node).unwrap();let queue=host.isolate.get_slot::<Rc<RefCell<ExternalTemplates>>>().unwrap();assert_eq!(queue.borrow().jobs.len(),1);settle(&mut host);
 }
}

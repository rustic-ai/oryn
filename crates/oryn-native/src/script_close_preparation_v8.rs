// Retained script-close departure. No navigation history entry or pageswap is
// created: closing discards the active Document after its prompt and unload.
type BeforeUnloadAncestor=(Rc<V8BindingState>,NodeId,RealmId,u64,u64,u64);
struct BeforeUnloadRecipient{owner:Rc<V8BindingState>,generation:u64,epoch:u64,authority:u64,ancestors:Vec<BeforeUnloadAncestor>,receipt:Option<oryn_js::v8_bindings::prompts::BeforeUnloadReceipt>}
pub struct NativeBeforeUnloadTree{root:RealmId,identity:(NodeId,u64,u64,u64),owner:Rc<V8BindingState>,authority:u64,recipients:std::collections::VecDeque<BeforeUnloadRecipient>,result:Option<bool>,prompt_shown:bool,unload_guards:Vec<oryn_js::v8_bindings::UnloadGuard>}
pub struct NativeScriptClose{root:RealmId,expected:oryn_js::worker_wire::Creator,before:NativeBeforeUnloadTree,hide:Option<NativePageTransition>,unload:Option<NativePageTransition>,phase:u8,result:Option<bool>,unload_guards:Vec<oryn_js::v8_bindings::UnloadGuard>}
impl RawV8Host{
    fn before_unload_owner_current(&self,owner:&Rc<V8BindingState>,generation:u64,epoch:u64,authority:u64)->bool{
        !owner.execution_retired()&&!owner.execution_suspended()&&owner.document_generation()==generation&&owner.document_stream_epoch()==epoch&&owner.document_authority_epoch()==authority&&self.contexts.contains_key(&owner.realm())&&self.binding_states.get(&owner.realm()).is_some_and(|current|Rc::ptr_eq(current,owner))
    }
    pub fn prepare_before_unload_tree(&mut self,root:RealmId)->Result<NativeBeforeUnloadTree,HostError>{
        self.adopt_created_realms();
        let identity=self.capture_document_identity(root).ok_or_else(||HostError::Execution("beforeunload root unavailable".into()))?;
        let owner=self.binding_states.get(&root).cloned().ok_or_else(||HostError::Execution("beforeunload owner unavailable".into()))?;let authority=owner.document_authority_epoch();
        let mut pending=vec![(root,Vec::<BeforeUnloadAncestor>::new())];let mut recipients=std::collections::VecDeque::new();let mut seen=BTreeSet::new();
        while let Some((realm,ancestors))=pending.pop(){
            if !seen.insert(realm){continue}let Some((_,generation,epoch,true))=self.automation_realm_identity(realm)else{continue};let Some(binding)=self.binding_states.get(&realm).cloned()else{continue};let authority=binding.document_authority_epoch();
            recipients.push_back(BeforeUnloadRecipient{owner:binding.clone(),generation,epoch,authority,ancestors:ancestors.clone(),receipt:None});
            let mut children=Vec::new();for frame in self.automation_frame_order(realm)?{
                let child=self.isolate.get_slot::<FrameLifetimes>().and_then(|registry|registry.live.values().find(|entry|entry.parent_environment.realm==realm&&entry.frame==frame&&!entry.closed.get()).map(|entry|entry.environment.get().realm));
                if let Some(child)=child.filter(|child|self.automation_frame_child_live(realm,frame,*child)){let mut path=ancestors.clone();path.push((binding.clone(),frame,child,generation,epoch,authority));children.push((child,path));}
            }pending.extend(children.into_iter().rev());
        }
        Ok(NativeBeforeUnloadTree{root,identity,owner,authority,recipients,result:None,prompt_shown:false,unload_guards:Vec::new()})
    }
    pub fn begin_native_unload_tree(&mut self,root:RealmId)->Result<Vec<oryn_js::v8_bindings::UnloadGuard>,HostError>{
        let tree=self.prepare_before_unload_tree(root)?;
        Ok(tree.recipients.iter().map(|recipient|recipient.owner.begin_unload()).collect())
    }
    pub fn poll_before_unload_tree(&mut self,tree:&mut NativeBeforeUnloadTree)->Result<Option<bool>,HostError>{
        if let Some(result)=tree.result{return Ok(Some(result))}
        if self.capture_document_identity(tree.root)!=Some(tree.identity)||tree.owner.execution_retired()||tree.owner.execution_suspended()||tree.owner.document_authority_epoch()!=tree.authority||!self.binding_states.get(&tree.root).is_some_and(|owner|Rc::ptr_eq(owner,&tree.owner)){tree.cancel();return Ok(Some(false))}
        if tree.unload_guards.is_empty(){tree.unload_guards=tree.recipients.iter().map(|recipient|recipient.owner.begin_unload()).collect();}
        while let Some(recipient)=tree.recipients.front_mut(){
            let current=self.before_unload_owner_current(&recipient.owner,recipient.generation,recipient.epoch,recipient.authority)&&recipient.ancestors.iter().all(|(parent,frame,child,generation,epoch,authority)|self.before_unload_owner_current(parent,*generation,*epoch,*authority)&&self.automation_frame_child_live(parent.realm(),*frame,*child));
            if !current{if let Some(receipt)=recipient.receipt.as_mut(){receipt.cancel()}tree.recipients.pop_front();continue}
            if recipient.receipt.is_none(){recipient.receipt=Some(oryn_js::v8_bindings::prompts::BeforeUnloadReceipt::new(&recipient.owner,recipient.owner.has_sticky_user_activation()&&!tree.prompt_shown).map_err(|error|HostError::Execution(error.to_string()))?);}
            let context=self.contexts.get(&recipient.owner.realm()).cloned().ok_or_else(||HostError::Execution("beforeunload context missing".into()))?;
            let deadline=ExecutionDeadline::arm(&self.isolate,self.execution_wall_limit);
            let result={v8::scope!(let handles,&mut self.isolate);let context=v8::Local::new(handles,&context);let scope=&mut v8::ContextScope::new(handles,context);v8::tc_scope!(let scope,scope);
                let result=recipient.receipt.as_mut().unwrap().poll(scope);
                if deadline.expired()||scope.is_execution_terminating(){scope.cancel_terminate_execution();reset_mutation_observer_microtask_after_termination(scope);Err(HostError::Execution("beforeunload listener exceeded execution deadline".into()))}else{result.map_err(|error|HostError::Execution(error.to_string()))}
            };
            if self.capture_document_identity(tree.root)!=Some(tree.identity)||tree.owner.execution_suspended()||tree.owner.document_authority_epoch()!=tree.authority||!self.binding_states.get(&tree.root).is_some_and(|owner|Rc::ptr_eq(owner,&tree.owner)){tree.cancel();return Ok(Some(false))}
            tree.prompt_shown|=recipient.receipt.as_ref().is_some_and(|receipt|receipt.prompt_shown());
            match result{Ok(Some(true))=>{tree.recipients.pop_front();},Ok(Some(false))=>{tree.cancel();return Ok(Some(false))},Ok(None)=>return Ok(None),Err(error)=>{if !tree.recipients.front().and_then(|recipient|recipient.receipt.as_ref()).is_some_and(|receipt|receipt.retry_safe()){tree.cancel();}return Err(error)}}
        }
        tree.unload_guards.clear();tree.result=Some(true);Ok(Some(true))
    }
    pub(crate) fn prepare_native_script_close(&mut self,root:RealmId,expected:oryn_js::worker_wire::Creator)->Result<NativeScriptClose,HostError>{
        if self.native_creator_for_realm(root)?!=expected{return Err(HostError::Execution("script-close source replaced".into()))}
        let before=self.prepare_before_unload_tree(root)?;Ok(NativeScriptClose{root,expected,before,hide:None,unload:None,phase:0,result:None,unload_guards:Vec::new()})
    }
    pub(crate) fn poll_native_script_close(&mut self,close:&mut NativeScriptClose)->Result<Option<bool>,HostError>{
        if let Some(result)=close.result{return Ok(Some(result))}
        if close.phase==0&&self.native_creator_for_realm(close.root).ok().as_ref()!=Some(&close.expected){
            // A replaced Document cannot veto an already admitted traversable
            // close. Its captured receipt is obsolete; traversal unload selects
            // the current active Document below without repeating old listeners.
            close.before.cancel();close.phase=1;
        }
        if close.phase==0{match self.poll_before_unload_tree(&mut close.before)?{Some(true)=>close.phase=1,Some(false)=>{close.result=Some(false);return Ok(Some(false))},None=>return Ok(None)}}
        #[cfg(test)] if close.phase==1&&CLOSE_PAUSE_AFTER_BEFORE.with(|pause|pause.replace(false)){return Ok(None)}
        if close.phase==1{if close.hide.is_none(){
            // Definitely-close chooses the current active Document again when
            // its traversal unload step starts, after beforeunload permission.
            close.expected=self.native_creator_for_realm(close.root)?;
            close.unload_guards=self.begin_native_unload_tree(close.root)?;
            close.hide=Some(self.prepare_native_page_transition(close.root,"pagehide",false)?)}if !self.resume_native_page_transition(close.hide.as_mut().unwrap())?{return Ok(None)}close.phase=2;}
        if self.native_creator_for_realm(close.root).ok().as_ref()!=Some(&close.expected){close.unload_guards.clear();close.result=Some(false);return Ok(Some(false))}
        if close.phase==2{if close.unload.is_none(){close.unload=Some(self.prepare_native_page_transition(close.root,"unload",false)?)}if !self.resume_native_page_transition(close.unload.as_mut().unwrap())?{return Ok(None)}close.phase=3;}
        close.result=Some(true);close.unload_guards.clear();Ok(close.result)
    }
}
impl NativeBeforeUnloadTree{fn cancel(&mut self){for recipient in &mut self.recipients{if let Some(receipt)=recipient.receipt.as_mut(){receipt.cancel()}}self.recipients.clear();self.unload_guards.clear();self.result=Some(false);}}
impl Drop for NativeBeforeUnloadTree{fn drop(&mut self){if self.result.is_none(){self.cancel()}}}

#[cfg(test)]thread_local!{static CLOSE_PAUSE_AFTER_BEFORE:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(all(test,unix))]
mod script_close_departure_tests{
 use super::*;
 fn host()->RawV8Host{let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body>retained</body>").into_runtime().unwrap()));host.install_dom(dom,1).unwrap();host}
 #[test]fn close_retains_beforeunload_prefix_then_orders_descendant_hide_and_unload(){
  let mut host=host();host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('before');onpagehide=e=>trace.push('hide:'+e.persisted);onunload=()=>trace.push('unload');onpageswap=()=>trace.push('unexpected-swap');const f=document.createElement('iframe');document.body.append(f);f.contentWindow.onbeforeunload=()=>trace.push('child-before');f.contentWindow.onpagehide=e=>trace.push('child-hide:'+e.persisted);f.contentWindow.onunload=()=>trace.push('child-unload')").unwrap();
  let expected=host.native_creator_for_realm(RealmId(0)).unwrap();let mut job=host.prepare_native_script_close(RealmId(0),expected).unwrap();CLOSE_PAUSE_AFTER_BEFORE.with(|pause|pause.set(true));assert_eq!(host.poll_native_script_close(&mut job).unwrap(),None);assert_eq!(host.evaluate_string("trace.join(',')").unwrap(),"before,child-before");assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));host.activate_realm(RealmId(0)).unwrap();assert_eq!(host.evaluate_string("trace.join(',')").unwrap(),"before,child-before,hide:false,child-hide:false,unload,child-unload");
 }
 #[test]fn close_visibility_is_ordered_and_cannot_replace_selected_parent_document(){
  let mut host=host();
  host.execute(r#"
    globalThis.trace=[];
    onbeforeunload=()=>trace.push('root-before');
    onpagehide=()=>trace.push('root-hide');
    document.addEventListener('visibilitychange',()=>trace.push('root-visibility:'+document.visibilityState));
    onunload=()=>trace.push('root-unload');
    const f=document.createElement('iframe');document.body.append(f);
    const child=f.contentWindow;
    child.onbeforeunload=()=>trace.push('child-before');
    child.onpagehide=()=>trace.push('child-hide');
    child.document.addEventListener('visibilitychange',()=>{
      trace.push('child-visibility:'+child.document.visibilityState);
      document.open();document.write('<body>unexpected replacement</body>');document.close();
    });
    child.onunload=()=>trace.push('child-unload');
  "#).unwrap();
  let expected=host.native_creator_for_realm(RealmId(0)).unwrap();
  let mut job=host.prepare_native_script_close(RealmId(0),expected).unwrap();
  assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));
  assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));
  host.activate_realm(RealmId(0)).unwrap();
  assert_eq!(host.evaluate_string("trace.join(',')").unwrap(),"root-before,child-before,root-hide,root-visibility:hidden,child-hide,child-visibility:hidden,root-unload,child-unload");
  assert_eq!(host.evaluate_string("document.body.firstChild.textContent").unwrap(),"retained");
 }
 #[test]fn dismissed_beforeunload_cancels_close_without_hide_or_unload(){
  struct Dismiss;impl oryn_js::prompts::PromptBackend for Dismiss{fn request(&self,_request:oryn_js::prompts::PromptRequest)->Result<oryn_js::prompts::PromptResponse,oryn_js::prompts::PromptError>{Ok(oryn_js::prompts::PromptResponse::Dismissed)}fn cancel(&self,_token:oryn_js::prompts::PromptToken){}}
  let mut host=host();host.binding_states[&RealmId(0)].prompt_controller().set_backend(Arc::new(Dismiss)).unwrap();host.notify_user_activation().unwrap();host.execute("globalThis.trace=[];onbeforeunload=e=>{trace.push('before');e.preventDefault()};onpagehide=()=>trace.push('hide');onunload=()=>trace.push('unload')").unwrap();let expected=host.native_creator_for_realm(RealmId(0)).unwrap();let mut job=host.prepare_native_script_close(RealmId(0),expected).unwrap();assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(false));assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(false));assert_eq!(host.evaluate_string("trace.join(',')+':'+document.body.textContent").unwrap(),"before:retained");
 }
 #[test]fn pagehide_destructive_writes_are_ignored_until_unload_completes(){
  let mut host=host();host.execute("globalThis.trace=[];onpagehide=()=>{trace.push('hide');document.open();document.write('<body>new stream</body>');document.close();onunload=()=>trace.push('new-unload')}").unwrap();let expected=host.native_creator_for_realm(RealmId(0)).unwrap();let mut job=host.prepare_native_script_close(RealmId(0),expected).unwrap();assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));assert_eq!(host.evaluate_string("trace.join(',')+':'+document.body.textContent").unwrap(),"hide,new-unload:retained");
 }
 #[test]fn traversal_unload_selects_document_active_after_beforeunload_permission(){
  let mut host=host();host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('old-before')").unwrap();
  let expected=host.native_creator_for_realm(RealmId(0)).unwrap();let mut job=host.prepare_native_script_close(RealmId(0),expected).unwrap();
  CLOSE_PAUSE_AFTER_BEFORE.with(|pause|pause.set(true));assert_eq!(host.poll_native_script_close(&mut job).unwrap(),None);
  let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body>new active</body>").into_runtime().unwrap()));host.install_dom(dom,2).unwrap();
  host.execute("globalThis.trace=[];onbeforeunload=()=>trace.push('unexpected-second-before');onpagehide=()=>trace.push('new-hide');onunload=()=>trace.push('new-unload')").unwrap();
  assert_eq!(host.poll_native_script_close(&mut job).unwrap(),Some(true));assert_eq!(host.evaluate_string("trace.join(',')").unwrap(),"new-hide,new-unload");
 }

 #[test]fn one_beforeunload_prompt_is_shared_across_descendant_documents(){
  struct Accept(Arc<std::sync::atomic::AtomicUsize>);impl oryn_js::prompts::PromptBackend for Accept{fn request(&self,_request:oryn_js::prompts::PromptRequest)->Result<oryn_js::prompts::PromptResponse,oryn_js::prompts::PromptError>{self.0.fetch_add(1,std::sync::atomic::Ordering::SeqCst);Ok(oryn_js::prompts::PromptResponse::Accepted(String::new()))}fn cancel(&self,_token:oryn_js::prompts::PromptToken){}}
  let mut host=host();host.execute("globalThis.trace=[];onbeforeunload=e=>{trace.push('root');e.preventDefault()};const f=document.createElement('iframe');document.body.append(f);f.contentWindow.onbeforeunload=e=>{trace.push('child');e.preventDefault()}").unwrap();host.adopt_created_realms();
  let count=Arc::new(std::sync::atomic::AtomicUsize::new(0));let realms=host.binding_states.keys().copied().collect::<Vec<_>>();
  for realm in realms{host.binding_states[&realm].prompt_controller().set_backend(Arc::new(Accept(count.clone()))).unwrap();host.activate_realm(realm).unwrap();host.notify_user_activation().unwrap();}
  let mut tree=host.prepare_before_unload_tree(RealmId(0)).unwrap();assert_eq!(host.poll_before_unload_tree(&mut tree).unwrap(),Some(true));assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst),1);host.activate_realm(RealmId(0)).unwrap();assert_eq!(host.evaluate_string("trace.join(',')").unwrap(),"root,child");
 }

 #[test]fn beforeunload_protects_selected_parent_through_child_event_only(){
  for whole_parent in [true,false]{
   let mut host=host();host.execute("const f=document.createElement('iframe');document.body.append(f);f.contentWindow.onbeforeunload=()=>parent.document.open()").unwrap();host.adopt_created_realms();
   let child=*host.binding_states.keys().find(|realm|**realm!=RealmId(0)).unwrap();let selected=if whole_parent{RealmId(0)}else{child};
   let mut tree=host.prepare_before_unload_tree(selected).unwrap();let _=host.poll_before_unload_tree(&mut tree).unwrap();host.activate_realm(RealmId(0)).unwrap();
   assert_eq!(host.evaluate_string("String(document.childNodes.length===0)").unwrap(),if whole_parent{"false"}else{"true"});
  }
 }

}

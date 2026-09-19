// Each standalone history Document owns a separate suspended task queue. The
// agent itself stays available to opener and auxiliary roots.
struct StandaloneParkedRoot{environments:Vec<EnvironmentDescriptor>,microtasks:v8::UniqueRef<v8::MicrotaskQueue>}
#[derive(Default)]
struct StandaloneParkedRoots(BTreeMap<RealmId,StandaloneParkedRoot>);
impl RawV8Host {
 pub(crate) fn park_standalone_root(&mut self,realm:RealmId)->Result<(),HostError>{
  if self.realm_is_parked(realm){return Err(HostError::Execution("standalone document already parked".into()))}
  self.activate_realm(realm)?;self.run_microtask_checkpoint()?;self.adopt_created_realms();
  let environment=self.isolate.get_slot::<RealmEndpointRegistry>().and_then(|registry|registry.0.get(&realm)).map(|entry|entry.0).ok_or_else(||HostError::Execution("standalone cache environment absent".into()))?;
  let environments=self.document_environments(environment);let microtasks=self.suspend_root_microtasks(&environments)?;
  self.isolate.get_slot_mut::<NativeRealmIdentities>().unwrap().parked_environments.extend(environments.iter().map(|environment|environment.id));
  for environment in &environments{if let Some(state)=self.binding_states.get(&environment.realm){state.set_execution_suspended(true)}}
  if let Some(timers)=self.isolate.get_slot_mut::<TimerRegistry>(){timers.scheduler.advance_to(timers.started.elapsed().as_millis() as u64);for environment in &environments{timers.scheduler.suspend_document(environment.realm,environment.document_generation);}}
  if self.isolate.get_slot::<StandaloneParkedRoots>().is_none(){self.isolate.set_slot(StandaloneParkedRoots::default());}self.isolate.get_slot_mut::<StandaloneParkedRoots>().unwrap().0.insert(realm,StandaloneParkedRoot{environments,microtasks});Ok(())
 }
 pub(crate) fn restore_standalone_root(&mut self,realm:RealmId)->Result<(),HostError>{
  self.restore_standalone_navigation_proxy(realm)?;
  let Some(parked)=self.isolate.get_slot_mut::<StandaloneParkedRoots>().and_then(|roots|roots.0.remove(&realm))else{return Ok(())};
  if let Err(error)=self.resume_root_microtasks(&parked.environments,&parked.microtasks,false){self.isolate.get_slot_mut::<StandaloneParkedRoots>().unwrap().0.insert(realm,parked);return Err(error)}
  for environment in &parked.environments{
   self.isolate.get_slot_mut::<NativeRealmIdentities>().unwrap().parked_environments.remove(&environment.id);
   if let Some(state)=self.binding_states.get(&environment.realm){state.prepare_history_reveal();state.set_execution_suspended(false)}
  }
  if let Some(timers)=self.isolate.get_slot_mut::<TimerRegistry>(){for environment in &parked.environments{timers.scheduler.resume_document(environment.realm,environment.document_generation);}}
  if let Some(global)=self.contexts.get(&realm).cloned(){v8::scope!(let handles,&mut self.isolate);let context=v8::Local::new(handles,&global);let scope=&mut v8::ContextScope::new(handles,context);if let Some(state)=context.get_slot::<V8BindingState>(){state.defer_custom_element_reactions(scope);}oryn_js::v8_bindings::queue_mutation_observer_microtask(scope);}
  Ok(())
 }
 pub(crate) fn retire_standalone_root(&mut self,realm:RealmId)->Result<(),HostError>{
  self.adopt_created_realms();
  if let Some(parked)=self.isolate.get_slot_mut::<StandaloneParkedRoots>().and_then(|roots|roots.0.remove(&realm)){
   if let Err(error)=self.resume_root_microtasks(&parked.environments,&parked.microtasks,true){self.isolate.get_slot_mut::<StandaloneParkedRoots>().unwrap().0.insert(realm,parked);return Err(error)}
  }
  let Some(environment)=self.isolate.get_slot::<RealmEndpointRegistry>().and_then(|registry|registry.0.get(&realm)).map(|entry|entry.0)else{return Ok(())};
  let environments=self.document_environments(environment);let retired_realms=environments.iter().map(|environment|environment.realm).collect::<BTreeSet<_>>();let retired_environments=environments.iter().map(|environment|environment.id).collect::<BTreeSet<_>>();
  for environment in environments.into_iter().rev(){
   retire_environment(&mut self.isolate,environment.id);if let Some(state)=self.binding_states.remove(&environment.realm){state.retire_execution();}self.contexts.remove(&environment.realm);self.script_isolation.remove(&environment.realm);
   let identities=self.isolate.get_slot_mut::<NativeRealmIdentities>().unwrap();identities.parked_environments.remove(&environment.id);identities.pending_contexts.remove(&environment.realm);identities.contexts.remove(&environment.realm);identities.realms.remove(environment.realm);
  }
  if let Some(frames)=self.isolate.get_slot_mut::<FrameLifetimes>(){
   frames.live.retain(|_,frame|{let keep=!retired_realms.contains(&frame.root_realm)&&!retired_realms.contains(&frame.environment.get().realm);if !keep{frame.closed.set(true)}keep});
   frames.creating.retain(|(parent,_)|!retired_environments.contains(parent));frames.discarded_owners.retain(|retired,owner|!retired_realms.contains(retired)&&!retired_realms.contains(owner));frames.discarded_realms.retain(|retired|frames.discarded_owners.contains_key(retired));
  }
  if let Some(roots)=self.isolate.get_slot_mut::<StandaloneAuxiliaryRoots>(){roots.roots.remove(&realm);roots.pending.retain(|pending|*pending!=realm);}
  Ok(())
 }
 fn release_standalone_parked_roots(&mut self){
  let roots=self.isolate.get_slot_mut::<StandaloneParkedRoots>().map(|roots|std::mem::take(&mut roots.0)).unwrap_or_default();for(_,parked)in roots{self.release_owned_root_microtasks(&parked.environments,&parked.microtasks,true);}
 }
 pub(crate) fn standalone_root_discarded(&self,realm:RealmId)->bool{self.isolate.get_slot::<StandaloneScriptCloses>().is_some_and(|queue|queue.discarded.contains(&realm))}
}

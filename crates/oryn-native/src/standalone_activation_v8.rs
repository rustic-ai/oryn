// Standalone activation uses native document identities and frame lifetimes.
use super::*;
use oryn_js::{user_activation::LocalActivationBackend,worker_wire::Creator};
use std::time::{Duration,Instant};
#[derive(Default)]
struct Registry(RefCell<BTreeMap<usize,Entry>>);
struct Entry {
    state:std::rc::Weak<V8BindingState>, parent:Option<std::rc::Weak<FrameLifetime>>,
    identity:Creator, environment:EnvironmentDescriptor, parent_identity:Option<Creator>,
    origin:Origin, sticky:bool, transient:Option<Instant>,
}
impl Entry {
    fn current(&self)->Option<Rc<V8BindingState>> {
        let state=self.state.upgrade()?;
        if state.execution_retired()||state.execution_suspended(){return None}
        if let Some(edge)=&self.parent {
            let edge=edge.upgrade()?;let parent=edge.parent.upgrade()?;
            if edge.closed.get()||parent.execution_retired()||parent.execution_suspended()
                ||self.parent_identity.as_ref()!=parent.native_local_creator_identity().ok().as_ref()
                ||!edge.child.borrow().upgrade().is_some_and(|child|Rc::ptr_eq(&child,&state))
                ||parent.dom().borrow().frame_context_generation(edge.frame)!=Some(edge.connection_generation)
                ||!parent.dom().borrow().is_document_connected(edge.frame).unwrap_or(false){return None}
        }
        Some(state)
    }
    fn parent_key(&self)->Option<usize>{self.parent.as_ref()?.upgrade()?.parent.upgrade().map(|state|Rc::as_ptr(&state) as usize)}
}
struct Backend{registry:Rc<Registry>,key:usize,environment:EnvironmentId}
// Validate the complete route, including every intermediate document. An
// invalid edge never creates an artificial standalone root for its child.
fn live_path(entries:&BTreeMap<usize,Entry>,mut child:usize)->Option<Vec<usize>>{
    let mut path=Vec::new();let mut seen=BTreeSet::new();
    loop {
        if path.len()>=4096||!seen.insert(child){return None}
        let entry=entries.get(&child)?;let state=entry.current()?;
        if state.native_local_creator_identity().ok().as_ref()!=Some(&entry.identity){return None}
        path.push(child);
        let Some(edge)=&entry.parent else{return Some(path)};
        let edge=edge.upgrade()?;let parent=entry.parent_key()?;
        if entries.get(&parent)?.environment!=edge.parent_environment{return None}
        child=parent;
    }
}

impl Backend {
    fn access<T>(&self,creator:Creator,f:impl FnOnce(&mut BTreeMap<usize,Entry>,usize)->T)->Result<T,String>{
        let mut entries=self.registry.0.borrow_mut();
        entries.retain(|_,entry|entry.state.upgrade().is_some_and(|state|!state.execution_retired()));
        // Changed streams/documents never inherit a previous transient grant.
        for entry in entries.values_mut(){if let Some(state)=entry.current(){let identity=state.native_local_creator_identity().map_err(|error|error.to_string())?;if identity!=entry.identity{entry.identity=identity;entry.sticky=false;entry.transient=None;}}}
        let entry=entries.get(&self.key).ok_or("retired standalone activation owner")?;
        if entry.environment.id!=self.environment||live_path(&entries,self.key).is_none()||entry.identity!=creator{return Err("stale standalone activation identity".into())}
        Ok(f(&mut entries,self.key))
    }
}
impl LocalActivationBackend for Backend {
    fn notify(&self,creator:Creator)->Result<(),String>{self.access(creator,|entries,source|{
        let origin=entries[&source].origin.clone();
        let ancestors=live_path(entries,source).expect("validated activation source");let top=*ancestors.last().unwrap();
        let recipients=entries.iter().filter(|(key,entry)|live_path(entries,**key).is_some_and(|path|path.last()==Some(&top))&&(ancestors.contains(key)||entry.origin==origin)).map(|(key,_)|*key).collect::<Vec<_>>();
        let now=Instant::now();for key in recipients{let entry=entries.get_mut(&key).unwrap();entry.sticky=true;entry.transient=Some(now);}
    })}
    fn sticky(&self,creator:Creator)->Result<bool,String>{self.access(creator,|entries,key|entries[&key].sticky)}
    fn transient(&self,creator:Creator,consume:bool)->Result<bool,String>{self.access(creator,|entries,key|{
        let active=entries[&key].transient.is_some_and(|time|time.elapsed()<Duration::from_secs(5));
        if active&&consume {
            let top=*live_path(entries,key).expect("validated activation source").last().unwrap();
            let recipients=entries.keys().copied().filter(|candidate|live_path(entries,*candidate).is_some_and(|path|path.last()==Some(&top))).collect::<Vec<_>>();
            for key in recipients{entries.get_mut(&key).unwrap().transient=None;}
        }active
    })}
}
pub(super) fn install(scope:&mut v8::PinScope,context:v8::Local<v8::Context>)->Result<(),HostError>{
    if context_has_parent_platform(scope,context){return Ok(())}
    let Some(state)=context.get_slot::<V8BindingState>()else{return Ok(())};
    let origin=context.get_slot::<RealmDescriptor>().ok_or_else(||HostError::Execution("activation realm identity absent".into()))?.origin.clone();
    let registry=match scope.get_slot::<Rc<Registry>>(){Some(registry)=>registry.clone(),None=>{let registry=Rc::new(Registry::default());scope.set_slot(registry.clone());registry}};
    let key=Rc::as_ptr(&state) as usize;
    let identity=state.native_local_creator_identity().map_err(|error|HostError::Execution(error.to_string()))?;
    let environment=*context.get_slot::<EnvironmentDescriptor>().ok_or_else(||HostError::Execution("activation environment absent".into()))?;
    let parent=context.get_slot::<FrameLifetime>().map(|edge|Rc::downgrade(&edge));
    let parent_identity=parent.as_ref().and_then(|edge|edge.upgrade()).and_then(|edge|edge.parent.upgrade()).and_then(|state|state.native_local_creator_identity().ok());
    {let mut entries=registry.0.borrow_mut();entries.retain(|_,entry|entry.state.upgrade().is_some_and(|state|!state.execution_retired()));
        if !entries.contains_key(&key)&&entries.len()>=4096{return Err(HostError::Execution("standalone activation capacity".into()))}
        if let Some(entry)=entries.get_mut(&key){entry.parent=parent;entry.parent_identity=parent_identity;entry.origin=origin;let environment_changed=entry.environment!=environment;entry.environment=environment;if environment_changed||entry.identity!=identity{entry.identity=identity;entry.sticky=false;entry.transient=None;}}
        else{entries.insert(key,Entry{state:Rc::downgrade(&state),parent,identity,environment,parent_identity,origin,sticky:false,transient:None});}}
    state.set_local_activation_backend(Rc::new(Backend{registry,key,environment:environment.id}));Ok(())
}

pub(super) fn retire_environment(isolate:&mut v8::Isolate,id:EnvironmentId){
    if let Some(registry)=isolate.get_slot::<Rc<Registry>>(){registry.0.borrow_mut().retain(|_,entry|entry.environment.id!=id);}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_grants_expire_consume_and_reject_foreign_identity() {
        let mut host=RawV8Host::new();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())),1).unwrap();
        let context=host.contexts.get(&host.active_realm).unwrap().clone();
        v8::scope!(let scope,&mut host.isolate);
        let context=v8::Local::new(scope,&context);
        let state=context.get_slot::<V8BindingState>().unwrap();
        let identity=state.native_local_creator_identity().unwrap();
        let registry=scope.get_slot::<Rc<Registry>>().unwrap().clone();
        let key=Rc::as_ptr(&state) as usize;
        let backend=Backend{registry:registry.clone(),key,environment:registry.0.borrow()[&key].environment.id};
        assert!(!backend.transient(identity.clone(),false).unwrap());
        backend.notify(identity.clone()).unwrap();
        assert!(backend.sticky(identity.clone()).unwrap());
        assert!(backend.transient(identity.clone(),false).unwrap());
        assert!(backend.transient(identity.clone(),true).unwrap());
        assert!(!backend.transient(identity.clone(),false).unwrap());
        backend.notify(identity.clone()).unwrap();
        registry.0.borrow_mut().get_mut(&key).unwrap().transient=Some(Instant::now()-Duration::from_secs(6));
        assert!(!backend.transient(identity.clone(),false).unwrap());
        assert!(backend.sticky(identity.clone()).unwrap());
        let mut foreign=identity.clone();
        if let Creator::Window{document_generation,..}=&mut foreign{*document_generation+=1;}
        assert!(backend.notify(foreign).is_err());
        assert!(!backend.transient(identity.clone(),false).unwrap());
        let old=registry.0.borrow()[&key].environment;
        backend.notify(identity.clone()).unwrap();
        retire_environment(scope,old.id);
        assert!(backend.transient(identity.clone(),false).is_err());
        let replacement=scope.get_slot_mut::<AgentEnvironments>().unwrap().create(old.realm,old.document_generation,None).unwrap();
        context.set_slot(Rc::new(replacement));
        {let activation_scope=&mut v8::ContextScope::new(scope,context);
            install(activation_scope,context).unwrap();}
        let successor=Backend{registry:registry.clone(),key,environment:replacement.id};
        assert!(!successor.transient(identity.clone(),false).unwrap());
        successor.notify(identity.clone()).unwrap();
        retire_environment(scope,old.id);
        assert!(successor.transient(identity.clone(),false).unwrap());
        assert!(backend.transient(identity,false).is_err());
    }
    #[test]
    fn native_graph_includes_siblings_and_cousins_but_fences_every_ancestor() {
        // One real isolate owns all native document realms. Unwinding never
        // depends on Vec<OwnedIsolate>'s forward destruction order.
        let mut host=RawV8Host::new();
        host.install_dom(Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap())),1).unwrap();
        let root=host.active_realm;let mut realms=vec![root];
        for index in 1..6 {
            let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body></body>").into_runtime().unwrap()));
            let realm=host.create_realm_with_dom(Origin::from_url(if index==5{"https://cross.test"}else{"https://same.test"}),1,None,dom).unwrap();
            realms.push(realm);
        }
        let mut states=Vec::new();let mut environments=Vec::new();
        for realm in realms {
            let context=host.contexts[&realm].clone();
            v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,&context);
            states.push(context.get_slot::<V8BindingState>().unwrap());
            environments.push(*context.get_slot::<EnvironmentDescriptor>().unwrap());
        }
        let registry=Rc::new(Registry::default());let mut edges=Vec::new();
        // root -> source,sibling,branch,cross; branch -> cousin.
        let parents=[None,Some(0),Some(0),Some(0),Some(3),Some(0)];
        for index in 0..states.len(){
            let state=&states[index];let parent=parents[index].map(|parent_index|{
                let parent=&states[parent_index];let dom=parent.dom();let mut dom=dom.borrow_mut();
                let document=parent.document_root();
                let container=dom.children(document).unwrap().iter().copied().find(|node|matches!(dom.arena().get(*node).unwrap().kind,oryn_dom::NodeKind::Element{..}));
                let container=match container{Some(container)=>container,None=>{let root=dom.create_element("html");dom.append_child(document,root).unwrap();root}};
                let frame=dom.create_element("iframe");dom.append_child(container,frame).unwrap();let generation=dom.track_frame_context(frame);drop(dom);
                let token=format!("activation-frame-{index}");state.set_message_target(oryn_js::message_wire::WindowTarget::Frame(token.clone()));
                let edge=Rc::new(FrameLifetime{root_realm:environments[0].realm,message_token:token,
                    #[cfg(unix)] deferred_cleanup:RefCell::new(None),message_generation:std::cell::Cell::new(None),frame,connection_generation:generation,
                    parent:Rc::downgrade(parent),parent_environment:environments[parent_index],environment:std::cell::Cell::new(environments[index]),
                    initial_input_document:std::cell::Cell::new(None),
                    child:RefCell::new(Rc::downgrade(state)),opener:RefCell::new(None),closed:std::cell::Cell::new(false)});
                edges.push(edge.clone());Rc::downgrade(&edge)
            });
            registry.0.borrow_mut().insert(Rc::as_ptr(state) as usize,Entry{state:Rc::downgrade(state),parent,
                identity:state.native_local_creator_identity().unwrap(),environment:environments[index],
                parent_identity:parents[index].map(|parent|states[parent].native_local_creator_identity().unwrap()),
                origin:Origin::from_url(if index==5{"https://cross.test"}else{"https://same.test"}),sticky:false,transient:None});
        }
        let backend=|index:usize|Backend{registry:registry.clone(),key:Rc::as_ptr(&states[index]) as usize,environment:environments[index].id};
        let identity=|index:usize|states[index].native_local_creator_identity().unwrap();
        backend(1).notify(identity(1)).unwrap();
        for index in 0..5{assert!(backend(index).transient(identity(index),false).unwrap(),"recipient {index}");}
        assert!(!backend(5).transient(identity(5),false).unwrap());
        assert!(backend(2).transient(identity(2),true).unwrap());
        for index in 0..6{assert!(!backend(index).transient(identity(index),false).unwrap());}
        states[3].set_execution_suspended(true);
        backend(1).notify(identity(1)).unwrap();
        assert!(backend(4).transient(identity(4),false).is_err());
        assert!(backend(2).transient(identity(2),true).unwrap());
        states[3].set_execution_suspended(false);
        assert!(!backend(4).transient(identity(4),false).unwrap());
        // A successor environment on the same state cannot reuse the old
        // frozen parent's FrameLifetime edge, even with the same DOM nodes.
        let branch_key=Rc::as_ptr(&states[3]) as usize;
        registry.0.borrow_mut().get_mut(&branch_key).unwrap().environment.id=EnvironmentId(900);
        backend(1).notify(identity(1)).unwrap();
        assert!(backend(4).transient(identity(4),false).is_err());
        // Exact old-environment retirement must not delete its successor.
        host.isolate.set_slot(registry.clone());
        retire_environment(&mut host.isolate,environments[3].id);
        assert_eq!(registry.0.borrow()[&branch_key].environment.id,EnvironmentId(900));
        retire_environment(&mut host.isolate,EnvironmentId(900));
        assert!(!registry.0.borrow().contains_key(&branch_key));
        assert!(backend(4).notify(identity(4)).is_err());
        drop(edges);
    }

}

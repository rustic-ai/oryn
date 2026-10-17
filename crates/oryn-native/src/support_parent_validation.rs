// Canonical authority guards precede borrowing the original HTTP Creator.
impl BrowserContext {
    pub(crate) fn with_support_original_child<T>(&self,page:PageId,parent:&oryn_js::worker_wire::Creator,
        original:&std::sync::Mutex<Option<oryn_js::worker_wire::Creator>>,
        consume:impl FnOnce(&oryn_js::worker_wire::Creator,u64,u64)->Result<T,RuntimeError>)
        ->Result<T,RuntimeError>{
        use oryn_js::{message_wire::WindowTarget,worker_wire::Creator};
        let Creator::Window{target:WindowTarget::Root,document_generation:pg,stream_epoch:pe}=parent
            else{return Err(RuntimeError::NavigationCancelled);};
        let pages=self.inner.group.pages.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        if self.inner.workers_retired.load(Ordering::Acquire)||!pages.get(&page).is_some_and(PageRecord::is_live){return Err(RuntimeError::NavigationCancelled);}
        let origins=self.inner.security_origins.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        if !origins.contains_key(&page){return Err(RuntimeError::NavigationCancelled);}
        let policies=self.inner.worker_policies.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        let hub=self.inner.group.messages.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        let original=original.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        let child=original.as_ref().ok_or(RuntimeError::NavigationCancelled)?;
        let Creator::Window{target:target @ WindowTarget::Frame(frame),document_generation:cg,stream_epoch:ce}=child
            else{return Err(RuntimeError::NavigationCancelled);};
        let root=policies.root_lifetimes.get(&page).ok_or(RuntimeError::NavigationCancelled)?;
        if !root.current(*pg)||hub.prerender_scope_for_document(page.0,&WindowTarget::Root,*pg,*pe).is_some()
            ||hub.prerender_scope_for_document(page.0,target,*cg,*ce).is_some(){return Err(RuntimeError::NavigationCancelled);}
        hub.root_document_authority(page.0,*pg,*pe).map_err(RuntimeError::ProtocolViolation)?;
        let registered=hub.frame_document_authority(page.0,frame,*cg,*ce).map_err(RuntimeError::ProtocolViolation)?;
        let matches_key=|key:&(PageId,String,u64,u64)|key.0==page&&key.1==*frame&&key.2==*cg&&key.3==*ce;
        if policies.frame_security_epochs.iter().find(|(key,_)|matches_key(key)).map(|(_,epoch)|*epoch)!=Some(root.epoch)
            ||policies.frame_origins.iter().find(|(key,_)|matches_key(key)).map(|(_,origin)|origin)!=Some(&registered){return Err(RuntimeError::NavigationCancelled);}
        let lineage=policies.activation_parents.iter().any(|(owner,parent)|{
            matches!(owner,crate::agent_registry::AgentOwner::Window{page:p,target:t,generation:g,epoch:e}
                if *p==page&&t==target&&*g==*cg&&*e==*ce)
            &&matches!(parent,crate::agent_registry::AgentOwner::Window{page:p,target:WindowTarget::Root,generation:g,epoch:e}
                if *p==page&&*g==*pg&&*e==*pe)
        });
        if !lineage{return Err(RuntimeError::NavigationCancelled);}
        consume(child,root.epoch,root.epoch)
    }
}

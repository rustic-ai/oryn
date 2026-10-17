// Decoded backing is separate from font-display visual eligibility. The bytes
// and their prepaid custody share one allocation owner across cache/adapters.
struct ImplicitFontBacking {
    identity:usize,
    rule:oryn_layout::cssom::native_import_graph::NativeRule,
    epoch:dynamic_stylesheet_preparation::RetainedFontDemandEpoch,
    binary:dynamic_stylesheet_preparation::CssBinary,
    _bytes:ScriptSourceReservation,
    _metadata:Arc<dyn Send+Sync>,
    _payload_admission:Arc<dyn Send+Sync>,
    _store_metadata:Arc<dyn Send+Sync>,
}
#[derive(Default)]
struct ImplicitFontBackingEntries {
    entries:Vec<Arc<ImplicitFontBacking>>,
    capacity_charge:Option<Arc<dyn Send+Sync>>,
    _root_charge:Option<Arc<dyn Send+Sync>>,
}
impl LiveLayoutProvider {
    fn implicit_font_store(&self,admission:&dyn oryn_layout::cssom::native_import_graph::ImportAdmission)->Result<ImplicitFontBackingStore,String>{
        let owner=self.native_document_identity.get().ok_or("implicit backing owner retired")?;
        if let Some(store)=self.implicit_font_backing.borrow().as_ref(){return Ok(store.clone());}
        let charge=admission.reserve(std::mem::size_of::<RefCell<ImplicitFontBackingEntries>>()+2*std::mem::size_of::<usize>())?;
        if self.native_document_identity.get()!=Some(owner){return Err("implicit backing owner retired".into());}
        let store=Rc::new(RefCell::new(ImplicitFontBackingEntries{_root_charge:Some(charge),entries:Vec::new(),capacity_charge:None}));
        let mut slot=self.implicit_font_backing.try_borrow_mut().map_err(|_|"implicit backing installation borrow")?;
        if let Some(existing)=slot.as_ref(){let existing=existing.clone();drop(slot);drop(store);return Ok(existing);}
        *slot=Some(store.clone());
        Ok(store)
    }
}
type ImplicitFontBackingStore=Rc<RefCell<ImplicitFontBackingEntries>>;
impl ImplicitFontBacking {
    fn matches(&self,identity:usize,publication:&Rc<RetainedStylesheetRecord>)->bool{
        self.identity==identity
            && Rc::ptr_eq(self.epoch.publication(),publication) && self.epoch.current()
    }
}
impl RustExecutableDocument {
    fn prepare_implicit_font_backing(
        &mut self,realm:RealmId,rule:oryn_layout::cssom::native_import_graph::NativeRule,
        epoch:dynamic_stylesheet_preparation::RetainedFontDemandEpoch,
        binary:&dynamic_stylesheet_preparation::CssBinary,
    )->Result<Arc<ImplicitFontBacking>,HostError>{
        if !epoch.current(){return Err(HostError::CachedAuthorityExpired);}
        let identity=epoch.publication().graph.borrow().connected_font_context(&rule)
            .map_err(HostError::Execution)?.identity();
        let admission=self.agent.host.native_retained_admission_for_realm(realm)?.ok_or(HostError::CachedAuthorityExpired)?;
        let _scope=self.agent.host.retained_allocation_scope(realm)?;
        let known=std::mem::size_of::<ImplicitFontBacking>().checked_add(2*std::mem::size_of::<usize>())
            .and_then(|bytes|bytes.checked_add(binary.url.len()))
            .ok_or_else(||HostError::Execution("implicit font backing metadata overflow".into()))?;
        let metadata=admission.reserve(known).map_err(HostError::Execution)?;
        let payload_admission=admission.reserve(binary.bytes.len()).map_err(HostError::Execution)?;
        let store_metadata=admission.reserve(std::mem::size_of::<Arc<ImplicitFontBacking>>()).map_err(HostError::Execution)?;
        let bytes=ScriptSourceReservation::reserve(&self.dynamic_scripts.source_bytes,binary.bytes.len())
            .ok_or_else(||HostError::LimitExceeded{resource:"CSS binary retained bytes".into()})?;
        if !epoch.current()||!self.agent.host.realm_execution_live(realm){return Err(HostError::CachedAuthorityExpired);}
        let mut payload=Vec::new();
        payload.try_reserve_exact(binary.bytes.len()).map_err(|_|HostError::Execution("implicit font backing bytes allocation".into()))?;
        if payload.capacity()>binary.bytes.len(){return Err(HostError::Execution("implicit font backing bytes exceeded admission".into()));}
        payload.extend_from_slice(&binary.bytes);
        let mut url=String::new();
        url.try_reserve_exact(binary.url.len()).map_err(|_|HostError::Execution("implicit font backing URL allocation".into()))?;
        if url.capacity()>binary.url.len(){return Err(HostError::Execution("implicit font backing URL exceeded admission".into()));}
        url.push_str(&binary.url);
        Ok(Arc::new(ImplicitFontBacking{identity,rule,epoch,binary:dynamic_stylesheet_preparation::CssBinary{url,bytes:payload},_bytes:bytes,_metadata:metadata,_payload_admission:payload_admission,_store_metadata:store_metadata}))
    }
}
fn prepare_implicit_font_store(store:&ImplicitFontBackingStore,admission:&dyn oryn_layout::cssom::native_import_graph::ImportAdmission)->Result<(),HostError>{
    // The metadata reservation is held by the prepared backing before this
    // capacity allocation; no admission callback runs under the store borrow.
    let (len,capacity)={let store=store.try_borrow().map_err(|_|HostError::Execution("implicit backing preparation borrow".into()))?;
        (store.entries.len(),store.entries.capacity())};
    if len<capacity{return Ok(());}
    if len>=NATIVE_LAYOUT_FONT_CAPACITY{return Err(HostError::LimitExceeded{resource:"CSS binary resources".into()});}
    let next=len.checked_add(1).ok_or_else(||HostError::Execution("implicit backing capacity overflow".into()))?;
    let charge=admission.reserve(next.checked_mul(std::mem::size_of::<Arc<ImplicitFontBacking>>()).ok_or_else(||HostError::Execution("implicit backing capacity bytes overflow".into()))?)
        .map_err(HostError::Execution)?;
    let mut replacement=Vec::new();
    replacement.try_reserve_exact(next).map_err(|_|HostError::Execution("implicit backing store allocation".into()))?;
    if replacement.capacity()>next{return Err(HostError::Execution("implicit backing allocator exceeds admitted capacity".into()));}
    let (old,old_charge)={let mut store=store.try_borrow_mut().map_err(|_|HostError::Execution("implicit backing preparation borrow".into()))?;
        if store.entries.len()!=len||store.entries.capacity()!=capacity{return Err(HostError::Execution("implicit backing store changed during admission".into()));}
        replacement.append(&mut store.entries);
        (std::mem::replace(&mut store.entries,replacement),store.capacity_charge.replace(charge))};
    drop(old);drop(old_charge);
    Ok(())
}
fn commit_implicit_font_backing(store:&ImplicitFontBackingStore,backing:Arc<ImplicitFontBacking>)->Result<(),HostError>{
    if !backing.epoch.current(){return Err(HostError::CachedAuthorityExpired);}
    let mut entries=store.try_borrow_mut().map_err(|_|HostError::Execution("implicit backing publication borrow".into()))?;
    let index=entries.entries.iter().position(|old|old.identity==backing.identity&&Rc::ptr_eq(old.epoch.publication(),backing.epoch.publication()));
    if index.is_none()&&entries.entries.len()==entries.entries.capacity(){return Err(HostError::Execution("implicit backing capacity changed before commit".into()));}
    let old=if let Some(index)=index{Some(std::mem::replace(&mut entries.entries[index],backing))}else{entries.entries.push(backing);None};
    drop(entries);
    drop(old);
    Ok(())
}
fn prune_implicit_font_backing(store:&ImplicitFontBackingStore,admission:&dyn oryn_layout::cssom::native_import_graph::ImportAdmission)->Result<(),HostError>{
    if store.try_borrow().map_err(|_|HostError::Execution("implicit backing prune borrow".into()))?
        .entries.iter().all(|entry|entry.epoch.current()){return Ok(());}
    let count=store.try_borrow().map_err(|_|HostError::Execution("implicit backing prune borrow".into()))?.entries.len();
    let charge=admission.reserve(count.checked_mul(std::mem::size_of::<Arc<ImplicitFontBacking>>()).ok_or_else(||HostError::Execution("implicit backing prune overflow".into()))?).map_err(HostError::Execution)?;
    let mut removed=Vec::new();removed.try_reserve_exact(count).map_err(|_|HostError::Execution("implicit backing prune allocation".into()))?;
    if removed.capacity()>count{return Err(HostError::Execution("implicit backing prune allocator capacity".into()));}
    let mut kept=Vec::new();
    let kept_charge=admission.reserve(count.checked_mul(std::mem::size_of::<Arc<ImplicitFontBacking>>()).ok_or_else(||HostError::Execution("implicit backing prune retained overflow".into()))?).map_err(HostError::Execution)?;
    kept.try_reserve_exact(count).map_err(|_|HostError::Execution("implicit backing prune retained allocation".into()))?;
    if kept.capacity()>count{return Err(HostError::Execution("implicit backing prune retained allocator capacity".into()));}
    let (old,old_charge)={let mut store=store.try_borrow_mut().map_err(|_|HostError::Execution("implicit backing prune borrow".into()))?;
        if store.entries.len()!=count{return Err(HostError::Execution("implicit backing prune changed during admission".into()));}
        for entry in store.entries.drain(..){if entry.epoch.current(){kept.push(entry);}else{removed.push(entry);}}
        (std::mem::replace(&mut store.entries,kept),store.capacity_charge.replace(kept_charge))
    };
    drop(old);drop(old_charge);drop(removed);drop(charge);
    Ok(())
}

// Private pre-paid capacity. This never invokes V8/native admission off-thread.
pub(crate) struct InputAllowance {
    capacity: usize,
    used: std::sync::Mutex<usize>,
    cancelled: std::sync::Arc<dyn Fn()->bool+Send+Sync>,
    _lease: std::sync::Arc<dyn Send+Sync>,
}
pub(crate) struct InputPayloadLease {
    owner:std::sync::Arc<InputAllowance>,
    bytes:usize,
}
#[repr(C)]
struct ArcCharge<T>{strong:std::sync::atomic::AtomicUsize,weak:std::sync::atomic::AtomicUsize,payload:T}
impl Drop for InputPayloadLease {
    fn drop(&mut self){
        let mut used=self.owner.used.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let remaining=used.checked_sub(self.bytes);
        debug_assert!(remaining.is_some(),"native input allowance lease accounting underflow");
        if let Some(remaining)=remaining{*used=remaining;}
    }
}
impl InputAllowance {
    pub(crate) fn new(capacity:usize,cancelled:std::sync::Arc<dyn Fn()->bool+Send+Sync>,lease:std::sync::Arc<dyn Send+Sync>)->Self{
        Self{capacity,used:std::sync::Mutex::new(0),cancelled,_lease:lease}
    }
    pub(crate) fn claim(self:&std::sync::Arc<Self>,bytes:usize)->Result<InputPayloadLease,String>{
        if (self.cancelled)(){return Err("native input allowance owner retired".into())}
        let mut used=self.used.lock().map_err(|_|"native input allowance poisoned")?;
        let next=used.checked_add(bytes).ok_or("native input allowance overflow")?;
        if next>self.capacity{return Err("native input allowance capacity exhausted".into())}
        if (self.cancelled)(){return Err("native input allowance owner retired".into())}
        *used=next;drop(used);Ok(InputPayloadLease{owner:self.clone(),bytes})
    }
    pub(crate) fn canonical_capacity()->Option<usize>{
        // Existing native operation bound, not a new quota. Account producer
        // text+queued clone and original/new run overlap conservatively.
        let units=1024usize.checked_mul(1024)?;
        units.checked_mul(std::mem::size_of::<u16>())?.checked_mul(2)?
            .checked_add(units.checked_mul(std::mem::size_of::<oryn_js::composition::NativeTextFormat>())?.checked_mul(2)?)?
            .checked_add(std::mem::size_of::<crate::owned_action_request::OwnedActionRequest>())?
            .checked_add(std::mem::size_of::<ArcCharge<InputAllowance>>())?
    }
    pub(crate) fn payload_capacity(units:usize,formats:bool)->Option<usize>{
        if units>1024*1024{return None}
        units.checked_mul(std::mem::size_of::<u16>())?.checked_mul(2)?
            .checked_add(if formats{units.checked_mul(std::mem::size_of::<oryn_js::composition::NativeTextFormat>())?.checked_mul(2)?}else{0})?
            .checked_add(std::mem::size_of::<crate::owned_action_request::OwnedActionRequest>())?
            .checked_add(std::mem::size_of::<ArcCharge<InputAllowance>>())?
    }
}
#[cfg(test)]
mod allowance_controls {
    use super::*;
    #[test]
    fn admission_capacity_claims_are_bounded_and_drop_releases_without_callbacks(){
        let cancel=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal=cancel.clone();
        let owner=std::sync::Arc::new(InputAllowance::new(8,std::sync::Arc::new(move||signal.load(std::sync::atomic::Ordering::Acquire)),std::sync::Arc::new(())));
        let first=owner.claim(6).unwrap();assert!(owner.claim(3).is_err());
        drop(first);let next=owner.claim(8).unwrap();drop(next);
        cancel.store(true,std::sync::atomic::Ordering::Release);assert!(owner.claim(0).is_err());
    }
    #[test]
    fn shared_claim_custody_does_not_release_owner_until_queue_payload_drops(){
        let lease=std::sync::Arc::new(());let weak=std::sync::Arc::downgrade(&lease);
        let owner=std::sync::Arc::new(InputAllowance::new(16,std::sync::Arc::new(||false),lease));
        let payload=owner.claim(16).unwrap();drop(owner);
        assert!(weak.upgrade().is_some());drop(payload);assert!(weak.upgrade().is_none());
    }
    #[test]
    fn multiple_small_marked_and_commit_payloads_share_one_canonical_allowance(){
        let owner=std::sync::Arc::new(InputAllowance::new(InputAllowance::canonical_capacity().unwrap(),std::sync::Arc::new(||false),std::sync::Arc::new(())));
        let marked=owner.claim(InputAllowance::payload_capacity(2,true).unwrap()).unwrap();
        let commit=owner.claim(InputAllowance::payload_capacity(2,false).unwrap()).unwrap();
        assert!(owner.claim(InputAllowance::canonical_capacity().unwrap()).is_err());
        drop(marked);drop(commit);
        assert!(owner.claim(InputAllowance::canonical_capacity().unwrap()).is_ok());
    }
}

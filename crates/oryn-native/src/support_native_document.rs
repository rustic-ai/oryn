//! Native adapter: exact owning JS Document holds the original response lease.
use oryn_js::v8_bindings::{NativeSupportCustody, V8BindingState};
use std::{rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}};

pub(crate) struct NativeSupportDocument {
    identity: (oryn_dom::NodeId,u64,u64,u64),
    pub(crate) body: super::frame_response_stream::FrameResponseBody,
    terminal: AtomicBool,
    retired: AtomicBool,
    _metadata: crate::executable::ScriptSourceReservation,
}
impl NativeSupportDocument {
    pub(crate) fn install(state:&Rc<V8BindingState>,
        body:super::frame_response_stream::FrameResponseBody,
        budget:&Arc<std::sync::atomic::AtomicUsize>) -> Result<Rc<Self>,crate::host::HostError> {
        let bytes=std::mem::size_of::<Self>()+2*std::mem::size_of::<usize>();
        let metadata=crate::executable::ScriptSourceReservation::reserve(budget,bytes)
            .ok_or_else(||crate::host::HostError::LimitExceeded{resource:"support Document custody".into()})?;
        let owner=Rc::new(Self{identity:(state.document_root(),state.document_generation(),
            state.document_stream_epoch(),state.document_authority_epoch()),body,
            terminal:AtomicBool::new(false),retired:AtomicBool::new(false),_metadata:metadata});
        state.install_native_support_custody(owner.clone()).map_err(|error|
            crate::host::HostError::Execution(error.to_string()))?;
        // Read back the actual committed slot, not an installer-supplied bool.
        let installed=state.native_support_custody().ok_or(crate::host::HostError::CachedAuthorityExpired)?;
        let Some(actual)=installed.as_any().downcast_ref::<Self>() else {
            return Err(crate::host::HostError::CachedAuthorityExpired);
        };
        if !std::ptr::eq(actual,owner.as_ref()){return Err(crate::host::HostError::CachedAuthorityExpired);}
        Ok(owner)
    }
    pub(crate) fn commit_terminal(&self,state:&V8BindingState)->Result<(),crate::host::HostError>{
        if self.retired.load(Ordering::Acquire)||state.execution_retired()
            ||state.execution_suspended()||self.identity!=(state.document_root(),
                state.document_generation(),state.document_stream_epoch(),state.document_authority_epoch()){
            return Err(crate::host::HostError::CachedAuthorityExpired);
        }
        if self.terminal.swap(true,Ordering::AcqRel){return Err(crate::host::HostError::Execution("duplicate support EOF".into()));}
        Ok(())
    }
}
impl NativeSupportCustody for NativeSupportDocument {
    fn identity(&self)->(oryn_dom::NodeId,u64,u64,u64){self.identity}
    fn retire(&self){self.retired.store(true,Ordering::Release);self.body.lease.cancel();}
    fn as_any(&self)->&dyn std::any::Any{self}
}
impl Drop for NativeSupportDocument{fn drop(&mut self){self.retire();}}

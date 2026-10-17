//! Native adapter: exact owning JS Document holds the original response lease.
use oryn_js::v8_bindings::{NativeSupportCustody, V8BindingState};
use std::{rc::Rc, sync::{Arc, atomic::{AtomicBool, Ordering}}};

pub(crate) struct NativeSupportDocument {
    identity: (oryn_dom::NodeId,u64,u64,u64),
    creator: oryn_js::worker_wire::Creator,
    budget: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) body: super::frame_response_stream::FrameResponseBody,
    terminal: AtomicBool,
    retired: AtomicBool,
    _metadata: crate::executable::ScriptSourceReservation,
}
impl NativeSupportDocument {
    pub(crate) fn install(state:&Rc<V8BindingState>,
        body:super::frame_response_stream::FrameResponseBody,
        creator:oryn_js::worker_wire::Creator,
        budget:&Arc<std::sync::atomic::AtomicUsize>) -> Result<Rc<Self>,crate::host::HostError> {
        if !state.native_creator_identity_matches(&creator){return Err(crate::host::HostError::CachedAuthorityExpired);}
        let capacity=match &creator{oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Frame(frame),..}=>frame.capacity(),_=>return Err(crate::host::HostError::CachedAuthorityExpired)};
        let bytes=std::mem::size_of::<Self>().checked_add(2*std::mem::size_of::<usize>()).and_then(|n|n.checked_add(capacity)).ok_or_else(||crate::host::HostError::LimitExceeded{resource:"support Document custody".into()})?;
        let metadata=crate::executable::ScriptSourceReservation::reserve(budget,bytes)
            .ok_or_else(||crate::host::HostError::LimitExceeded{resource:"support Document custody".into()})?;
        let owner=Rc::new(Self{identity:(state.document_root(),state.document_generation(),
            state.document_stream_epoch(),state.document_authority_epoch()),body,creator,budget:budget.clone(),
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
        let oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Frame(frame),document_generation,stream_epoch}=&self.creator else{return Err(crate::host::HostError::CachedAuthorityExpired)};
        let _request_bytes=crate::executable::ScriptSourceReservation::reserve(&self.budget,frame.len()).ok_or_else(||crate::host::HostError::LimitExceeded{resource:"support decoder request".into()})?;
        let mut copy=String::new();copy.try_reserve_exact(frame.len()).map_err(|_|crate::host::HostError::LimitExceeded{resource:"support decoder request".into()})?;
        let _capacity_extra=crate::executable::ScriptSourceReservation::reserve(&self.budget,copy.capacity()-frame.len()).ok_or_else(||crate::host::HostError::LimitExceeded{resource:"support decoder request".into()})?;
        copy.push_str(frame);
        let creator=oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Frame(copy),document_generation:*document_generation,stream_epoch:*stream_epoch};
        let response=self.body.client.request(crate::context_service::ContextServiceRequest::CommitSupportFrameDecoded{
            page_id:self.body.page_id,stream_id:self.body.id,creator,
        }).map_err(|error|crate::host::HostError::Execution(error.to_string()))?;
        if !matches!(response,crate::context_service::ContextServiceResponse::SupportFrameResponseSelected{selected:true}){
            return Err(crate::host::HostError::CachedAuthorityExpired);
        }
        Ok(())
    }
}
impl NativeSupportCustody for NativeSupportDocument {
    fn identity(&self)->(oryn_dom::NodeId,u64,u64,u64){self.identity}
    fn retire(&self){self.retired.store(true,Ordering::Release);self.body.lease.cancel();}
    fn as_any(&self)->&dyn std::any::Any{self}
}
impl Drop for NativeSupportDocument{fn drop(&mut self){self.retire();}}

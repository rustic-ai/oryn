// Included in frame_response_stream.rs: keep exact Document custody across EOF.
struct SupportDecoderTransfer {
    owner: std::rc::Weak<oryn_js::v8_bindings::V8BindingState>,
    custody: std::rc::Rc<super::support_native_document::NativeSupportDocument>,
    committed: bool,
}
impl SupportDecoderTransfer {
    fn new(owner:&std::rc::Rc<oryn_js::v8_bindings::V8BindingState>,
        custody:std::rc::Rc<super::support_native_document::NativeSupportDocument>)->Self{
        Self{owner:std::rc::Rc::downgrade(owner),custody,committed:false}
    }
    fn eof(&mut self)->Result<(),HostError>{
        let owner=self.owner.upgrade().ok_or(HostError::CachedAuthorityExpired)?;
        let installed=owner.native_support_custody().ok_or(HostError::CachedAuthorityExpired)?;
        let actual=installed.as_any().downcast_ref::<super::support_native_document::NativeSupportDocument>()
            .ok_or(HostError::CachedAuthorityExpired)?;
        if !std::ptr::eq(actual,self.custody.as_ref()){return Err(HostError::CachedAuthorityExpired);}
        // Slot/identity validation precedes the irreversible terminal marker.
        self.custody.commit_terminal(&owner)?;
        // Preserve cleanup capacity/request. Only network activity ends here.
        self.custody.body.lease.finish_support_network_activity();
        self.committed=true;
        Ok(())
    }
}
impl Drop for SupportDecoderTransfer{
    fn drop(&mut self){if !self.committed{self.custody.body.lease.cancel();}}
}

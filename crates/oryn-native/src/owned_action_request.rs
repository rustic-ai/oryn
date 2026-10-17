// Private queue custody: wire/public ActionRequest fields are unchanged.
#[doc(hidden)]
pub struct OwnedActionRequest {
    pub(crate) request:crate::accessibility_platform::ActionRequest,
    pub(crate) payload:Option<crate::native_input_allowance::InputPayloadLease>,
}
impl OwnedActionRequest {
    #[doc(hidden)]
    pub fn request_action(&self)->&crate::accessibility_platform::Action{&self.request.action}
    pub(crate) fn plain(request:crate::accessibility_platform::ActionRequest)->Self{Self{request,payload:None}}
}
pub(crate) enum NativeActionSender {
    Plain(std::sync::mpsc::SyncSender<crate::accessibility_platform::ActionRequest>),
    Owned(std::sync::mpsc::SyncSender<OwnedActionRequest>),
}
impl Clone for NativeActionSender {
    fn clone(&self)->Self{match self{Self::Plain(sender)=>Self::Plain(sender.clone()),Self::Owned(sender)=>Self::Owned(sender.clone())}}
}
impl NativeActionSender {
    pub(crate) fn send(&self,request:crate::accessibility_platform::ActionRequest,payload:Option<crate::native_input_allowance::InputPayloadLease>)->Result<(),String>{
        match self {
            Self::Plain(sender)=>{
                if payload.is_some(){return Err("native input owned queue unavailable".into())}
                sender.try_send(request).map_err(|_|"native action queue unavailable".into())
            }
            Self::Owned(sender)=>sender.try_send(OwnedActionRequest{request,payload}).map_err(|_|"native action queue unavailable".into()),
        }
    }
}

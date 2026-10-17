// Internal wire custody: serializes shared immutable receipts without cloning
// their UTF16 payload into a temporary public WorkerResponse.
pub(crate) struct CustodiedWorkerResponse {
    pub(crate) response: WorkerResponse,
    pub(crate) receipts: Option<Vec<crate::webdriver_actions::OwnedOsKeyReceipt>>,
}
impl From<WorkerResponse> for CustodiedWorkerResponse {
    fn from(response: WorkerResponse) -> Self { Self { response, receipts: None } }
}
impl CustodiedWorkerResponse {
    pub(crate) fn actions(
        result: Result<(), crate::host::webdriver_script::ScriptError>,
        feedback: oryn_js::drag_image::FeedbackBatch,
        receipts: Vec<crate::webdriver_actions::OwnedOsKeyReceipt>,
    ) -> Self {
        Self { response: WorkerResponse::WebdriverActions {
            result, feedback, receipts: Vec::new(),
        }, receipts: Some(receipts) }
    }
}
impl serde::Serialize for CustodiedWorkerResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok,S::Error> {
        use serde::ser::SerializeStruct;
        match (&self.response,&self.receipts) {
            (WorkerResponse::WebdriverActions { result, feedback, .. },Some(receipts)) => {
                let mut state=serializer.serialize_struct("WorkerResponse",4)?;
                state.serialize_field("status","webdriver_actions")?;
                state.serialize_field("result",result)?;
                state.serialize_field("feedback",feedback)?;
                state.serialize_field("receipts",receipts)?;
                state.end()
            }
            (_,None)=>self.response.serialize(serializer),
            (_,Some(_))=>Err(serde::ser::Error::custom("receipt custody requires actions response")),
        }
    }
}
pub(crate) struct CustodiedWorkerEnvelope {
    pub(crate) envelope: WorkerEnvelope,
    pub(crate) receipts: Option<Vec<crate::webdriver_actions::OwnedOsKeyReceipt>>,
}
impl serde::Serialize for CustodiedWorkerEnvelope {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error> {
        use serde::ser::SerializeStruct;
        struct Response<'a>(&'a WorkerResponse,&'a Option<Vec<crate::webdriver_actions::OwnedOsKeyReceipt>>);
        impl serde::Serialize for Response<'_> {
            fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error>{
                match (self.0,self.1) {
                    (WorkerResponse::WebdriverActions{result,feedback,..},Some(receipts))=>{
                        let mut state=serializer.serialize_struct("WorkerResponse",4)?;
                        state.serialize_field("status","webdriver_actions")?;
                        state.serialize_field("result",result)?;
                        state.serialize_field("feedback",feedback)?;
                        state.serialize_field("receipts",receipts)?;
                        state.end()
                    }
                    (_,None)=>self.0.serialize(serializer),
                    (_,Some(_))=>Err(serde::ser::Error::custom("receipt custody requires actions response")),
                }
            }
        }
        let mut state=serializer.serialize_struct("WorkerEnvelope",6)?;
        state.serialize_field("protocol_version",&self.envelope.protocol_version)?;
        state.serialize_field("route",&self.envelope.route)?;
        state.serialize_field("session_id",&self.envelope.session_id)?;
        state.serialize_field("message_id",&self.envelope.message_id)?;
        state.serialize_field("correlation_id",&self.envelope.correlation_id)?;
        state.serialize_field("response",&Response(&self.envelope.response,&self.receipts))?;
        state.end()
    }
}
impl Drop for CustodiedWorkerResponse {
    fn drop(&mut self){
        let response=std::mem::replace(&mut self.response,WorkerResponse::Exiting);
        dispose_worker_response(response);
    }
}
impl Drop for CustodiedWorkerEnvelope {
    fn drop(&mut self){
        let response=std::mem::replace(&mut self.envelope.response,WorkerResponse::Exiting);
        dispose_worker_response(response);
    }
}

#[cfg(test)]
mod parent_custody_wire_controls {
    use super::*;
    #[test]
    fn owned_wire_preserves_completion_overlay_and_drop_custody(){
        let node=oryn_dom::NodeId{slot:1,generation:1};
        let receipt=crate::webdriver_actions::OsKeyReceipt{
            continuation_done:false,source:"keyboard".into(),state:Some(oryn_js::composition::StateSnapshot{
                mutation_revision:7,aborted:true,protected:false,text:vec![0xd800,0x61,0xd83d,0xde00],
                selection:oryn_js::composition::TextRange{start:1,length:2},marked:Some(oryn_js::composition::TextRange{start:0,length:1}),
            }),
            target:Some(node),token:1,accepted:true,interpret:true,
            context:crate::host::webdriver_script::AutomationRealmSnapshot{
                realm:oryn_js::realm::RealmId(0),parent_realm:None,frame:None,frame_generation:None,
                frame_token:None,document_root:node,document_generation:1,stream_epoch:1,
                document_authority_epoch:0,url:"https://example.test/".into(),live:true},
        };
        let lease=std::sync::Arc::new(());let weak=std::sync::Arc::downgrade(&lease);
        let owned=crate::webdriver_actions::OwnedOsKeyReceipt::new(receipt,Some(lease)).completed(true);
        let plain=owned.to_plain();
        assert_eq!(serde_json::to_value(&owned).unwrap(),serde_json::to_value(&plain).unwrap());
        let clone=owned.clone();assert!(std::sync::Arc::ptr_eq(&owned.receipt,&clone.receipt));
        drop(owned);assert!(weak.upgrade().is_some());
        let feedback=oryn_js::drag_image::FeedbackBatch::default();
        let expected=WorkerResponse::WebdriverActions{result:Ok(()),feedback:feedback.clone(),receipts:vec![plain]};
        let response=CustodiedWorkerResponse::actions(Ok(()),feedback,vec![clone]);
        assert_eq!(serde_json::to_value(&response).unwrap(),serde_json::to_value(&expected).unwrap());
        let (sender,receiver)=tokio::sync::oneshot::channel();
        sender.send(response).ok().unwrap();assert!(weak.upgrade().is_some());
        let mut response=receiver.blocking_recv().unwrap();
        let envelope=CustodiedWorkerEnvelope{envelope:WorkerEnvelope{
            protocol_version:WORKER_PROTOCOL_VERSION,route:None,session_id:1,message_id:2,correlation_id:2,
            response:std::mem::replace(&mut response.response,WorkerResponse::Exiting),
        },receipts:response.receipts.take()};
        struct Denied;
        impl std::io::Write for Denied{
            fn write(&mut self,_:&[u8])->std::io::Result<usize>{Err(std::io::Error::other("denied"))}
            fn flush(&mut self)->std::io::Result<()>{Err(std::io::Error::other("denied"))}
        }
        assert!(serde_json::to_writer(Denied,&envelope).is_err());
        assert!(weak.upgrade().is_some());drop(envelope);assert!(weak.upgrade().is_none());
        let lease=std::sync::Arc::new(());let weak=std::sync::Arc::downgrade(&lease);
        let receipt=crate::webdriver_actions::OwnedOsKeyReceipt::new(expected_receipt(),Some(lease));
        let response=CustodiedWorkerResponse::actions(Ok(()),oryn_js::drag_image::FeedbackBatch::default(),vec![receipt]);
        struct FlushDenied(Vec<u8>);
        impl std::io::Write for FlushDenied{
            fn write(&mut self,data:&[u8])->std::io::Result<usize>{self.0.extend_from_slice(data);Ok(data.len())}
            fn flush(&mut self)->std::io::Result<()>{Err(std::io::Error::other("flush denied"))}
        }
        let mut writer=FlushDenied(Vec::new());
        serde_json::to_writer(&mut writer,&response).unwrap();
        assert!(!writer.0.is_empty());assert!(std::io::Write::flush(&mut writer).is_err());
        assert!(weak.upgrade().is_some());drop(response);assert!(weak.upgrade().is_none());
    }
    fn expected_receipt()->crate::webdriver_actions::OsKeyReceipt{
        let node=oryn_dom::NodeId{slot:1,generation:1};
        crate::webdriver_actions::OsKeyReceipt{continuation_done:true,source:"os".into(),state:None,target:Some(node),token:1,accepted:true,interpret:false,
            context:crate::host::webdriver_script::AutomationRealmSnapshot{realm:oryn_js::realm::RealmId(0),parent_realm:None,frame:None,frame_generation:None,frame_token:None,document_root:node,document_generation:1,stream_epoch:1,document_authority_epoch:0,url:"https://example.test/".into(),live:true}}
    }
    #[test]
    fn queue_refusal_keeps_producer_payload_for_retry(){
        let mut input=crate::webdriver_actions::InputState::default();
        for _ in 0..128{input.push_os_receipt(expected_receipt()).unwrap();}
        let lease=std::sync::Arc::new(());let weak=std::sync::Arc::downgrade(&lease);
        let completed=crate::webdriver_actions::OwnedOsKeyReceipt::new(expected_receipt(),Some(lease));
        assert!(input.push_owned_os_receipt(completed.clone()).is_err());
        assert!(weak.upgrade().is_some());
        drop(input.take_owned_os_receipts());
        input.push_owned_os_receipt(completed.clone()).unwrap();
        drop(completed);assert!(weak.upgrade().is_some());
        drop(input.take_owned_os_receipts());assert!(weak.upgrade().is_none());
    }
}

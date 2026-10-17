// Private custody wrapper. Public OsKeyReceipt wire fields remain unchanged.
#[derive(Clone)]
#[doc(hidden)]
pub struct OwnedOsKeyReceipt {
    pub(crate) receipt: std::sync::Arc<OsKeyReceipt>,
    pub(crate) allocation: Option<std::sync::Arc<dyn Send + Sync>>,
    pub(crate) completion: Option<(bool,bool)>,
    pub(crate) input_allowance: Option<std::sync::Arc<crate::native_input_allowance::InputAllowance>>,
}
impl std::fmt::Debug for OwnedOsKeyReceipt {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        f.debug_struct("OwnedOsKeyReceipt").field("receipt",&self.receipt)
            .field("allocation_retained",&self.allocation.is_some()).finish()
    }
}
impl std::ops::Deref for OwnedOsKeyReceipt {
    type Target=OsKeyReceipt;
    fn deref(&self)->&OsKeyReceipt{&self.receipt}
}
impl From<OsKeyReceipt> for OwnedOsKeyReceipt {
    fn from(receipt:OsKeyReceipt)->Self{Self{receipt:std::sync::Arc::new(receipt),allocation:None,completion:None,input_allowance:None}}
}
impl OwnedOsKeyReceipt {
    pub(crate) fn metadata_bytes(source:&str,context:&crate::host::webdriver_script::AutomationRealmSnapshot)->Result<usize,crate::host::webdriver_script::ScriptError>{
        std::mem::size_of::<OsKeyReceipt>().checked_add(8*std::mem::size_of::<usize>())
            .and_then(|n|n.checked_add(source.len())).and_then(|n|n.checked_add(context.url.len()))
            .and_then(|n|n.checked_add(context.frame_token.as_ref().map_or(0,|token|token.len())))
            .ok_or_else(||crate::host::webdriver_script::ScriptError::invalid("OS receipt metadata overflow"))
    }
    pub(crate) fn new(receipt:OsKeyReceipt,allocation:Option<std::sync::Arc<dyn Send+Sync>>)->Self{Self{receipt:std::sync::Arc::new(receipt),allocation,completion:None,input_allowance:None}}
    pub(crate) fn with_input_allowance(mut self,allowance:std::sync::Arc<crate::native_input_allowance::InputAllowance>)->Self{self.input_allowance=Some(allowance);self}
    pub(crate) fn completed(mut self,done:bool)->Self{self.completion=Some((done,false));self}
    pub(crate) fn should_interpret(&self)->bool{self.completion.map(|(_,interpret)|interpret).unwrap_or(self.receipt.interpret)}
    pub(crate) fn to_plain(&self)->OsKeyReceipt{
        let mut receipt=self.receipt.as_ref().clone();
        if let Some((done,interpret))=self.completion{receipt.continuation_done=done;receipt.interpret=interpret;}
        receipt
    }
    /// Replacement preserves source/token identity, transfers exactly one
    /// current snapshot lease and releases the previous state's ownership.
    // State replacement must create a newly admitted receipt; Arc clones share
    // the original immutable payload and never duplicate its UTF16 Vec.
}
impl serde::Serialize for OwnedOsKeyReceipt {
    fn serialize<S:serde::Serializer>(&self,serializer:S)->Result<S::Ok,S::Error>{
        use serde::ser::SerializeStruct;
        let (done,interpret)=self.completion.unwrap_or((self.receipt.continuation_done,self.receipt.interpret));
        let mut state=serializer.serialize_struct("OsKeyReceipt",8)?;
        state.serialize_field("continuation_done",&done)?;
        state.serialize_field("source",&self.receipt.source)?;
        state.serialize_field("state",&self.receipt.state)?;
        state.serialize_field("target",&self.receipt.target)?;
        state.serialize_field("token",&self.receipt.token)?;
        state.serialize_field("accepted",&self.receipt.accepted)?;
        state.serialize_field("interpret",&interpret)?;
        state.serialize_field("context",&self.receipt.context)?;
        state.end()
    }
}

#[cfg(test)]
mod owned_os_receipt_controls {
    use super::*;
    #[test]
    fn private_lease_follows_clones_and_serialization_without_wire_fields(){
        let node=oryn_dom::NodeId{slot:1,generation:1};
        let receipt=OsKeyReceipt{continuation_done:true,source:"keyboard".into(),state:None,
            target:Some(node),token:1,accepted:true,interpret:false,
            context:crate::host::webdriver_script::AutomationRealmSnapshot{
                realm:oryn_js::realm::RealmId(0),parent_realm:None,frame:None,frame_generation:None,
                frame_token:None,document_root:node,document_generation:1,stream_epoch:1,
                document_authority_epoch:0,url:"https://example.test/".into(),live:true}};
        let lease=std::sync::Arc::new(());let weak=std::sync::Arc::downgrade(&lease);
        let owned=OwnedOsKeyReceipt::new(receipt.clone(),Some(lease));
        let clone=owned.clone();drop(owned);
        assert!(weak.upgrade().is_some());
        assert_eq!(serde_json::to_value(&clone).unwrap(),serde_json::to_value(receipt).unwrap());
        drop(clone);assert!(weak.upgrade().is_none());
    }
}

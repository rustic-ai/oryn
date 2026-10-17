//! Fixed metadata stored inline in the original admitted HTTP stream entry.
//! Child Creator custody remains in HttpStreamEntry.creator; no dynamic clone.
use super::{support_descriptor::PinnedSupportDescriptor,
    support_response_state::{SupportOwner, SupportReceipt, SupportResponseState}};
use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};

pub(crate) struct BrokerOccurrence {
    pub(crate) parent: (u64, u64),
    request: u64,
    stream: u64,
    state: SupportResponseState,
    imports: u8,
    descriptor: PinnedSupportDescriptor,
    bound: bool,
    decoded: bool,
}
fn owner(page:u64, occurrence:u64, creator:&Creator, authority_epoch:u64)->Option<SupportOwner> {
    let Creator::Window{document_generation,stream_epoch,..}=creator else{return None;};
    Some(SupportOwner{page,occurrence,document_generation:*document_generation,
        stream_epoch:*stream_epoch,authority_epoch})
}
impl BrokerOccurrence {
    pub(crate) fn start(descriptor:PinnedSupportDescriptor,page:u64,request:u64,
        stream:u64,parent:&Creator,parent_epoch:u64,source_url:&str,final_url:&str)->Option<Self> {
        let Creator::Window{target:WindowTarget::Root,document_generation,stream_epoch}=parent else{return None;};
        let (descriptor_hash,candidate,body)=descriptor.response(source_url,final_url)?;
        Some(Self{parent:(*document_generation,*stream_epoch),request,stream,
            state:SupportResponseState::new(descriptor_hash,candidate,
                owner(page,request,parent,parent_epoch)?,final_url,body),
            imports:0,descriptor,bound:false,decoded:false})
    }
    pub(crate) fn root(&self)->Creator {
        Creator::Window{target:WindowTarget::Root,document_generation:self.parent.0,stream_epoch:self.parent.1}
    }
    pub(crate) fn bind(&mut self,page:u64,parent:&Creator,parent_epoch:u64,
        child:&Creator,child_epoch:u64)->bool {
        if &self.root()!=parent || self.bound || !matches!(child,Creator::Window{target:WindowTarget::Frame(_),..}){return false;}
        let (Some(p),Some(c))=(owner(page,self.request,parent,parent_epoch),owner(page,self.stream,child,child_epoch)) else{return false;};
        if !self.state.bind(p,c){return false;}
        self.bound=true; true
    }
    pub(crate) fn body(&mut self,bytes:&[u8],limit:usize)->bool{self.bound&&self.state.chunk(bytes,limit)}
    pub(crate) fn eof(&mut self,page:u64,parent:&Creator,pe:u64,child:&Creator,ce:u64)->bool {
        if &self.root()!=parent || !self.bound{return false;}
        let (Some(p),Some(c))=(owner(page,self.request,parent,pe),owner(page,self.stream,child,ce)) else{return false;};
        self.state.finish(p,c)
    }
    pub(crate) fn imported(&mut self,resource:&url::Url,source:&[u8])->bool {
        if !self.bound{return false;}
        for index in 0..6 {if self.descriptor.import(index,resource,source){self.imports|=1<<index;return true;}}
        false
    }
    pub(crate) fn export(&self,page:u64,parent:&Creator,pe:u64,child:&Creator,ce:u64)->Option<SupportReceipt>{
        if &self.root()!=parent || !self.bound || !self.decoded || self.imports!=63{return None;}
        self.state.read(owner(page,self.request,parent,pe)?,owner(page,self.stream,child,ce)?)
    }
    pub(crate) fn decoded(&mut self,page:u64,parent:&Creator,pe:u64,child:&Creator,ce:u64)->bool{
        if self.decoded||&self.root()!=parent||!self.bound{return false;}
        let(Some(p),Some(c))=(owner(page,self.request,parent,pe),owner(page,self.stream,child,ce))else{return false;};
        if self.state.read(p,c).is_none(){return false;}
        self.decoded=true;true
    }
    pub(crate) fn retire(&mut self){self.state.cancel();}
}
impl Drop for BrokerOccurrence{fn drop(&mut self){self.retire();}}

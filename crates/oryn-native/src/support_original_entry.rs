// Borrow-safe access to fixed proof and original admitted child custody.
impl ParentContextService {
    fn with_support_entry<T>(&self,page:PageId,entry:&Arc<HttpStreamEntry>,
        consume:impl FnOnce(&mut crate::support_broker_occurrence::BrokerOccurrence,
            &oryn_js::worker_wire::Creator,&oryn_js::worker_wire::Creator,u64,u64)->Result<T,RuntimeError>)
        ->Result<Option<T>,RuntimeError>{
        if entry.page_id!=page{return Err(RuntimeError::NavigationCancelled);}
        let parent={entry.support.lock().map_err(|_|RuntimeError::NavigationCancelled)?
            .as_ref().map(|support|support.root())};
        let Some(parent)=parent else{return Ok(None);};
        self.context_owner()?.with_support_original_child(page,&parent,&entry.creator,|child,pe,ce|{
            if entry.cleanup.as_ref().is_some_and(|ticket|ticket.cancelled()){return Err(RuntimeError::NavigationCancelled);}
            let mut support=entry.support.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
            let support=support.as_mut().ok_or(RuntimeError::NavigationCancelled)?;
            if support.root()!=parent{return Err(RuntimeError::NavigationCancelled);}
            consume(support,&parent,child,pe,ce).map(Some)
        })
    }
    fn publish_support_body(&self,page:PageId,entry:&Arc<HttpStreamEntry>,chunk:&[u8],done:bool)->Result<bool,RuntimeError>{
        Ok(self.with_support_entry(page,entry,|support,parent,child,pe,ce|{
            if !support.body(chunk,16*1024*1024)||(done&&!support.eof(page.0,parent,pe,child,ce)){
                support.retire();return Err(RuntimeError::ProtocolViolation("support response closure mismatch".into()));}
            Ok(true)
        })?.unwrap_or(false))
    }
    fn support_response_snapshot(&self,page:PageId,stream:u64)->Result<Option<crate::support_response_state::SupportReceipt>,RuntimeError>{
        self.validate_socket_owner(page)?;
        let entry=self.http_streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?.get(&stream).cloned();
        let Some(entry)=entry else{return Ok(None);};
        Ok(self.with_support_entry(page,&entry,|support,parent,child,pe,ce|Ok(support.export(page.0,parent,pe,child,ce)))?.flatten())
    }
    fn select_support_frame_response(&self,page:PageId,stream:u64,creator:&oryn_js::worker_wire::Creator)->Result<ContextServiceResponse,RuntimeError>{
        self.validate_socket_owner(page)?;
        let entry=self.http_streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?.get(&stream).cloned().ok_or(RuntimeError::NavigationCancelled)?;
        let selected=self.with_support_entry(page,&entry,|_,_,child,_,_|{
            if child!=creator{return Err(RuntimeError::NavigationCancelled);} Ok(true)
        })?.unwrap_or(false);
        Ok(ContextServiceResponse::SupportFrameResponseSelected{selected})
    }
    fn commit_support_frame_decoded(&self,page:PageId,stream:u64,creator:&oryn_js::worker_wire::Creator)->Result<ContextServiceResponse,RuntimeError>{
        self.validate_socket_owner(page)?;
        let entry=self.http_streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?.get(&stream).cloned().ok_or(RuntimeError::NavigationCancelled)?;
        let committed=self.with_support_entry(page,&entry,|support,parent,child,pe,ce|{
            if child!=creator||!support.decoded(page.0,parent,pe,child,ce){return Err(RuntimeError::NavigationCancelled);}
            Ok(true)
        })?.unwrap_or(false);
        if !committed{return Err(RuntimeError::NavigationCancelled);}
        Ok(ContextServiceResponse::SupportFrameResponseSelected{selected:true})
    }
}

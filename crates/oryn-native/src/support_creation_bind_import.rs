impl ParentContextService {
    fn prepare_support_occurrence(&self,page:PageId,request:u64,stream:u64,creator:&oryn_js::worker_wire::Creator,
        source:&str,final_url:&str)->Result<Option<crate::support_broker_occurrence::BrokerOccurrence>,RuntimeError>{
        let context=self.context_owner()?;
        let Some(descriptor)=context.pinned_support_descriptor() else{return Ok(None);};
        if descriptor.response(source,final_url).is_none(){return Ok(None);}
        let oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Root,document_generation,stream_epoch}=creator
            else{return Err(RuntimeError::NavigationCancelled);};
        context.window_worker_creator(page,&oryn_js::message_wire::WindowTarget::Root,*document_generation,*stream_epoch)?;
        let epoch=context.support_root_epoch(page,*document_generation,*stream_epoch)?;
        crate::support_broker_occurrence::BrokerOccurrence::start(descriptor,page.0,request,stream,creator,epoch,source,final_url)
            .map(Some).ok_or(RuntimeError::NavigationCancelled)
    }
    fn bind_support_occurrence(&self,page:PageId,policy:&str,child:&oryn_js::worker_wire::Creator)->Result<(),RuntimeError>{
        let entry={
            let streams=self.http_streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
            let mut found=streams.values().filter(|entry|entry.page_id==page&&entry.prepared_frame_policy.as_deref()==Some(policy));
            let entry=found.next().cloned();
            if found.next().is_some(){return Err(RuntimeError::ProtocolViolation("duplicate frame response policy".into()));}
            entry
        };
        let Some(entry)=entry else{return Ok(());};
        // The existing bind_frame_stream stores the authenticated child in
        // entry.creator before this call. Borrow it under canonical guards;
        // never allocate a second retained Creator or overwrite it here.
        self.with_support_entry(page,&entry,|support,parent,original,pe,ce|{
            if original!=child||!support.bind(page.0,parent,pe,original,ce){return Err(RuntimeError::NavigationCancelled);}
            Ok(())
        })?;
        Ok(())
    }
    fn record_support_import(&self,page:PageId,creator:&oryn_js::worker_wire::Creator,final_url:&url::Url,bytes:&[u8])->Result<(),RuntimeError>{
        if !matches!(creator,oryn_js::worker_wire::Creator::Window{target:oryn_js::message_wire::WindowTarget::Frame(_),..}){return Ok(());}
        // Fixed stack snapshot, exactly the existing admitted HTTP capacity.
        let mut entries:[Option<Arc<HttpStreamEntry>>;32]=std::array::from_fn(|_|None);
        {
            let registry=self.http_streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
            if registry.len()>entries.len(){return Err(RuntimeError::ProtocolViolation("HTTP occurrence capacity exceeded".into()));}
            for (slot,entry) in entries.iter_mut().zip(registry.values()){if entry.page_id==page{*slot=Some(entry.clone());}}
        }
        for entry in entries.into_iter().flatten(){
            // A nonmatching ordinary stream does not need child validation.
            let matches=entry.creator.lock().map_err(|_|RuntimeError::NavigationCancelled)?.as_ref()==Some(creator);
            if !matches{continue;}
            self.with_support_entry(page,&entry,|support,_,original,_,_|{
                if original!=creator{return Err(RuntimeError::NavigationCancelled);}
                support.imported(final_url,bytes);Ok(())
            })?;
        }
        Ok(())
    }
}

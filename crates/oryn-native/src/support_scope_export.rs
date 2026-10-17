impl SocketLifecycle {
    pub(crate) fn support_current(&self,context:&crate::runtime::BrowserContext,page:PageId)
        ->Result<Option<crate::support_response_state::SupportReceipt>,RuntimeError>{
        if self.page!=page||self.sealed.load(Ordering::Acquire){return Ok(None);}
        let mut entries:[Option<Arc<HttpStreamEntry>>;32]=std::array::from_fn(|_|None);
        {
            let streams=self.streams.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
            if streams.len()>entries.len(){return Err(RuntimeError::NavigationCancelled);}
            for (slot,entry) in entries.iter_mut().zip(streams.values()){*slot=Some(entry.clone());}
        }
        let mut receipt=None;
        for entry in entries.into_iter().flatten(){
            if entry.page_id!=page{continue;}
            let parent=entry.support.lock().map_err(|_|RuntimeError::NavigationCancelled)?.as_ref().map(|support|support.root());
            let Some(parent)=parent else{continue;};
            let current=context.with_support_original_child(page,&parent,&entry.creator,|child,pe,ce|{
                if self.sealed.load(Ordering::Acquire)||entry.cleanup.as_ref().is_some_and(|ticket|ticket.cancelled()){return Ok(None);}
                let support=entry.support.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
                Ok(support.as_ref().filter(|support|support.root()==parent)
                    .and_then(|support|support.export(page.0,&parent,pe,child,ce)))
            })?;
            if let Some(current)=current{
                if receipt.replace(current).is_some(){return Err(RuntimeError::ProtocolViolation("duplicate current support occurrence".into()));}
            }
        }
        Ok(receipt)
    }
}

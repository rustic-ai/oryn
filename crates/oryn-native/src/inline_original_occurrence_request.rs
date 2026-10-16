impl RetainedStylesheetRecord {
    fn request_for_original_occurrence(
        self: &Rc<Self>,
        imported: oryn_layout::cssom::native_import_graph::NativeImport,
    ) -> Result<NativePendingImportRequest, HostError> {
        let url = imported.href().ok_or_else(||HostError::Execution("inline occurrence URL absent".into()))?;
        let bytes = {
            let store = self.responses.try_borrow().map_err(|_|HostError::Execution("inline parent response conflict".into()))?;
            let response = store.entries.iter().find(|response|response.sheet.as_ref().is_some_and(|sheet|sheet.same_sheet(&imported.parent)))
                .ok_or_else(||HostError::Execution("inline parent proof absent".into()))?;
            std::mem::size_of::<NativePendingImportRequest>().checked_add(url.capacity())
                .and_then(|n|n.checked_add(response.final_url.len()))
                .and_then(|n|n.checked_add(response.referrer_policy.len()))
                .and_then(|n|n.checked_add(response.encoding.len()))
                .ok_or_else(||HostError::Execution("inline occurrence request overflow".into()))?
        };
        let charge = self.admission.reserve(bytes).map_err(HostError::Execution)?;
        fn copy(text:&str)->Result<String,HostError> {
            let mut value=String::new(); value.try_reserve_exact(text.len()).map_err(|e|HostError::Execution(e.to_string()))?;
            value.push_str(text); Ok(value)
        }
        let (parent_source,parent_url,referrer_policy,encoding) = {
            let store=self.responses.try_borrow().map_err(|_|HostError::Execution("inline parent recapture conflict".into()))?;
            let response=store.entries.iter().find(|response|response.sheet.as_ref().is_some_and(|sheet|sheet.same_sheet(&imported.parent)))
                .ok_or_else(||HostError::Execution("inline parent proof changed".into()))?;
            (response.source.clone(),copy(&response.final_url)?,copy(&response.referrer_policy)?,copy(&response.encoding)?)
        };
        self.admission.checkpoint().map_err(HostError::Execution)?;
        if self.owner_publication_retired()? {return Err(HostError::CachedAuthorityExpired);}
        let graph=self.graph.try_borrow().map_err(|_|HostError::Execution("inline token capture conflict".into()))?;
        let current=graph.occurrence(imported.id).ok_or(HostError::CachedAuthorityExpired)?;
        if !current.same_rule(&imported) || !current.parent.same_sheet(&imported.parent)
            || current.index!=imported.index || current.href().as_deref()!=Some(url.as_str()) {
            return Err(HostError::CachedAuthorityExpired);
        }
        let token=graph.begin_import(imported.id).ok_or(HostError::CachedAuthorityExpired)?;
        drop(graph);
        Ok(NativePendingImportRequest {
            publication:self.clone(),token,occurrence:imported.id,parent:imported.parent,url,parent_url,
            _parent_source:parent_source,referrer_policy,encoding,depth:imported.depth,_charge:charge,
        })
    }
}

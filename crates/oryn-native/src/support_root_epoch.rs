impl BrowserContext {
    pub(crate) fn support_root_epoch(&self,page:PageId,generation:u64,epoch:u64)
        ->Result<u64,RuntimeError>{
        let pages=self.inner.group.pages.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        if !pages.get(&page).is_some_and(PageRecord::is_live){return Err(RuntimeError::NavigationCancelled);}
        let policies=self.inner.worker_policies.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        let hub=self.inner.group.messages.lock().map_err(|_|RuntimeError::NavigationCancelled)?;
        hub.root_document_authority(page.0,generation,epoch).map_err(RuntimeError::ProtocolViolation)?;
        let root=policies.root_lifetimes.get(&page).filter(|root|root.current(generation))
            .ok_or(RuntimeError::NavigationCancelled)?;
        Ok(root.epoch)
    }
}

// Included in rust_executable; transport consumes only this exact child job.
struct NativePendingImportRequest {
    publication: Rc<RetainedStylesheetRecord>,
    token: oryn_layout::cssom::native_import_graph::CompletionToken,
    occurrence: oryn_layout::cssom::native_import_graph::OccurrenceId,
    parent: oryn_layout::cssom::native_import_graph::NativeSheet,
    url: String,
    parent_url: String,
    _parent_source: std::sync::Arc<str>,
    referrer_policy: String,
    encoding: String,
    depth: usize,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl RetainedStylesheetRecord {
    /// None is an empty queue or canceled occurrence. Retry on operational Err.
    /// Caller owns self outside registry/DOM borrows and retains returned job
    /// across transport admission failure; no refetch of the root is requested.
    fn take_pending_import(
        self: &Rc<Self>,
    ) -> Result<Option<NativePendingImportRequest>, HostError> {
        use oryn_layout::cssom::native_import_graph::ImportState;
        let id = self
            .pending_imports
            .try_borrow()
            .map_err(|_| HostError::Execution("pending import queue borrow conflict".into()))?
            .first()
            .copied();
        let Some(id) = id else {
            return Ok(None);
        };
        let imported = {
            let graph = self
                .graph
                .try_borrow()
                .map_err(|_| HostError::Execution("pending import graph borrow conflict".into()))?;
            graph.occurrence(id)
        };
        let Some(imported) = imported else {
            return Err(HostError::Execution("pending occurrence absent".into()));
        };
        if imported.state != ImportState::Pending {
            let mut queue = self
                .pending_imports
                .try_borrow_mut()
                .map_err(|_| HostError::Execution("pending import queue borrow conflict".into()))?;
            if queue.first() == Some(&id) {
                queue.remove(0);
            }
            return Ok(None);
        }
        let responses = self
            .responses
            .try_borrow()
            .map_err(|_| HostError::Execution("pending parent proof borrow conflict".into()))?;
        let response = responses
            .entries
            .iter()
            .find(|response| {
                response
                    .sheet
                    .as_ref()
                    .is_some_and(|sheet| sheet.same_sheet(&imported.parent))
            })
            .ok_or_else(|| {
                HostError::Execution("pending import parent source proof absent".into())
            })?;
        // href serialization is existing native scratch allocation, not a new
        // physical allocator upper-bound claim. Charge retained descriptor.
        let url = imported
            .href()
            .ok_or_else(|| HostError::Execution("pending import URL absent".into()))?;
        let bytes = std::mem::size_of::<NativePendingImportRequest>()
            .checked_add(url.capacity())
            .and_then(|n| n.checked_add(response.final_url.len()))
            .and_then(|n| n.checked_add(response.referrer_policy.len()))
            .and_then(|n| n.checked_add(response.encoding.len()))
            .ok_or_else(|| HostError::Execution("pending import descriptor overflow".into()))?;
        drop(responses);
        let charge = self
            .admission
            .reserve(bytes)
            .map_err(HostError::Execution)?;
        fn copy(value: &str) -> Result<String, HostError> {
            let mut out = String::new();
            out.try_reserve_exact(value.len())
                .map_err(|error| HostError::Execution(error.to_string()))?;
            out.push_str(value);
            Ok(out)
        }
        let responses = self
            .responses
            .try_borrow()
            .map_err(|_| HostError::Execution("pending parent recapture conflict".into()))?;
        let response = responses
            .entries
            .iter()
            .find(|response| {
                response
                    .sheet
                    .as_ref()
                    .is_some_and(|sheet| sheet.same_sheet(&imported.parent))
            })
            .ok_or_else(|| HostError::Execution("pending parent proof changed".into()))?;
        let parent_source = response.source.clone();
        let parent_url = copy(&response.final_url)?;
        let referrer_policy = copy(&response.referrer_policy)?;
        let encoding = copy(&response.encoding)?;
        drop(responses);
        self.admission.checkpoint().map_err(HostError::Execution)?;
        // Recheck exact head and native occurrence after callbacks. Neither
        // opaque token nor queue ownership moves until all fallibles succeed.
        let mut queue = self.pending_imports.try_borrow_mut().map_err(|_| {
            HostError::Execution("pending import final queue borrow conflict".into())
        })?;
        let graph = self.graph.try_borrow().map_err(|_| {
            HostError::Execution("pending import final graph borrow conflict".into())
        })?;
        if queue.first() != Some(&id) {
            return Ok(None);
        }
        let Some(token) = graph.begin_import(id) else {
            queue.remove(0);
            return Ok(None);
        };
        queue.remove(0);
        let job = NativePendingImportRequest {
            publication: self.clone(),
            token,
            occurrence: id,
            parent: imported.parent,
            url,
            parent_url,
            _parent_source: parent_source,
            referrer_policy,
            encoding,
            depth: imported.depth,
            _charge: charge,
        };
        drop(graph);
        drop(queue);
        Ok(Some(job))
    }
}
impl NativePendingImportRequest {
    fn current(&self) -> Result<bool, HostError> {
        if self.publication.owner_publication_retired()? {
            return Ok(false);
        }
        let graph = self.publication.graph.try_borrow().map_err(|_| {
            HostError::Execution("pending import currentness borrow conflict".into())
        })?;
        Ok(graph.begin_import(self.occurrence).is_some())
    }
}

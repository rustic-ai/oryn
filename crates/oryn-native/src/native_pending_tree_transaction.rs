enum PendingImportCommitOutcome {
    Published,
    Retry,
    Cancelled,
}
struct PreparedPendingTreeCompletion {
    graph: oryn_layout::cssom::native_import_graph::PreparedImportCompletion,
    tree: Option<StylesheetSourceTree>,
    sheets: Vec<oryn_layout::cssom::native_import_graph::NativeSheet>,
    responses: Vec<RetainedStylesheetResponse>,
    guards: Vec<crate::stylesheet_response_proof::ResponseGuard>,
    old_allocations: Vec<std::sync::Arc<dyn Send + Sync>>,
    allocation: std::sync::Arc<dyn Send + Sync>,
    expected_count: usize,
    expected_revision: u64,
    next_revision: u64,
}
impl PreparedPendingTreeCompletion {
    fn cancel(&self) {
        if let Some(tree) = &self.tree {
            tree.cancel_proofs();
        }
        for guard in &self.guards {
            guard.cancel();
        }
    }
    /// False commit caused by a changed graph/registry snapshot is retried
    /// from the exact still-owned response tree, never a new network fetch.
    /// Caller keeps reservations and request token, reparses into a fresh
    /// unpublished branch using prepare_tree after releasing all borrows.
    fn into_tree(mut self) -> Option<StylesheetSourceTree> {
        self.tree.take()
    }
    /// Owns the entire source tree on false/error; no child proof is consumed
    /// until final graph+response-map publication succeeds.
    fn commit(
        &mut self,
        request: &NativePendingImportRequest,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<PendingImportCommitOutcome, HostError> {
        fn validate(
            tree: &StylesheetSourceTree,
            host: &RawV8Host,
            arena: &SharedDomRuntime,
        ) -> Result<bool, HostError> {
            if !tree.response.proof.validate_native_owner(host, arena)? {
                return Ok(false);
            }
            if matches!(tree.response.proof, NativeStylesheetResponseAuthority::Fetched(_)) {
            let guard = tree
                .response
                .guard
                .as_ref()
                .ok_or_else(|| HostError::Execution("tree proof custody absent".into()))?;
            if guard
                .validate(&tree.response.final_url, &tree.response.source)
                .map_err(stylesheet_fetch_host_error)?
                != Some(tree.response.proof.origin_clean())
            {
                return Ok(false);
            }
            }
            for edge in &tree.imports {
                if let StylesheetSourceOutcome::Loaded(child) = &edge.response
        && !validate(child, host, arena)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        let Some(tree) = &self.tree else {
            return Ok(PendingImportCommitOutcome::Cancelled);
        };
        if !validate(tree, host, arena)? || !request.current()? {
            return Ok(PendingImportCommitOutcome::Cancelled);
        }
        let mut map = request
            .publication
            .responses
            .try_borrow_mut()
            .map_err(|_| HostError::Execution("tree response commit borrow conflict".into()))?;
        let mut graph = request
            .publication
            .graph
            .try_borrow_mut()
            .map_err(|_| HostError::Execution("tree graph commit borrow conflict".into()))?;
        if map.entries.len() != self.expected_count
            || request.publication.mutation_revision.get() != self.expected_revision
            || self.responses.capacity() < map.entries.len() + self.sheets.len()
            || self.guards.capacity() < self.sheets.len()
            || self.old_allocations.capacity() < map.allocations.len() + 1
        {
            return Ok(PendingImportCommitOutcome::Retry);
        }
        if !graph
            .commit_prepared_completion(&mut self.graph)
            .map_err(HostError::Execution)?
        {
            drop(graph); drop(map);
            return Ok(if !request.current()? {
                PendingImportCommitOutcome::Cancelled
            } else {
                PendingImportCommitOutcome::Retry
            });
        }
        // Recursive move uses pre-reserved destination vectors, no clone,
        // admission, validation, parse or fallible work after graph commit.
        fn move_tree(
            tree: StylesheetSourceTree,
            sheets: &mut std::slice::Iter<'_, oryn_layout::cssom::native_import_graph::NativeSheet>,
            responses: &mut Vec<RetainedStylesheetResponse>,
            guards: &mut Vec<crate::stylesheet_response_proof::ResponseGuard>,
        ) {
            let mut response = tree.response;
            response.sheet = Some(sheets.next().expect("validated tree sheet count").clone());
            if let Some(guard) = response.guard.take() {
                guards.push(guard);
            }
            responses.push(response);
            for edge in tree.imports {
                if let StylesheetSourceOutcome::Loaded(child) = edge.response {
                    move_tree(*child, sheets, responses, guards);
                }
            }
        }
        self.responses.append(&mut map.entries);
        move_tree(
            self.tree.take().expect("checked tree custody"),
            &mut self.sheets.iter(),
            &mut self.responses,
            &mut self.guards,
        );
        std::mem::swap(&mut map.entries, &mut self.responses);
        self.old_allocations.append(&mut map.allocations);
        self.old_allocations.push(self.allocation.clone());
        std::mem::swap(&mut map.allocations, &mut self.old_allocations);
        request
            .publication
            .mutation_revision
            .set(self.next_revision);
        drop(graph);
        drop(map);
        self.guards.clear();
        Ok(PendingImportCommitOutcome::Published)
    }
}

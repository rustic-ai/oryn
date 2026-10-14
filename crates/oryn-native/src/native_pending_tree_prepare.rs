impl NativePendingImportRequest {
    fn prepare_tree(
        &self,
        slot: &mut Option<StylesheetSourceTree>,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<Option<PreparedPendingTreeCompletion>, HostError> {
        fn count(tree: &StylesheetSourceTree) -> Result<usize, HostError> {
            tree.imports
                .iter()
                .try_fold(1usize, |n, edge| match &edge.response {
                    StylesheetSourceOutcome::Loaded(child) => n
                        .checked_add(count(child)?)
                        .ok_or_else(|| HostError::Execution("tree response count overflow".into())),
                    _ => Ok(n),
                })
        }
        let tree = slot
            .as_mut()
            .ok_or_else(|| HostError::Execution("pending tree absent".into()))?;
        if !self.current()? {
            return Ok(None);
        }
        let nodes = count(tree)?;
        let context_count = self
            .publication
            .graph
            .try_borrow()
            .map_err(|_| HostError::Execution("tree context count conflict".into()))?
            .completion_context_count();
        let map = self
            .publication
            .responses
            .try_borrow()
            .map_err(|_| HostError::Execution("tree response count conflict".into()))?;
        let expected_count = map.entries.len();
        let allocation_count = map.allocations.len();
        drop(map);
        let expected_revision = self.publication.mutation_revision.get();
        let next_revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| HostError::Execution("tree revision exhausted".into()))?;
        let capacity = expected_count
            .checked_add(nodes)
            .ok_or_else(|| HostError::Execution("tree response capacity overflow".into()))?;
        let bytes = capacity
            .checked_mul(std::mem::size_of::<RetainedStylesheetResponse>())
            .and_then(|n| {
                n.checked_add(nodes.checked_mul(
                    std::mem::size_of::<oryn_layout::cssom::native_import_graph::NativeSheet>()
                        + std::mem::size_of::<crate::stylesheet_response_proof::ResponseGuard>(),
                )?)
            })
            .and_then(|n| {
                n.checked_add(context_count.checked_mul(std::mem::size_of::<
                    oryn_layout::cssom::native_import_graph::NativeImport,
                >())?)
            })
            .and_then(|n| {
                n.checked_add(
                    (allocation_count + 1)
                        .checked_mul(std::mem::size_of::<std::sync::Arc<dyn Send + Sync>>())?,
                )
            })
            .and_then(|n| n.checked_add(tree.queued_source_bytes().ok()?))
            .ok_or_else(|| HostError::Execution("tree retained allocation overflow".into()))?;
        let allocation = self
            .publication
            .admission
            .reserve(bytes)
            .map_err(HostError::Execution)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(context_count)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let context = self
            .publication
            .graph
            .try_borrow()
            .map_err(|_| HostError::Execution("tree context capture conflict".into()))?
            .completion_context(entries)
            .map_err(HostError::Execution)?;
        let Some(graph) = context
            .prepare(
                &self.token,
                &tree.response.source,
                &tree.response.final_url,
                self.publication.admission.as_ref(),
            )
            .map_err(HostError::Execution)?
        else {
            return Ok(None);
        };
        let mut sheets = Vec::new();
        sheets
            .try_reserve_exact(nodes)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let root = graph.child();
        sheets.push(root.clone());
        let mut responses = Vec::new();
        responses
            .try_reserve_exact(capacity)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let mut guards = Vec::new();
        guards
            .try_reserve_exact(nodes)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let mut old_allocations = Vec::new();
        old_allocations
            .try_reserve_exact(allocation_count + 1)
            .map_err(|e| HostError::Execution(e.to_string()))?;
        let mut partial = PreparedPendingImportCompletion {
            graph,
            _snapshot_charge: allocation.clone(),
        };
        partial.stage_source_descendants(
            &root,
            tree,
            &mut sheets,
            self.publication.admission.as_ref(),
            host,
            arena,
        )?;
        self.publication
            .admission
            .checkpoint()
            .map_err(HostError::Execution)?;
        Ok(Some(PreparedPendingTreeCompletion {
            graph: partial.graph,
            tree: slot.take(),
            sheets,
            responses,
            guards,
            old_allocations,
            allocation,
            expected_count,
            expected_revision,
            next_revision,
        }))
    }
}

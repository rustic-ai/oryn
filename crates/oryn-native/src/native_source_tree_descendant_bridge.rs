impl PreparedPendingImportCompletion {
    /// Tree remains caller-owned on error. Sheet metadata is recorded on each
    /// original staged child Arc; proof guard movement belongs to finalization.
    fn stage_source_descendants(
        &mut self,
        parent: &oryn_layout::cssom::native_import_graph::NativeSheet,
        tree: &mut StylesheetSourceTree,
        sheets: &mut Vec<oryn_layout::cssom::native_import_graph::NativeSheet>,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<(), HostError> {
        for edge in &mut tree.imports {
            match &mut edge.response {
                StylesheetSourceOutcome::Loaded(child) => {
                    if !child.response.proof.validate_native_owner(host, arena)? {
                        return Err(HostError::CachedAuthorityExpired);
                    }
                    let guard = child.response.guard.as_ref().ok_or_else(|| {
                        HostError::Execution("descendant proof custody absent".into())
                    })?;
                    if guard
                        .validate(&child.response.final_url, &child.response.source)
                        .map_err(stylesheet_fetch_host_error)?
                        != Some(child.response.proof.origin_clean())
                    {
                        return Err(HostError::CachedAuthorityExpired);
                    }
                    sheets
                        .try_reserve(1)
                        .map_err(|e| HostError::Execution(e.to_string()))?;
                    let sheet = self
                        .graph
                        .prepare_descendant(
                            parent,
                            edge.index,
                            Some((&child.response.source, &child.response.final_url)),
                            admission,
                        )
                        .map_err(HostError::Execution)?
                        .ok_or_else(|| {
                            HostError::Execution(
                                "loaded descendant refused by native parser".into(),
                            )
                        })?;
                    sheets.push(sheet.clone());
                    self.stage_source_descendants(&sheet, child, sheets, admission, host, arena)?;
                }
                StylesheetSourceOutcome::Refused
                | StylesheetSourceOutcome::Failed { .. }
                | StylesheetSourceOutcome::Cycle => {
                    self.graph
                        .prepare_descendant(parent, edge.index, None, admission)
                        .map_err(HostError::Execution)?;
                }
            }
        }
        Ok(())
    }
}

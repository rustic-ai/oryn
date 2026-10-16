// Reuse a previously published inline root: fetched trees complete its exact
// native occurrence tokens, never parse or fetch the root a second time.
struct InlinePendingRootDelivery {
    record: Rc<RetainedStylesheetRecord>,
    root: oryn_layout::cssom::native_import_graph::NativeSheet,
    remaining: Vec<StylesheetSourceEdge>,
    current: Option<(NativePendingImportRequest, Option<StylesheetSourceTree>, Option<PreparedPendingTreeCompletion>)>,
    _reservations: Vec<ScriptSourceReservation>,
    _source_allocation: Option<Arc<dyn Send + Sync>>,
    pending_dependencies: Option<CssDependencies>,
    dependency_allocation: Option<Arc<dyn Send + Sync>>,
}

impl InlinePendingRootDelivery {
    fn poll(&mut self, host: &RawV8Host, arena: &SharedDomRuntime) -> Result<bool, HostError> {
        loop {
            if self.record.owner_publication_retired()? { self.cancel(); return Ok(true); }
            if let Some((request, tree, prepared)) = &mut self.current {
                if prepared.is_none() {
                    let Some(value) = request.prepare_tree(tree, host, arena)? else {
                        if !request.current()? {
                            if let Some(tree)=tree.as_ref() {tree.cancel_proofs();}
                            self.current = None; continue;
                        }
                        return Ok(false);
                    };
                    *prepared = Some(value);
                }
                match prepared.as_mut().expect("prepared child").commit(request, host, arena)? {
                    PendingImportCommitOutcome::Published => { self.current = None; }
                    PendingImportCommitOutcome::Cancelled => {
                        if let Some(prepared)=prepared.as_ref() {prepared.cancel();}
                        request.fail_current(host, arena)?;
                        self.current = None;
                    }
                    PendingImportCommitOutcome::Retry => {
                        *tree = prepared.take().expect("retry child").into_tree();
                        return Ok(false);
                    }
                }
                continue;
            }
            let Some(edge) = self.remaining.last() else { return Ok(true) };
            let imported = {
                let graph = self.record.graph.try_borrow().map_err(|_|HostError::Execution("inline child occurrence conflict".into()))?;
                graph.imports().find(|entry| entry.parent.same_sheet(&self.root) && entry.index == edge.index).cloned()
            }.ok_or_else(||HostError::Execution("inline fetched occurrence absent".into()))?;
            if imported.href().as_deref() != Some(edge.requested_url.as_str()) {
                return Err(HostError::Execution("inline fetched occurrence URL changed".into()));
            }
            if imported.state != oryn_layout::cssom::native_import_graph::ImportState::Pending {
                if let StylesheetSourceOutcome::Loaded(tree)=&edge.response {
                    tree.cancel_proofs();
                }
                self.remaining.pop();
                continue;
            }
            match &edge.response {
                StylesheetSourceOutcome::Loaded(_) => {
                    // Descriptor/token construction is preflighted without
                    // mutating the pending queue; that queue belongs to CSSOM
                    // insertions, not the already scheduled authored root.
                    let request = self.record.request_for_original_occurrence(imported)?;
                    let edge = self.remaining.pop().expect("checked source edge");
                    let StylesheetSourceOutcome::Loaded(tree) = edge.response else {unreachable!()};
                    self.current = Some((request, Some(*tree), None));
                }
                StylesheetSourceOutcome::Refused => {
                    // Nonmatching supports() was indexed Refused by Stylo.
                    // A fetched Refused edge cannot silently abandon Pending.
                    return Err(HostError::Execution("inline supports refusal state mismatch".into()));
                }
                StylesheetSourceOutcome::Failed { .. } | StylesheetSourceOutcome::Cycle => {
                    let request = self.record.request_for_original_occurrence(imported)?;
                    request.fail_current(host, arena)?;
                    self.remaining.pop();
                }
            }
        }
    }
    fn cancel(&self) {
        for edge in &self.remaining {
            if let StylesheetSourceOutcome::Loaded(tree) = &edge.response {tree.cancel_proofs();}
        }
        if let Some((_, tree, prepared)) = &self.current {
            if let Some(tree) = tree {tree.cancel_proofs();}
            if let Some(prepared) = prepared {prepared.cancel();}
        }
    }
}

impl RetainedParserCompletion {
    fn capture_incumbent_inline_root(
        &mut self,
        record: Rc<RetainedStylesheetRecord>,
        host: &RawV8Host,
        arena: &SharedDomRuntime,
        registry: &RefCell<RetainedStylesheets>,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<bool, HostError> {
        let RetainedParserCompletionPhase::Source(envelope) = &mut self.phase else {return Ok(false)};
        let tree = envelope.tree.as_ref().ok_or_else(||HostError::Execution("inline root tree absent".into()))?;
        if !matches!(&tree.response.proof,NativeStylesheetResponseAuthority::Inline(_))
            || tree.response.source.as_ref()!=record._source.as_ref() {return Ok(false);}
        // Dependency metadata is assembled before moving tree/guard custody.
        // A failure leaves the original Source envelope untouched for retry.
        let mut dependencies=Vec::new();
        fn demand(tree:&StylesheetSourceTree,depth:usize)->Result<(usize,usize),HostError> {
            if depth>8 {return Err(HostError::LimitExceeded {resource:"stylesheet import depth".into()});}
            let bytes=tree.response.final_url.len().checked_mul(2)
                .and_then(|n|n.checked_add(tree.response.referrer_policy.len()))
                .and_then(|n|n.checked_add(tree.response.source.len()))
                .and_then(|n|n.checked_add(std::mem::size_of::<(String,String,String,String)>()))
                .ok_or_else(||HostError::Execution("inline dependency demand overflow".into()))?;
            let mut result=(1usize,bytes);
            for edge in &tree.imports {
                if let StylesheetSourceOutcome::Loaded(child)=&edge.response {
                    let child=demand(child,depth+1)?;
                    result.0=result.0.checked_add(child.0).ok_or_else(||HostError::Execution("inline dependency count overflow".into()))?;
                    result.1=result.1.checked_add(child.1).ok_or_else(||HostError::Execution("inline dependency bytes overflow".into()))?;
                }
            }
            Ok(result)
        }
        let (count,dependency_bytes)=demand(tree,0)?;
        let dependency_charge=admission.reserve(dependency_bytes).map_err(HostError::Execution)?;
        let charge=admission.reserve(std::mem::size_of::<InlinePendingRootDelivery>()+2*std::mem::size_of::<usize>())
            .map_err(HostError::Execution)?;
        dependencies.try_reserve_exact(count).map_err(|e|HostError::Execution(e.to_string()))?;
        fn dependencies_for(tree:&StylesheetSourceTree,out:&mut CssDependencies,depth:usize)->Result<(),HostError> {
            if depth>8 {return Err(HostError::LimitExceeded {resource:"stylesheet import depth".into()});}
            collect_css_dependencies(out,&tree.response.final_url,&tree.response.final_url,
                &tree.response.referrer_policy,&tree.response.source)?;
            for edge in &tree.imports {
                if let StylesheetSourceOutcome::Loaded(child)=&edge.response {dependencies_for(child,out,depth+1)?;}
            }
            Ok(())
        }
        dependencies_for(tree,&mut dependencies,0)?;
        admission.checkpoint().map_err(HostError::Execution)?;
        let root=record.graph.try_borrow().map_err(|_|HostError::Execution("inline root capture conflict".into()))?.root();
        if !self.authority.current(host,arena)? || record.owner_publication_retired()? || record.owner.revision!=self.authority.source_revision {
            return Ok(false);
        }
        let current=registry.try_borrow().map_err(|_|HostError::Execution("inline root final registry conflict".into()))?
            .current(arena,self.authority.document,self.authority.owner,self.authority.source_revision);
        if !current.as_ref().is_some_and(|entry|Rc::ptr_eq(entry,&record)) {return Ok(false);}
        if envelope.tree.as_ref().is_none_or(|tree|tree.response.source.as_ref()!=record._source.as_ref()) {return Ok(false);}
        let tree=envelope.take_tree()?;
        let reservations=std::mem::take(&mut envelope.reservations);
        let source_allocation=envelope._allocation.take();
        let mut remaining=tree.imports;
        remaining.reverse();
        self.phase=RetainedParserCompletionPhase::InlineRoot {
            delivery:Box::new(InlinePendingRootDelivery {
                record,root,remaining,current:None,_reservations:reservations,
                _source_allocation:source_allocation,pending_dependencies:Some(dependencies),
                dependency_allocation:Some(dependency_charge),
            }),
            _allocation:charge,
        };
        Ok(true)
    }
}

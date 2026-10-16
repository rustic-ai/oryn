// Native main-thread import occurrence work and its immutable broker reply.
// This work never serializes the opaque graph token or original Stylo Arc.
struct NativePendingImportJob {
    request: NativePendingImportRequest,
    reply: Option<RetainedStylesheetSourceEnvelope>,
    failure: Option<String>,
    failed: bool,
    prepared: Option<PreparedPendingTreeCompletion>,
    _request_allocation: Arc<dyn Send + Sync>,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    dependencies: Option<CssDependencies>,
    committed: bool,
    _dependency_allocation: Option<Arc<dyn Send + Sync>>,
    settlement_failures: (Option<HostError>, Option<HostError>),
}
impl NativePendingImportJob {
    fn publish(
        &mut self,
        host: &mut RawV8Host,
        arena: &SharedDomRuntime,
    ) -> Result<bool, HostError> {
        if self.request.publication.owner_publication_retired()? {
            return Ok(true);
        }
        if self.committed {
            host.queue_owned_css_dependencies(
                self.request.publication.owner.node,
                self.request.publication.owner.revision,
                &mut self.dependencies,
            )?;
            return Ok(true);
        }
        if !self.request.current()? {
            self.cancellation
                .flag
                .store(true, std::sync::atomic::Ordering::Release);
            if let Some(reply) = &self.reply
        && let Some(tree) = &reply.tree {
                tree.cancel_proofs();
            }
            return Ok(true);
        }
        if let Some(error) = &self.failure {
            return Err(HostError::Execution(error.clone()));
        }
        if self.failed {
            return self.request.fail_current(host, arena);
        }
        let Some(envelope) = &mut self.reply else {
            return Ok(false);
        };
        if self.prepared.is_none() {
            if self.dependencies.is_none() {
                fn dependency_bytes(tree: &StylesheetSourceTree) -> Result<usize, HostError> {
                    let bytes = tree
                        .response
                        .source
                        .len()
                        .checked_add(tree.response.final_url.len().checked_mul(2).ok_or_else(
                            || HostError::Execution("dependency URL size overflow".into()),
                        )?)
                        .and_then(|value| value.checked_add(tree.response.referrer_policy.len()))
                        .and_then(|value| {
                            value.checked_add(std::mem::size_of::<(
                                    String,
                                    String,
                                    String,
                                    String,
                                )>())
                        })
                        .ok_or_else(|| {
                            HostError::Execution("dependency payload size overflow".into())
                        })?;
                    tree.imports
                        .iter()
                        .try_fold(bytes, |total, edge| match &edge.response {
                            StylesheetSourceOutcome::Loaded(child) => {
                                total.checked_add(dependency_bytes(child)?).ok_or_else(|| {
                                    HostError::Execution("dependency total overflow".into())
                                })
                            }
                            _ => Ok(total),
                        })
                }
                let charge = if let Some(tree) = &envelope.tree {
                    Some(
                        self.request
                            .publication
                            .admission
                            .reserve(dependency_bytes(tree)?)
                            .map_err(HostError::Execution)?,
                    )
                } else {
                    None
                };
                fn collect(
                    tree: &StylesheetSourceTree,
                    out: &mut CssDependencies,
                ) -> Result<(), HostError> {
                    collect_css_dependencies(
                        out,
                        &tree.response.final_url,
                        &tree.response.final_url,
                        &tree.response.referrer_policy,
                        &tree.response.source,
                    )?;
                    for edge in &tree.imports {
                        if let StylesheetSourceOutcome::Loaded(child) = &edge.response {
                            collect(child, out)?;
                        }
                    }
                    Ok(())
                }
                let mut dependencies = Vec::new();
                if let Some(tree) = &envelope.tree {
                    collect(tree, &mut dependencies)?;
                }
                self.dependencies = Some(dependencies);
                self._dependency_allocation = charge;
            }
            self.prepared = self.request.prepare_tree(&mut envelope.tree, host, arena)?;
        }
        let Some(prepared) = &mut self.prepared else {
            return Ok(false);
        };
        match prepared.commit(&self.request, host, arena)? {
            PendingImportCommitOutcome::Retry => {
                // An unrelated publication changed graph capacities/revision.
                // Keep the exact fetched tree and proof custody, then prepare
                // against the current graph on the next owning-thread turn.
                if let Some(prepared) = self.prepared.take() {
                    envelope.tree = prepared.into_tree();
                }
                return Ok(false);
            }
            PendingImportCommitOutcome::Cancelled => {
                if let Some(prepared) = &self.prepared {
                    prepared.cancel();
                }
                self.failed = true;
                return self.request.fail_current(host, arena);
            }
            PendingImportCommitOutcome::Published => {}
        }
        // Native graph and exact response metadata now own retained source.
        // Transient broker/source-queue custody ends only after atomic commit.
        self.reply = None;
        self.committed = true;
        host.queue_owned_css_dependencies(
            self.request.publication.owner.node,
            self.request.publication.owner.revision,
            &mut self.dependencies,
        )?;
        Ok(true)
    }
}
impl Drop for NativePendingImportJob {
    fn drop(&mut self) {
        if self.committed { return; } // Published source authority outlives transport.
        self.cancellation
            .flag
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(reply) = &self.reply
        && let Some(tree) = &reply.tree {
            tree.cancel_proofs();
        }
        if let Some(prepared) = &mut self.prepared {
            prepared.cancel();
        }
    }
}
struct NativePendingImportReply {
    key: NativePendingImportKey,
    document: ParserDocumentIdentity,
    publication_revision: u64,
    occurrence: oryn_layout::cssom::native_import_graph::OccurrenceId,
    result: Result<RetainedStylesheetSourceEnvelope, RetainedStylesheetFailure>,
}

#[cfg(unix)]
struct NativePendingImportFetch {
    endpoint: Option<(crate::context_service::SharedContextClient, oryn_common::v3::PageId)>,
    standalone: Option<standalone_stylesheet::Loader>,
    initiator: crate::context_service::ResourceInitiator,
    key: NativePendingImportKey,
    document: ParserDocumentIdentity,
    owner_node: NodeId,
    publication_revision: u64,
    occurrence: oryn_layout::cssom::native_import_graph::OccurrenceId,
    url: String,
    parent_url: String,
    referrer_policy: String,
    encoding: String,
    depth: usize,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    source_budget: Arc<std::sync::atomic::AtomicUsize>,
    queued_source: ScriptSourceReservation,
    _request_allocation: Arc<dyn Send + Sync>,
}

#[cfg(unix)]
impl NativePendingImportFetch {
    fn execute(mut self) -> NativePendingImportReply {
        let result = (|| {
            if self.depth > 8 {
                return Err(HostError::LimitExceeded {
                    resource: "stylesheet import depth".into(),
                });
            }
            if let Some(loader) = self.standalone.take() {
                return loader.load_retained_import_tree(
                    self.document, self.owner_node,
                    self.publication_revision, self.cancellation.clone(),
                    &self.url, &self.parent_url, &self.referrer_policy,
                    &self.encoding, self.depth,
                );
            }
            let (client, page_id) = self.endpoint.as_ref()
                .ok_or_else(|| HostError::Execution("pending import transport absent".into()))?;
            let mut budget =
                StylesheetExpansionBudget::new(self.source_budget, self.cancellation.flag.clone())
                    .with_native(self.cancellation.clone());
            let mut walk = ImportWalk {
                client,
                page_id: *page_id,
                initiator: &self.initiator,
                budget: &mut budget,
            };
            let response = walk
                .fetch_response_options_typed(
                    &self.url,
                    &self.parent_url,
                    &self.referrer_policy,
                    &self.encoding,
                    true,
                )
                .map_err(stylesheet_fetch_host_error)?;
            let mut ancestors = Vec::new();
            ancestors
                .try_reserve_exact(2)
                .map_err(|_| HostError::Execution("pending import ancestry allocation".into()))?;
            ancestors.push(stylesheet_cycle_key(&self.parent_url));
            ancestors.push(stylesheet_cycle_key(&response.final_url));
            let tree = walk.collect_source_tree(response, self.depth, &mut ancestors)?;
            walk.budget.finish_retained_tree(tree)
        })()
        .map_err(RetainedStylesheetFailure::from);
        drop(self.queued_source);
        NativePendingImportReply {
            key: self.key,
            document: self.document,
            publication_revision: self.publication_revision,
            occurrence: self.occurrence,
            result,
        }
    }
}

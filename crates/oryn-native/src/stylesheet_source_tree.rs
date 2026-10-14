// Cross-thread broker output: immutable original responses, not a Stylo graph.
// The owning realm realizes the graph after proof/current-owner checks and
// native allocation admission, using its one existing SharedRwLock.
struct StylesheetSourceTree {
    response: RetainedStylesheetResponse,
    imports: Vec<StylesheetSourceEdge>,
}
impl StylesheetSourceTree {
    fn cancel_proofs(&self) {
        if let Some(guard) = &self.response.guard {
            guard.cancel();
        }
        for edge in &self.imports {
            if let StylesheetSourceOutcome::Loaded(child) = &edge.response {
                child.cancel_proofs();
            }
        }
    }
    fn queued_source_bytes(&self) -> Result<usize, HostError> {
        let mut bytes = 0usize;
        let mut pending = Vec::new();
        pending
            .try_reserve(1)
            .map_err(|_| HostError::Execution("stylesheet source walk allocation".into()))?;
        pending.push(self);
        while let Some(tree) = pending.pop() {
            bytes = bytes
                .checked_add(tree.response.source.len())
                .and_then(|value| value.checked_add(tree.response.final_url.len()))
                .and_then(|value| value.checked_add(tree.response.referrer_policy.len()))
                .and_then(|value| value.checked_add(tree.response.encoding.len()))
                .ok_or_else(|| HostError::LimitExceeded {
                    resource: "stylesheet source bytes".into(),
                })?;
            for edge in &tree.imports {
                bytes = bytes.checked_add(edge.requested_url.len()).ok_or_else(|| {
                    HostError::LimitExceeded {
                        resource: "stylesheet occurrence bytes".into(),
                    }
                })?;
                if let StylesheetSourceOutcome::Loaded(child) = &edge.response {
                    pending.try_reserve(1).map_err(|_| {
                        HostError::Execution("stylesheet source walk allocation".into())
                    })?;
                    pending.push(child);
                }
            }
        }
        Ok(bytes)
    }
}
struct StylesheetSourceEdge {
    /// Parsed parent rule index; duplicate URLs are distinct occurrences.
    index: usize,
    requested_url: String,
    response: StylesheetSourceOutcome,
}
enum StylesheetSourceOutcome {
    Loaded(Box<StylesheetSourceTree>),
    Refused,
    Failed { _diagnostic: crate::RuntimeError },
    Cycle,
}
impl ImportWalk<'_> {
    fn collect_root_source_tree(
        &mut self,
        url: &str,
        referrer: &str,
        policy: &str,
        encoding: &str,
    ) -> Result<StylesheetSourceTree, HostError> {
        let response = self
            .fetch_root_response_typed(url, referrer, policy, encoding)
            .map_err(stylesheet_fetch_host_error)?;
        let mut ancestors = Vec::new();
        ancestors
            .try_reserve_exact(2)
            .map_err(|_| HostError::Execution("stylesheet root ancestry allocation".into()))?;
        ancestors.push(stylesheet_cycle_key(url));
        let final_key = stylesheet_cycle_key(&response.final_url);
        if ancestors.last() != Some(&final_key) {
            ancestors.push(final_key)
        }
        self.collect_source_tree(response, 0, &mut ancestors)
    }

    fn collect_source_tree(
        &mut self,
        response: RetainedStylesheetResponse,
        depth: usize,
        ancestors: &mut Vec<String>,
    ) -> Result<StylesheetSourceTree, HostError> {
        if depth > 8 {
            return Err(HostError::LimitExceeded {
                resource: "stylesheet import depth".into(),
            });
        }
        self.budget.check()?;
        self.validate_response(&response)?;
        // Native parsed discovery retains original rule positions and source;
        // it does not serialize generated condition wrappers or remove imports.
        // Discovery uses the existing native parser under resource deadline and
        // process RSS monitoring. Its opaque allocator is not a source-length
        // upper bound and is not represented as a controlled allocation grant.
        let rules =
            oryn_layout::cssom::read_stylesheet(&response.source, &response.final_url, false, true)
                .map_err(HostError::Execution)?;
        let mut imports = Vec::new();
        for (index, rule) in rules.into_iter().enumerate() {
            if rule.rule_type != 3 {
                continue;
            }
            self.budget.check()?;
            let Some(requested_url) = rule.properties.get("href").cloned() else {
                continue;
            };
            let outcome = if rule
                .properties
                .get("supportsMatches")
                .is_some_and(|value| value == "false")
            {
                StylesheetSourceOutcome::Refused
            } else if ancestors.contains(&stylesheet_cycle_key(&requested_url)) {
                StylesheetSourceOutcome::Cycle
            } else {
                // A resource failure is a terminal failed occurrence. Native
                // limit/cancellation errors remain errors of the whole graph;
                // callers must not swallow them into an empty loaded sheet.
                match self
                    .fetch_outcome(&requested_url, &response)
                    .map_err(stylesheet_fetch_host_error)?
                {
                    StylesheetFetchOutcome { response: None, failure: Some(error) } => StylesheetSourceOutcome::Failed { _diagnostic: error },
                    StylesheetFetchOutcome { response: Some(_), failure: Some(_) } => {
                        return Err(HostError::Execution("inconsistent stylesheet fetch outcome".into()));
                    }
                    StylesheetFetchOutcome { response: None, failure: None } => {
                        return Err(HostError::CachedAuthorityExpired);
                    }
                    StylesheetFetchOutcome { response: Some(child), failure: None } => {
                        let final_key = stylesheet_cycle_key(&child.final_url);
                        if ancestors.contains(&final_key) {
                            StylesheetSourceOutcome::Cycle
                        } else {
                            ancestors.push(final_key);
                            let nested = self.collect_source_tree(child, depth + 1, ancestors);
                            ancestors.pop();
                            StylesheetSourceOutcome::Loaded(Box::new(nested?))
                        }
                    }
                }
            };
            imports.try_reserve(1).map_err(|_| {
                HostError::Execution("stylesheet source occurrence allocation".into())
            })?;
            imports.push(StylesheetSourceEdge {
                index,
                requested_url,
                response: outcome,
            });
        }
        Ok(StylesheetSourceTree { response, imports })
    }
}
impl StylesheetSourceTree {
    fn realize(
        self,
        lock: oryn_layout::cssom::NativeStyleLock,
        quirks: bool,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<RetainedStylesheetSource, HostError> {
        admission.checkpoint().map_err(HostError::Execution)?;
        let original = self.response.source.clone();
        let final_url = self.response.final_url.clone();
        let mut graph = oryn_layout::cssom::native_import_graph::ImportGraph::parse(
            &original, &final_url, quirks, 8, lock, admission,
        )
        .map_err(HostError::Execution)?;
        let root = graph.root();
        let mut responses = Vec::new();
        self.realize_children(&mut graph, &root, admission, &mut responses)?;
        let mut dependencies = Vec::new();
        for response in &responses {
            collect_css_dependencies(
                &mut dependencies,
                &response.final_url,
                &response.final_url,
                &response.referrer_policy,
                &response.source,
            )?;
        }
        Ok(RetainedStylesheetSource {
            final_url,
            original,
            graph,
            responses,
            dependencies,
            retained_allocation: None,
        })
    }

    fn realize_children(
        self,
        graph: &mut oryn_layout::cssom::native_import_graph::ImportGraph,
        sheet: &oryn_layout::cssom::native_import_graph::NativeSheet,
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
        responses: &mut Vec<RetainedStylesheetResponse>,
    ) -> Result<(), HostError> {
        let mut response = self.response;
        if matches!(
            response.proof,
            NativeStylesheetResponseAuthority::Fetched(_)
        ) {
            let clean = response
                .guard
                .as_ref()
                .ok_or_else(|| {
                    HostError::Execution("stylesheet response custody already released".into())
                })?
                .validate(&response.final_url, &response.source)
                .map_err(stylesheet_fetch_host_error)?;
            match clean {
                Some(clean) if clean == response.proof.origin_clean() => {}
                None => return Err(HostError::CachedAuthorityExpired),
                Some(_) => {
                    return Err(HostError::Execution(
                        "stylesheet response origin proof mismatch".into(),
                    ));
                }
            }
        }
        response.sheet = Some(sheet.clone());
        for edge in self.imports {
            admission.checkpoint().map_err(HostError::Execution)?;
            let occurrence = graph
                .imports()
                .find(|entry| entry.parent.same_sheet(sheet) && entry.index == edge.index)
                .map(|entry| (entry.id, entry.href()));
            let Some((id, href)) = occurrence else {
                return Err(HostError::Execution(
                    "stylesheet import occurrence mismatch".into(),
                ));
            };
            if href.as_deref() != Some(edge.requested_url.as_str()) {
                return Err(HostError::Execution(
                    "stylesheet import parsed URL mismatch".into(),
                ));
            }
            match edge.response {
                StylesheetSourceOutcome::Loaded(child) => {
                    let Some(token) = graph.begin_import(id) else {
                        continue;
                    };
                    let clean = child
                        .response
                        .guard
                        .as_ref()
                        .ok_or_else(|| {
                            HostError::Execution("stylesheet child custody already released".into())
                        })?
                        .validate(&child.response.final_url, &child.response.source)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if clean != Some(child.response.proof.origin_clean()) {
                        return Err(HostError::CachedAuthorityExpired);
                    }
                    graph
                        .commit(
                            token,
                            &child.response.source,
                            &child.response.final_url,
                            admission,
                        )
                        .map_err(HostError::Execution)?;
                    let child_sheet = graph
                        .occurrence(id)
                        .and_then(|entry| entry.child.clone())
                        .ok_or_else(|| {
                            HostError::Execution("stylesheet child publication missing".into())
                        })?;
                    child.realize_children(graph, &child_sheet, admission, responses)?;
                }
                StylesheetSourceOutcome::Refused => {}
                StylesheetSourceOutcome::Failed { .. } | StylesheetSourceOutcome::Cycle => {
                    graph.cancel(id).map_err(HostError::Execution)?;
                }
            }
        }
        responses
            .try_reserve(1)
            .map_err(|_| HostError::Execution("stylesheet response graph allocation".into()))?;
        responses.push(response);
        Ok(())
    }
}

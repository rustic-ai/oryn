/// Select retained execution only after the owning provider has a real shared
/// allocation authority. This is a native configuration decision, not a test
/// fixture or source-string branch.
impl ParserResourceScheduler {
    #[cfg(unix)]
    fn schedule_retained_auto(
        &mut self,
        params: AutomaticStylesheetScheduleInput<'_>,
        admission: SharedRetainedAdmission,
    ) -> Result<(), HostError> {
        let AutomaticStylesheetScheduleInput {
            context,
            node,
            revision,
            href,
            css,
            encoding,
            mut fetch,
            host,
            resources,
        } = params;
        if self.local_stylesheets.contains_key(&(node, revision)) {
            return Ok(());
        }
        let arena = host
            .automation_dom(fetch.realm)
            .ok_or_else(|| HostError::Execution("retained stylesheet realm retired".into()))?;
        self.prepare_render_stylesheet(&arena, node, revision, true)?;
        fetch.render_blocking = self
            .render_stylesheets
            .get(&(node, revision))
            .is_some_and(|(_, blocking)| *blocking);
        host.reconcile_inline_style_policy()?;
        if href.is_none()
            && !arena
                .borrow()
                .arena()
                .get(node)
                .map_err(|error| HostError::Execution(error.to_string()))?
                .stylesheet_policy_allowed
        {
            self.complete_render_stylesheet(node, revision);
            return Ok(());
        }
        let Some((_, page_id, _)) = context else {
            if href.is_none()
                && !oryn_layout::cssom::stylesheet_requires_resources(&css, &fetch.base_url)
                    .map_err(HostError::Execution)?
            {
                // Preflight may parse rules; recapture receipt in the current
                // safe host context before native allocation callbacks.
                let admission = host.native_retained_admission_for_realm(fetch.realm)?
                    .ok_or(HostError::CachedAuthorityExpired)?;
                let _scope = host.retained_allocation_scope(fetch.realm)?;
                let realm = fetch.realm;
                let result = self.schedule_retained_standalone_inline(
                    node, revision, css, encoding, fetch, admission,
                );
                let invalidated = host.invalidate_native_retained_allocation(realm);
                return match result { Err(error) => Err(error), Ok(()) => invalidated };
            }
            let realm = fetch.realm;
            let result = self.schedule_retained_standalone_resources(
                node, revision, href, css, encoding, fetch, host, resources, admission,
            );
            let invalidated = host.invalidate_native_retained_allocation(realm);
            return match result { Err(error) => Err(error), Ok(()) => invalidated };
        };
        let client = host.resource_client_for_realm(fetch.realm, *page_id)?;
        let source = match href {
            Some(url) => RetainedStylesheetFetchSource::External(url),
            None => RetainedStylesheetFetchSource::Inline(css),
        };
        let realm = fetch.realm;
        let scheduled = self.schedule_retained_stylesheet(RetainedStylesheetFetchWork {
            client,
            page_id: *page_id,
            node,
            revision,
            fetch,
            environment_encoding: encoding,
            source,
            admission,
            cancellation: Arc::new(crate::context_service::ResourceCancellation::default()),
            source_budget: self.source_bytes.clone(),
        });
        let invalidated = host.invalidate_native_retained_allocation(realm);
        match scheduled {
            Err(error) => Err(error),
            Ok(()) => invalidated,
        }
    }

    // Authored inline CSS needs DOM/CSP authority, not a fetched response or
    // a fabricated parent client. Resource-bearing sources retain the existing
    // broker requirement; this path emits the normal retained delivery recipe.
    #[cfg(unix)]
    fn schedule_retained_standalone_inline(
        &mut self,
        node: NodeId,
        revision: u64,
        css: String,
        encoding: String,
        fetch: ParserFetchContext,
        admission: SharedRetainedAdmission,
    ) -> Result<(), HostError> {
        self.ensure_capacity()?;
        admission.checkpoint().map_err(HostError::Execution)?;
        let bytes = css.len().checked_add(fetch.base_url.len())
            .and_then(|n| n.checked_add(fetch.referrer_policy.len()))
            .and_then(|n| n.checked_add(encoding.len()))
            .and_then(|n| n.checked_add(std::mem::size_of::<RetainedStylesheetReady>()))
            .ok_or_else(|| HostError::Execution("standalone inline source size overflow".into()))?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        let identity = fetch.identity();
        let mut budget = StylesheetExpansionBudget::new(
            self.source_bytes.clone(), cancellation.flag.clone(),
        ).with_native(cancellation.clone());
        budget.retain(bytes)?;
        let response = RetainedStylesheetResponse {
            sheet: None,
            final_url: fetch.base_url,
            source: Arc::from(css),
            referrer_policy: fetch.referrer_policy,
            encoding,
            proof: NativeStylesheetResponseAuthority::Inline(NativeInlineStylesheetAuthority {
                document: identity, node, source_revision: revision,
                cancellation: cancellation.clone(),
            }),
            guard: None,
        };
        let mut envelope = budget.finish_retained_tree(StylesheetSourceTree {
            response, imports: Vec::new(),
        })?;
        envelope._allocation = Some(charge);
        self.ready_completions.try_reserve(1)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        admission.checkpoint().map_err(HostError::Execution)?;
        self.reserve_stylesheet(node, revision)?;
        self.stylesheet_identities.insert((node, revision), identity);
        self.local_stylesheets.insert((node, revision), (
            standalone_stylesheet::Cancel::native(cancellation.clone()),
            Arc::new(std::sync::Mutex::new(None)),
        ));
        self.receive_completion(ParserResourceCompletion::RetainedStylesheet(
            RetainedStylesheetReady {
                identity, node, revision, cancellation, result: Ok(envelope),
            },
        ));
        // Source queue reservations own transient payload until publication;
        // the graph/publication transaction acquires its retained allocation.
        Ok(())
    }
    #[cfg(unix)]
    #[expect(clippy::too_many_arguments, reason = "scheduling transfers exact source, owner admission and cancellation without additional allocation")]
    fn schedule_retained_standalone_resources(
        &mut self, node: NodeId, revision: u64, href: Option<String>,
        css: String, encoding: String, fetch: ParserFetchContext,
        host: &mut RawV8Host, resources: &BTreeMap<String, String>,
        admission: SharedRetainedAdmission,
    ) -> Result<(), HostError> {
        self.ensure_capacity()?;
        admission.checkpoint().map_err(HostError::Execution)?;
        let (mut loader, cancel) = standalone_stylesheet::Loader::capture(
            host, resources, &fetch, self.source_bytes.clone())?;
        // Transfer cancellation, not a second transport lifetime.
        cancel.complete();
        let cancellation = Arc::new(crate::context_service::ResourceCancellation::default());
        loader.bind_retained_cancellation(cancellation.clone());
        let queued_bytes = [css.len(), href.as_ref().map_or(0, String::len),
            encoding.len(), fetch.base_url.len(), fetch.referrer_policy.len(),
            fetch.initiator_url.len()].into_iter().try_fold(0usize, usize::checked_add)
            .ok_or_else(|| HostError::Execution("standalone request size overflow".into()))?;
        let queued = ScriptSourceReservation::reserve(&self.source_bytes, queued_bytes)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "queued standalone stylesheet bytes".into() })?;
        let identity = fetch.identity();
        let key = (node, revision);
        admission.checkpoint().map_err(HostError::Execution)?;
        self.reserve_stylesheet(node, revision)?;
        self.stylesheet_identities.insert(key, identity);
        self.local_stylesheets.insert(key, (
            standalone_stylesheet::Cancel::native(cancellation.clone()),
            Arc::new(std::sync::Mutex::new(None)),
        ));
        let result_cancellation = cancellation.clone();
        spawn_parser_resource(self.sender.clone(), Arc::downgrade(&self.lifetime), queued,
            move || Ok(loader.load_retained_tree(
                identity, node, revision, cancellation,
                href.as_deref(), &fetch.base_url, &css, &encoding,
            ).map_err(RetainedStylesheetFailure::from)),
            move |result| ParserResourceCompletion::RetainedStylesheet(RetainedStylesheetReady {
                identity, node, revision, cancellation: result_cancellation,
                result: result.unwrap_or_else(|error| Err(RetainedStylesheetFailure::Operational(error))),
            }),
        );
        Ok(())
    }
}

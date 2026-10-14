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
            resources: _,
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
            return Err(HostError::Execution(
                "retained stylesheet broker context absent".into(),
            ));
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
}

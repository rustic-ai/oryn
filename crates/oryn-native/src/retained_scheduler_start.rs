impl ParserResourceScheduler {
    #[cfg(unix)]
    fn schedule_retained_stylesheet(
        &mut self,
        mut work: RetainedStylesheetFetchWork,
    ) -> Result<(), HostError> {
        self.ensure_capacity()?;
        work.admission.checkpoint().map_err(HostError::Execution)?;
        let source_len = match &work.source {
            RetainedStylesheetFetchSource::External(value)
            | RetainedStylesheetFetchSource::Inline(value) => value.len(),
        };
        let queued_bytes = [
            source_len,
            work.environment_encoding.len(),
            work.fetch.base_url.len(),
            work.fetch.url_encoding.len(),
            work.fetch.initiator_url.len(),
            work.fetch.referrer_policy.len(),
            work.fetch.stylesheet_policy.as_ref().map_or(0, String::len),
            work.fetch.integrity.as_ref().map_or(0, String::len),
            work.fetch.cross_origin.as_ref().map_or(0, String::len),
            work.fetch.nonce.as_ref().map_or(0, String::len),
        ]
        .into_iter()
        .try_fold(0usize, usize::checked_add)
        .ok_or_else(|| HostError::Execution("retained queued source size overflow".into()))?;
        let queued_source = ScriptSourceReservation::reserve(&self.source_bytes, queued_bytes)
            .ok_or_else(|| {
                HostError::Execution("configured queued resource source limit".into())
            })?;
        let identity = work.fetch.identity();
        let key = (work.node, work.revision);
        let job = work
            .client
            .track_resource_job()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let sender = self.sender.clone();
        let lifetime = Arc::downgrade(&self.lifetime);
        let cancellation = work.cancellation.clone();
        work.source_budget = self.source_bytes.clone();
        // Register the same native owner cancellation used by existing parser
        // resource jobs. Source envelope retains body custody through delivery.
        self.reserve_stylesheet(key.0, key.1)?;
        self.stylesheet_identities.insert(key, identity);
        let held = Arc::new(std::sync::Mutex::new(None));
        self.local_stylesheets.insert(
            key,
            (
                standalone_stylesheet::Cancel::native(cancellation.clone()),
                held,
            ),
        );
        spawn_parser_resource(
            sender,
            lifetime,
            job,
            move || {
                // The existing transient source bound also covers the owned
                // request while it waits for the executor. Body reservations
                // take over before this request custody is released.
                let result = work.execute();
                drop(queued_source);
                Ok(result)
            },
            move |result| {
                let ready = match result {
                    Ok(ready) => ready,
                    Err(error) => RetainedStylesheetReady {
                        identity,
                        node: key.0,
                        revision: key.1,
                        cancellation,
                        result: Err(error),
                    },
                };
                ParserResourceCompletion::RetainedStylesheet(ready)
            },
        );
        Ok(())
    }
}

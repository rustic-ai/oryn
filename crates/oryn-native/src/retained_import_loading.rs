// Replacement for source-string import expansion. Included in the native
// executable module after parent response proof integration.
// No condition is serialized back into a synthetic @media/@layer wrapper.

struct RetainedStylesheetSource {
    final_url: String,
    original: std::sync::Arc<str>,
    graph: oryn_layout::cssom::native_import_graph::ImportGraph,
    responses: Vec<RetainedStylesheetResponse>,
    dependencies: CssDependencies,
    retained_allocation: Option<std::sync::Arc<dyn Send + Sync>>,
}
struct RetainedStylesheetResponse {
    sheet: Option<oryn_layout::cssom::native_import_graph::NativeSheet>,
    final_url: String,
    source: std::sync::Arc<str>,
    referrer_policy: String,
    encoding: String,
    proof: NativeStylesheetResponseAuthority,
    guard: Option<crate::stylesheet_response_proof::ResponseGuard>,
}

struct ImportWalk<'a> {
    client: &'a crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    initiator: &'a crate::context_service::ResourceInitiator,
    budget: &'a mut StylesheetExpansionBudget,
}

impl ImportWalk<'_> {
    fn fetch_outcome(
        &mut self,
        url: &str,
        parent: &RetainedStylesheetResponse,
    ) -> Result<StylesheetFetchOutcome, crate::RuntimeError> {
        // Retain parent RuntimeError before HostError string conversion so
        // worker limits/protocol failures never masquerade as web fetch errors.
        self.fetch_response_typed(url, parent)
            .map(StylesheetFetchOutcome::response)
            .or_else(classify_stylesheet_fetch_error)
    }

    fn validate_response(&self, response: &RetainedStylesheetResponse) -> Result<(), HostError> {
        self.budget.check()?;
        if !matches!(response.proof, NativeStylesheetResponseAuthority::Fetched(_)) {
            return Ok(()); // Exact inline owner is revalidated on owning realm.
        }
        let verified = response
            .guard
            .as_ref()
            .ok_or_else(|| {
                HostError::Execution("stylesheet response custody already released".into())
            })?
            .validate(&response.final_url, &response.source)
            .map_err(stylesheet_fetch_host_error)?;
        match verified {
            Some(clean) if clean == response.proof.origin_clean() => Ok(()),
            None => Err(HostError::CachedAuthorityExpired),
            _ => Err(HostError::Execution(
                "stylesheet response proof mismatch".into(),
            )),
        }
    }

    fn fetch_response_typed(
        &mut self,
        url: &str,
        parent: &RetainedStylesheetResponse,
    ) -> Result<RetainedStylesheetResponse, crate::RuntimeError> {
        self.fetch_response_options_typed(
            url,
            &parent.final_url,
            &parent.referrer_policy,
            &parent.encoding,
            true,
        )
    }

    fn fetch_root_response_typed(
        &mut self,
        url: &str,
        referrer: &str,
        policy: &str,
        encoding: &str,
    ) -> Result<RetainedStylesheetResponse, crate::RuntimeError> {
        self.fetch_response_options_typed(url, referrer, policy, encoding, false)
    }

    fn fetch_response_options_typed(
        &mut self,
        url: &str,
        referrer: &str,
        policy: &str,
        encoding: &str,
        imported: bool,
    ) -> Result<RetainedStylesheetResponse, crate::RuntimeError> {
        self.budget
            .request()
            .map_err(stylesheet_budget_runtime_error)?;
        let mut initiator = self.initiator.clone();
        // Import requests inherit policy and rendering context, never the
        // originating link's nonce, integrity or crossorigin attributes.
        if imported {
            initiator.stylesheet = Some(crate::context_service::StylesheetRequest {
                policy: self
                    .initiator
                    .stylesheet
                    .as_ref()
                    .and_then(|options| options.policy.clone()),
                render_blocking: self
                    .initiator
                    .stylesheet
                    .as_ref()
                    .is_some_and(|options| options.render_blocking),
                quirks: self
                    .initiator
                    .stylesheet
                    .as_ref()
                    .is_some_and(|options| options.quirks),
                ..Default::default()
            });
        }
        // Cleanup custody is admitted before fetching. It is bound to the
        // returned proof immediately, before any consumer validation can fail.
        let pending_response = crate::stylesheet_response_proof::PendingResponse::reserve(
            self.client.clone(),
            self.page_id,
            self.initiator.creator.clone(),
        )?;
        let fetched = self.budget.load(
            self.client,
            self.page_id,
            self.initiator.creator.clone(),
            crate::context_service::ContextServiceRequest::LoadPageResource {
                download: None,
                script_authority: None,
                initiator: Box::new(initiator),
                form_authority: None,
                parent_encoding: Some(encoding.into()),
                page_id: self.page_id,
                url: url.into(),
                destination: "stylesheet".into(),
                initiator_url: Some(referrer.into()),
                referrer_policy: Some(policy.into()),
                post: None,
            },
        )?;
        let crate::context_service::ContextServiceResponse::PageResource {
            url,
            source,
            referrer_policy,
            encoding,
            destination,
            stylesheet_response: Some(proof),
            ..
        } = fetched
        else {
            return Err(crate::RuntimeError::ProtocolViolation(
                "stylesheet response missing native proof".into(),
            ));
        };
        // Acquire the release guard before destination/hash checks: malformed
        // or rejected responses must not leave parent proof records rooted.
        let guard = pending_response.accept(proof.clone());
        if destination != "stylesheet" {
            return Err(crate::RuntimeError::ProtocolViolation(
                "stylesheet response destination mismatch".into(),
            ));
        }
        use sha2::{Digest, Sha256};
        let actual: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        if actual != proof.body_sha256 {
            return Err(crate::RuntimeError::ProtocolViolation(
                "stylesheet response body proof mismatch".into(),
            ));
        }
        // Establish release ownership before any consumer failure, including
        // budget denial and body verification errors.
        self.budget
            .retain(source.len())
            .map_err(stylesheet_budget_runtime_error)?;
        Ok(RetainedStylesheetResponse {
            sheet: None,
            final_url: url,
            source: std::sync::Arc::from(source),
            referrer_policy,
            encoding,
            proof: NativeStylesheetResponseAuthority::Fetched(proof),
            guard: Some(guard),
        })
    }
}

impl RetainedStylesheetSource {
    /// Revalidate every retained response at the owning publication boundary,
    /// not only when individual IO completed. This is separate from DOM owner
    /// validation and graph-issued occurrence identity validation.
    fn validate_responses(&self) -> Result<bool, HostError> {
        for response in &self.responses {
            if !matches!(response.proof, NativeStylesheetResponseAuthority::Fetched(_)) {
                continue;
            }
            let result = response
                .guard
                .as_ref()
                .ok_or_else(|| {
                    HostError::Execution("stylesheet publication custody already released".into())
                })?
                .validate(&response.final_url, &response.source)
                .map_err(stylesheet_fetch_host_error)?;
            match result {
                Some(clean) if clean == response.proof.origin_clean() => {}
                None => return Ok(false),
                _ => {
                    return Err(HostError::Execution(
                        "stylesheet publication response proof mismatch".into(),
                    ));
                }
            }
        }
        Ok(true)
    }
}

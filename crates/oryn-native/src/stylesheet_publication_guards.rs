// Temporary parent evidence custody. Loaded graph metadata does not retain
// a parent request lease after the final validated publication boundary.
struct StylesheetPublicationGuard {
    guard: crate::stylesheet_response_proof::ResponseGuard,
    url: String,
    source: std::sync::Arc<str>,
    origin_clean: bool,
}
struct StylesheetPublicationGuards {
    entries: Vec<StylesheetPublicationGuard>,
    _charge: std::sync::Arc<dyn Send + Sync>,
}
impl StylesheetPublicationGuards {
    fn take(
        responses: &mut [RetainedStylesheetResponse],
        admission: &dyn oryn_layout::cssom::native_import_graph::ImportAdmission,
    ) -> Result<Self, HostError> {
        let bytes = responses.iter().try_fold(
            responses
                .len()
                .checked_mul(
                    std::mem::size_of::<StylesheetPublicationGuard>()
                        + std::mem::size_of::<(String, std::sync::Arc<str>, bool)>(),
                )
                .ok_or_else(|| HostError::Execution("stylesheet proof metadata overflow".into()))?,
            |bytes, response| {
                bytes.checked_add(response.final_url.len()).ok_or_else(|| {
                    HostError::Execution("stylesheet proof URL metadata overflow".into())
                })
            },
        )?;
        let charge = admission.reserve(bytes).map_err(HostError::Execution)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(responses.len())
            .map_err(|error| HostError::Execution(error.to_string()))?;
        // Stage every fallible URL copy before moving any cleanup guard.
        for response in responses.iter() {
            if matches!(response.proof, NativeStylesheetResponseAuthority::Inline(_)) {
                continue;
            }
            let mut url = String::new();
            url.try_reserve_exact(response.final_url.len())
                .map_err(|error| HostError::Execution(error.to_string()))?;
            url.push_str(&response.final_url);
            entries.push((url, response.source.clone(), response.proof.origin_clean()));
        }
        let mut guards = Vec::new();
        guards
            .try_reserve_exact(responses.len())
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if responses.iter().any(|response| {
            matches!(
                response.proof,
                NativeStylesheetResponseAuthority::Fetched(_)
            ) && response.guard.is_none()
        }) {
            return Err(HostError::Execution(
                "stylesheet proof custody missing".into(),
            ));
        }
        for (response, (url, source, origin_clean)) in responses
            .iter_mut()
            .filter(|response| {
                matches!(
                    response.proof,
                    NativeStylesheetResponseAuthority::Fetched(_)
                )
            })
            .zip(entries)
        {
            let guard = response
                .guard
                .take()
                .expect("exclusive validated response custody");
            guards.push(StylesheetPublicationGuard {
                guard,
                url,
                source,
                origin_clean,
            });
        }
        Ok(Self {
            entries: guards,
            _charge: charge,
        })
    }
    fn validate(&self) -> Result<bool, HostError> {
        for entry in &self.entries {
            match entry
                .guard
                .validate(&entry.url, &entry.source)
                .map_err(stylesheet_fetch_host_error)?
            {
                Some(clean) if clean == entry.origin_clean => {}
                None => return Ok(false),
                Some(_) => {
                    return Err(HostError::Execution(
                        "stylesheet proof classification changed".into(),
                    ));
                }
            }
        }
        Ok(true)
    }
    fn cancel(&self) {
        for entry in &self.entries {
            entry.guard.cancel();
        }
    }
}

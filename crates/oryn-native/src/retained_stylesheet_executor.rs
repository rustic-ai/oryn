// Resource-executor work captures no main-thread DOM or Stylo handles.
#[cfg(unix)]
struct RetainedStylesheetFetchWork {
    client: crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    node: NodeId,
    revision: u64,
    fetch: ParserFetchContext,
    environment_encoding: String,
    source: RetainedStylesheetFetchSource,
    admission: SharedRetainedAdmission,
    cancellation: Arc<crate::context_service::ResourceCancellation>,
    source_budget: Arc<std::sync::atomic::AtomicUsize>,
}
#[cfg(unix)]
enum RetainedStylesheetFetchSource {
    External(String),
    Inline(String),
}

#[cfg(unix)]
impl RetainedStylesheetFetchWork {
    fn execute(self) -> RetainedStylesheetReady {
        let identity = self.fetch.identity();
        let result = (|| {
            self.admission.checkpoint().map_err(HostError::Execution)?;
            let initiator = self.fetch.stylesheet_initiator()?;
            let mut budget =
                StylesheetExpansionBudget::new(self.source_budget, self.cancellation.flag.clone())
                    .with_native(self.cancellation.clone());
            match self.source {
                RetainedStylesheetFetchSource::External(url) => {
                    let url = self.fetch.resolve(&url)?;
                    load_retained_parser_stylesheet(ParserStylesheetInput {
                        client: &self.client,
                        page_id: self.page_id,
                        initiator: &initiator,
                        requested_url: &url,
                        initiator_url: Some(self.fetch.initiator_url),
                        referrer_policy: Some(self.fetch.referrer_policy),
                        environment_encoding: &self.environment_encoding,
                        depth: 0,
                        budget: &mut budget,
                    })
                }
                RetainedStylesheetFetchSource::Inline(source) => {
                    let authority = NativeInlineStylesheetAuthority {
                        document: identity,
                        node: self.node,
                        source_revision: self.revision,
                        cancellation: self.cancellation.clone(),
                    };
                    load_retained_inline_stylesheet(
                        InlineCssResourceInput {
                            client: &self.client,
                            page_id: self.page_id,
                            initiator: &initiator,
                            document_url: &self.fetch.initiator_url,
                            base_url: &self.fetch.base_url,
                            referrer_policy: &self.fetch.referrer_policy,
                            css: &source,
                            environment_encoding: &self.environment_encoding,
                            budget: &mut budget,
                        },
                        authority,
                    )
                }
            }
        })()
        .map_err(RetainedStylesheetFailure::from);
        RetainedStylesheetReady {
            identity,
            node: self.node,
            revision: self.revision,
            cancellation: self.cancellation,
            result,
        }
    }
}

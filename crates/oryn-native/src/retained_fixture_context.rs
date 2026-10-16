#[cfg(all(test, unix))]
struct RetainedContextFixture {
    executable: RustExecutableDocument,
    _page: crate::PageHandle,
    _context: crate::BrowserContext,
}
#[cfg(all(test, unix))]
impl std::ops::Deref for RetainedContextFixture {
    type Target = RustExecutableDocument;
    fn deref(&self) -> &Self::Target { &self.executable }
}
#[cfg(all(test, unix))]
impl std::ops::DerefMut for RetainedContextFixture {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.executable }
}
#[cfg(all(test, unix))]
fn retained_fixture_stream(
    stream: crate::host::DocumentStream,
    url: &str,
    broker: SharedNetworkBroker,
) -> Result<RetainedContextFixture, HostError> {
    let context = crate::Browser::in_process_probe().new_context_with_broker(
        crate::ContextOptions::default(), broker);
    let handle = context.new_page();
    let id = handle.page_id();
    context.install_document_security(id, url.to_owned(), &[])
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let client = crate::context_service::SharedContextClient::direct(context.clone(), id);
    let executable = RustExecutableDocument::load_with_options(
        DocumentLoad::Scheduled(stream), &BTreeMap::new(), None,
        Some(Url::parse(url).map_err(|error| HostError::Execution(error.to_string()))?),
        256 * 1024 * 1024, Some((client, id, None)),
    )?;
    Ok(RetainedContextFixture { executable, _page: handle, _context: context })
}

#[cfg(all(test, unix))]
fn retained_fixture_stylesheet_stream(
    source: &str,
    mime: &str,
    url: &str,
    css: &str,
) -> Result<RetainedContextFixture, HostError> {
    struct Css(String);
    impl crate::network::NetworkBroker for Css {
        fn send(&self, request: crate::network::NetworkRequest) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
            Ok(crate::network::NetworkResponse {
                response_policy: None, timing: None, request_id: request.request_id,
                status: 200, headers: vec![("content-type".into(), "text/css".into())],
                body: self.0.as_bytes().to_vec(),
            })
        }
    }
    retained_fixture_stream(crate::host::DocumentStream {
        content_language: None,
        chunks: Box::new(crate::host::ReadyDocumentInput(vec![Ok(source.to_owned())].into_iter())),
        content_type: Some(mime.into()), encoding: Default::default(),
    }, url, SharedNetworkBroker::new(Css(css.into())))
}
#[cfg(all(test, not(unix)))]
fn retained_fixture_stream(
    stream: crate::host::DocumentStream,
    url: &str,
    broker: SharedNetworkBroker,
) -> Result<RustExecutableDocument, HostError> {
    RustExecutableDocument::load_stream_with_limits(
        stream, &BTreeMap::new(), Some(broker),
        Some(Url::parse(url).map_err(|error| HostError::Execution(error.to_string()))?),
        256 * 1024 * 1024, true,
    )
}

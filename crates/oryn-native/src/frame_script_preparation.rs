//! Executable-owned frame script preparation. Network and module graph waits
//! retain native document identity and leases instead of blocking the page actor.

/// Inputs retained together for written.
pub(super) struct WrittenFrameScriptInput<'a> {
    pub(super) load: PreparedDocumentLoad,
    pub(super) parent: RealmId,
    pub(super) frame: NodeId,
    pub(super) dom: SharedDomRuntime,
    pub(super) root: bool,
    pub(super) gate: Rc<oryn_js::v8_bindings::DocumentParserGate>,
    pub(super) limits: &'a DynamicScriptScheduler,
    pub(super) host: &'a mut RawV8Host,
}
/// Inputs retained together for prepare frame scripts.
pub(super) struct FrameScriptInput<'a> {
    pub(super) load: PreparedDocumentLoad,
    pub(super) parent: RealmId,
    pub(super) frame: NodeId,
    pub(super) dom: SharedDomRuntime,
    pub(super) records: Vec<ScriptRecord>,
    pub(super) lifecycle_allowed: bool,
    pub(super) _refresh_url: Option<String>,
    pub(super) limits: &'a DynamicScriptScheduler,
    pub(super) host: &'a mut RawV8Host,
}
use super::*;
use std::collections::VecDeque;

pub(super) fn streams_frame_document(content_type: Option<&str>) -> bool {
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    essence == "text/html"
        || oryn_dom::xml::is_text_mime(&essence)
        || matches!(essence.as_str(), "application/xml" | "text/xml")
        || essence.ends_with("+xml")
}
pub(super) fn frame_document_shell(
    source: &str,
    content_type: Option<&str>,
    url: &str,
) -> ParsedDocument {
    if !streams_frame_document(content_type) {
        return oryn_dom::xml::parse_navigation(source, content_type, url);
    }
    let runtime = DomRuntime::new();
    let (mut dom, document) = runtime.snapshot();
    let essence = content_type
        .unwrap_or("text/html")
        .split(';')
        .next()
        .unwrap_or("text/html")
        .trim()
        .to_ascii_lowercase();
    if let Some(metadata) = dom
        .get_mut(document)
        .ok()
        .and_then(|node| node.document_metadata.as_mut())
    {
        metadata.is_html = !(matches!(essence.as_str(), "application/xml" | "text/xml")
            || essence.ends_with("+xml"));
        metadata.content_type = essence;
    }
    ParsedDocument {
        dom,
        document,
        errors: Vec::new(),
        quirks_mode: html5ever::interface::QuirksMode::NoQuirks,
        events: Vec::new(),
    }
}

#[cfg(unix)]
struct RetainedParserStylesheet {
    identity: ParserDocumentIdentity,
    node: NodeId,
    revision: u64,
    result: Result<StylesheetSource, String>,
    published: Option<Option<bool>>,
}

#[cfg(not(unix))]
struct PortableScriptSource {
    receiver:
        std::sync::mpsc::Receiver<Result<std::sync::Arc<ParserLoadedSource>, ParserScriptFailure>>,
    ready: Option<Result<std::sync::Arc<ParserLoadedSource>, ParserScriptFailure>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
#[cfg(not(unix))]
impl Drop for PortableScriptSource {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}
#[cfg(not(unix))]
struct PortableScriptPermit(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[cfg(not(unix))]
impl Drop for PortableScriptPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
#[cfg(not(unix))]
impl PortableScriptSource {
    fn ready(&mut self) -> bool {
        if self.ready.is_none() {
            match self.receiver.try_recv() {
                Ok(result) => self.ready = Some(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.ready = Some(Err(ParserScriptFailure::Infrastructure(
                        "portable script producer disconnected".into(),
                    )))
                }
            }
        }
        self.ready.is_some()
    }
    fn start(
        page: &mut RustExecutableDocument,
        record: &ScriptRecord,
    ) -> Result<Option<Self>, HostError> {
        let slots = page.dynamic_scripts.graph_slots.clone();
        if slots
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |used| (used < 8).then_some(used + 1),
            )
            .is_err()
        {
            return Ok(None);
        }
        let permit = PortableScriptPermit(slots);
        let host = &mut page.agent.host;
        let backend = host
            .module_fetch_backend()?
            .with_local_script_policy(record.8.local_script_policy.as_ref())?;
        let base = record
            .8
            .base_url
            .clone()
            .unwrap_or(host.document_base_url()?);
        let initiator_url = host.request_initiator_url()?;
        let requested = record.1.clone().expect("external classic preparation");
        let url = backend.resolve_html_request(&base, &requested, record.8.url_encoding())?;
        let request_id = if matches!(
            &backend,
            module_graph_load::ModuleGraphBackend::Broker { .. }
        ) {
            Some(host.reserve_module_request_id()?)
        } else {
            None
        };
        let policy = record
            .8
            .referrer_policy
            .clone()
            .unwrap_or(host.document_referrer_policy()?);
        let integrity = record.8.integrity.clone();
        let crossorigin = record.8.cross_origin.clone();
        let nonce = record.8.nonce.clone();
        let encoding = record.8.fallback_encoding.clone();
        let blocking = record.8.render_blocking;
        let budget = page.dynamic_scripts.source_bytes.clone();
        let size = requested
            .len()
            .saturating_add(url.len())
            .saturating_add(initiator_url.len())
            .saturating_add(policy.len())
            .saturating_add(integrity.as_ref().map_or(0, String::len))
            .saturating_add(crossorigin.as_ref().map_or(0, String::len))
            .saturating_add(nonce.as_ref().map_or(0, String::len));
        let metadata = ScriptSourceReservation::reserve(&budget, size).ok_or_else(|| {
            HostError::LimitExceeded {
                resource: "portable parser request bytes".into(),
            }
        })?;
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let admitted = resource_io_executor::submit(move || {
            let _permit = permit;
            let _metadata = metadata;
            let result = backend
                .fetch_classic(crate::executable::module_graph_load::ClassicFetchInput {
                    requested: &requested,
                    initiator_url: &initiator_url,
                    url,
                    integrity,
                    cross_origin: crossorigin,
                    policy,
                    nonce,
                    parser_inserted: true,
                    render_blocking: blocking,
                    fallback_encoding: encoding.as_deref(),
                    request_id,
                    cancel: &worker_cancel,
                })
                .map_err(|error| match error {
                    HostError::ScriptLoad(failure) => ParserScriptFailure::native(failure, &budget),
                    HostError::LimitExceeded { .. } => ParserScriptFailure::Capacity,
                    error => ParserScriptFailure::Infrastructure(error.to_string()),
                });
            let result = retain_parser_script_source(result, &budget);
            if !worker_cancel.load(std::sync::atomic::Ordering::Acquire) {
                let _ = sender.send(result);
            }
        });
        let ready = admitted
            .err()
            .map(|error| Err(ParserScriptFailure::Infrastructure(error)));
        Ok(Some(Self {
            receiver,
            ready,
            cancel,
        }))
    }
}

pub(super) struct FrameScriptJob {
    execution: ParserScriptProgress,
    _inline_reservation: Option<ScriptSourceReservation>,
    execution_finished: bool,
    #[cfg(not(unix))]
    portable_source: Option<PortableScriptSource>,
    #[cfg(unix)]
    source_completion: Option<ParserResourceCompletion>,
    record: ScriptRecord,
    _lease: ParserScriptLease,
    #[cfg(unix)]
    source: Option<ParserResourceScheduler>,
    graph: Option<module_graph_load::ModuleGraphLoad>,
}

struct PendingParserEvent {
    event: html::ParserEvent,
    checkpoint: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParserCompletionKind {
    Html,
    Xml,
    Snapshot,
}
struct XmlCompletion {
    kind: ParserCompletionKind,
    document: Option<ParsedDocument>,
    errors: Option<Vec<String>>,
    stage: u8,
    receipt: Option<u64>,
}
pub(super) struct FrameScriptPreparation {
    pub(super) restored_form_state: Option<Vec<oryn_js::form_state::SavedFormControl>>,
    form_restore_admitted: bool,
    pub(super) load: PreparedDocumentLoad,
    root_document: bool,
    parent: RealmId,
    frame: NodeId,
    dom: SharedDomRuntime,
    _document: ParserScriptLease,
    ordered: VecDeque<FrameScriptJob>,
    asynchronous: VecDeque<FrameScriptJob>,
    pub(super) lifecycle_allowed: bool,
    pub(super) lifecycle_queued: bool,
    lifecycle_active: bool,
    #[cfg(unix)]
    styles: Option<ParserResourceScheduler>,
    #[cfg(unix)]
    pending_stylesheet: Option<RetainedParserStylesheet>,
    #[cfg(not(unix))]
    portable_styles: BTreeSet<(NodeId, u64)>,
    #[cfg(unix)]
    style_nodes: VecDeque<(NodeId, u64)>,
    #[cfg(unix)]
    style_seen: BTreeSet<(NodeId, u64)>,
    #[cfg(unix)]
    style_leases: Vec<ParserScriptLease>,
    _source_reservation: ScriptSourceReservation,
    snapshot_events: Option<VecDeque<(NodeId, Option<html::ParserEvent>)>>,
    live_parser: Option<LiveParserState>,
    encoding_replay: Option<String>,
    xml_parser: Option<oryn_dom::xml::XmlDocumentParser>,
    parser_events: VecDeque<PendingParserEvent>,
    xml_completion: Option<XmlCompletion>,
    xslt_processed: bool,
    xslt_result: Option<ParsedDocument>,
    pub(super) parser_retry: Option<std::time::Instant>,
    xml_input: Option<crate::host::DocumentStream>,
    xml_input_limit: usize,
    xml_input_error: Option<HostError>,
    xml_deferred: VecDeque<FrameScriptJob>,
    parser_waiting: bool,
    parser_failed: bool,
    html_publication_pending: bool,
    html_input_error: Option<HostError>,
    html_eof: bool,
    html_bytes_remaining: usize,
    _input_reservation: Option<ScriptSourceReservation>,
    #[cfg(unix)]
    parser_async: Option<PendingParserAsyncModules>,
    child_loads: Vec<PreparedDocumentLoad>,
}

struct WrittenDocumentInput(Rc<oryn_js::v8_bindings::DocumentParserGate>);
impl crate::host::DocumentInput for WrittenDocumentInput {
    fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
        if self.0.closed() {
            std::task::Poll::Ready(None)
        } else {
            std::task::Poll::Pending
        }
    }
}

impl FrameScriptPreparation {
    pub(super) fn written(params: WrittenFrameScriptInput<'_>) -> Result<Self, HostError> {
        let WrittenFrameScriptInput {
            load,
            parent,
            frame,
            dom,
            root,
            gate,
            limits,
            host,
        } = params;

        let mut owner = Self::new(FrameScriptInput {
            load,
            parent,
            frame,
            dom: dom.clone(),
            records: vec![],
            lifecycle_allowed: true,
            _refresh_url: None,
            limits,
            host,
        })?;
        owner.root_document = root;
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(WrittenDocumentInput(gate.clone())),
            content_type: Some("text/html".into()),
            encoding: Default::default(),
        };
        let mut parser = LiveParserState::new(stream, &dom, host)?;
        parser.parser = gate.clone();
        host.install_document_parser_gate(gate)?;
        #[cfg(unix)]
        {
            parser.resource_scheduler = ParserResourceScheduler::with_page_limits(limits);
        }
        owner.live_parser = Some(parser);
        Ok(owner)
    }
    pub(super) fn new(params: FrameScriptInput<'_>) -> Result<Self, HostError> {
        let FrameScriptInput {
            load,
            parent,
            frame,
            dom,
            records,
            lifecycle_allowed,
            _refresh_url,
            limits,
            host,
        } = params;

        host.set_dynamic_import_limits(
            load.realm,
            limits.source_bytes.clone(),
            limits.graph_slots.clone(),
            limits.graph_fetches.clone(),
        )?;
        if records.len() > 1024 {
            return Err(HostError::LimitExceeded {
                resource: "frame script preparation records".into(),
            });
        }
        let bytes = records
            .iter()
            .try_fold(0usize, |bytes, record| {
                bytes
                    .checked_add(record.3.len())
                    .and_then(|bytes| bytes.checked_add(record.1.as_ref().map_or(0, String::len)))
            })
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "frame script preparation bytes".into(),
            })?;
        let reservation = ScriptSourceReservation::reserve(&limits.source_bytes, bytes)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "frame script preparation bytes".into(),
            })?;
        let mut ordered = VecDeque::new();
        let mut deferred = VecDeque::new();
        let asynchronous = VecDeque::new();
        for record in records {
            // Preparation owns these records before yielding to author tasks.
            // Reinsertion must not enqueue a second dynamic execution.
            if (record.1.is_some() || !record.3.is_empty())
                && record.2.as_deref().is_none_or(|kind| {
                    matches!(kind, "module" | "importmap" | "speculationrules")
                        || html::is_javascript_mime_type(kind)
                })
            {
                host.mark_script_started(script_record_node(&record))?;
            }
            let job = FrameScriptJob {
                _inline_reservation: None,
                execution: ParserScriptProgress {
                    retained: true,
                    ..Default::default()
                },
                execution_finished: false,
                #[cfg(not(unix))]
                portable_source: None,
                #[cfg(unix)]
                source_completion: None,
                _lease: ParserScriptLease::new(&dom, script_record_node(&record))?,
                record,
                #[cfg(unix)]
                source: None,
                graph: None,
            };
            if job.record.6 {
                deferred.push_back(job);
            } else {
                ordered.push_back(job);
            }
        }
        ordered.append(&mut deferred);
        let document_lease = ParserScriptLease::new(&dom, load.identity.0)?;
        let mut pending = Self {
            restored_form_state: None,
            form_restore_admitted: false,
            load,
            root_document: false,
            parent,
            frame,
            dom,
            _document: document_lease,
            ordered,
            asynchronous,
            lifecycle_allowed,
            lifecycle_queued: false,
            lifecycle_active: false,
            _source_reservation: reservation,
            snapshot_events: None,
            live_parser: None,
            encoding_replay: None,
            xml_parser: None,
            parser_events: VecDeque::new(),
            xml_completion: None,
            xslt_processed: false,
            xslt_result: None,
            parser_retry: None,
            xml_input: None,
            xml_input_limit: MAX_QUEUED_SCRIPT_SOURCE_BYTES,
            xml_input_error: None,
            xml_deferred: VecDeque::new(),
            parser_waiting: false,
            parser_failed: false,
            html_publication_pending: false,
            html_input_error: None,
            html_eof: false,
            html_bytes_remaining: 0,
            _input_reservation: None,
            #[cfg(unix)]
            parser_async: None,
            child_loads: Vec::new(),
            #[cfg(not(unix))]
            portable_styles: BTreeSet::new(),
            #[cfg(unix)]
            styles: None,
            #[cfg(unix)]
            pending_stylesheet: None,
            #[cfg(unix)]
            style_nodes: VecDeque::new(),
            #[cfg(unix)]
            style_seen: BTreeSet::new(),
            #[cfg(unix)]
            style_leases: Vec::new(),
        };
        #[cfg(unix)]
        {
            let nodes = subtree_nodes(&pending.dom.borrow(), load.identity.0)?;
            for node in nodes {
                let owner = {
                    let dom = pending.dom.borrow();
                    dom.node_document(node).ok().flatten() == Some(load.identity.0)
                        && dom.arena().is_css_stylesheet_owner(node).unwrap_or(false)
                };
                if owner {
                    pending.queue_style(node, false)?;
                }
            }
        }
        Ok(pending)
    }

    pub(super) fn snapshot(
        load: PreparedDocumentLoad,
        parent: RealmId,
        frame: NodeId,
        dom: SharedDomRuntime,
        root: bool,
        limits: &DynamicScriptScheduler,
        host: &mut RawV8Host,
    ) -> Result<Self, HostError> {
        let mut events = VecDeque::new();
        let mut styled = Vec::new();
        let mut bytes = 0usize;
        {
            let dom = dom.borrow();
            let mut stack = vec![load.identity.0];
            while let Some(node) = stack.pop() {
                if dom.node_document(node).ok().flatten() != Some(load.identity.0) {
                    continue;
                }
                let record = dom
                    .arena()
                    .get(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                stack.extend(record.children.iter().rev().copied());
                let owner = dom.arena().is_css_stylesheet_owner(node).unwrap_or(false);
                if owner || record.attributes.contains_null("style") {
                    styled.push(node);
                    events.push_back((node, None));
                }
                if owner {
                    if let Some(sheet) = dom.arena().xml_stylesheet(node).ok().flatten() {
                        events.push_back((
                            node,
                            Some(html::ParserEvent::StylesheetDiscovered {
                                node,
                                href: {
                                    bytes = bytes.saturating_add(sheet.href.len());
                                    sheet.href
                                },
                            }),
                        ));
                    } else if matches!(&record.kind,NodeKind::Element{name}if name.local.as_ref()=="style")
                    {
                        events
                            .push_back((node, Some(html::ParserEvent::InlineStyleReady { node })));
                    } else if let Some(href) = record.attributes.get_null("href") {
                        events.push_back((
                            node,
                            Some(html::ParserEvent::StylesheetDiscovered {
                                node,
                                href: {
                                    bytes = bytes.saturating_add(href.len());
                                    href.clone()
                                },
                            }),
                        ));
                    }
                }
                if dom.is_element(node, "http://www.w3.org/1999/xhtml", "link")
                    && record.attributes.get_null("rel").is_some_and(|rel| {
                        rel.split_ascii_whitespace().any(|token| {
                            token.eq_ignore_ascii_case("modulepreload")
                                || token.eq_ignore_ascii_case("preload")
                        })
                    })
                {
                    events.push_back((
                        node,
                        Some(html::ParserEvent::ModulePreloadDiscovered { node }),
                    ));
                }
                if is_script_element(&dom, node) {
                    events.push_back((
                        node,
                        Some(html::ParserEvent::ScriptReady {
                            node,
                            scheduling: html::ScriptScheduling::Blocking,
                        }),
                    ));
                }
                if dom.is_element(node, "http://www.w3.org/1999/xhtml", "meta")
                    && let Some(content) = record.attributes.get_null("content")
                {
                    if record
                        .attributes
                        .get_null("http-equiv")
                        .is_some_and(|name| name.eq_ignore_ascii_case("content-security-policy"))
                        && dom.is_in_document_head(node).unwrap_or(false)
                    {
                        bytes = bytes.saturating_add(content.len());
                        events.push_back((
                            node,
                            Some(html::ParserEvent::MetaSecurity {
                                content: content.clone(),
                            }),
                        ));
                    }
                    if record
                        .attributes
                        .get_null("name")
                        .is_some_and(|name| name.eq_ignore_ascii_case("referrer"))
                    {
                        bytes = bytes.saturating_add(content.len());
                        events.push_back((
                            node,
                            Some(html::ParserEvent::MetaReferrer {
                                content: content.clone(),
                            }),
                        ));
                    }
                }
                if let Some(kind) = dom.embedded_content_kind(node) {
                    let event =
                        if kind.supports_srcdoc() && record.attributes.contains_null("srcdoc") {
                            Some(html::ParserEvent::FrameLocalDocument { node })
                        } else if let Some(source) = record
                            .attributes
                            .get_null(kind.source_attribute())
                            .filter(|source| kind.loads_source(Some(source.as_str())))
                        {
                            Some(html::ParserEvent::FrameDiscovered {
                                node,
                                source: {
                                    bytes = bytes.saturating_add(source.len());
                                    source.clone()
                                },
                            })
                        } else {
                            kind.creates_initial_document()
                                .then_some(html::ParserEvent::FrameLocalDocument { node })
                        };
                    if let Some(event) = event {
                        events.push_back((node, Some(event)));
                    }
                }
                if events.len() > 8192 || bytes > MAX_QUEUED_SCRIPT_SOURCE_BYTES {
                    return Err(HostError::LimitExceeded {
                        resource: "snapshot parser event custody".into(),
                    });
                }
            }
        }
        host.stage_snapshot_inline_styles(&styled)?;
        let mut owner = Self::new(FrameScriptInput {
            load,
            parent,
            frame,
            dom,
            records: Vec::new(),
            lifecycle_allowed: true,
            _refresh_url: None,
            limits,
            host,
        })?;
        owner.root_document = root;
        #[cfg(unix)]
        {
            owner.style_nodes.clear();
            owner.style_seen.clear();
            owner.style_leases.clear();
        }
        owner._source_reservation = ScriptSourceReservation::reserve(&limits.source_bytes, bytes)
            .ok_or_else(|| HostError::LimitExceeded {
            resource: "snapshot parser metadata bytes".into(),
        })?;
        owner.snapshot_events = Some(events);
        host.set_parser_input_active(true)?;
        host.set_document_readiness(oryn_dom::DocumentReadyState::Loading)?;
        Ok(owner)
    }

    pub(super) fn root_xml(
        load: PreparedDocumentLoad,
        dom: SharedDomRuntime,
        parser: oryn_dom::xml::XmlDocumentParser,
        bytes: usize,
        limits: &DynamicScriptScheduler,
        host: &mut RawV8Host,
    ) -> Result<Self, HostError> {
        let mut owner = Self::new(FrameScriptInput {
            load,
            parent: load.realm,
            frame: load.identity.0,
            dom,
            records: Vec::new(),
            lifecycle_allowed: true,
            _refresh_url: None,
            limits,
            host,
        })?;
        owner.root_document = true;
        owner.xml_parser = Some(parser);
        owner._input_reservation = Some(
            ScriptSourceReservation::reserve(&limits.source_bytes, bytes).ok_or_else(|| {
                HostError::LimitExceeded {
                    resource: "root XML input bytes".into(),
                }
            })?,
        );
        host.set_parser_input_active(true)?;
        Ok(owner)
    }
    pub(super) fn set_xml_input(
        &mut self,
        input: Option<crate::host::DocumentStream>,
        limit: usize,
    ) {
        self.xml_input = input;
        self.xml_input_limit = limit;
    }
    fn poll_xml_input(&mut self) {
        if !self
            .xml_parser
            .as_ref()
            .is_some_and(|parser| parser.needs_input())
        {
            return;
        }
        let Some(input) = self.xml_input.as_mut() else {
            return;
        };
        match input.chunks.poll_next() {
            std::task::Poll::Pending => {}
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                let used = self
                    ._input_reservation
                    .as_ref()
                    .map_or(0, |lease| lease.bytes);
                let Some(total) = used
                    .checked_add(chunk.len())
                    .filter(|bytes| *bytes <= self.xml_input_limit)
                else {
                    self.xml_input_error = Some(HostError::LimitExceeded {
                        resource: "streamed XML input bytes".into(),
                    });
                    self.xml_input = None;
                    return;
                };
                let budget = self._source_reservation.used.clone();
                let Some(mut extra) = ScriptSourceReservation::reserve(&budget, chunk.len()) else {
                    self.xml_input_error = Some(HostError::LimitExceeded {
                        resource: "streamed XML input custody".into(),
                    });
                    self.xml_input = None;
                    return;
                };
                if let Some(lease) = self._input_reservation.as_mut() {
                    lease.bytes = total;
                    extra.bytes = 0;
                } else {
                    self._input_reservation = Some(extra);
                }
                if let Some(encoding) = *input.encoding.lock().expect("XML input encoding")
                    && let Ok(node) = self
                        .dom
                        .borrow_mut()
                        .arena_mut()
                        .get_mut(self.load.identity.0)
                    && let Some(metadata) = node.document_metadata.as_mut()
                {
                    metadata.encoding = encoding.into();
                }
                self.xml_parser.as_mut().unwrap().push_decoded(chunk);
            }
            std::task::Poll::Ready(Some(Err(error))) => {
                self.xml_input_error = Some(error);
                self.xml_input = None;
            }
            std::task::Poll::Ready(None) => {
                self.xml_input = None;
                self.xml_parser.as_mut().unwrap().finish_input();
            }
        }
    }
    pub(super) fn root_parser_pending(&self) -> bool {
        self.root_document && !self.lifecycle_queued
    }
    pub(super) fn construction_progress(&self) -> String {
        // Structural counters only: timeout diagnostics must not disclose
        // response bodies, source text, attributes, or author data.
        let buffered = self.live_parser.as_ref().map_or(0, |parser| {
            parser.input.iter().map(|input| input.len()).sum::<usize>()
        });
        let nodes = self.dom.borrow().arena().iter().count();
        format!(
            "nodes={nodes},html={},xml={},buffered_bytes={buffered},events={},ordered={},deferred={},async={},eof={},waiting={},completion={},lifecycle={}",
            self.live_parser.is_some(),
            self.xml_parser.is_some(),
            self.parser_events.len(),
            self.ordered.len(),
            self.xml_deferred.len(),
            self.asynchronous.len(),
            self.html_eof,
            self.parser_waiting,
            self.xml_completion.is_some(),
            self.lifecycle_queued
        )
    }
    pub(super) fn retrying(&self, host: &RawV8Host) -> bool {
        self.live(host) && !host.realm_is_parked(self.load.realm) && self.parser_retry.is_some()
    }
    pub(super) fn has_async_work(&self, host: &RawV8Host) -> bool {
        if !self.live(host) {
            return false;
        }
        if self
            .asynchronous
            .iter()
            .any(|job| parser_script_owner_live(host, &self.dom, &job.record))
        {
            return true;
        }
        #[cfg(unix)]
        if let Some(pending) = &self.parser_async {
            return pending
                .scheduler
                .live_script_records()
                .any(|record| parser_script_owner_live(host, &self.dom, record));
        }
        false
    }
    #[cfg(all(test, unix))]
    pub(super) fn inject_async_source(
        &mut self,
        record: ScriptRecord,
        limits: &DynamicScriptScheduler,
    ) -> (
        std::sync::mpsc::Sender<ParserResourceCompletion<u64>>,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        u64,
    ) {
        let mut source = ParserResourceScheduler::with_page_limits(limits);
        source.pending = 1;
        let ticket = source.retain_preparation(record.clone()).unwrap();
        let sender = source.sender.clone();
        let budget = source.source_bytes.clone();
        self.asynchronous.push_back(FrameScriptJob {
            _inline_reservation: None,
            execution: ParserScriptProgress {
                retained: true,
                ..Default::default()
            },
            execution_finished: false,
            #[cfg(not(unix))]
            portable_source: None,
            source_completion: None,
            _lease: ParserScriptLease::new(&self.dom, script_record_node(&record)).unwrap(),
            record,
            source: Some(source),
            graph: None,
        });
        (sender, budget, ticket)
    }
    #[cfg(all(test, unix))]
    pub(super) fn parser_resource_scheduler(&self) -> Option<&ParserResourceScheduler> {
        self.live_parser
            .as_ref()
            .map(|parser| &parser.resource_scheduler)
    }
    pub(super) fn waiting_input(&self) -> bool {
        self.parser_waiting
            && self.ordered.is_empty()
            && self.asynchronous.is_empty()
            && self.parser_events.is_empty()
    }

    fn live(&self, host: &RawV8Host) -> bool {
        !self.parser_failed
            && host.automation_realm_identity(self.load.realm) == Some(self.load.identity)
            && !host.realm_document_load_stopped(self.load.realm)
            && (self.root_document
                || host.automation_frame_child_live(self.parent, self.frame, self.load.realm))
    }

    #[cfg(test)]
    pub(super) fn root_or_live_for_test(&self, host: &RawV8Host) -> bool {
        self.root_document || self.live(host)
    }

    fn current_styles_block_scripts(&self, host: &mut RawV8Host) -> Result<bool, HostError> {
        // Actual parser barrier; polling helpers below consume cached viewport.
        host.native_media_matches_in_realm(self.load.realm, "all")?;

        #[cfg(test)]
        xml_publication_failure(XmlPublicationFailure::BeforeStylesBarrier)?;
        #[cfg(unix)]
        {
            self.styles_block_scripts(host)
        }
        #[cfg(not(unix))]
        {
            self.portable_styles_block_scripts(host)
        }
    }
    fn retained_resources(&self) -> bool {
        #[cfg(unix)]
        {
            self.pending_stylesheet.is_some()
                || !self.style_nodes.is_empty()
                || self
                    .styles
                    .as_ref()
                    .is_some_and(ParserResourceScheduler::has_stylesheet_owner_work)
                || self.parser_async.is_some()
        }
        #[cfg(not(unix))]
        {
            !self.portable_styles.is_empty()
        }
    }
    pub(super) fn awaiting_author_input(&self) -> bool {
        self.parser_waiting
            && !self.lifecycle_active
            && !self.retained_resources()
            && self.ordered.is_empty()
            && self.asynchronous.is_empty()
            && !self
                .xml_deferred
                .iter()
                .any(|job| job.preparation_pending())
            && self.parser_events.is_empty()
            && !self.html_publication_pending
            && self.xml_completion.is_none()
            && self.live_parser.as_ref().is_some_and(|parser| {
                parser.parser.owned()
                    && !parser.parser.closed()
                    && !parser.parser.has_input()
                    && !parser.parser.pending_events()
                    && parser.input.is_empty()
            })
    }
    pub(super) fn construction_pending(&self, host: &RawV8Host) -> bool {
        self.live(host) && !self.awaiting_author_input()
    }
    fn retains_work(&self) -> bool {
        self.live_parser.is_some()
            || self.xml_parser.is_some()
            || self.xml_completion.is_some()
            || !self.parser_events.is_empty()
            || self.snapshot_events.is_some()
            || !self.ordered.is_empty()
            || !self.asynchronous.is_empty()
            || !self.lifecycle_queued
            || self.lifecycle_active
            || self.retained_resources()
    }
    // Fetch and prepare deferred jobs without granting ordered execution. XML
    // and streaming HTML share xml_deferred; snapshot jobs use ordered instead.
    // Never cross an earlier blocking script/import map in a snapshot batch.
    fn poll_deferred_preparations(
        &mut self,
        page: &mut RustExecutableDocument,
    ) -> Result<(), HostError> {
        let mut budget = 64;
        for before_eof in [false, true] {
            let mut index = 0;
            loop {
                if budget == 0 || !self.live(&page.agent.host) {
                    return Ok(());
                }
                let queue = if before_eof {
                    &mut self.xml_deferred
                } else {
                    &mut self.ordered
                };
                let Some(job) = queue.get_mut(index) else {
                    break;
                };
                if !job.record.6 {
                    break;
                }
                // A begun completion publication belongs to the normal ordered
                // retry path. Ready sources likewise wait for their queue turn.
                if job.execution.entered
                    || !job.preparation_pending()
                    || !job.runnable(&page.agent.host, &self.dom)
                {
                    index += 1;
                    continue;
                }
                page.agent.host.activate_realm(self.load.realm)?;
                budget -= 1;
                if job.poll(page, &self.dom, false, None)? {
                    // Terminal fetch errors may publish their normal error event.
                    // Remove only that completed job, without rotating siblings.
                    queue.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        Ok(())
    }

    fn runnable(&mut self, host: &mut RawV8Host) -> bool {
        if !self.live(host) {
            return true;
        }
        if host.realm_is_parked(self.load.realm) {
            return false;
        }
        if self.lifecycle_active {
            return false;
        }
        if !self.retains_work() {
            return true;
        }
        if self
            .parser_retry
            .is_some_and(|retry| retry > std::time::Instant::now())
        {
            return false;
        }
        if self.html_publication_pending {
            return true;
        }
        if !self.parser_events.is_empty() || self.xml_completion.is_some() {
            return true;
        }
        if host.activate_realm(self.load.realm).is_err() {
            return true;
        }
        if self.snapshot_events.is_some() && self.ordered.is_empty() {
            return true;
        }
        if self.xml_parser.is_some() && self.ordered.is_empty() {
            self.poll_xml_input();
            if self.xml_input_error.is_some()
                || self
                    .xml_parser
                    .as_ref()
                    .is_some_and(|parser| !parser.needs_input())
            {
                return true;
            }
        }
        if self.ordered.is_empty()
            && let Some(parser) = &mut self.live_parser
        {
            if self.html_input_error.is_some() || self.html_eof || !parser.input.is_empty() {
                return true;
            }
            if parser.parser.pending_events() || parser.parser.has_input() || !self.parser_waiting {
                return true;
            }
            #[cfg(unix)]
            if parser.resource_scheduler.has_ready_completion()
                || parser
                    .resource_scheduler
                    .async_modules
                    .iter()
                    .any(|(_, graph)| graph.is_runnable())
            {
                return true;
            }
        }
        if self.parser_waiting
            && self.ordered.is_empty()
            && let Some(parser) = &mut self.live_parser
        {
            match parser.source.poll_next() {
                std::task::Poll::Pending => {}
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    parser
                        .input
                        .push_back(oryn_dom::html::ParserInput::unicode(&chunk));
                    self.parser_waiting = false;
                    return true;
                }
                std::task::Poll::Ready(Some(Err(error))) => {
                    self.html_input_error = Some(error);
                    return true;
                }
                std::task::Poll::Ready(None) => {
                    self.html_eof = true;
                    return true;
                }
            }
        }
        #[cfg(unix)]
        if let Some(pending) = &mut self.parser_async
            && (pending.scheduler.has_ready_completion()
                || pending
                    .scheduler
                    .async_modules
                    .iter()
                    .any(|(_, graph)| graph.is_runnable()))
        {
            return true;
        }
        #[cfg(not(unix))]
        if !self.portable_styles.is_empty() {
            return true;
        }
        #[cfg(unix)]
        if self.pending_stylesheet.is_some()
            || !self.style_nodes.is_empty()
            || self
                .styles
                .as_mut()
                .is_some_and(|styles| styles.has_ready_completion())
        {
            return true;
        }
        if self.live_parser.is_none()
            && self.xml_parser.is_none()
            && self.xml_completion.is_none()
            && self.parser_events.is_empty()
            && self.snapshot_events.is_none()
            && self.ordered.is_empty()
            && self.lifecycle_allowed
            && !self.lifecycle_queued
        {
            return true;
        }
        #[cfg(unix)]
        let blocked = self.styles_block_scripts(host).unwrap_or(false);
        #[cfg(not(unix))]
        let blocked = self.portable_styles_block_scripts(host).unwrap_or(false);
        self.ordered
            .iter_mut()
            .take_while(|job| job.record.6)
            .chain(self.xml_deferred.iter_mut())
            .any(|job| {
                !job.execution.entered && job.preparation_pending() && job.runnable(host, &self.dom)
            })
            || self.ordered.front_mut().is_some_and(|job| {
                (!blocked || (!job.record.4 && !job.record.6) || job.preparation_pending())
                    && job.runnable(host, &self.dom)
            })
            || self
                .asynchronous
                .iter_mut()
                .any(|job| job.runnable(host, &self.dom))
    }
}

impl FrameScriptPreparation {
    pub(super) fn start_document_stream(
        &mut self,
        stream: crate::host::DocumentStream,
        host: &mut RawV8Host,
        limits: &DynamicScriptScheduler,
    ) -> Result<(), HostError> {
        let essence = stream
            .content_type
            .as_deref()
            .unwrap_or("text/html")
            .split(';')
            .next()
            .unwrap_or("text/html")
            .trim()
            .to_ascii_lowercase();
        if matches!(essence.as_str(), "application/xml" | "text/xml") || essence.ends_with("+xml") {
            self.xml_parser = Some(oryn_dom::xml::XmlDocumentParser::streaming_with_document(
                &essence,
                self.dom.clone(),
                self.load.identity.0,
            ));
            self.set_xml_input(Some(stream), 16 * 1024 * 1024);
            host.set_parser_input_active(true)?;
        } else {
            let mut parser = LiveParserState::new(stream, &self.dom, host)?;
            #[cfg(unix)]
            {
                parser.resource_scheduler = ParserResourceScheduler::with_page_limits(limits);
            }
            self.live_parser = Some(parser);
        }
        Ok(())
    }
    pub(super) fn start_document_bytes(
        &mut self,
        encoded: String,
        content_type: Option<String>,
        fallback: Option<&str>,
        host: &mut RawV8Host,
        limits: &DynamicScriptScheduler,
    ) -> Result<(), HostError> {
        if encoded.len() > (16usize * 1024 * 1024).div_ceil(3) * 4 {
            return Err(HostError::LimitExceeded {
                resource: "buffered frame raw bytes".into(),
            });
        }
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &encoded)
            .map_err(|_| HostError::Execution("invalid buffered frame bytes".into()))?;
        self._input_reservation = Some(
            ScriptSourceReservation::reserve(&limits.source_bytes, bytes.len()).ok_or_else(
                || HostError::LimitExceeded {
                    resource: "frame parser raw bytes".into(),
                },
            )?,
        );
        let stream = crate::html_encoding_replay::BufferedDecodedInput::stream(
            bytes,
            content_type,
            fallback,
            None,
        );
        self.start_document_stream(stream, host, limits)
    }
    pub(super) fn start_document_source(
        &mut self,
        source: String,
        content_type: Option<String>,
        host: &mut RawV8Host,
        limits: &DynamicScriptScheduler,
    ) -> Result<(), HostError> {
        self._input_reservation = Some(
            ScriptSourceReservation::reserve(&limits.source_bytes, source.len()).ok_or_else(
                || HostError::LimitExceeded {
                    resource: "frame parser input bytes".into(),
                },
            )?,
        );
        let essence = content_type
            .as_deref()
            .unwrap_or("text/html")
            .split(';')
            .next()
            .unwrap_or("text/html")
            .trim()
            .to_ascii_lowercase();
        if matches!(essence.as_str(), "application/xml" | "text/xml") || essence.ends_with("+xml") {
            self.xml_parser = Some(oryn_dom::xml::XmlDocumentParser::with_document(
                source,
                &essence,
                self.dom.clone(),
                self.load.identity.0,
            ));
            host.set_parser_input_active(true)?;
        } else {
            let mut stream =
                crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(source))));
            stream.content_type = content_type;
            let mut parser = LiveParserState::new(stream, &self.dom, host)?;
            #[cfg(unix)]
            {
                parser.resource_scheduler = ParserResourceScheduler::with_page_limits(limits);
            }
            self.live_parser = Some(parser);
        }
        Ok(())
    }
    fn accept_xml_events(
        &mut self,
        page: &mut RustExecutableDocument,
        events: Vec<html::ParserEvent>,
    ) -> Result<(), HostError> {
        if !self.xslt_processed && self.has_xslt_source()? {
            // No resource, inline script, metadata, or author checkpoint from
            // the source XML is published before its selected transformation.
            return Ok(());
        }
        self.parser_events
            .extend(events.into_iter().map(|event| PendingParserEvent {
                event,
                checkpoint: false,
            }));
        self.drain_parser_events(page)
    }
    fn drain_parser_events(&mut self, page: &mut RustExecutableDocument) -> Result<(), HostError> {
        while let Some(mut pending) = self.parser_events.pop_front() {
            let result = self.accept_parser_event(page, &mut pending);
            if let Err(error) = result {
                self.parser_events.push_front(pending);
                return Err(error);
            }
            if !self.live(&page.agent.host) {
                self.parser_events.clear();
                break;
            }
        }
        Ok(())
    }
    fn accept_parser_event(
        &mut self,
        page: &mut RustExecutableDocument,
        pending: &mut PendingParserEvent,
    ) -> Result<(), HostError> {
        match pending.event.clone() {
            html::ParserEvent::EncodingCommitted => {
                if let Some(parser) = &mut self.live_parser {
                    parser.source.commit_encoding();
                    parser.encoding_policy.take();
                }
            }
            html::ParserEvent::EncodingIndicator { label } => {
                let Some(parser) = &mut self.live_parser else {
                    return Ok(());
                };
                if parser.parser.author_effect() {
                    parser.source.commit_encoding();
                    parser.encoding_policy.take();
                    return Ok(());
                }
                if self.encoding_replay.is_none() {
                    // Failure leaves decoder, original bytes and queued event
                    // untouched; a later author effect can still commit safely.
                    if parser.source.encoding_restart_needed(&label) {
                        #[cfg(test)]
                        xml_publication_failure(XmlPublicationFailure::BeforeEncodingRestore)?;
                        if let Some(policy) = &parser.encoding_policy {
                            page.agent.host.restore_parser_encoding_policy(policy)?;
                        }
                    }
                    self.encoding_replay = parser.source.restart_encoding(&label)?;
                }
                if self.encoding_replay.is_some() {
                    ensure_live_parser_identity(&page.agent.host, &self.dom, &parser.identity)?;
                    parser.parser.abort();
                    self.dom
                        .borrow_mut()
                        .clear_document_in(parser.identity.document)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    let replacement = html::StreamingParser::with_document(
                        self.dom.clone(),
                        parser.identity.document,
                    );
                    parser.parser = oryn_js::v8_bindings::DocumentParserGate::new(
                        replacement,
                        parser.identity.document,
                        parser.identity.stream_epoch,
                        false,
                    );
                    page.agent
                        .host
                        .install_document_parser_gate(parser.parser.clone())?;
                    parser.input.clear();
                    parser.input.push_back(oryn_dom::html::ParserInput::unicode(
                        &self.encoding_replay.take().unwrap(),
                    ));
                    parser.all_events.clear();
                    parser.loaded_frames.clear();
                    parser.script_leases.clear();
                    self.parser_events.clear();
                    self.ordered.clear();
                    self.asynchronous.clear();
                    self.xml_deferred.clear();
                    #[cfg(unix)]
                    {
                        parser.resource_scheduler =
                            ParserResourceScheduler::with_page_limits(&page.dynamic_scripts);
                        parser.deferred_scripts.clear();
                    }
                    #[cfg(not(unix))]
                    {
                        parser.ordered_scripts.clear();
                        self.portable_styles.clear();
                    }
                    #[cfg(unix)]
                    {
                        self.styles = None;
                        self.pending_stylesheet = None;
                        self.style_nodes.clear();
                        self.style_seen.clear();
                        self.style_leases.clear();
                        self.parser_async = None;
                    }
                    self.html_publication_pending = true;
                }
                if parser.source.encoding_confidence() == oryn_net::HtmlEncodingConfidence::Certain
                {
                    parser.encoding_policy.take();
                }
            }

            html::ParserEvent::ScriptReady { node, .. } => {
                // XML's parser checkpoints before preparing the element, so
                // observers may change its type/source or detach it entirely.
                if !pending.checkpoint {
                    page.agent.host.run_microtask_checkpoint()?;
                    pending.checkpoint = true;
                }
                if !self.live(&page.agent.host) {
                    return Ok(());
                }
                if self
                    .dom
                    .borrow_mut()
                    .reject_moved_parser_script(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                {
                    return Ok(());
                }
                if !self.dom.borrow().is_connected(node).unwrap_or(false)
                    || self.dom.borrow().node_document(node).ok().flatten()
                        != Some(self.load.identity.0)
                    || !self
                        .dom
                        .borrow()
                        .script_parser_document_matches(node, self.load.identity.0)
                        .unwrap_or(false)
                    || self
                        .dom
                        .borrow()
                        .script_already_started(node)
                        .unwrap_or(true)
                {
                    return Ok(());
                }
                if self
                    .ordered
                    .iter()
                    .chain(self.asynchronous.iter())
                    .chain(self.xml_deferred.iter())
                    .filter(|job| job._inline_reservation.is_some())
                    .count()
                    >= 1024
                {
                    return Err(HostError::LimitExceeded {
                        resource: "XML frame scripts".into(),
                    });
                }
                let Some(source) = page.agent.host.prepare_script_text(node)? else {
                    return Ok(());
                };
                if !self.live(&page.agent.host) {
                    return Ok(());
                }
                let mut record = capture_parser_script_node(&page.agent.host, &self.dom, node)?;
                record.3 = source;
                let classic = record
                    .2
                    .as_deref()
                    .is_none_or(html::is_javascript_mime_type);
                let executable = classic
                    || matches!(
                        record.2.as_deref(),
                        Some("module" | "importmap" | "speculationrules")
                    );
                if !executable || (record.1.is_none() && record.3.is_empty()) {
                    return Ok(());
                }
                if classic && (record.5 || record.8.legacy_event_blocked) {
                    page.agent.host.mark_script_started(node)?;
                    return Ok(());
                }
                let bytes = record
                    .3
                    .len()
                    .saturating_add(record.1.as_ref().map_or(0, String::len));
                let reservation =
                    ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, bytes)
                        .ok_or_else(|| HostError::LimitExceeded {
                            resource: "XML frame script bytes".into(),
                        })?;
                let invalid_source = FrameScriptJob::invalid_source(&record);
                if invalid_source {
                    record.8.before_fetch_failure = true;
                    record.8.failed = true;
                }
                let deferred = record.6 && !invalid_source;
                let asynchronous = invalid_source || (record.1.is_some() && !record.4 && !deferred);
                let job = FrameScriptJob {
                    _inline_reservation: Some(reservation),
                    execution: ParserScriptProgress {
                        retained: true,
                        ..Default::default()
                    },
                    execution_finished: false,
                    #[cfg(not(unix))]
                    portable_source: None,
                    #[cfg(unix)]
                    source_completion: None,
                    record,
                    _lease: ParserScriptLease::new(&self.dom, node)?,
                    #[cfg(unix)]
                    source: None,
                    graph: None,
                };
                let queue = if deferred {
                    &mut self.xml_deferred
                } else if asynchronous {
                    &mut self.asynchronous
                } else {
                    &mut self.ordered
                };
                queue.try_reserve(1).map_err(|_| HostError::LimitExceeded {
                    resource: "XML script job slots".into(),
                })?;
                #[cfg(test)]
                xml_publication_failure(XmlPublicationFailure::BeforeScriptCommit)?;
                page.agent.host.mark_script_started(node)?;
                queue.push_back(job);
            }
            html::ParserEvent::ExternalTemplateDiscovered { node } => {
                page.agent.host.prepare_external_template(node)?
            }
            html::ParserEvent::ModulePreloadDiscovered { node } => {
                page.agent.host.prepare_module_preload(node)?
            }
            html::ParserEvent::StylesheetDiscovered { node, .. }
            | html::ParserEvent::InlineStyleReady { node } => {
                #[cfg(unix)]
                self.queue_style(node, true)?;
                #[cfg(not(unix))]
                self.queue_portable_style(page, node)?;
            }
            html::ParserEvent::FrameDiscovered { node, .. }
            | html::ParserEvent::FrameLocalDocument { node } => {
                page.agent.host.ensure_provisional_frame_window(node)?;
                page.agent.host.schedule_dynamic_frame(node)?;
            }
            html::ParserEvent::EventHandlerAttributeChanged { node, name } => page
                .agent
                .host
                .prepare_event_handler_attribute(node, &name)?,
            html::ParserEvent::MetaSecurity { content } => {
                #[cfg(unix)]
                if page.context.is_none() {
                    page.agent.host.install_standalone_meta_policy(&content)?;
                }
                #[cfg(not(unix))]
                page.agent.host.install_standalone_meta_policy(&content)?;
                #[cfg(unix)]
                if let Some((client, page_id, _)) = &page.context {
                    let response = client
                        .request(
                            crate::context_service::ContextServiceRequest::InstallRealmMetadata {
                                page_id: *page_id,
                                creator: page
                                    .agent
                                    .host
                                    .native_creator_for_realm(self.load.realm)?,
                                meta_csp: Some(content),
                                meta_referrer: None,
                            },
                        )
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if !matches!(
                        response,
                        crate::context_service::ContextServiceResponse::DocumentSecurityInstalled
                    ) {
                        return Err(HostError::Execution("invalid XML metadata response".into()));
                    }
                }
            }
            html::ParserEvent::MetaReferrer { content } => {
                page.agent.host.set_document_referrer_policy(&content)?;
                #[cfg(unix)]
                if let Some((client, page_id, _)) = &page.context {
                    let response = client
                        .request(
                            crate::context_service::ContextServiceRequest::InstallRealmMetadata {
                                page_id: *page_id,
                                creator: page
                                    .agent
                                    .host
                                    .native_creator_for_realm(self.load.realm)?,
                                meta_csp: None,
                                meta_referrer: Some(content),
                            },
                        )
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if !matches!(
                        response,
                        crate::context_service::ContextServiceResponse::DocumentSecurityInstalled
                    ) {
                        return Err(HostError::Execution("invalid XML metadata response".into()));
                    }
                }
            }
        }
        Ok(())
    }
    fn has_xslt_source(&self) -> Result<bool, HostError> {
        oryn_dom::document_xslt_stylesheet(self.dom.borrow().arena(), self.load.identity.0)
            .map(|sheet| sheet.is_some())
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    fn publish_xslt_result(
        &mut self,
        page: &mut RustExecutableDocument,
        completion: &mut XmlCompletion,
    ) -> Result<(), HostError> {
        if completion.kind == ParserCompletionKind::Html {
            return Ok(());
        }
        if !self.xslt_processed {
            self.xslt_processed = true;
            if !self.has_xslt_source()?
                || completion
                    .errors
                    .as_ref()
                    .is_some_and(|errors| !errors.is_empty())
            {
                return Ok(());
            }
            let result = page.agent.host.transform_document_xslt();
            if !self.live(&page.agent.host) {
                return Ok(());
            }
            let Some((output, kind)) = result? else {
                return Ok(());
            };
            let mut parsed = match kind {
                1 => html::parse(&output),
                2 => html::parse_text_document(&output, "text/plain"),
                _ => oryn_dom::xml::parse(&output, "application/xml"),
            };
            set_parsed_encoding(&mut parsed, "UTF-8")?;
            self.xslt_result = Some(parsed);
        }
        let Some(parsed) = self.xslt_result.as_ref() else {
            return Ok(());
        };
        let mut published =
            publish_buffered_navigation_document_ref(&self.dom, self.load.identity.0, parsed)?;
        self.xslt_result = None;
        // Removed source jobs keep no source/graph reservations and cannot
        // execute against the transformed document identity.
        self.ordered.clear();
        self.asynchronous.clear();
        self.xml_deferred.clear();
        self.parser_events.clear();
        if Rc::ptr_eq(&self.dom, &page.agent.dom) {
            for node in subtree_nodes(&self.dom.borrow(), self.load.identity.0)? {
                page.node_realms.insert(node, self.load.realm);
            }
        }
        let events = std::mem::take(&mut published.events);
        completion.errors = Some(published.errors.clone());
        completion.document = Some(published);
        // Queue before author entry; a failed callback resumes the retained
        // event instead of replaying the transform or losing later scripts.
        self.accept_xml_events(page, events)
    }
    fn finish_xml_completion(
        &mut self,
        page: &mut RustExecutableDocument,
    ) -> Result<(), HostError> {
        let Some(mut completion) = self.xml_completion.take() else {
            return Ok(());
        };
        let result = (|| -> Result<bool, HostError> {
            if completion.stage == 0 {
                if let Some(document) = completion.document.as_mut()
                    && !document.errors.is_empty()
                {
                    let encoding = document_encoding_at(&self.dom, self.load.identity.0);
                    set_parsed_encoding(document, &encoding)?;
                    let published = publish_buffered_navigation_document_ref(
                        &self.dom,
                        self.load.identity.0,
                        document,
                    )?;
                    *document = published;
                }
                completion.stage = 1;
            }
            if completion.stage == 1 {
                let events = completion
                    .document
                    .as_mut()
                    .map(|document| std::mem::take(&mut document.events))
                    .unwrap_or_default();
                completion.stage = 2;
                self.accept_xml_events(page, events)?;
            }
            self.drain_parser_events(page)?;
            if !self.live(&page.agent.host) {
                return Ok(true);
            }
            if completion.stage == 2 {
                self.publish_xslt_result(page, &mut completion)?;
                if !self.live(&page.agent.host) {
                    return Ok(true);
                }
                // Fatal XML completion replaces the parsed tree with parsererror.
                // Jobs retained for end-of-parsing must not execute from that
                // discarded tree; dropping them also releases source/graph custody.
                if completion.kind == ParserCompletionKind::Xml
                    && completion
                        .errors
                        .as_ref()
                        .is_some_and(|errors| !errors.is_empty())
                {
                    self.xml_deferred.clear();
                }
                self.ordered
                    .try_reserve(self.xml_deferred.len())
                    .map_err(|_| HostError::LimitExceeded {
                        resource: "XML deferred publication".into(),
                    })?;
                self.ordered.append(&mut self.xml_deferred);
                self._input_reservation.take();
                if self.root_document {
                    if let Some(errors) = &completion.errors {
                        page.document.errors = errors.clone();
                    }
                    if let Some(document) = &completion.document {
                        page.document.quirks_mode = document.quirks_mode;
                    }
                }
                completion.stage = 3;
            }
            if completion.stage == 3 {
                page.agent.host.set_parser_input_active(false)?;
                #[cfg(test)]
                xml_publication_failure(XmlPublicationFailure::BeforeReadiness)?;
                completion.receipt = Some(page.agent.host.start_native_readiness_publication(
                    oryn_dom::DocumentReadyState::Interactive,
                )?);
                completion.stage = 4;
            }
            if !self.live(&page.agent.host) {
                return Ok(true);
            }
            if !page
                .agent
                .host
                .resume_native_history_publication(completion.receipt.unwrap())?
            {
                return Ok(false);
            }
            Ok(true)
        })();
        match result {
            Ok(true) => Ok(()),
            other => {
                self.xml_completion = Some(completion);
                self.parser_retry = Some(std::time::Instant::now() + Duration::from_millis(25));
                other.map(|_| ())
            }
        }
    }
    pub(super) fn root_html(
        load: PreparedDocumentLoad,
        dom: SharedDomRuntime,
        limits: &DynamicScriptScheduler,
        host: &mut RawV8Host,
    ) -> Result<Self, HostError> {
        let mut owner = Self::new(FrameScriptInput {
            load,
            parent: load.realm,
            frame: load.identity.0,
            dom,
            records: vec![],
            lifecycle_allowed: true,
            _refresh_url: None,
            limits,
            host,
        })?;
        owner.root_document = true;
        Ok(owner)
    }
    pub(super) fn install_html_parser(&mut self, parser: LiveParserState) {
        self.live_parser = Some(parser);
    }
    fn publish_html_slice(&mut self, page: &mut RustExecutableDocument) -> Result<(), HostError> {
        if !self.html_publication_pending {
            return Ok(());
        }
        #[cfg(test)]
        xml_publication_failure(XmlPublicationFailure::BeforeHtmlPublication)?;
        page.agent.host.prepare_document_event_handlers()?;
        if Rc::ptr_eq(&self.dom, &page.agent.dom) {
            for node in subtree_nodes(&self.dom.borrow(), self.load.identity.0)? {
                page.node_realms.insert(node, self.load.realm);
            }
        }
        self.html_publication_pending = false;
        Ok(())
    }
    fn poll_html_parser(&mut self, page: &mut RustExecutableDocument) -> Result<(), HostError> {
        for _ in 0..64 {
            if self.html_bytes_remaining == 0 {
                break;
            }
            if self
                .poll_html_parser_slice(page, self.html_bytes_remaining)?
                .is_none()
            {
                break;
            }
        }
        Ok(())
    }
    fn poll_html_parser_slice(
        &mut self,
        page: &mut RustExecutableDocument,
        byte_limit: usize,
    ) -> Result<Option<usize>, HostError> {
        if !self.ordered.is_empty() || self.live_parser.is_none() {
            return Ok(None);
        }
        if let Some(error) = self.html_input_error.take() {
            self.parser_failed = true;
            page.agent
                .host
                .stop_pending_document_load(self.load.realm)?;
            return Err(error);
        }
        let parser = self.live_parser.as_mut().unwrap();
        ensure_live_parser_identity(&page.agent.host, &self.dom, &parser.identity)?;
        if parser.parser.author_effect() {
            parser.source.commit_encoding();
            parser.encoding_policy.take();
        }
        // All fallible metadata publication precedes consuming another source
        // chunk. Input errors are terminal; native publication/admission errors
        // leave this owner and its existing suffix/event queue intact.
        if let Some(encoding) = *parser.encoding.lock().expect("navigation encoding")
            && let Some(metadata) = self
                .dom
                .borrow_mut()
                .arena_mut()
                .get_mut(parser.identity.document)
                .map_err(|error| HostError::Execution(error.to_string()))?
                .document_metadata
                .as_mut()
        {
            metadata.encoding = encoding.into();
        }
        let mut writes = parser.parser.take_input();
        writes.append(&mut parser.input);
        parser.input = writes;
        if parser.parser.pending_events() {
            parser
                .input
                .push_front(oryn_dom::html::ParserInput::unicode(""));
        }
        let next = match parser.input.pop_front() {
            Some(chunk) => Some(chunk),
            None if self.html_eof => None,
            None => match parser.source.poll_next() {
                std::task::Poll::Pending => {
                    self.parser_waiting = true;
                    return Ok(None);
                }
                std::task::Poll::Ready(chunk) => match chunk {
                    Some(Ok(chunk)) => Some(oryn_dom::html::ParserInput::unicode(&chunk)),
                    None => None,
                    Some(Err(error)) => {
                        self.parser_failed = true;
                        page.agent
                            .host
                            .stop_pending_document_load(self.load.realm)?;
                        return Err(error);
                    }
                },
            },
        };
        // Retain accepted EOF before any fallible metadata publication too.
        if next.is_none() {
            self.html_eof = true;
        }
        // A source read can select its encoding (including after a buffered
        // detection sample). Publish it before this text can run author code.
        let selected_encoding = *parser.encoding.lock().expect("navigation encoding");
        if let Some(encoding) = selected_encoding {
            let publication = (|| -> Result<(), HostError> {
                if let Some(metadata) = self
                    .dom
                    .borrow_mut()
                    .arena_mut()
                    .get_mut(parser.identity.document)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .document_metadata
                    .as_mut()
                {
                    metadata.encoding = encoding.into();
                }
                Ok(())
            })();
            if let Err(error) = publication {
                if let Some(chunk) = next {
                    parser.input.push_front(chunk);
                }
                return Err(error);
            }
        }
        self.parser_waiting = false;
        if let Some(mut chunk) = next {
            let mut end = chunk.len().min(byte_limit);
            while !chunk.is_char_boundary(end) {
                end -= 1;
            }
            parser.parser.resume();
            let (progress, construction_error) =
                match page.agent.host.with_parser_construction(|| {
                    parser
                        .parser
                        .push_input_until_resource_or_script(&chunk.prefix(end))
                }) {
                    Ok(progress) => progress,
                    Err(error) => {
                        parser.input.push_front(chunk);
                        return Err(error);
                    }
                };
            let suffix = chunk.split_off(progress.consumed_bytes);
            self.html_bytes_remaining = self
                .html_bytes_remaining
                .saturating_sub(progress.consumed_bytes);
            if !suffix.is_empty() {
                parser.input.push_front(suffix);
            }
            let mut events = parser.parser.take_events();
            if let Some(event) = progress.blocked_on {
                events.push(event);
                parser.parser.pause();
            }
            let data_only = !events.is_empty()
                && events.iter().all(|event| {
                    matches!(
                        event,
                        html::ParserEvent::ScriptReady {
                            scheduling: html::ScriptScheduling::Data,
                            ..
                        }
                    )
                });
            // Transfer every consumed tokenizer event before touching wrappers,
            // resource admission, or author callbacks. Retrying publication
            // resumes this queue and never tokenizes the consumed prefix again.
            self.parser_events
                .extend(events.into_iter().map(|event| PendingParserEvent {
                    event,
                    checkpoint: false,
                }));
            self.html_publication_pending = true;
            if let Some(error) = construction_error {
                return Err(error);
            }
            #[cfg(test)]
            xml_publication_failure(XmlPublicationFailure::AfterHtmlTokenization)?;
            self.publish_html_slice(page)?;
            self.drain_parser_events(page)?;
            // Data preparation still runs its observer/microtask checkpoint
            // before subsequent tokens. Batch only retained input after that
            // checkpoint, with no new source read or executable script job.
            if data_only
                && self.live(&page.agent.host)
                && self.ordered.is_empty()
                && self.asynchronous.is_empty()
                && self.xml_deferred.is_empty()
                && self
                    .live_parser
                    .as_ref()
                    .is_some_and(|parser| !parser.input.is_empty() || parser.parser.has_input())
            {
                return Ok(Some(progress.consumed_bytes));
            }
        } else {
            // EOF is an accepted source result even if native construction or
            // publication must retry. Never poll that terminal input again.
            self.html_eof = true;
            parser.source.commit_encoding();
            parser.encoding_policy.take();
            let (ready, construction_error) = page
                .agent
                .host
                .with_parser_construction(|| parser.parser.prepare_finish())?;
            if let Some(error) = construction_error {
                return Err(error);
            }
            #[cfg(test)]
            xml_publication_failure(XmlPublicationFailure::AfterHtmlEof)?;
            if !ready {
                return Ok(None);
            }
            let document = parser.parser.finish();
            let errors = document.errors.clone();
            self.live_parser = None;
            self.xml_completion = Some(XmlCompletion {
                kind: ParserCompletionKind::Html,
                document: Some(document),
                errors: Some(errors),
                stage: 1,
                receipt: None,
            });
        }
        Ok(None)
    }
    fn poll_parser(&mut self, page: &mut RustExecutableDocument) -> Result<(), HostError> {
        self.html_bytes_remaining = 64 * 1024;
        self.publish_html_slice(page)?;
        self.drain_parser_events(page)?;
        if !self.live(&page.agent.host) {
            return Ok(());
        }
        if self.ordered.is_empty() && self.snapshot_events.is_some() {
            if !self.xslt_processed && self.has_xslt_source()? {
                self.snapshot_events.as_mut().unwrap().clear();
            }
            if let Some((node, event)) = self
                .snapshot_events
                .as_ref()
                .and_then(|events| events.front().cloned())
            {
                if self.dom.borrow().is_connected(node).unwrap_or(false)
                    && self.dom.borrow().node_document(node).ok().flatten()
                        == Some(self.load.identity.0)
                {
                    page.agent.host.release_snapshot_inline_style(node)?;
                    // Source ownership transfers into parser_events before a
                    // script checkpoint or resource preparation can fail.
                    self.snapshot_events.as_mut().unwrap().pop_front();
                    if let Some(event) = event {
                        self.accept_xml_events(page, vec![event])?;
                    }
                } else {
                    self.snapshot_events.as_mut().unwrap().pop_front();
                }
            }
            if self
                .snapshot_events
                .as_ref()
                .is_some_and(|events| events.is_empty())
            {
                self.snapshot_events = None;
                self.xml_completion = Some(XmlCompletion {
                    kind: ParserCompletionKind::Snapshot,
                    document: None,
                    errors: None,
                    stage: 2,
                    receipt: None,
                });
            }
        }
        if !self.live(&page.agent.host) {
            return Ok(());
        }
        if self.ordered.is_empty() {
            self.poll_xml_input();
        }
        if let Some(error) = self.xml_input_error.take()
            && let Some(parser) = self.xml_parser.as_mut()
        {
            parser.fail_input(error.to_string());
        }
        self.poll_html_parser(page)?;
        if self.ordered.is_empty() && self.xml_parser.is_some() {
            let (step, construction_error) = page.agent.host.with_parser_construction(|| {
                self.xml_parser
                    .take()
                    .expect("retained XML parser")
                    .advance(256)
            })?;
            match step {
                oryn_dom::xml::XmlParseStep::Pending(mut parser) => {
                    let events = parser.take_events();
                    self.xml_parser = Some(*parser);
                    self.accept_xml_events(page, events)?;
                }
                oryn_dom::xml::XmlParseStep::LiveComplete {
                    document,
                    events,
                    errors,
                } => {
                    if document != self.load.identity.0 {
                        return Err(HostError::Execution(
                            "XML completion changed native Document".into(),
                        ));
                    }
                    self.xml_completion = Some(XmlCompletion {
                        kind: ParserCompletionKind::Xml,
                        document: None,
                        errors: Some(errors),
                        stage: 1,
                        receipt: None,
                    });
                    self.accept_xml_events(page, events)?;
                }
                oryn_dom::xml::XmlParseStep::Complete(document) => {
                    let errors = document.errors.clone();
                    self.xml_completion = Some(XmlCompletion {
                        kind: ParserCompletionKind::Xml,
                        document: Some(*document),
                        errors: Some(errors),
                        stage: 0,
                        receipt: None,
                    });
                }
            }
            if let Some(error) = construction_error {
                return Err(error);
            }
        }
        self.finish_xml_completion(page)?;
        #[cfg(unix)]
        if let Some(pending) = self.parser_async.as_mut() {
            for _ in 0..64 {
                let Some(completion) = pending.scheduler.try_next_with_host(&mut page.agent.host)
                else {
                    break;
                };
                pending
                    .scheduler
                    .apply_completion(crate::executable::ParserCompletionInput {
                        completion,
                        live_dom: &self.dom,
                        host: &mut page.agent.host,
                        resources: &mut page.resources,
                        deferred: &mut BTreeMap::new(),
                        diagnostics: &mut page.diagnostics,
                        context: page.context.as_ref(),
                    })?;
            }
            pending.scheduler.poll_async_modules(
                &self.dom,
                &mut page.agent.host,
                &mut page.resources,
                &mut page.diagnostics,
                page.context.as_ref(),
            )?;
            if !pending.scheduler.live_stylesheets(
                &page.agent.host,
                &self.dom,
                Some(self.load.realm),
            ) && !pending
                .scheduler
                .live_script_records()
                .any(|record| parser_script_owner_live(&page.agent.host, &self.dom, record))
            {
                self.parser_async = None;
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
impl FrameScriptPreparation {
    fn queue_style(&mut self, node: NodeId, implicit: bool) -> Result<(), HostError> {
        let revision = self
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if self.style_seen.contains(&(node, revision)) {
            return Ok(());
        }
        if self.style_seen.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "frame stylesheet preparation records".into(),
            });
        }
        self.styles
            .get_or_insert_with(ParserResourceScheduler::new)
            .prepare_render_stylesheet(&self.dom, node, revision, implicit)?;
        self.style_leases
            .push(ParserScriptLease::new(&self.dom, node)?);
        self.style_seen.insert((node, revision));
        self.style_nodes.push_back((node, revision));
        Ok(())
    }
    fn styles_block_scripts(&self, host: &RawV8Host) -> Result<bool, HostError> {
        if let Some(pending) = &self.pending_stylesheet
            && self
                .dom
                .borrow()
                .arena()
                .stylesheet_revision(pending.node)
                .ok()
                == Some(pending.revision)
            && host.cached_stylesheet_blocks_scripts(pending.node)?
        {
            return Ok(true);
        }

        if let Some(pending) = &self.parser_async
            && pending
                .scheduler
                .scripts_blocked_by_stylesheets(host, &self.dom)?
        {
            return Ok(true);
        }
        for &(node, revision) in &self.style_nodes {
            if self.dom.borrow().arena().stylesheet_revision(node).ok() == Some(revision)
                && host.cached_stylesheet_blocks_scripts(node)?
            {
                return Ok(true);
            }
        }
        self.styles.as_ref().map_or(Ok(false), |styles| {
            styles.scripts_blocked_by_stylesheets(host, &self.dom)
        })
    }
    fn poll_styles(&mut self, page: &mut RustExecutableDocument) -> Result<(), HostError> {
        let styles = self.styles.get_or_insert_with(|| {
            ParserResourceScheduler::with_page_limits(&page.dynamic_scripts)
        });
        // Queue admission can precede the first scheduler poll; bind its IO
        // accounting to the page before any resource is launched.
        if styles.pending == 0 && styles.local_stylesheets.is_empty() {
            styles.source_bytes = page.dynamic_scripts.source_bytes.clone();
        }
        for _ in 0..64 {
            let Some(&(node, revision)) = self.style_nodes.front() else {
                break;
            };
            if self.dom.borrow().arena().stylesheet_revision(node).ok() != Some(revision)
                || !self.dom.borrow().is_connected(node).unwrap_or(false)
            {
                styles.complete_render_stylesheet(node, revision);
                self.style_nodes.pop_front();
                continue;
            }
            let (link, href, css, encoding) = {
                let dom = self.dom.borrow();
                let processing = dom
                    .arena()
                    .xml_stylesheet(node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let link = processing.is_some()
                    || dom.is_element(node, "http://www.w3.org/1999/xhtml", "link");
                let href = processing.map(|sheet| sheet.href).unwrap_or_else(|| {
                    dom.attribute(node, "href")
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                        .to_owned()
                });
                let css = dom.arena().stylesheet_text(node).unwrap_or_default();
                let encoding = dom
                    .arena()
                    .get(self.load.identity.0)
                    .ok()
                    .and_then(|node| node.document_metadata.as_ref())
                    .map(|metadata| metadata.encoding.clone())
                    .unwrap_or_else(|| "UTF-8".into());
                (link, href, css, encoding)
            };
            let fetch = ParserFetchContext::capture(&page.agent.host, &self.dom, node)?;
            #[cfg(test)]
            xml_publication_failure(XmlPublicationFailure::BeforeStyleAdmission)?;
            if link {
                if !href.trim().is_empty() {
                    styles.schedule_stylesheet_auto(
                        crate::executable::AutomaticStylesheetScheduleInput {
                            context: page.context.as_ref(),
                            node,
                            revision,
                            href: Some(href),
                            css: String::new(),
                            encoding,
                            fetch,
                            host: &mut page.agent.host,
                            resources: &page.resources,
                        },
                    )?;
                } else {
                    styles.complete_render_stylesheet(node, revision);
                }
            } else if page.agent.host.native_retained_admission_for_realm(fetch.realm)?.is_some()
                || oryn_layout::cssom::stylesheet_requires_resources(&css, &fetch.base_url)
                    .map_err(HostError::Execution)?
            {
                styles.schedule_stylesheet_auto(
                    crate::executable::AutomaticStylesheetScheduleInput {
                        context: page.context.as_ref(),
                        node,
                        revision,
                        href: None,
                        css,
                        encoding,
                        fetch,
                        host: &mut page.agent.host,
                        resources: &page.resources,
                    },
                )?;
            } else {
                styles.complete_render_stylesheet(node, revision);
            }
            // A failed capture/admission must keep the stylesheet blocking the
            // following parser script until this exact queue entry is retried.
            self.style_nodes.pop_front();
        }
        for _ in 0..64 {
            if self.pending_stylesheet.is_none() {
                let Some(completion) = styles.try_next() else {
                    break;
                };
                if parser_stylesheet_owner_completion(&completion)? {
                    styles.apply_completion(crate::executable::ParserCompletionInput {
                        completion,
                        live_dom: &self.dom,
                        host: &mut page.agent.host,
                        resources: &mut page.resources,
                        deferred: &mut BTreeMap::new(),
                        diagnostics: &mut page.diagnostics,
                        context: page.context.as_ref(),
                    })?;
                    continue;
                }
                let ParserResourceCompletion::Stylesheet {
                    identity,
                    node,
                    revision,
                    result,
                } = completion
                else {
                    return Err(HostError::Execution(
                        "non-stylesheet completion in parser stylesheet owner".into(),
                    ));
                };
                self.pending_stylesheet = Some(RetainedParserStylesheet {
                    identity,
                    node,
                    revision,
                    result,
                    published: None,
                });
            }
            let pending = self.pending_stylesheet.as_mut().unwrap();
            if pending.published.is_none() {
                pending.published = Some(publish_parser_stylesheet_completion(
                    crate::executable::StylesheetPublicationInput {
                        identity: pending.identity,
                        node: pending.node,
                        revision: pending.revision,
                        result: &pending.result,
                        live_dom: &self.dom,
                        host: &mut page.agent.host,
                        resources: &mut page.resources,
                        diagnostics: &mut page.diagnostics,
                    },
                )?);
            }
            #[cfg(test)]
            xml_publication_failure(XmlPublicationFailure::BeforeStyleEvent)?;
            if self.dom.borrow().node_document(pending.node).ok().flatten()
                == Some(pending.identity.document)
                && self
                    .dom
                    .borrow()
                    .arena()
                    .stylesheet_revision(pending.node)
                    .ok()
                    == Some(pending.revision)
                && let Some(success) = pending.published.unwrap()
            {
                page.agent
                    .host
                    .queue_stylesheet_event(pending.node, pending.revision, success)?;
            }
            styles.complete_render_stylesheet(pending.node, pending.revision);
            self.pending_stylesheet = None;
        }
        styles.poll_async_modules(
            &self.dom,
            &mut page.agent.host,
            &mut page.resources,
            &mut page.diagnostics,
            page.context.as_ref(),
        )?;
        Ok(())
    }
}

#[cfg(unix)]
fn parser_stylesheet_owner_completion(
    completion: &ParserResourceCompletion,
) -> Result<bool, HostError> {
    match completion {
        ParserResourceCompletion::Stylesheet { .. } => Ok(false),
        ParserResourceCompletion::RetainedStylesheet(_)
        | ParserResourceCompletion::NativePendingImport { .. } => Ok(true),
        ParserResourceCompletion::Script { .. } => Err(HostError::Execution(
            "script completion in parser stylesheet owner".into(),
        )),
        #[cfg(test)]
        ParserResourceCompletion::Frame { .. } => Err(HostError::Execution(
            "frame completion in parser stylesheet owner".into(),
        )),
    }
}

#[cfg(unix)]
pub(super) fn retryable_script_preparation(error: &HostError) -> bool {
    matches!(
        error,
        HostError::LimitExceeded { .. }
            | HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Infrastructure(_)
                    | crate::script_load_failure::ScriptLoadFailure::Capacity(_)
                    | crate::script_load_failure::ScriptLoadFailure::Cancelled
            )
    )
}

impl FrameScriptJob {
    #[cfg(test)]
    fn diagnostic(&self, phase: &'static str) {
        crate::runtime::loading_stage_diagnostics::script(
            self.record.8.base_url.as_deref(),
            phase,
            || {
                format!(
                    "source={} owner={:?} loaded={} entered={} finished={} completion={:?}",
                    self.record
                        .1
                        .as_deref()
                        .unwrap_or("[inline]")
                        .chars()
                        .take(256)
                        .collect::<String>(),
                    self.record.7.map(|owner| (
                        owner.realm.0,
                        owner.document_generation,
                        owner.stream_epoch,
                        owner.document
                    )),
                    self.record.8.loaded.is_some(),
                    self.execution.entered,
                    self.execution.finished,
                    self.execution.completion
                )
            },
        );
    }
    fn invalid_source(record: &ScriptRecord) -> bool {
        let Some(source) = &record.1 else {
            return false;
        };
        source.is_empty()
            || matches!(record.2.as_deref(), Some("importmap" | "speculationrules"))
            || record
                .8
                .base_url
                .as_ref()
                .is_some_and(|base| Url::parse(base).and_then(|base| base.join(source)).is_err())
    }
    #[cfg(all(test, unix))]
    pub(super) fn pending_sources(&self) -> usize {
        self.source.as_ref().map_or(0, |source| source.pending)
    }
    pub(super) fn from_completion(
        dom: &SharedDomRuntime,
        record: ScriptRecord,
        #[cfg(unix)] source: Option<ParserResourceScheduler>,
        #[cfg(unix)] graph: Option<module_graph_load::ModuleGraphLoad>,
    ) -> Result<Self, HostError> {
        Ok(Self {
            _inline_reservation: None,
            _lease: ParserScriptLease::for_record(dom, &record)?,
            record,
            execution: ParserScriptProgress {
                retained: true,
                ..Default::default()
            },
            execution_finished: false,
            #[cfg(not(unix))]
            portable_source: None,
            #[cfg(unix)]
            source_completion: None,
            #[cfg(unix)]
            source,
            #[cfg(unix)]
            graph,
            #[cfg(not(unix))]
            graph: None,
        })
    }
    fn finish_error(
        &mut self,
        page: &mut RustExecutableDocument,
        dom: &SharedDomRuntime,
        parser_gate: Option<&Rc<oryn_js::v8_bindings::DocumentParserGate>>,
    ) -> Result<bool, HostError> {
        if self.record.4
            && !self.record.8.before_fetch_failure
            && let Some(gate) = parser_gate
        {
            gate.resume();
        }
        self.execution.entered = true;
        self.execution.completion = Some(false);
        complete_parser_script(&mut page.agent.host, dom, &self.record, &mut self.execution)?;
        Ok(self.execution.finished)
    }
    pub(super) fn preparation_pending(&self) -> bool {
        #[cfg(unix)]
        {
            self.source.is_some()
                || (self.graph.is_some() && !self.record.8.graph_preloaded)
                || (self.record.1.as_deref().is_some_and(|src| !src.is_empty())
                    && self.record.8.loaded.is_none())
                || (self.record.2.as_deref() == Some("module") && !self.record.8.graph_preloaded)
        }
        #[cfg(not(unix))]
        {
            (self.record.2.as_deref() == Some("module") && !self.record.8.graph_preloaded)
                || (self
                    .record
                    .2
                    .as_deref()
                    .is_none_or(html::is_javascript_mime_type)
                    && self.record.1.as_deref().is_some_and(|src| !src.is_empty())
                    && self.record.8.loaded.is_none())
        }
    }
    pub(super) fn runnable(&mut self, host: &RawV8Host, dom: &SharedDomRuntime) -> bool {
        if !parser_script_owner_live(host, dom, &self.record) {
            return true;
        }
        #[cfg(unix)]
        {
            if self.source_completion.is_some() {
                return true;
            }
            if let Some(source) = &mut self.source {
                return source.has_ready_completion();
            }
            if let Some(graph) = &self.graph
                && !self.record.8.graph_preloaded
            {
                return graph.is_runnable();
            }
        }
        #[cfg(not(unix))]
        if let Some(source) = &mut self.portable_source {
            return source.ready();
        }
        #[cfg(not(unix))]
        if let Some(graph) = &self.graph {
            if !self.record.8.graph_preloaded {
                return graph.is_runnable();
            }
        }
        true
    }

    pub(super) fn poll(
        &mut self,
        page: &mut RustExecutableDocument,
        dom: &SharedDomRuntime,
        execution_allowed: bool,
        parser_gate: Option<&Rc<oryn_js::v8_bindings::DocumentParserGate>>,
    ) -> Result<bool, HostError> {
        #[cfg(test)]
        self.diagnostic("script-poll-enter");
        if !parser_script_owner_live(&page.agent.host, dom, &self.record) {
            self.execution.receipt = None;
            self.execution.receipt_owner = None;
            return Ok(true);
        }
        if self
            .record
            .2
            .as_deref()
            .is_none_or(html::is_javascript_mime_type)
            && (self.record.5 || self.record.8.legacy_event_blocked)
        {
            if self.record.1.is_some() || !self.record.3.is_empty() {
                page.agent
                    .host
                    .mark_script_started(script_record_node(&self.record))?;
            }
            return Ok(true);
        }
        if self.record.8.failed || self.execution.completion == Some(false) {
            return self.finish_error(page, dom, parser_gate);
        }
        if Self::invalid_source(&self.record) {
            self.record.8.before_fetch_failure = true;
            return self.finish_error(page, dom, parser_gate);
        }
        #[cfg(unix)]
        {
            let executable = self.record.2.as_deref() == Some("module")
                || (self
                    .record
                    .2
                    .as_deref()
                    .is_none_or(html::is_javascript_mime_type)
                    && !self.record.5);
            if executable
                && self.record.1.as_deref().is_some_and(|src| !src.is_empty())
                && self.record.8.loaded.is_none()
            {
                if self.source.is_none() {
                    let mut scheduler =
                        ParserResourceScheduler::with_page_limits(&page.dynamic_scripts);
                    let mut fetch =
                        ParserFetchContext::for_script(&page.agent.host, dom, &self.record)?;
                    if let Some(base) = &self.record.8.base_url {
                        fetch.base_url = base.clone();
                    }
                    if let Some(policy) = &self.record.8.referrer_policy {
                        fetch.referrer_policy = policy.clone();
                    }
                    scheduler.schedule_script_auto(
                        page.context.as_ref(),
                        0,
                        self.record.clone(),
                        false,
                        fetch,
                        &mut page.agent.host,
                    )?;
                    self.source = Some(scheduler);
                    return Ok(false);
                }
                if self.source_completion.is_none() {
                    self.source_completion = self
                        .source
                        .as_mut()
                        .expect("frame source custody")
                        .try_next_with_host(&mut page.agent.host);
                    #[cfg(test)]
                    crate::runtime::loading_stage_diagnostics::script(
                        self.record.8.base_url.as_deref(),
                        "script-source-poll",
                        || {
                            format!(
                                "source={} completion={}",
                                self.record
                                    .1
                                    .as_deref()
                                    .unwrap_or("[inline]")
                                    .chars()
                                    .take(256)
                                    .collect::<String>(),
                                self.source_completion.is_some()
                            )
                        },
                    );
                }
                let Some(ParserResourceCompletion::Script { result, record, .. }) =
                    self.source_completion.as_ref()
                else {
                    return Ok(false);
                };
                if !parser_script_owner_live(&page.agent.host, dom, record) {
                    self.source_completion = None;
                    self.source = None;
                    return Ok(true);
                }
                if let Some(identity) = record.7 {
                    page.agent.host.activate_realm(identity.realm)?;
                }
                let fetched_transient = matches!(
                    result,
                    Err(ParserScriptFailure::Infrastructure(_) | ParserScriptFailure::Capacity)
                );
                let loaded =
                    adopt_parser_module_result(&mut page.agent.host, record, result.clone());
                match loaded {
                    Ok(loaded) => {
                        if let Some(requested) = &record.1 {
                            page.resources
                                .insert(requested.clone(), loaded.source.clone());
                            page.resources.insert(
                                format!("oryn:module-alias:{requested}"),
                                loaded.url.clone(),
                            );
                        }
                        page.resources
                            .insert(loaded.url.clone(), loaded.source.clone());
                        self.record = record.clone();
                        self.record.8.loaded = Some(loaded);
                        self.source_completion = None;
                        self.source = None;
                        #[cfg(test)]
                        self.diagnostic("script-source-adopted");
                    }
                    Err(
                        error @ (ParserScriptFailure::Fetch { .. }
                        | ParserScriptFailure::NativeTerminal(_)),
                    ) => {
                        page.diagnostics
                            .push(script_diagnostic(HostError::Execution(format!(
                                "scheduled script failed: {error}"
                            ))));
                        self.source_completion = None;
                        self.source = None;
                        return self.finish_error(page, dom, parser_gate);
                    }
                    Err(error) => {
                        // Retry failed IO admission, or retain the successful
                        // response while local module-map publication recovers.
                        if fetched_transient {
                            self.source_completion = None;
                            self.source = None;
                        }
                        return Err(match error {
                            ParserScriptFailure::Capacity => HostError::LimitExceeded {
                                resource: "parser source bytes".into(),
                            },
                            other => HostError::ScriptLoad(
                                crate::script_load_failure::ScriptLoadFailure::Infrastructure(
                                    other.to_string(),
                                ),
                            ),
                        });
                    }
                }
            }
            if self.record.2.as_deref() == Some("module")
                && !self.record.8.graph_preloaded
                && (self.record.1.is_some() || !self.record.3.is_empty())
                && (self.record.1.is_none() || self.record.8.loaded.is_some())
            {
                if self.graph.is_none() {
                    match prepare_parser_module_graph(
                        &mut page.agent.host,
                        page.context.as_ref(),
                        &self.record,
                        page.dynamic_scripts.graph_slots.clone(),
                        page.dynamic_scripts.graph_fetches.clone(),
                        page.dynamic_scripts.source_bytes.clone(),
                    ) {
                        Ok(graph) => self.graph = Some(graph),
                        Err(error) if retryable_script_preparation(&error) => return Err(error),
                        Err(error) => {
                            page.diagnostics.push(script_diagnostic(error));
                            return self.finish_error(page, dom, parser_gate);
                        }
                    }
                }
                match self
                    .graph
                    .as_mut()
                    .expect("frame module custody")
                    .poll_native(&mut page.agent.host, &mut page.resources)
                {
                    Ok(false) => return Ok(false),
                    Ok(true) => {
                        self.record.8.graph_error =
                            self.graph.as_ref().and_then(|graph| graph.error_token);
                        self.record.8.graph_preloaded = true;
                    }
                    Err(error)
                        if retryable_script_preparation(&error)
                            && self
                                .graph
                                .as_mut()
                                .is_some_and(|graph| graph.defer_retry(&mut page.agent.host)) =>
                    {
                        return Err(error);
                    }
                    Err(error) => {
                        self.graph = None;
                        page.diagnostics.push(script_diagnostic(error));
                        return self.finish_error(page, dom, parser_gate);
                    }
                }
            }
        }
        #[cfg(not(unix))]
        if self
            .record
            .2
            .as_deref()
            .is_none_or(html::is_javascript_mime_type)
            && self.record.1.as_deref().is_some_and(|src| !src.is_empty())
            && self.record.8.loaded.is_none()
        {
            if self.portable_source.is_none() {
                self.portable_source = PortableScriptSource::start(page, &self.record)?;
                return Ok(false);
            }
            let source = self
                .portable_source
                .as_mut()
                .expect("portable source owner");
            if !source.ready() {
                return Ok(false);
            }
            match source.ready.as_ref().expect("portable completed source") {
                Ok(loaded) => {
                    self.record.8.loaded = Some(loaded.clone());
                    self.portable_source = None;
                }
                Err(
                    error @ (ParserScriptFailure::Fetch { .. }
                    | ParserScriptFailure::NativeTerminal(_)),
                ) => {
                    page.diagnostics
                        .push(script_diagnostic(HostError::Execution(error.to_string())));
                    self.portable_source = None;
                    return self.finish_error(page, dom, parser_gate);
                }
                Err(error) => {
                    let error = match error {
                        ParserScriptFailure::Capacity => HostError::LimitExceeded {
                            resource: "portable parser source bytes".into(),
                        },
                        error => HostError::ScriptLoad(
                            crate::script_load_failure::ScriptLoadFailure::Infrastructure(
                                error.to_string(),
                            ),
                        ),
                    };
                    self.portable_source = None;
                    return Err(error);
                }
            }
        }
        #[cfg(not(unix))]
        if self.record.2.as_deref() == Some("module")
            && !self.record.8.graph_preloaded
            && (self.record.1.is_some() || !self.record.3.is_empty())
        {
            if self.graph.is_none() {
                self.graph = Some(prepare_parser_module_graph(
                    &mut page.agent.host,
                    &self.record,
                    page.dynamic_scripts.graph_slots.clone(),
                    page.dynamic_scripts.graph_fetches.clone(),
                    page.dynamic_scripts.source_bytes.clone(),
                )?);
            }
            match self
                .graph
                .as_mut()
                .expect("portable module graph custody")
                .poll_native(&mut page.agent.host, &mut page.resources)
            {
                Ok(false) => return Ok(false),
                Ok(true) => {}
                Err(error)
                    if retryable_script_preparation(&error)
                        && self
                            .graph
                            .as_mut()
                            .is_some_and(|graph| graph.defer_retry(&mut page.agent.host)) =>
                {
                    return Err(error);
                }
                Err(error) => {
                    self.graph = None;
                    page.diagnostics.push(script_diagnostic(error));
                    return self.finish_error(page, dom, parser_gate);
                }
            }
            if let Some(requested) = &self.record.1 {
                let base = self
                    .record
                    .8
                    .base_url
                    .clone()
                    .unwrap_or(page.agent.host.document_base_url()?);
                let requested = self
                    .record
                    .8
                    .resolve_url(&base, requested)
                    .map_err(|error| HostError::Execution(error.to_string()))?
                    .to_string();
                let loaded = page
                    .agent
                    .host
                    .cached_module_script_response(&requested)
                    .ok_or_else(|| {
                        HostError::Execution("prepared portable module response absent".into())
                    })?;
                match loaded {
                    Ok(loaded) => {
                        self.record.8.loaded = Some(
                            retain_parser_script_source(
                                Ok(loaded),
                                &page.dynamic_scripts.source_bytes,
                            )
                            .map_err(|failure| {
                                HostError::ScriptLoad(
                                    crate::script_load_failure::ScriptLoadFailure::Capacity(
                                        failure.to_string(),
                                    ),
                                )
                            })?,
                        )
                    }
                    Err(error) => {
                        page.diagnostics.push(script_diagnostic(error));
                        return self.finish_error(page, dom, parser_gate);
                    }
                }
            }
            self.record.8.graph_error = self.graph.as_ref().and_then(|graph| graph.error_token);
            self.record.8.graph_preloaded = true;
        }
        if !execution_allowed {
            return Ok(false);
        }
        if !self.execution_finished {
            if let Some(gate) = parser_gate {
                gate.resume();
            }
            #[cfg(test)]
            self.diagnostic("script-evaluate-enter");
            run_script_record_retained(
                &mut page.agent.host,
                dom,
                &mut page.resources,
                &self.record,
                &mut page.diagnostics,
                #[cfg(unix)]
                page.context.as_ref(),
                &mut self.execution,
            )?;
            #[cfg(test)]
            self.diagnostic("script-evaluate-return");
            if !self.execution.finished {
                return Ok(false);
            }
            self.execution_finished = true;
        }
        #[cfg(test)]
        xml_publication_failure(XmlPublicationFailure::AfterScriptExecution)?;
        parser_checkpoint(&mut page.agent.host, &mut page.diagnostics)?;
        if parser_script_owner_live(&page.agent.host, dom, &self.record) {
            page.agent.host.refresh_named_window_properties()?;
        }
        Ok(true)
    }
}

impl RustExecutableDocument {
    #[cfg(unix)]
    pub(super) fn queue_preparing_frame_stylesheet(
        &mut self,
        realm: RealmId,
        node: NodeId,
    ) -> Result<bool, HostError> {
        let Some(pending) = self
            .frame_script_preparations
            .iter_mut()
            .find(|pending| pending.load.realm == realm && pending.live(&self.agent.host))
        else {
            return Ok(false);
        };
        let eligible = {
            let dom = pending.dom.borrow();
            dom.node_document(node).ok().flatten() == Some(pending.load.identity.0)
                && dom.arena().is_css_stylesheet_owner(node).unwrap_or(false)
        };
        if !eligible {
            return Ok(false);
        }
        pending.queue_style(node, false)?;
        Ok(true)
    }
    pub(super) fn frame_scripts_pending(&self, realm: Option<RealmId>) -> bool {
        if self
            .ready_frame_jobs
            .iter()
            .any(|(parent, _, _)| realm.is_none_or(|realm| realm == *parent))
        {
            return true;
        }
        if self
            .frame_departures
            .keys()
            .any(|(parent, _)| realm.is_none_or(|realm| realm == *parent))
        {
            return true;
        }
        self.frame_script_preparations.iter().any(|pending| {
            realm.is_none_or(|realm| realm == pending.load.realm) && pending.live(&self.agent.host)
        })
    }
    pub(super) fn frame_scripts_runnable(&mut self) -> bool {
        self.frame_scripts_runnable_in_scope(false)
    }
    pub(super) fn frame_scripts_runnable_in_scope(&mut self, within_checkpoint: bool) -> bool {
        if self
            .frame_departure_delay()
            .is_some_and(|delay| delay.is_zero())
        {
            return true;
        }
        let previous = self.agent.host.active_realm();
        let ready = self.frame_script_preparations.iter_mut().any(|pending| {
            // The poller admits one slice per document in this page task.
            // Consumed slices remain ready for the next task, but cannot
            // make another pass of the current checkpoint progress.
            if within_checkpoint && pending.live(&self.agent.host) {
                let admitted = if pending.root_document {
                    self.parser_turn.can_claim()
                } else {
                    self.parser_turn.can_claim_frame(pending.load.realm)
                };
                if !admitted {
                    return false;
                }
            }
            pending.runnable(&mut self.agent.host)
        });
        let _ = self.agent.host.activate_realm(previous);
        ready
    }
    pub(super) fn poll_frame_script_preparations(&mut self) -> Result<(), HostError> {
        let _parser_turn = self.parser_turn.enter();
        if self.frame_script_preparations.is_empty() {
            return Ok(());
        }
        let previous = self.agent.host.active_realm();
        let result = (|| {
            // Round-robin documents and asynchronous jobs: a slow first frame
            // or async source cannot starve a ready sibling or ordered script.
            let count = self.frame_script_preparations.len().min(64);
            for _ in 0..count {
                let Some(mut pending) = self.frame_script_preparations.pop_front() else {
                    break;
                };
                if !pending.live(&self.agent.host) {
                    continue;
                }
                if self.agent.host.realm_is_parked(pending.load.realm) {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                if pending.lifecycle_active {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                if pending
                    .parser_retry
                    .is_some_and(|retry| retry > std::time::Instant::now())
                {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                let admitted = if pending.root_document {
                    self.parser_turn.claim()
                } else {
                    self.parser_turn.claim_frame(pending.load.realm)
                };
                if !admitted {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                if !pending.root_document && !self.pending_frame_navigation_restore.is_empty() {
                    match self.restore_pending_frame_before_scripts(pending.load.realm) {
                        Ok(Some(saved)) => pending.restored_form_state = Some(saved),
                        Ok(None) => {}
                        Err(error) => {
                            pending.parser_retry =
                                Some(std::time::Instant::now() + Duration::from_millis(25));
                            self.frame_script_preparations.push_back(pending);
                            return Err(error);
                        }
                    }
                    if self
                        .pending_frame_navigations
                        .contains_key(&(pending.parent, pending.frame))
                    {
                        continue;
                    }
                }
                if !pending.live(&self.agent.host) {
                    continue;
                }
                let setup = (|| {
                    #[cfg(test)]
                    if XML_SETUP_FAILURE_REALM.with(|realm| realm.get() == Some(pending.load.realm))
                    {
                        return Err(HostError::Execution(
                            "injected repeated realm setup failure".into(),
                        ));
                    }
                    #[cfg(test)]
                    xml_publication_failure(XmlPublicationFailure::BeforeSetup)?;
                    if !pending.root_document {
                        self.agent.host.sync_frame_viewport(
                            pending.parent,
                            pending.frame,
                            pending.load.realm,
                            true,
                        )?;
                    }
                    self.agent.host.activate_realm(pending.load.realm)
                })();
                if let Err(error) = setup {
                    pending.parser_retry =
                        Some(std::time::Instant::now() + Duration::from_millis(25));
                    self.frame_script_preparations.push_back(pending);
                    return Err(error);
                }
                pending.parser_retry = None;
                if let Err(error) = pending.poll_parser(self) {
                    if pending.parser_failed {
                        // Input failure stops the native load too, but is not
                        // an author stop() request whose cancellation is normal.
                        if pending.root_document {
                            self.observe_native_root_stop();
                        }
                        return Err(error);
                    }
                    if pending.live(&self.agent.host) {
                        pending.parser_retry =
                            Some(std::time::Instant::now() + Duration::from_millis(25));
                        self.frame_script_preparations.push_back(pending);
                        return Err(error);
                    }
                    continue;
                }
                if !pending.live(&self.agent.host) {
                    continue;
                }
                if pending.lifecycle_active {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                if pending
                    .parser_retry
                    .is_some_and(|retry| retry > std::time::Instant::now())
                {
                    self.frame_script_preparations.push_back(pending);
                    continue;
                }
                #[cfg(not(unix))]
                {
                    if let Err(error) = self.poll_dynamic_stylesheets() {
                        pending.parser_retry =
                            Some(std::time::Instant::now() + Duration::from_millis(25));
                        self.frame_script_preparations.push_back(pending);
                        return Err(error);
                    }
                    pending.portable_styles.retain(|(node, revision)| {
                        self.portable_stylesheet_pending(pending.load.realm, *node, *revision)
                    });
                    if !pending.portable_styles.is_empty() {
                        pending.parser_retry =
                            Some(std::time::Instant::now() + Duration::from_millis(25));
                    }
                }
                #[cfg(unix)]
                if let Err(error) = pending.poll_styles(self) {
                    pending.parser_retry =
                        Some(std::time::Instant::now() + Duration::from_millis(25));
                    self.frame_script_preparations.push_back(pending);
                    return Err(error);
                }
                // Discover async records in parser order. They may overtake
                // later work, but cannot resolve imports before an earlier
                // parser-blocking import map has been installed.
                for _ in 0..64 {
                    let asynchronous = pending.ordered.front().is_some_and(|job| {
                        !job.record.4
                            && !job.record.6
                            && (job.record.1.is_some() || job.record.2.as_deref() == Some("module"))
                    });
                    if !asynchronous {
                        break;
                    }
                    pending.asynchronous.push_back(
                        pending
                            .ordered
                            .pop_front()
                            .expect("discovered frame async script"),
                    );
                }
                if let Err(error) = pending.poll_deferred_preparations(self) {
                    pending.parser_retry =
                        Some(std::time::Instant::now() + Duration::from_millis(25));
                    self.frame_script_preparations.push_back(pending);
                    return Err(error);
                }
                if !pending.live(&self.agent.host) {
                    continue;
                }
                let scripts_blocked = match pending.current_styles_block_scripts(&mut self.agent.host) {
                    Ok(blocked) => blocked,
                    Err(error) => {
                        pending.parser_retry =
                            Some(std::time::Instant::now() + Duration::from_millis(25));
                        self.frame_script_preparations.push_back(pending);
                        return Err(error);
                    }
                };
                if let Some(mut job) = pending.ordered.pop_front() {
                    let execution_allowed = !scripts_blocked || (!job.record.4 && !job.record.6);
                    match job.poll(
                        self,
                        &pending.dom,
                        execution_allowed,
                        pending.live_parser.as_ref().map(|parser| &parser.parser),
                    ) {
                        Ok(true) => {
                            // The completed script can synchronously remove its
                            // frame. Drop that owner before resuming its tokenizer.
                            if !pending.live(&self.agent.host) {
                                continue;
                            }
                            // Script completion resumes its tokenizer before
                            // unrelated task sources. Spend only this turn's
                            // remaining byte budget; a new script stays queued
                            // for the next turn and slow input never blocks it.
                            if let Err(error) = pending.poll_html_parser(self) {
                                pending.parser_retry =
                                    Some(std::time::Instant::now() + Duration::from_millis(25));
                                self.frame_script_preparations.push_back(pending);
                                return Err(error);
                            }
                        }
                        Ok(false) => {
                            if job.execution.receipt.is_some() {
                                pending.parser_retry =
                                    Some(std::time::Instant::now() + Duration::from_millis(25));
                            }
                            pending.ordered.push_front(job);
                        }
                        Err(error) => {
                            pending.ordered.push_front(job);
                            pending.parser_retry =
                                Some(std::time::Instant::now() + Duration::from_millis(25));
                            self.frame_script_preparations.push_back(pending);
                            return Err(error);
                        }
                    }
                }
                if !pending.live(&self.agent.host) {
                    continue;
                }
                for _ in 0..pending.asynchronous.len().min(8) {
                    if let Err(error) = self.agent.host.activate_realm(pending.load.realm) {
                        pending.parser_retry =
                            Some(std::time::Instant::now() + Duration::from_millis(25));
                        self.frame_script_preparations.push_back(pending);
                        return Err(error);
                    }
                    let mut job = pending
                        .asynchronous
                        .pop_front()
                        .expect("bounded async frame queue");
                    match job.poll(self, &pending.dom, true, None) {
                        Ok(true) => {}
                        Ok(false) => {
                            if job.execution.receipt.is_some() {
                                pending.parser_retry =
                                    Some(std::time::Instant::now() + Duration::from_millis(25));
                            }
                            pending.asynchronous.push_back(job);
                        }
                        Err(error) => {
                            pending.asynchronous.push_back(job);
                            pending.parser_retry =
                                Some(std::time::Instant::now() + Duration::from_millis(25));
                            self.frame_script_preparations.push_back(pending);
                            return Err(error);
                        }
                    }
                    if !pending.live(&self.agent.host) {
                        break;
                    }
                }
                if !pending.live(&self.agent.host) {
                    continue;
                }
                let lifecycle = pending.live_parser.is_none()
                    && pending.xml_parser.is_none()
                    && pending.xml_completion.is_none()
                    && pending.parser_events.is_empty()
                    && pending.snapshot_events.is_none()
                    && pending.ordered.is_empty()
                    && pending.lifecycle_allowed
                    && !pending.lifecycle_queued;
                if lifecycle {
                    for child in pending.child_loads.drain(..) {
                        if let Some(batch) =
                            self.frame_script_preparations.iter_mut().find(|batch| {
                                batch.load.realm == child.realm
                                    && batch.load.identity == child.identity
                            })
                        {
                            batch.lifecycle_allowed = true;
                        }
                    }
                }
                let load = pending.load;
                let root_document = pending.root_document;
                let restored_forms = if lifecycle {
                    pending.restored_form_state.clone()
                } else {
                    None
                };
                if lifecycle {
                    pending.lifecycle_queued = true;
                    pending.lifecycle_active = true;
                }
                // Publish the async load barrier before dispatching DOMContentLoaded.
                // Its handlers can recursively checkpoint or retire this document.
                if lifecycle || pending.retains_work() {
                    self.frame_script_preparations.push_back(pending);
                }
                if lifecycle {
                    let lifecycle_result = (|| -> Result<(), HostError> {
                        #[cfg(test)]
                        xml_publication_failure(XmlPublicationFailure::BeforeLifecycle)?;
                        self.agent.host.activate_realm(load.realm)?;
                        if root_document {
                            self.resume_pending_root_form_history()?;
                            return self.finish_parser_lifecycle(Vec::new());
                        }
                        if let Some(saved) = restored_forms {
                            let owner = self
                                .frame_script_preparations
                                .iter_mut()
                                .find(|owner| {
                                    owner.load.realm == load.realm
                                        && owner.load.identity == load.identity
                                })
                                .ok_or_else(|| {
                                    HostError::Execution(
                                        "form restore parser owner unavailable".into(),
                                    )
                                })?;
                            self.agent.host.resume_form_history_in_realm(
                                load.realm,
                                &saved,
                                &mut owner.form_restore_admitted,
                            )?;
                            owner.restored_form_state = None;
                        }
                        self.refresh_embedded_intrinsics()?;
                        self.agent.host.set_current_script(None)?;
                        self.agent.host.prepare_document_images()?;
                        self.agent.host.drain_async_fetches(false)?;
                        if self.agent.host.automation_realm_identity(load.realm)
                            == Some(load.identity)
                        {
                            self.queue_document_load(load.realm)?;
                        }
                        Ok(())
                    })();
                    if let Some(owner) = self.frame_script_preparations.iter_mut().find(|owner| {
                        owner.load.realm == load.realm && owner.load.identity == load.identity
                    }) {
                        owner.lifecycle_active = false;
                        if lifecycle_result.is_err() {
                            owner.lifecycle_queued = false;
                            owner.parser_retry =
                                Some(std::time::Instant::now() + Duration::from_millis(25));
                        }
                    }
                    lifecycle_result?;
                    if let Some(index) = self.frame_script_preparations.iter().position(|owner| {
                        owner.load.realm == load.realm
                            && owner.load.identity == load.identity
                            && !owner.retains_work()
                    }) {
                        self.frame_script_preparations.remove(index);
                    }
                    self.flush_document_loads()?;
                }
            }
            Ok(())
        })();
        let restored = self.agent.host.activate_realm(previous);
        result.and(restored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_root_input_failure_releases_child_input_and_resource_reservation() {
        struct Input {
            failed: bool,
            dropped: Rc<Cell<usize>>,
        }
        impl crate::host::DocumentInput for Input {
            fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
                if self.failed {
                    std::task::Poll::Ready(Some(Err(HostError::ScriptLoad(
                        crate::script_load_failure::ScriptLoadFailure::Cancelled,
                    ))))
                } else {
                    std::task::Poll::Pending
                }
            }
        }
        impl Drop for Input {
            fn drop(&mut self) {
                self.dropped.set(self.dropped.get() + 1);
            }
        }
        let (mut page, child, child_dom, _, _) = frame_fixture();
        let dropped = Rc::new(Cell::new(0));
        let baseline = page
            .dynamic_scripts
            .source_bytes
            .load(std::sync::atomic::Ordering::SeqCst);
        let child_load = PreparedDocumentLoad {
            realm: child,
            identity: page.agent.host.automation_realm_identity(child).unwrap(),
        };
        let frame = page.frame_realms[&child].frame;
        let child_parser = LiveParserState::new(
            crate::host::DocumentStream {
                content_language: None,
                chunks: Box::new(Input {
                    failed: false,
                    dropped: dropped.clone(),
                }),
                content_type: Some("text/html".into()),
                encoding: Default::default(),
            },
            &child_dom,
            &mut page.agent.host,
        )
        .unwrap();
        let mut child_owner = FrameScriptPreparation::new(FrameScriptInput {
            load: child_load,
            parent: page.top_realm,
            frame,
            dom: child_dom,
            records: Vec::new(),
            lifecycle_allowed: true,
            _refresh_url: None,
            limits: &page.dynamic_scripts,
            host: &mut page.agent.host,
        })
        .unwrap();
        child_owner.install_html_parser(child_parser);
        child_owner._source_reservation =
            ScriptSourceReservation::reserve(&page.dynamic_scripts.source_bytes, 4096).unwrap();
        page.frame_script_preparations.push_back(child_owner);
        page.agent.host.activate_realm(page.top_realm).unwrap();
        let dom = page.agent.dom.clone();
        page.pending_parser = Some(
            LiveParserState::new(
                crate::host::DocumentStream {
                    content_language: None,
                    chunks: Box::new(Input {
                        failed: true,
                        dropped: dropped.clone(),
                    }),
                    content_type: Some("text/html".into()),
                    encoding: Default::default(),
                },
                &dom,
                &mut page.agent.host,
            )
            .unwrap(),
        );
        let identity = page
            .agent
            .host
            .automation_realm_identity(page.top_realm)
            .unwrap();
        page.pending_document_loads
            .insert(page.top_realm, (identity.1, identity.2));
        assert!(matches!(
            page.resume_pending_parser(),
            Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Cancelled
            ))
        ));
        assert_eq!(dropped.get(), 2);
        assert!(page.frame_script_preparations.is_empty());
        assert!(!page.pending_document_loads.contains_key(&page.top_realm));
        assert_eq!(
            page.dynamic_scripts
                .source_bytes
                .load(std::sync::atomic::Ordering::SeqCst),
            baseline
        );
        assert!(page.document_load_stopped);
        assert!(!page.document_load_complete());
    }

    #[test]
    fn repeated_setup_failure_rotates_behind_healthy_sibling() {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                XML_SETUP_FAILURE_REALM.with(|realm| realm.set(None));
            }
        }
        let (mut page, child, dom, node, _) = frame_fixture();
        let script = record(&page, &dom, node, "order.push('healthy')");
        page.agent.host.activate_realm(page.top_realm).unwrap();
        let load = PreparedDocumentLoad {
            realm: page.top_realm,
            identity: page
                .agent
                .host
                .automation_realm_identity(page.top_realm)
                .unwrap(),
        };
        let root = FrameScriptPreparation::root_html(
            load,
            page.agent.dom.clone(),
            &page.dynamic_scripts,
            &mut page.agent.host,
        )
        .unwrap();
        page.frame_script_preparations.push_back(root);
        page.agent.host.activate_realm(child).unwrap();
        enqueue(&mut page, child, dom, vec![script]);
        let _reset = Reset;
        XML_SETUP_FAILURE_REALM.with(|realm| realm.set(Some(page.top_realm)));
        assert!(page.poll_frame_script_preparations().is_err());
        // Simulate the failed owner's deadline already expiring. Progress
        // cannot depend on future turns running faster than its retry delay.
        for pending in &mut page.frame_script_preparations {
            pending.parser_retry = None;
        }
        assert!(page.poll_frame_script_preparations().is_err());
        page.agent.host.activate_realm(child).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "healthy"
        );
        for pending in &mut page.frame_script_preparations {
            pending.parser_retry = None;
        }
        assert!(page.poll_frame_script_preparations().is_err());
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "healthy",
            "completed sibling prefix cannot replay"
        );
        assert!(
            page.frame_script_preparations
                .iter()
                .any(|pending| pending.load.realm == page.top_realm)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unclosed_author_stream_does_not_hide_retained_network_script_work() {
        let mut page=RustExecutableDocument::load(r#"<iframe id=child srcdoc='<body>initial'></iframe><script>
            const child=document.getElementById('child').contentDocument;
            child.open();child.write('<body>unclosed');
        </script><iframe id=sibling srcdoc='<script>parent.healthySibling=true</script>'></iframe>"#).unwrap();
        assert!(!page.document_load_complete());
        assert!(!page.construction_pending());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(healthySibling)")
                .unwrap(),
            "true",
            "ordinary sibling work completes before returning the passive stream"
        );
        let frame = page.agent.dom.borrow().find_by_id("child").unwrap();
        let realm = *page
            .frame_realms
            .iter()
            .find(|(_, owner)| owner.frame == frame)
            .unwrap()
            .0;
        page.agent.host.activate_realm(realm).unwrap();
        page.agent.host.execute("const script=document.createElement('script');script.id='resource';script.type='application/json';document.body.appendChild(script)").unwrap();
        let dom = page.agent.host.active_dom().unwrap();
        let document = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let node = dom
            .borrow()
            .get_element_by_id(document, "resource")
            .unwrap()
            .unwrap();
        let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
        record.1 = Some("https://example.test/resource.js".into());
        record.2 = None;
        record.4 = false;
        let identity = record.7.unwrap();
        let owner = page
            .frame_script_preparations
            .iter_mut()
            .find(|owner| owner.load.realm == realm)
            .unwrap();
        let (sender, budget, ticket) = owner.inject_async_source(record, &page.dynamic_scripts);
        assert!(
            page.construction_pending(),
            "pending network receipt is not passive author input"
        );
        let result = retain_parser_script_source(
            Ok((
                "https://example.test/resource.js".into(),
                "globalThis.mixedResourceRan=true".into(),
                false,
                "no-referrer".into(),
            )),
            &budget,
        );
        sender
            .send(ParserResourceCompletion::Script {
                realm,
                stream_epoch: identity.stream_epoch,
                document_generation: identity.document_generation,
                document: identity.document,
                order: 0,
                record: ticket,
                asynchronous: true,
                result,
            })
            .unwrap();
        for _ in 0..16 {
            page.poll_context_events().unwrap();
            if !page.construction_pending() {
                break;
            }
        }
        assert!(!page.construction_pending());
        assert!(
            !page.document_load_complete(),
            "actual load still waits for document.close"
        );
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(mixedResourceRan)")
                .unwrap(),
            "true"
        );
    }

    fn frame_fixture() -> (
        RustExecutableDocument,
        RealmId,
        SharedDomRuntime,
        NodeId,
        NodeId,
    ) {
        let mut page=RustExecutableDocument::load_with_environment(
            r#"<body><iframe id=f srcdoc="<script id=a type=application/json></script><script id=b type=application/json></script>"></iframe>"#,
            &BTreeMap::new(),None,Some(Url::parse("https://example.test/root.html").unwrap())).unwrap();
        let realm = *page.frame_realms.keys().next().unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        let dom = page.agent.host.active_dom().unwrap();
        let root = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let a = dom.borrow().query_selector(root, "#a").unwrap().unwrap();
        let b = dom.borrow().query_selector(root, "#b").unwrap().unwrap();
        page.agent.host.execute("globalThis.order=[]").unwrap();
        (page, realm, dom, a, b)
    }
    #[test]
    fn support_decoder_terminal_rejects_retired_actual_document_slot() {
        use oryn_js::v8_bindings::NativeSupportCustody;
        let (page,realm,_,_,_)=frame_fixture();
        let state=page.agent.host.retained_binding_state(realm).unwrap();
        let context=crate::runtime::Browser::new().new_context();
        let native_page=context.new_page();
        let client=SharedContextClient::direct(context,native_page.page_id());
        let lease=client.reserve_frame_response(native_page.page_id()).unwrap();
        let body=frame_response_stream::FrameResponseBody{progress:None,client,page_id:native_page.page_id(),id:7,lease};
        let budget=Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let creator=page.agent.host.native_creator_for_realm(realm).unwrap();
        let custody=support_native_document::NativeSupportDocument::install(&state,body,creator,&budget).unwrap();
        assert!(state.native_support_custody().is_some());
        custody.retire();
        assert!(frame_response_stream::test_support_decoder_eof(&state,custody.clone()).is_err(),"retired decoder cannot publish a terminal acknowledgement");
        state.retire_execution();
        assert!(state.native_support_custody().is_none(),"retirement clears actual installed custody");
        drop(custody);
        assert_eq!(budget.load(std::sync::atomic::Ordering::Acquire),0,"owned Creator and Document reservation release");
    }
    #[test]
    fn console_collection_drains_child_and_root_once_and_restores_selection() {
        let (mut page, realm, _, _, _) = frame_fixture();
        page.agent
            .host
            .execute("console.error('child original')")
            .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent
            .host
            .execute("console.error('root original')")
            .unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        let entries = page.take_console_entries().unwrap();
        assert_eq!(page.agent.host.active_realm(), realm);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.message == "child original")
                .count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.message == "root original")
                .count(),
            1
        );
        assert!(page.take_console_entries().unwrap().is_empty());
        assert_eq!(page.agent.host.active_realm(), realm);
    }

    #[test]
    fn console_collection_failure_preserves_other_realms_and_marks_incomplete() {
        let (mut page, realm, _, _, _) = frame_fixture();
        page.agent
            .host
            .execute("__oryn_take_console=()=>{throw Error('child drain failure')}")
            .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent
            .host
            .execute("console.error('root retained')")
            .unwrap();
        let entries = page.take_console_entries().unwrap();
        assert_eq!(page.agent.host.active_realm(), page.top_realm);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.message == "root retained")
                .count(),
            1
        );
        assert!(page.diagnostics.iter().any(|entry| {
            entry.capability == "diagnostics.collection"
                && entry
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("child drain failure"))
        }));
        page.agent.host.activate_realm(realm).unwrap();
        page.agent
            .host
            .execute("__oryn_take_console=()=>[]")
            .unwrap();
        assert!(page.take_console_entries().unwrap().is_empty());
        assert_eq!(page.agent.host.active_realm(), realm);
    }

    #[test]
    fn console_collection_restore_failure_retains_drained_records_for_retry() {
        let (mut page, realm, _, _, _) = frame_fixture();
        page.agent
            .host
            .execute("console.error('retained across restore failure')")
            .unwrap();
        let failed = page.take_console_entries_with_restore(|_, _| {
            Err(HostError::Execution("injected restore failure".into()))
        });
        assert!(failed.is_err());
        assert!(page.diagnostics.iter().any(|entry| {
            entry.capability == "diagnostics.collection"
                && entry
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("injected restore failure"))
        }));
        page.agent.host.activate_realm(realm).unwrap();
        let entries = page.take_console_entries().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.message == "retained across restore failure")
                .count(),
            1
        );
        assert!(page.take_console_entries().unwrap().is_empty());
        assert_eq!(page.agent.host.active_realm(), realm);
    }

    fn record(
        page: &RustExecutableDocument,
        dom: &SharedDomRuntime,
        node: NodeId,
        source: &str,
    ) -> ScriptRecord {
        let mut record = capture_parser_script_node(&page.agent.host, dom, node).unwrap();
        record.2 = None;
        record.3 = source.into();
        record.4 = true;
        record.6 = false;
        record
    }
    fn enqueue(
        page: &mut RustExecutableDocument,
        realm: RealmId,
        dom: SharedDomRuntime,
        records: Vec<ScriptRecord>,
    ) {
        let owner = page.frame_realms[&realm].clone();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        // Injected parser records begin a fresh fixture publication cycle;
        // the real synchronous load has already consumed the original receipt.
        page.pending_document_loads.remove(&realm);
        page.document_publications.remove(&realm);
        page.agent
            .host
            .set_document_readiness(oryn_dom::DocumentReadyState::Loading)
            .unwrap();
        let pending = FrameScriptPreparation::new(FrameScriptInput {
            load: PreparedDocumentLoad { realm, identity },
            parent: owner.parent_realm,
            frame: owner.frame,
            dom,
            records,
            lifecycle_allowed: true,
            _refresh_url: None,
            limits: &page.dynamic_scripts,
            host: &mut page.agent.host,
        })
        .unwrap();
        page.frame_script_preparations.push_back(pending);
    }

    #[cfg(unix)]
    #[test]
    fn native_parked_frame_parser_keeps_jobs_without_runnable_work_until_restore() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let handle = context.new_page();
        let url = Url::parse("https://example.test/root.html").unwrap();
        context
            .install_document_security(handle.page_id(), url.to_string(), &[])
            .unwrap();
        let client = crate::context_service::SharedContextClient::direct(context, handle.page_id());
        let mut page = RustExecutableDocument::load_with_context_and_limits(
            r#"<body><iframe srcdoc="<script id=a type=application/json></script>"></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(url),
            256 * 1024 * 1024,
            (client, handle.page_id(), None),
        )
        .unwrap();
        let realm = *page.frame_realms.keys().next().unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        let dom = page.agent.host.active_dom().unwrap();
        let document = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let node = dom
            .borrow()
            .query_selector(document, "#a")
            .unwrap()
            .unwrap();
        page.agent.host.execute("globalThis.order=[]").unwrap();
        let script = record(&page, &dom, node, "order.push('restored')");
        enqueue(&mut page, realm, dom, vec![script]);
        let root = page.top_realm;
        assert!(
            page.agent
                .host
                .park_document_authority(root)
                .unwrap()
                .is_some()
        );
        assert!(page.agent.host.realm_is_parked(realm));
        assert!(!page.frame_scripts_runnable());
        page.poll_frame_script_preparations().unwrap();
        assert!(page.frame_scripts_pending(Some(realm)));
        assert_eq!(
            page.frame_script_preparations
                .iter()
                .find(|pending| pending.load.realm == realm)
                .unwrap()
                .ordered
                .len(),
            1
        );
        page.agent.host.restore_document_authority(root).unwrap();
        assert!(page.frame_scripts_runnable());
        for _ in 0..8 {
            page.poll_frame_script_preparations().unwrap();
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "restored"
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn portable_classic_source_is_retained_before_execution_and_cancelled_on_drop() {
        let (mut page, realm, dom, node, _) = frame_fixture();
        let url = "https://example.test/classic.js";
        page.agent.host.set_module_sources(&BTreeMap::from([(
            url.into(),
            "order.push('captured')".into(),
        )]));
        let mut script = record(&page, &dom, node, "");
        script.1 = Some(url.into());
        let mut job = FrameScriptJob::from_completion(&dom, script).unwrap();
        assert!(!job.poll(&mut page, &dom, false, None).unwrap());
        let cancel = job.portable_source.as_ref().unwrap().cancel.clone();
        dom.borrow_mut()
            .set_attribute(node, "src", "changed.js")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.record.8.loaded.is_none() {
            job.poll(&mut page, &dom, false, None).unwrap();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            cancel.load(std::sync::atomic::Ordering::Acquire),
            "consumed source owner released"
        );
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length.toString()")
                .unwrap(),
            "0"
        );
        assert!(job.poll(&mut page, &dom, true, None).unwrap());
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "captured"
        );
        let mut second = record(&page, &dom, node, "");
        second.1 = Some(url.into());
        let source = PortableScriptSource::start(&mut page, &second)
            .unwrap()
            .unwrap();
        let cancel = source.cancel.clone();
        drop(source);
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }

    #[cfg(not(unix))]
    #[test]
    fn portable_parser_module_prepares_graph_before_execution_and_releases_owner() {
        let (mut page, realm, dom, node, _) = frame_fixture();
        page.agent.host.set_module_sources(&BTreeMap::from([(
            "https://example.test/dependency.js".into(),
            "export const value=42".into(),
        )]));
        let mut script = record(
            &page,
            &dom,
            node,
            "import {value} from './dependency.js';order.push(value)",
        );
        script.2 = Some("module".into());
        script.8.base_url = Some("https://example.test/root.html".into());
        let mut job = FrameScriptJob::from_completion(&dom, script).unwrap();
        assert!(job.preparation_pending());
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.preparation_pending() {
            job.poll(&mut page, &dom, false, None).unwrap();
            assert!(
                Instant::now() < deadline,
                "portable module graph did not complete"
            );
            std::thread::yield_now();
        }
        assert!(job.graph.is_some());
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length.toString()")
                .unwrap(),
            "0"
        );
        assert!(job.poll(&mut page, &dom, true, None).unwrap());
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "42"
        );
        assert!(job.poll(&mut page, &dom, true, None).unwrap());
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "42"
        );
    }

    #[test]
    fn stylesheet_barrier_failure_retains_original_parser_jobs_and_load_fence() {
        let (mut page, realm, dom, a, b) = frame_fixture();
        let first = record(&page, &dom, a, "order.push('first')");
        let second = record(&page, &dom, b, "order.push('second')");
        enqueue(&mut page, realm, dom, vec![first, second]);
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeStylesBarrier)));
        assert!(page.poll_frame_script_preparations().is_err());
        assert!(page.frame_scripts_pending(Some(realm)));
        let owner = page
            .frame_script_preparations
            .iter()
            .find(|owner| owner.load.realm == realm)
            .unwrap();
        assert_eq!(owner.load.identity, identity);
        assert_eq!(owner.ordered.len(), 2);
        assert!(!owner.lifecycle_queued);
        assert!(owner.parser_retry.is_some());
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            ""
        );
        for _ in 0..8 {
            for owner in &mut page.frame_script_preparations {
                owner.parser_retry = None;
            }
            page.poll_frame_script_preparations().unwrap();
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "first,second"
        );
    }

    #[test]
    fn parser_record_round_trip_adoption_uses_current_node_and_original_document() {
        let (mut page, realm, dom, a, _) = frame_fixture();
        let record = record(&page, &dom, a, "order.push('once')");
        let original = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let foreign = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let foreign_document = foreign.borrow().document();
        let mut outgoing =
            DomRuntime::prepare_subtree_migration(&dom, &foreign, a, foreign_document).unwrap();
        let moved = DomRuntime::commit_subtree_migration(&dom, &foreign, &mut outgoing).unwrap();
        assert!(!parser_script_identity_live(
            &page.agent.host,
            &dom,
            &record
        ));
        assert!(parser_script_owner_live(&page.agent.host, &dom, &record));
        let mut incoming =
            DomRuntime::prepare_subtree_migration(&foreign, &dom, moved.root, original).unwrap();
        let restored = DomRuntime::commit_subtree_migration(&foreign, &dom, &mut incoming).unwrap();
        assert_ne!(a, restored.root);
        assert_eq!(script_record_node(&record), restored.root);
        assert!(parser_script_identity_live(&page.agent.host, &dom, &record));
        enqueue(&mut page, realm, dom, vec![record]);
        for _ in 0..8 {
            page.poll_frame_script_preparations().unwrap();
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "once"
        );
    }

    #[cfg(unix)]
    #[test]
    fn transferred_async_source_error_retains_admission_without_executing_body() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.errors=0;document.getElementById('a').addEventListener('error',()=>errors++)").unwrap();
        let mut record = record(&page, &dom, node, "order.push('must-not-run')");
        record.1 = Some("https://example.test/missing.js".into());
        let identity = record.7.unwrap();
        let mut scheduler = ParserResourceScheduler::new();
        let completion = ParserResourceCompletion::Script {
            realm: identity.realm,
            stream_epoch: identity.stream_epoch,
            document_generation: identity.document_generation,
            document: identity.document,
            order: 0,
            record,
            asynchronous: true,
            result: Err(ParserScriptFailure::Fetch {
                message: "missing source".into(),
                source_bytes: scheduler.source_bytes.clone(),
            }),
        };
        scheduler
            .apply_completion(crate::executable::ParserCompletionInput {
                completion,
                live_dom: &dom,
                host: &mut page.agent.host,
                resources: &mut page.resources,
                deferred: &mut BTreeMap::new(),
                diagnostics: &mut page.diagnostics,
                context: None,
            })
            .unwrap();
        FAIL_PARSER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            scheduler
                .poll_async_modules(
                    &dom,
                    &mut page.agent.host,
                    &mut page.resources,
                    &mut page.diagnostics,
                    None
                )
                .is_err()
        );
        assert_eq!(scheduler.async_executions.len(), 1);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length+'|'+errors")
                .unwrap(),
            "0|0"
        );
        scheduler
            .poll_async_modules(
                &dom,
                &mut page.agent.host,
                &mut page.resources,
                &mut page.diagnostics,
                None,
            )
            .unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length+'|'+errors")
                .unwrap(),
            "0|1"
        );
        assert!(scheduler.async_executions.is_empty());
    }

    #[test]
    fn invalid_script_source_is_rejected_before_any_module_graph_admission() {
        for kind in [None, Some("module"), Some("importmap")] {
            let (mut page, _, dom, node, _) = frame_fixture();
            page.agent.host.execute("globalThis.errors=0;document.getElementById('a').addEventListener('error',()=>errors++)").unwrap();
            let mut record = record(&page, &dom, node, "order.push('forbidden')");
            record.1 = Some(String::new());
            record.2 = kind.map(str::to_owned);
            let mut job = FrameScriptJob::from_completion(
                &dom,
                record,
                #[cfg(unix)]
                None,
                #[cfg(unix)]
                None,
            )
            .unwrap();
            assert!(job.poll(&mut page, &dom, true, None).unwrap());
            #[cfg(unix)]
            {
                assert!(job.graph.is_none());
                assert!(job.source.is_none());
            }
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("order.length+'|'+errors")
                    .unwrap(),
                "0|1"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn resource_materialization_defers_error_to_retained_job() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.errors=0;document.getElementById('a').addEventListener('error',()=>errors++)").unwrap();
        let mut record = record(&page, &dom, node, "order.push('must-not-run')");
        record.1 = Some("https://example.test/missing.js".into());
        let identity = record.7.unwrap();
        let completion = ParserResourceCompletion::Script {
            realm: identity.realm,
            stream_epoch: identity.stream_epoch,
            document_generation: identity.document_generation,
            document: identity.document,
            order: 0,
            record,
            asynchronous: false,
            result: Err(ParserScriptFailure::Fetch {
                message: "missing".into(),
                source_bytes: Default::default(),
            }),
        };
        let mut deferred = BTreeMap::new();
        apply_parser_resource_completion(
            completion,
            &dom,
            &mut page.agent.host,
            &mut page.resources,
            &mut deferred,
            &mut page.diagnostics,
            None,
        )
        .unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("String(errors)").unwrap(),
            "0"
        );
        let record = deferred.remove(&0).unwrap();
        assert!(record.8.failed);
        let mut job = FrameScriptJob::from_completion(&dom, record, None, None).unwrap();
        FAIL_PARSER_COMPLETION.with(|flag| flag.set(true));
        assert!(job.poll(&mut page, &dom, true, None).is_err());
        assert!(job.poll(&mut page, &dom, true, None).unwrap());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length+'|'+errors")
                .unwrap(),
            "0|1"
        );
    }

    #[test]
    fn compatibility_eof_retains_body_progress_on_completion_failure() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<script id=pending type=application/json>{}</script>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/").unwrap()),
        )
        .unwrap();
        let dom = page.agent.dom.clone();
        let node = page.node_by_id("pending").unwrap();
        let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
        record.2 = None;
        record.3 = "globalThis.runs=(globalThis.runs||0)+1".into();
        page.pending_parser_completion = Some(PendingParserCompletion {
            job: None,
            identity: capture_live_parser_identity(&page.agent.host, &dom).unwrap(),
            script_leases: retain_parser_scripts(&dom, &[record.clone()]).unwrap(),
            scripts: vec![record].into(),
            initial_frame_realms: Vec::new(),
            graph: None,
            #[cfg(unix)]
            source_load: None,
        });
        FAIL_PARSER_COMPLETION.with(|flag| flag.set(true));
        assert!(page.resume_parser_completion().is_err());
        assert!(
            page.pending_parser_completion
                .as_ref()
                .unwrap()
                .job
                .is_some()
        );
        assert_eq!(
            page.agent.host.evaluate_string("String(runs)").unwrap(),
            "1"
        );
        assert!(page.resume_parser_completion().unwrap());
        assert!(
            page.pending_parser_completion
                .as_ref()
                .unwrap()
                .scripts
                .is_empty()
        );
        assert_eq!(
            page.agent.host.evaluate_string("String(runs)").unwrap(),
            "1"
        );
    }

    #[test]
    fn old_script_receipt_guard_cannot_cancel_replacement_stream_publication() {
        let (mut page, realm, _, node, _) = frame_fixture();
        let (_, old) = page
            .agent
            .host
            .own_script_completion_publication(node, true, false)
            .unwrap();
        page.agent.host.execute("(()=>{const decoy=parent.document.createElement('div');decoy.id='replacement';parent.document.body.append(decoy)})()").unwrap();
        page.agent.host.execute("document.open();document.write('<script id=replacement type=application/json></script>');document.close()").unwrap();
        // Closing hands the parser to the native page scheduler. Finish that
        // publication before creating a receipt for its replacement script.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let node = loop {
            page.poll_context_events().unwrap();
            page.agent.host.activate_realm(realm).unwrap();
            let dom = page.agent.host.active_dom().unwrap();
            let document = page.agent.host.automation_realm_identity(realm).unwrap().0;
            let replacement = dom
                .borrow()
                .get_element_by_id(document, "replacement")
                .unwrap();
            if let Some(node) = replacement {
                assert!(
                    dom.borrow()
                        .is_element(node, "http://www.w3.org/1999/xhtml", "script")
                );
                break node;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "replacement parser did not publish its script: {:?}",
                page.diagnostics
            );
            std::thread::yield_now();
        };
        let (receipt, current) = page
            .agent
            .host
            .own_script_completion_publication(node, true, false)
            .unwrap();
        drop(old);
        assert!(
            page.agent
                .host
                .resume_native_history_publication(receipt)
                .unwrap()
        );
        drop(current);
    }

    #[test]
    fn dropping_script_progress_releases_its_native_event_receipt() {
        let (mut page, _, _, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.loads=0;document.getElementById('a').addEventListener('load',()=>loads++)").unwrap();
        let (receipt, owner) = page
            .agent
            .host
            .own_script_completion_publication(node, true, false)
            .unwrap();
        let progress = ParserScriptProgress {
            receipt: Some(receipt),
            receipt_owner: Some(owner),
            retained: true,
            ..Default::default()
        };
        assert_eq!(
            page.agent.host.evaluate_string("String(loads)").unwrap(),
            "1"
        );
        drop(progress);
        assert!(
            page.agent
                .host
                .resume_native_history_publication(receipt)
                .is_err(),
            "canceled native consumer must release receipt bookkeeping"
        );
        assert_eq!(
            page.agent.host.evaluate_string("String(loads)").unwrap(),
            "1"
        );
    }

    #[cfg(unix)]
    #[test]
    fn transferred_async_script_keeps_completion_progress_after_body() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.loads=0;document.getElementById('a').addEventListener('load',()=>loads++)").unwrap();
        let mut record = record(&page, &dom, node, "order.push('once')");
        record.1 = Some("https://example.test/async.js".into());
        record.4 = false;
        record.8.loaded = Some(
            retain_parser_script_source(
                Ok((
                    record.1.clone().unwrap(),
                    record.3.policy_text().into_owned(),
                    false,
                    "no-referrer".into(),
                )),
                &Default::default(),
            )
            .unwrap(),
        );
        page.resources.insert(
            record.1.clone().unwrap(),
            record.3.policy_text().into_owned(),
        );
        let mut scheduler = ParserResourceScheduler::new();
        scheduler.async_executions.push_back(ParserAsyncExecution {
            record,
            progress: ParserScriptProgress {
                retained: true,
                ..Default::default()
            },
            _graph: None,
        });
        FAIL_PARSER_COMPLETION.with(|flag| flag.set(true));
        assert!(
            scheduler
                .poll_async_modules(
                    &dom,
                    &mut page.agent.host,
                    &mut page.resources,
                    &mut page.diagnostics,
                    None
                )
                .is_err()
        );
        assert_eq!(scheduler.async_executions.len(), 1);
        assert!(scheduler.has_ready_completion());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.join(',')+'|'+loads")
                .unwrap(),
            "once|0"
        );
        scheduler
            .poll_async_modules(
                &dom,
                &mut page.agent.host,
                &mut page.resources,
                &mut page.diagnostics,
                None,
            )
            .unwrap();
        assert!(scheduler.async_executions.is_empty());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.join(',')+'|'+loads")
                .unwrap(),
            "once|1"
        );
    }

    #[test]
    fn admitted_load_receipt_survives_target_adoption() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.loads=[];globalThis.other=document.implementation.createHTMLDocument('other');document.getElementById('a').addEventListener('load',e=>{loads.push('first');other.adoptNode(e.target)});document.getElementById('a').addEventListener('load',()=>loads.push('second'))").unwrap();
        let mut record = record(
            &page,
            &dom,
            node,
            "globalThis.bodies=(globalThis.bodies||0)+1",
        );
        record.1 = Some("https://example.test/adopt-load.js".into());
        let (receipt, owner) = page
            .agent
            .host
            .own_script_completion_publication(node, true, false)
            .unwrap();
        assert!(!parser_script_identity_live(
            &page.agent.host,
            &dom,
            &record
        ));
        assert!(parser_script_owner_live(&page.agent.host, &dom, &record));
        let mut progress = ParserScriptProgress {
            entered: true,
            completion: Some(true),
            retained: true,
            receipt: Some(receipt),
            receipt_owner: Some(owner),
            ..Default::default()
        };
        complete_parser_script(&mut page.agent.host, &dom, &record, &mut progress).unwrap();
        assert!(progress.finished);
        assert!(progress.receipt.is_none());
        assert_eq!(
            page.agent.host.evaluate_string("loads.join(',')").unwrap(),
            "first,second"
        );
    }

    #[test]
    fn load_listener_document_replacement_finishes_old_script_without_stale_receipt_resume() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("document.getElementById('a').addEventListener('load',()=>{globalThis.loads=(globalThis.loads||0)+1;document.open();document.write('<p id=new>replacement</p>');document.close()})").unwrap();
        let mut record = record(
            &page,
            &dom,
            node,
            "globalThis.bodies=(globalThis.bodies||0)+1",
        );
        record.1 = Some("https://example.test/replace.js".into());
        record.8.loaded = Some(
            retain_parser_script_source(
                Ok((
                    record.1.clone().unwrap(),
                    record.3.policy_text().into_owned(),
                    false,
                    "no-referrer".into(),
                )),
                &Default::default(),
            )
            .unwrap(),
        );
        page.resources.insert(
            record.1.clone().unwrap(),
            record.3.policy_text().into_owned(),
        );
        let mut progress = ParserScriptProgress {
            retained: true,
            ..Default::default()
        };
        run_script_record_retained(
            &mut page.agent.host,
            &dom,
            &mut page.resources,
            &record,
            &mut page.diagnostics,
            #[cfg(unix)]
            None,
            &mut progress,
        )
        .unwrap();
        assert!(progress.finished);
        assert!(progress.receipt.is_none());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("bodies+'|'+loads+'|'+document.getElementById('new').textContent")
                .unwrap(),
            "1|1|replacement"
        );
    }

    #[test]
    fn parser_body_adoption_suppresses_old_document_load_publication() {
        let (mut page, _, dom, node, _) = frame_fixture();
        page.agent.host.execute("globalThis.loads=0;document.getElementById('a').addEventListener('load',()=>loads++)").unwrap();
        let source = "order.push('body');globalThis.foreign=document.implementation.createHTMLDocument('foreign');foreign.adoptNode(document.currentScript)";
        let mut record = record(&page, &dom, node, source);
        record.1 = Some("https://example.test/adopt.js".into());
        record.8.loaded = Some(
            retain_parser_script_source(
                Ok((
                    record.1.clone().unwrap(),
                    record.3.policy_text().into_owned(),
                    false,
                    "no-referrer".into(),
                )),
                &Default::default(),
            )
            .unwrap(),
        );
        page.resources
            .insert(record.1.clone().unwrap(), source.into());
        let mut progress = ParserScriptProgress {
            retained: true,
            ..Default::default()
        };
        run_script_record_retained(
            &mut page.agent.host,
            &dom,
            &mut page.resources,
            &record,
            &mut page.diagnostics,
            #[cfg(unix)]
            None,
            &mut progress,
        )
        .unwrap();
        assert!(progress.finished);
        assert!(progress.receipt.is_none());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.join(',')+'|'+loads")
                .unwrap(),
            "body|0"
        );
        assert!(parser_script_owner_live(&page.agent.host, &dom, &record));
        assert!(!parser_script_identity_live(
            &page.agent.host,
            &dom,
            &record
        ));
    }

    #[cfg(unix)]
    #[test]
    fn admitted_parser_fetch_survives_temporary_adoption_but_execution_checks_destination() {
        use std::sync::{Mutex, mpsc};
        struct Held {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl crate::network::NetworkBroker for Held {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: b"globalThis.runs=(globalThis.runs||0)+1".to_vec(),
                })
            }
        }
        for return_before_completion in [true, false] {
            let (started, observed) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let mut page = RustExecutableDocument::load_with_environment(
                "<script id=pending type=application/json>{}</script>",
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(Held {
                    started,
                    release: Mutex::new(blocked),
                })),
                Some(Url::parse("https://example.test/").unwrap()),
            )
            .unwrap();
            let dom = page.agent.dom.clone();
            let node = page.node_by_id("pending").unwrap();
            let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
            record.1 = Some("held.js".into());
            record.2 = None;
            let original = record.7.unwrap().document;
            let context = ParserFetchContext::for_script(&page.agent.host, &dom, &record).unwrap();
            let mut scheduler = ParserResourceScheduler::with_page_limits(&page.dynamic_scripts);
            scheduler
                .schedule_script_auto(
                    None,
                    0,
                    record.clone(),
                    false,
                    context.clone(),
                    &mut page.agent.host,
                )
                .unwrap();
            observed.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut job = FrameScriptJob {
                _inline_reservation: None,
                execution: ParserScriptProgress {
                    retained: true,
                    ..Default::default()
                },
                execution_finished: false,
                #[cfg(not(unix))]
                portable_source: None,
                source_completion: None,
                _lease: ParserScriptLease::new(&dom, node).unwrap(),
                record,
                source: Some(scheduler),
                graph: None,
            };
            let foreign = Rc::new(RefCell::new(
                html::parse("<body></body>").into_runtime().unwrap(),
            ));
            let foreign_document = foreign.borrow().document();
            let mut outgoing =
                DomRuntime::prepare_subtree_migration(&dom, &foreign, node, foreign_document)
                    .unwrap();
            let moved =
                DomRuntime::commit_subtree_migration(&dom, &foreign, &mut outgoing).unwrap();
            assert!(parser_script_owner_live(
                &page.agent.host,
                &dom,
                &job.record
            ));
            assert!(!parser_script_identity_live(
                &page.agent.host,
                &dom,
                &job.record
            ));
            assert!(
                !job.poll(&mut page, &dom, true, None).unwrap(),
                "pending fetch must retain the admitted owner while adopted"
            );
            assert_eq!(
                ParserFetchContext::for_script(&page.agent.host, &dom, &job.record)
                    .unwrap()
                    .initiator_url,
                context.initiator_url
            );
            if return_before_completion {
                let mut incoming =
                    DomRuntime::prepare_subtree_migration(&foreign, &dom, moved.root, original)
                        .unwrap();
                DomRuntime::commit_subtree_migration(&foreign, &dom, &mut incoming).unwrap();
            }
            release.send(()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !job.poll(&mut page, &dom, true, None).unwrap() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("String(globalThis.runs||0)")
                    .unwrap(),
                if return_before_completion { "1" } else { "0" }
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn decoded_http_frame_prefix_enters_child_parser_before_terminal_chunk() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ContextTransport,
        };
        struct Transport {
            reads: Arc<std::sync::atomic::AtomicUsize>,
            released: std::sync::mpsc::Sender<()>,
        }
        impl ContextTransport for Transport {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::HttpStreamRead {
                        stream_id: 7,
                        creator: Some(_),
                        ..
                    } => {
                        let first =
                            self.reads.fetch_add(1, std::sync::atomic::Ordering::AcqRel) == 0;
                        let bytes = if first {
                            b"<script>globalThis.prefixRan=true</script>".as_slice()
                        } else {
                            b"<p id=tail>tail</p>".as_slice()
                        };
                        Ok(ContextServiceResponse::HttpStreamChunk {
                            encoded_body_size: None,
                            stream_id: 7,
                            body_base64: base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                bytes,
                            ),
                            done: !first,
                        })
                    }
                    ContextServiceRequest::ReleaseFrameResponseStream { stream_id: 7, .. } => {
                        let _ = self.released.send(());
                        Ok(ContextServiceResponse::NetworkResultReleased)
                    }
                    _ => Err(crate::RuntimeError::ContextService(
                        "unexpected frame stream request".into(),
                    )),
                }
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let (released, release) = std::sync::mpsc::channel();
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = reads.clone();
        let server = std::thread::spawn(move || {
            crate::context_service::serve_context_transport(
                parent,
                Arc::new(Transport {
                    reads: observed,
                    released,
                }),
            )
        });
        let client = crate::context_service::SharedContextClient::new(
            crate::context_service::IpcContextClient::new(worker),
        );
        let (mut page, realm, dom, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        dom.borrow_mut().clear_document_in(identity.0).unwrap();
        let lease = client
            .reserve_frame_response(oryn_common::v3::PageId(1))
            .unwrap();
        lease.attach(7, "native-frame-policy");
        let body = frame_response_stream::FrameResponseBody {
            progress: None,
            client,
            page_id: oryn_common::v3::PageId(1),
            id: 7,
            lease,
        };
        let stream = body
            .decoded(
                page.agent.host.native_creator_for_realm(realm).unwrap(),
                Some("text/html; charset=utf-8".into()),
                None,
                &page.dynamic_scripts.source_bytes,
                None,
                None,
            )
            .unwrap();
        let mut pending = FrameScriptPreparation::new(FrameScriptInput {
            load: PreparedDocumentLoad { realm, identity },
            parent: owner.parent_realm,
            frame: owner.frame,
            dom: dom.clone(),
            records: vec![],
            lifecycle_allowed: true,
            _refresh_url: None,
            limits: &page.dynamic_scripts,
            host: &mut page.agent.host,
        })
        .unwrap();
        pending
            .start_document_stream(stream, &mut page.agent.host, &page.dynamic_scripts)
            .unwrap();
        page.frame_script_preparations.push_back(pending);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            page.poll_frame_script_preparations().unwrap();
            page.agent.host.activate_realm(realm).unwrap();
            if page
                .agent
                .host
                .evaluate_string("String(globalThis.prefixRan)")
                .unwrap()
                == "true"
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            dom.borrow()
                .query_selector(identity.0, "#tail")
                .unwrap()
                .is_none()
        );
        assert!(page.frame_scripts_pending(Some(realm)));
        page.stop_pending_document_load().unwrap();
        release
            .recv_timeout(Duration::from_secs(3))
            .expect("stop releases retained native response");
        assert!(reads.load(std::sync::atomic::Ordering::Acquire) >= 1);
        drop(page);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
    }

    #[test]
    fn frame_preparation_retains_ordered_records_and_document_open_cancels_suffix() {
        let (mut page, realm, dom, a, b) = frame_fixture();
        let first = record(
            &page,
            &dom,
            a,
            "order.push('first');document.open();document.write('<p>replacement</p>');document.close()",
        );
        let second = record(&page, &dom, b, "order.push('obsolete')");
        enqueue(&mut page, realm, dom.clone(), vec![first, second]);
        assert!(dom.borrow().script_already_started(b).unwrap());
        page.poll_frame_script_preparations().unwrap();
        assert!(!page.frame_scripts_pending(None));
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "first"
        );
    }

    #[cfg(unix)]
    #[test]
    fn deferred_preparation_readies_later_source_without_overtaking_held_head() {
        let (mut page, realm, dom, a, b) = frame_fixture();
        enqueue(&mut page, realm, dom.clone(), vec![]);
        let mut owner = page.frame_script_preparations.pop_back().unwrap();
        owner.lifecycle_allowed = false;
        let mut producers = Vec::new();
        for (node, name) in [(a, "first"), (b, "second")] {
            let mut script = record(&page, &dom, node, "");
            script.1 = Some(format!("https://example.test/{name}.js"));
            script.4 = false;
            script.6 = true;
            let identity = script.7.unwrap();
            let (sender, budget, ticket) = owner.inject_async_source(script, &page.dynamic_scripts);
            producers.push((sender, budget, ticket, identity, name));
        }
        owner.ordered.append(&mut owner.asynchronous);
        let complete = |producer: &_| {
            let (sender, budget, ticket, identity, name): &(
                std::sync::mpsc::Sender<ParserResourceCompletion<u64>>,
                Arc<std::sync::atomic::AtomicUsize>,
                u64,
                ParserDocumentIdentity,
                &str,
            ) = producer;
            sender
                .send(ParserResourceCompletion::Script {
                    realm: identity.realm,
                    stream_epoch: identity.stream_epoch,
                    document_generation: identity.document_generation,
                    document: identity.document,
                    order: 0,
                    record: *ticket,
                    asynchronous: false,
                    result: retain_parser_script_source(
                        Ok((
                            format!("https://example.test/{name}.js"),
                            format!("order.push('{name}')"),
                            false,
                            "no-referrer".into(),
                        )),
                        budget,
                    ),
                })
                .unwrap();
        };
        complete(&producers[1]);
        assert!(
            owner.runnable(&mut page.agent.host),
            "later completion must wake the owner"
        );
        owner.poll_deferred_preparations(&mut page).unwrap();
        assert_eq!(owner.ordered.len(), 2);
        assert!(owner.ordered[0].record.8.loaded.is_none());
        assert!(owner.ordered[1].record.8.loaded.is_some());
        assert!(
            !owner.runnable(&mut page.agent.host),
            "ready tail cannot spin or execute past held head"
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length.toString()")
                .unwrap(),
            "0"
        );
        complete(&producers[0]);
        owner.poll_deferred_preparations(&mut page).unwrap();
        assert!(
            owner
                .ordered
                .iter()
                .all(|job| job.record.8.loaded.is_some())
        );
        owner.lifecycle_allowed = true;
        page.frame_script_preparations.push_back(owner);
        page.poll_frame_script_preparations().unwrap();
        page.poll_frame_script_preparations().unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "first,second"
        );
        assert!(!page.frame_scripts_pending(Some(realm)));
    }

    #[test]
    fn deferred_prefetch_does_not_cross_an_unprocessed_snapshot_importmap() {
        let (mut page, realm, dom, a, b) = frame_fixture();
        let mut map = record(&page, &dom, a, r#"{"imports":{"helper":"./helper.js"}}"#);
        map.2 = Some("importmap".into());
        let mut deferred = record(&page, &dom, b, "");
        deferred.1 = Some("https://example.test/deferred.js".into());
        deferred.4 = false;
        deferred.6 = true;
        enqueue(&mut page, realm, dom, vec![map, deferred]);
        let mut owner = page.frame_script_preparations.pop_back().unwrap();
        owner.poll_deferred_preparations(&mut page).unwrap();
        assert_eq!(owner.ordered.len(), 2);
        let deferred = &owner.ordered[1];
        #[cfg(unix)]
        assert!(deferred.source.is_none());
        #[cfg(not(unix))]
        assert!(deferred.portable_source.is_none());
        assert!(deferred.graph.is_none());
        assert!(!owner.ordered[0].execution.entered);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length.toString()")
                .unwrap(),
            "0"
        );
    }

    #[cfg(unix)]
    #[test]
    fn deferred_prefetch_error_publication_retry_retains_job_and_fires_once() {
        let (mut page, realm, dom, a, _) = frame_fixture();
        page.agent.host.execute("globalThis.errors=0;document.getElementById('a').addEventListener('error',()=>errors++)").unwrap();
        enqueue(&mut page, realm, dom.clone(), vec![]);
        let mut owner = page.frame_script_preparations.pop_back().unwrap();
        let mut script = record(&page, &dom, a, "order.push('forbidden')");
        script.1 = Some("https://example.test/missing.js".into());
        script.4 = false;
        script.6 = true;
        let identity = script.7.unwrap();
        let (sender, budget, ticket) = owner.inject_async_source(script, &page.dynamic_scripts);
        owner.xml_deferred.append(&mut owner.asynchronous);
        sender
            .send(ParserResourceCompletion::Script {
                realm: identity.realm,
                stream_epoch: identity.stream_epoch,
                document_generation: identity.document_generation,
                document: identity.document,
                order: 0,
                record: ticket,
                asynchronous: false,
                result: Err(ParserScriptFailure::Fetch {
                    message: "missing".into(),
                    source_bytes: budget,
                }),
            })
            .unwrap();
        FAIL_PARSER_COMPLETION.with(|flag| flag.set(true));
        assert!(owner.poll_deferred_preparations(&mut page).is_err());
        assert_eq!(owner.xml_deferred.len(), 1);
        assert_eq!(owner.xml_deferred[0].execution.completion, Some(false));
        owner.poll_deferred_preparations(&mut page).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length+'|'+errors")
                .unwrap(),
            "0|0"
        );
        owner.ordered.append(&mut owner.xml_deferred);
        page.frame_script_preparations.push_back(owner);
        page.poll_frame_script_preparations().unwrap();
        page.poll_frame_script_preparations().unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.length+'|'+errors")
                .unwrap(),
            "0|1"
        );
        assert!(!page.frame_scripts_pending(Some(realm)));
    }

    #[cfg(unix)]
    #[test]
    fn passive_author_input_does_not_hide_deferred_preparation_or_execute_it_early() {
        let mut page = RustExecutableDocument::load(
            r#"<iframe id=child srcdoc='<body>initial'></iframe><script>
            const child=document.getElementById('child').contentDocument;
            child.open();child.write('<body>unclosed');
        </script>"#,
        )
        .unwrap();
        assert!(!page.construction_pending());
        assert!(!page.document_load_complete());
        let frame = page.agent.dom.borrow().find_by_id("child").unwrap();
        let realm = *page
            .frame_realms
            .iter()
            .find(|(_, owner)| owner.frame == frame)
            .unwrap()
            .0;
        page.agent.host.activate_realm(realm).unwrap();
        page.agent.host.execute("const script=document.createElement('script');script.id='resource';script.type='application/json';document.body.appendChild(script)").unwrap();
        let dom = page.agent.host.active_dom().unwrap();
        let document = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let node = dom
            .borrow()
            .get_element_by_id(document, "resource")
            .unwrap()
            .unwrap();
        let mut script = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
        script.1 = Some("https://example.test/deferred.js".into());
        script.2 = None;
        script.4 = false;
        script.6 = true;
        let identity = script.7.unwrap();
        let owner = page
            .frame_script_preparations
            .iter_mut()
            .find(|owner| owner.load.realm == realm)
            .unwrap();
        let (sender, budget, ticket) = owner.inject_async_source(script, &page.dynamic_scripts);
        owner.xml_deferred.append(&mut owner.asynchronous);
        assert!(
            !owner.awaiting_author_input(),
            "owned deferred network work is not passive author input"
        );
        assert!(page.construction_pending());
        sender
            .send(ParserResourceCompletion::Script {
                realm,
                stream_epoch: identity.stream_epoch,
                document_generation: identity.document_generation,
                document: identity.document,
                order: 0,
                record: ticket,
                asynchronous: false,
                result: retain_parser_script_source(
                    Ok((
                        "https://example.test/deferred.js".into(),
                        "globalThis.deferredResourceRan=true".into(),
                        false,
                        "no-referrer".into(),
                    )),
                    &budget,
                ),
            })
            .unwrap();
        for _ in 0..16 {
            page.poll_context_events().unwrap();
            if !page.construction_pending() {
                break;
            }
        }
        assert!(
            !page.construction_pending(),
            "completed source can wait passively for author EOF"
        );
        assert!(!page.document_load_complete());
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof deferredResourceRan")
                .unwrap(),
            "undefined"
        );
        page.agent.host.execute("document.close()").unwrap();
        for _ in 0..16 {
            page.poll_context_events().unwrap();
            if !page.frame_scripts_pending(Some(realm)) {
                break;
            }
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(deferredResourceRan)")
                .unwrap(),
            "true"
        );
        assert!(!page.frame_scripts_pending(Some(realm)));
    }

    #[test]
    fn frame_preparation_defers_defer_until_classics_and_keeps_native_barrier() {
        let (mut page, realm, dom, a, b) = frame_fixture();
        let mut deferred = record(&page, &dom, a, "order.push('defer')");
        deferred.4 = false;
        deferred.6 = true;
        let classic = record(&page, &dom, b, "order.push('classic')");
        enqueue(&mut page, realm, dom, vec![deferred, classic]);
        page.poll_frame_script_preparations().unwrap();
        assert!(page.frame_scripts_pending(Some(realm)));
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "classic"
        );
        page.poll_frame_script_preparations().unwrap();
        assert!(!page.frame_scripts_pending(Some(realm)));
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "classic,defer"
        );
    }

    #[cfg(unix)]
    #[test]
    fn asynchronous_frame_module_waits_without_blocking_domcontentloaded_and_stop_releases_it() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ContextTransport,
        };
        struct Policy {
            expected: std::sync::Arc<std::sync::Mutex<Option<(String, u64, u64)>>>,
            polls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        }
        impl ContextTransport for Policy {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::AuthorizeInlineScript { .. } => {
                        Ok(ContextServiceResponse::ScriptAuthorized)
                    }
                    ContextServiceRequest::MessageWire {
                        request:
                            oryn_js::message_wire::MessageRequest::RegisterRootDocument { .. }
                            | oryn_js::message_wire::MessageRequest::RegisterWindow { .. }
                            | oryn_js::message_wire::MessageRequest::RetireRootDocument { .. }
                            | oryn_js::message_wire::MessageRequest::RetireWindow { .. },
                        ..
                    } => Ok(ContextServiceResponse::MessageWire {
                        result: Ok(oryn_js::message_wire::MessageResponse::Done),
                    }),
                    ContextServiceRequest::MessageWire {
                        source_page_id,
                        request:
                            oryn_js::message_wire::MessageRequest::InDocument {
                                target: oryn_js::message_wire::WindowTarget::Frame(frame),
                                document_generation,
                                stream_epoch,
                                request,
                            },
                    } if source_page_id == oryn_common::v3::PageId(1)
                        && self.expected.lock().unwrap().as_ref()
                            == Some(&(frame.clone(), document_generation, stream_epoch)) =>
                    {
                        match *request {
                            oryn_js::message_wire::MessageRequest::PollWindow {
                                target: oryn_js::message_wire::WindowTarget::Frame(target),
                                max: 1,
                            } if target == frame => {
                                self.polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                Ok(ContextServiceResponse::MessageWire {
                                    result: Ok(
                                        oryn_js::message_wire::MessageResponse::WindowMessages(
                                            Vec::new(),
                                        ),
                                    ),
                                })
                            }
                            _ => Err(crate::RuntimeError::ContextService(
                                "unexpected scoped frame poll".into(),
                            )),
                        }
                    }
                    ContextServiceRequest::PublishNativeWindowInventory { .. } => {
                        Ok(ContextServiceResponse::NativeWindowInventoryPublished)
                    }
                    _ => Err(crate::RuntimeError::ContextService(
                        "unexpected frame preparation request".into(),
                    )),
                }
            }
        }
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let expected = std::sync::Arc::new(std::sync::Mutex::new(None));
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let policy = std::sync::Arc::new(Policy {
            expected: expected.clone(),
            polls: polls.clone(),
        });
        let server = std::thread::spawn(move || {
            crate::context_service::serve_context_transport(parent, policy)
        });
        let client = crate::context_service::SharedContextClient::new(
            crate::context_service::IpcContextClient::new(worker),
        );
        let (mut page, realm, dom, a, b) = frame_fixture();
        struct ShutdownOnDrop(std::os::unix::net::UnixStream);
        impl Drop for ShutdownOnDrop {
            fn drop(&mut self) {
                let _ = self.0.shutdown(std::net::Shutdown::Both);
            }
        }
        let _shutdown_on_unwind = ShutdownOnDrop(shutdown.try_clone().unwrap());
        page.agent.host.install_test_resource_endpoint(
            realm,
            client.clone(),
            oryn_common::v3::PageId(1),
        );
        let owner = &page.frame_realms[&realm];
        let frame = page
            .agent
            .host
            .automation_frame_token(owner.parent_realm, owner.frame)
            .unwrap();
        let generation = page
            .agent
            .host
            .realm_descriptor(realm)
            .unwrap()
            .document_generation;
        let epoch = page.agent.host.document_authority_epoch(realm).unwrap();
        *expected.lock().unwrap() = Some((frame, generation, epoch));
        page.context = Some((client, oryn_common::v3::PageId(1), None));
        page.agent.host.execute("document.addEventListener('DOMContentLoaded',()=>order.push('dom'));addEventListener('load',()=>order.push('load'))").unwrap();
        let mut asynchronous = record(
            &page,
            &dom,
            a,
            "import 'https://example.test/pending.js';order.push('module')",
        );
        asynchronous.2 = Some("module".into());
        asynchronous.4 = false;
        asynchronous.6 = false;
        let classic = record(&page, &dom, b, "order.push('classic')");
        let slots = page.dynamic_scripts.graph_slots.clone();
        slots.store(8, std::sync::atomic::Ordering::Release);
        enqueue(&mut page, realm, dom, vec![asynchronous, classic]);
        page.poll_frame_script_preparations().unwrap();
        assert!(
            polls.load(std::sync::atomic::Ordering::SeqCst) > 0,
            "the native child endpoint must actually be polled"
        );
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "classic,dom"
        );
        assert!(page.frame_scripts_pending(Some(realm)));
        assert!(page.pending_document_loads.contains_key(&realm));
        assert!(
            page.frame_script_preparations
                .front()
                .unwrap()
                .asynchronous
                .front()
                .unwrap()
                .graph
                .is_some()
        );
        page.stop_pending_document_load().unwrap();
        assert!(page.frame_script_preparations.is_empty());
        slots.store(0, std::sync::atomic::Ordering::Release);
        page.context = None;
        drop(page);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
    }
    #[cfg(unix)]
    #[test]
    fn frame_stylesheet_barrier_uses_native_media_disabled_and_revision_state() {
        let (mut page, realm, dom, a, _) = frame_fixture();
        page.agent.host.execute("const sheet=document.createElement('link');sheet.id='sheet';sheet.rel='stylesheet';sheet.href='gate.css';sheet.media='not all';document.head.appendChild(sheet)").unwrap();
        let root = page.agent.host.automation_realm_identity(realm).unwrap().0;
        let sheet = dom
            .borrow()
            .query_selector(root, "#sheet")
            .unwrap()
            .unwrap();
        let classic = record(&page, &dom, a, "order.push('classic')");
        enqueue(&mut page, realm, dom.clone(), vec![classic]);
        let pending = page.frame_script_preparations.back().unwrap();
        assert!(!pending.styles_block_scripts(&page.agent.host).unwrap());
        page.agent.host.execute("sheet.media='all'").unwrap();
        // The old revision cannot delay a script; the native mutation producer
        // contributes a fresh preparation, whose applicability is authoritative.
        page.queue_preparing_frame_stylesheet(realm, sheet).unwrap();
        assert!(
            page.frame_script_preparations
                .back()
                .unwrap()
                .styles_block_scripts(&page.agent.host)
                .unwrap()
        );
        page.agent.host.execute("sheet.disabled=true").unwrap();
        assert!(
            !page
                .frame_script_preparations
                .back()
                .unwrap()
                .styles_block_scripts(&page.agent.host)
                .unwrap()
        );
        page.agent
            .host
            .execute("sheet.disabled=false;sheet.remove()")
            .unwrap();
        assert!(
            !page
                .frame_script_preparations
                .back()
                .unwrap()
                .styles_block_scripts(&page.agent.host)
                .unwrap()
        );
    }

    #[test]
    fn frame_live_tokenizer_hides_suffix_and_document_write_uses_child_insertion_point() {
        let mut page =
            RustExecutableDocument::load("<body><script>globalThis.order=[]</script>").unwrap();
        let source = "<script>parent.order.push(document.getElementById('late')?'bad-suffix':'early');document.write('<b id=written>inserted</b>')</script><p id=late>later</p><script>parent.order.push(document.getElementById('written')?'written':'missing')</script>";
        let source = serde_json::to_string(source).unwrap();
        page.agent.host.execute(&format!("const f=document.createElement('iframe');f.srcdoc={source};document.body.appendChild(f)")).unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "early,written"
        );
        let root = page.agent.dom.borrow().document();
        let dom = page.agent.dom.borrow();
        for node in dom.query_selector_all(root, "#written").unwrap() {
            assert_ne!(
                dom.node_document(node).unwrap(),
                Some(root),
                "child parser must not append into the arena's top document"
            );
        }
    }
    #[test]
    fn frame_xml_parser_yields_and_publishes_only_its_native_child_document() {
        let (mut page, realm, dom, _, _) = frame_fixture();
        let top_document = page.agent.dom.borrow().document();
        page.agent
            .host
            .execute("document.documentElement.remove()")
            .unwrap();
        enqueue(&mut page, realm, dom.clone(), Vec::new());
        let mut pending = page.frame_script_preparations.pop_back().unwrap();
        let source = format!("<root>{}</root>", "<leaf/>".repeat(600));
        pending
            .start_document_source(
                source,
                Some("application/xml".into()),
                &mut page.agent.host,
                &page.dynamic_scripts,
            )
            .unwrap();
        page.frame_script_preparations.push_back(pending);
        page.poll_frame_script_preparations().unwrap();
        assert!(
            page.frame_script_preparations
                .front()
                .unwrap()
                .xml_parser
                .is_some()
        );
        for _ in 0..16 {
            page.poll_frame_script_preparations().unwrap();
            if !page.frame_scripts_pending(Some(realm)) {
                break;
            }
        }
        assert!(!page.frame_scripts_pending(Some(realm)));
        let child = page.agent.host.automation_realm_identity(realm).unwrap().0;
        assert_ne!(child, top_document);
        assert_eq!(
            dom.borrow()
                .query_selector_all(child, "leaf")
                .unwrap()
                .len(),
            600
        );
        for leaf in dom.borrow().query_selector_all(child, "leaf").unwrap() {
            assert_eq!(dom.borrow().node_document(leaf).unwrap(), Some(child));
        }
        assert!(
            dom.borrow()
                .query_selector(top_document, "iframe")
                .unwrap()
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn dynamic_frame_referrer_metadata_uses_native_child_creator() {
        use crate::context_service::{
            ContextServiceRequest, ContextServiceResponse, ContextTransport,
        };
        struct Metadata(std::sync::Mutex<Vec<oryn_js::worker_wire::Creator>>);
        impl ContextTransport for Metadata {
            fn request(
                &self,
                request: ContextServiceRequest,
            ) -> Result<ContextServiceResponse, crate::RuntimeError> {
                match request {
                    ContextServiceRequest::InstallRealmMetadata {
                        creator,
                        meta_csp: None,
                        meta_referrer: Some(policy),
                        ..
                    } => {
                        assert_eq!(policy, "no-referrer");
                        self.0.lock().unwrap().push(creator);
                        Ok(ContextServiceResponse::DocumentSecurityInstalled)
                    }
                    _ => Err(crate::RuntimeError::ContextService(
                        "unexpected metadata request".into(),
                    )),
                }
            }
        }
        let (mut page, realm, _, _, _) = frame_fixture();
        let expected = page.agent.host.native_creator_for_realm(realm).unwrap();
        let metadata = std::sync::Arc::new(Metadata(std::sync::Mutex::new(Vec::new())));
        let transport = metadata.clone();
        let (worker, parent) = std::os::unix::net::UnixStream::pair().unwrap();
        let shutdown = parent.try_clone().unwrap();
        let server = std::thread::spawn(move || {
            crate::context_service::serve_context_transport(parent, transport)
        });
        page.context = Some((
            crate::context_service::SharedContextClient::new(
                crate::context_service::IpcContextClient::new(worker),
            ),
            oryn_common::v3::PageId(1),
            None,
        ));
        page.agent.host.execute("const meta=document.createElement('meta');meta.name='referrer';meta.content='no-referrer';document.head.appendChild(meta)").unwrap();
        page.drain_dynamic_referrer_policies().unwrap();
        let captured = metadata.0.lock().unwrap();
        assert!(!captured.is_empty());
        assert!(captured.iter().all(|creator| creator == &expected));
        drop(captured);
        page.context = None;
        drop(page);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
        let _ = server.join();
    }

    #[test]
    fn frame_location_about_blank_commits_inherited_empty_document() {
        let (mut page, realm, _, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        page.agent
            .host
            .execute("location.href='about:blank#replacement'")
            .unwrap();
        page.drain_frame_location_navigations().unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        let replacement = page
            .frame_realms
            .iter()
            .find(|(_, candidate)| candidate.frame == owner.frame)
            .map(|(realm, _)| *realm)
            .unwrap();
        page.agent.host.activate_realm(replacement).unwrap();
        assert_eq!(
            page.agent.host.location_url().unwrap(),
            "about:blank#replacement"
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string(
                    "String(document.getElementById('a')===null&&parent.document!==null)"
                )
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn frame_navigation_api_destination_survives_native_commit_with_separate_history_state() {
        let (mut page, realm, _, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        page.agent
            .host
            .execute("navigation.navigate('about:blank#accepted',{state:{value:7}})")
            .unwrap();
        page.drain_frame_location_navigations().unwrap();
        let accepted = page.pending_frame_navigations[&(owner.parent_realm, owner.frame)]
            .destination
            .clone()
            .unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        let replacement = page
            .frame_realms
            .iter()
            .find(|(_, candidate)| candidate.frame == owner.frame)
            .map(|(realm, _)| *realm)
            .unwrap();
        page.agent.host.activate_realm(replacement).unwrap();
        let history = page.agent.host.navigation_history_snapshot().unwrap();
        let current = &history.entries[history.current];
        assert_eq!(current.key, accepted.key);
        assert_eq!(current.id, accepted.id);
        assert_eq!(page.agent.host.evaluate_string("String(navigation.currentEntry.getState().value===7&&history.state===null&&navigation.activation.entry===navigation.currentEntry)").unwrap(),"true");
    }

    #[test]
    fn frame_history_back_routes_native_destination_and_swap_before_hide() {
        let (mut page, realm, _, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        // Populate the shared joint session through real cross-document commits.
        // sync_navigation_history alone only changes the exposed Navigation list.
        let mut previous = None;
        page.agent.host.activate_realm(owner.parent_realm).unwrap();
        let second = page
            .agent
            .host
            .evaluate_string(
                "URL.createObjectURL(new Blob(['<body>current</body>'],{type:'text/html'}))",
            )
            .unwrap();
        page.agent.host.activate_realm(realm).unwrap();
        for url in ["about:blank#old", second.as_str()] {
            page.agent
                .host
                .execute(&format!(
                    "location.href={}",
                    serde_json::to_string(url).unwrap()
                ))
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                page.drain_tasks().unwrap();
                let current = page
                    .frame_realms
                    .iter()
                    .find(|(_, candidate)| candidate.frame == owner.frame)
                    .map(|(realm, _)| *realm);
                if let Some(current) = current {
                    page.agent.host.activate_realm(current).unwrap();
                    if page.agent.host.location_url().unwrap() == url
                        && !page.frame_scripts_pending(Some(current))
                        && !page.frame_resources_pending(Some(owner.parent_realm))
                    {
                        if previous.is_none() {
                            let history = page.agent.host.navigation_history_snapshot().unwrap();
                            previous = Some(history.entries[history.current].clone());
                        }
                        break;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "history fixture navigation did not commit: {:?}",
                    page.diagnostics
                );
                std::thread::yield_now();
            }
        }
        let first = previous.unwrap();
        page.agent.host.execute("parent.frameEvents=[];onpageswap=e=>parent.frameEvents.push('swap:'+e.isTrusted);onpagehide=e=>parent.frameEvents.push('hide:'+e.persisted);history.back()").unwrap();
        page.drain_frame_location_navigations().unwrap();
        let accepted = page.pending_frame_navigations[&(owner.parent_realm, owner.frame)]
            .destination
            .as_ref()
            .unwrap();
        assert_eq!(accepted.id, first.id);
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(owner.parent_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("frameEvents.join('|')")
                .unwrap(),
            "swap:true|hide:false"
        );
    }

    #[test]
    fn srcdoc_history_restores_native_source_after_element_attribute_is_removed() {
        let (mut page, realm, _, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        let first = page.agent.host.navigation_history_snapshot().unwrap();
        let id = first.entries[first.current].id.clone();
        page.agent
            .host
            .execute("location.href='about:blank#next'")
            .unwrap();
        page.drain_frame_location_navigations().unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        let next = *page
            .frame_realms
            .iter()
            .find(|(_, frame)| frame.frame == owner.frame)
            .unwrap()
            .0;
        page.agent.host.activate_realm(next).unwrap();
        page.agent.host.execute("history.back()").unwrap();
        page.drain_frame_location_navigations().unwrap();
        assert!(
            page.pending_frame_navigations[&(owner.parent_realm, owner.frame)]
                .archived_source
                .is_some()
        );
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        let restored = *page
            .frame_realms
            .iter()
            .find(|(_, frame)| frame.frame == owner.frame)
            .unwrap()
            .0;
        page.agent.host.activate_realm(restored).unwrap();
        assert_eq!(page.agent.host.evaluate_string("String(document.getElementById('a')!==null&&frameElement.getAttribute('srcdoc')===null)").unwrap(),"true");
        let history = page.agent.host.navigation_history_snapshot().unwrap();
        assert_eq!(history.entries[history.current].id, id);
    }
    #[test]
    fn frame_pageswap_removal_prevents_obsolete_document_commit() {
        let (mut page, realm, _, _, _) = frame_fixture();
        let owner = page.frame_realms[&realm].clone();
        page.agent
            .host
            .execute("onpageswap=()=>frameElement.remove();location.href='about:blank#obsolete'")
            .unwrap();
        page.drain_frame_location_navigations().unwrap();
        page.drain_dynamic_frames().unwrap();
        assert!(
            !page
                .agent
                .host
                .automation_frame_live(owner.parent_realm, owner.frame)
        );
        assert!(
            !page
                .frame_realms
                .values()
                .any(|frame| frame.committed_url == "about:blank#obsolete")
        );
    }
    #[test]
    fn frame_pageswap_new_location_prevents_obsolete_document_commit() {
        let (mut page, _realm, _, _, _) = frame_fixture();
        page.agent.host.execute("onpageswap=()=>{onpageswap=null;location.href='about:blank#winner'};location.href='about:blank#obsolete'").unwrap();
        page.drain_frame_location_navigations().unwrap();
        page.drain_dynamic_frames().unwrap();
        assert!(
            !page
                .frame_realms
                .values()
                .any(|frame| frame.committed_url == "about:blank#obsolete")
        );
        page.drain_frame_location_navigations().unwrap();
        for _ in 0..32 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        assert!(
            page.frame_realms
                .values()
                .any(|frame| frame.committed_url == "about:blank#winner")
        );
    }

    #[test]
    fn xml_frame_scripts_run_at_close_tag_before_suffix_publication() {
        let (mut page, realm, dom, _, _) = frame_fixture();
        page.agent
            .host
            .execute("document.documentElement.remove()")
            .unwrap();
        enqueue(&mut page, realm, dom.clone(), Vec::new());
        let mut pending = page.frame_script_preparations.pop_back().unwrap();
        let source = "<html xmlns='http://www.w3.org/1999/xhtml'><head/><body><script>order.push(document.getElementById('later')?'bad':'early')</script><p id='later'/><script>order.push(document.getElementById('later')?'later':'missing')</script></body></html>";
        pending
            .start_document_source(
                source.into(),
                Some("application/xhtml+xml".into()),
                &mut page.agent.host,
                &page.dynamic_scripts,
            )
            .unwrap();
        page.frame_script_preparations.push_back(pending);
        for _ in 0..32 {
            page.poll_frame_script_preparations().unwrap();
            if !page.frame_scripts_pending(Some(realm)) {
                break;
            }
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "early,later"
        );
    }

    #[test]
    fn same_url_parser_frames_keep_distinct_owners_and_publish_before_parent_eof() {
        let mut page=RustExecutableDocument::load_with_environment("<body><script>globalThis.order=[]</script><iframe id=one src='child.html'></iframe><iframe id=two src='child.html'></iframe>",&BTreeMap::new(),None,Some(Url::parse("https://example.test/root").unwrap())).unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        let dom = page.agent.dom.clone();
        let mut parser = LiveParserState::new(
            crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(
                "<p id=unconsumed/>".into()
            )))),
            &dom,
            &mut page.agent.host,
        )
        .unwrap();
        for id in ["one", "two"] {
            let node = dom.borrow().find_by_id(id).unwrap();
            page.agent
                .host
                .ensure_provisional_frame_window(node)
                .unwrap();
            let owner = ParserFrameOwner::capture(&page.agent.host, &dom, node).unwrap();
            parser.all_events.push(html::ParserEvent::FrameDiscovered {
                node,
                source: "child.html".into(),
            });
            parser.loaded_frames.insert(node,LoadedFrameResource{timing:None,inherited_encoding:None,response_headers:Vec::new(),request_state:Some(oryn_common::navigation_request::NavigationRequestState{initiator_url:Some("https://source.test/private".into()),opaque_initiator:false,referrer:Some(format!("https://source.test/{id}")),referrer_policy:"unsafe-url".into()}),document_referrer:format!("https://source.test/{id}"),raw_body_base64:None,content_language:None,#[cfg(unix)] body:None,native_origin:None,no_commit:false,embedded_unavailable:false,owner,_bytes:None,terminal_error:None,worker_policy_token:None,modals_allowed:true,
                referrer_policy:"no-referrer".into(),url:"https://example.test/child.html".into(),source:format!("<script>parent.order.push('{id}');parent.order.push(document.referrer)</script>"),encoding:"UTF-8".into(),content_type:Some("text/html".into())});
        }
        assert_eq!(parser.loaded_frames.len(), 2);
        assert!(page.publish_ready_parser_frames(&mut parser, &dom).unwrap());
        assert!(parser.loaded_frames.is_empty());
        assert!(
            !parser
                .all_events
                .iter()
                .any(|event| matches!(event, html::ParserEvent::FrameDiscovered { .. }))
        );
        for _ in 0..32 {
            page.poll_frame_script_preparations().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("order.slice().sort().join(',')")
                .unwrap(),
            "https://source.test/one,https://source.test/two,one,two"
        );
        for id in ["one", "two"] {
            let node = dom.borrow().find_by_id(id).unwrap();
            let frame = page
                .frame_realms
                .values()
                .find(|owner| owner.frame == node)
                .unwrap();
            let session = frame
                .session
                .as_ref()
                .expect("parser response must retain original request");
            let current = &session.update.entries[session.update.current];
            let state = session.requests[&current.document_id]
                .request_state
                .as_ref()
                .unwrap();
            assert_eq!(
                state.initiator_url.as_deref(),
                Some("https://source.test/private")
            );
            assert_eq!(state.referrer, Some(format!("https://source.test/{id}")));
            assert_eq!(state.referrer_policy, "unsafe-url");
        }
        assert!(
            dom.borrow().find_by_id("unconsumed").is_none(),
            "child completion must not depend on consuming the parent suffix"
        );
    }
    #[test]
    fn parser_frame_owner_rejects_attribute_changes_and_reconnections() {
        let (mut page, _, _, _, _) = frame_fixture();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        let dom = page.agent.dom.clone();
        let frame = dom.borrow().find_by_id("f").unwrap();
        let owner = ParserFrameOwner::capture(&page.agent.host, &dom, frame).unwrap();
        assert!(owner.live(&page.agent.host, &dom));
        page.agent
            .host
            .execute("document.getElementById('f').setAttribute('referrerpolicy','no-referrer')")
            .unwrap();
        assert!(
            owner.live(&page.agent.host, &dom),
            "referrer policy is frozen for the accepted request and does not start a navigation"
        );
        page.agent
            .host
            .execute("document.getElementById('f').removeAttribute('referrerpolicy')")
            .unwrap();
        assert!(owner.live(&page.agent.host, &dom));
        page.agent.host.execute("globalThis.originalFrameSource=document.getElementById('f').getAttribute('srcdoc');document.getElementById('f').setAttribute('srcdoc','<p>replacement</p>')").unwrap();
        assert!(!owner.live(&page.agent.host, &dom));
        page.agent
            .host
            .execute("document.getElementById('f').setAttribute('srcdoc',originalFrameSource)")
            .unwrap();
        assert!(
            !owner.live(&page.agent.host, &dom),
            "source ABA must not revive an obsolete navigation"
        );
        let replacement = ParserFrameOwner::capture(&page.agent.host, &dom, frame).unwrap();
        page.agent.host.execute("const moved=document.getElementById('f');moved.remove();document.body.appendChild(moved)").unwrap();
        assert!(!replacement.live(&page.agent.host, &dom));
    }

    #[test]
    fn root_xml_uses_live_script_resource_owner_and_preserves_encoding() {
        let source = "<html xmlns='http://www.w3.org/1999/xhtml'><head/><body><script>order.push(document.getElementById('suffix')?'bad':'early');Promise.resolve().then(()=>order.push('microtask'))</script><p id='suffix'/><script>order.push(document.getElementById('suffix')?'suffix':'missing')</script></body></html>";
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(std::iter::once(Ok(
                source.into()
            )))),
            content_type: Some("application/xhtml+xml".into()),
            encoding: std::sync::Arc::new(std::sync::Mutex::new(Some("windows-1252"))),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/live.xml").unwrap()),
            256 * 1024 * 1024,
            true,
        )
        .unwrap();
        page.agent
            .host
            .execute("globalThis.order=[];globalThis.saved=document")
            .unwrap();
        for _ in 0..64 {
            if !page.parser_pending() {
                break;
            }
            page.resume_pending_parser().unwrap();
        }
        assert!(!page.parser_pending());
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "early,microtask,suffix"
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(saved===document)+':'+document.characterSet")
                .unwrap(),
            "true:windows-1252"
        );
    }

    #[cfg(unix)]
    #[test]
    fn parser_frame_failure_is_terminal_and_cannot_retry_at_eof() {
        let (mut page, _, _, _, _) = frame_fixture();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent.host.execute("document.getElementById('f').removeAttribute('srcdoc');document.getElementById('f').setAttribute('src','failed.html')").unwrap();
        let dom = page.agent.dom.clone();
        let node = dom.borrow().find_by_id("f").unwrap();
        let owner = ParserFrameOwner::capture(&page.agent.host, &dom, node).unwrap();
        let completion = ParserResourceCompletion::Frame {
            identity: owner.identity,
            owner,
            node,
            requested_url: "failed.html".into(),
            result: Box::new(Err("denied".into())),
        };
        let outcome = apply_parser_resource_completion(
            completion,
            &dom,
            &mut page.agent.host,
            &mut page.resources,
            &mut BTreeMap::new(),
            &mut page.diagnostics,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(outcome.0, node);
        assert_eq!(outcome.1.terminal_error.as_deref(), Some("denied"));
        let mut parser = LiveParserState::new(
            crate::host::DocumentStream::html(Box::new(std::iter::empty::<
                Result<String, HostError>,
            >())),
            &dom,
            &mut page.agent.host,
        )
        .unwrap();
        parser.all_events.push(html::ParserEvent::FrameDiscovered {
            node,
            source: "failed.html".into(),
        });
        parser.loaded_frames.insert(node, outcome.1);
        let previous = page.frame_script_preparations.len();
        page.publish_ready_parser_frames(&mut parser, &dom).unwrap();
        assert!(parser.loaded_frames.is_empty());
        assert!(parser.all_events.is_empty());
        assert_eq!(page.frame_script_preparations.len(), previous);
    }

    #[test]
    fn retired_frame_top_level_await_rejection_cannot_report_to_replacement() {
        let (mut page, realm, _, _, _) = frame_fixture();
        page.agent
            .host
            .execute("addEventListener('error',()=>parent.lateErrors++)")
            .unwrap();
        page.agent
            .host
            .execute_module_script(
                "https://example.test/pending.js",
                "await new Promise((resolve,reject)=>{parent.rejectOldModule=reject});",
                &BTreeMap::new(),
                true,
            )
            .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        page.agent
            .host
            .execute("globalThis.lateErrors=0;document.getElementById('f').remove()")
            .unwrap();
        page.checkpoint().unwrap();
        assert!(!page.agent.host.realm_execution_live(realm));
        page.agent
            .host
            .execute("rejectOldModule(new Error('retired module'))")
            .unwrap();
        page.agent.host.run_microtask_checkpoint().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(lateErrors)")
                .unwrap(),
            "0"
        );
        assert!(page.agent.host.take_module_error_reports().is_empty());
    }
    #[test]
    fn frame_top_level_await_does_not_hold_script_or_document_load() {
        let (mut page, realm, dom, a, _) = frame_fixture();
        page.agent.host.execute("addEventListener('load',()=>order.push('load'));document.addEventListener('DOMContentLoaded',()=>order.push('dom'))").unwrap();
        let mut module = record(
            &page,
            &dom,
            a,
            "order.push('start');await new Promise(resolve=>globalThis.finishModule=resolve);order.push('settled')",
        );
        module.2 = Some("module".into());
        module.4 = false;
        module.6 = true;
        enqueue(&mut page, realm, dom, vec![module]);
        for _ in 0..8 {
            page.poll_frame_script_preparations().unwrap();
            page.flush_document_loads().unwrap();
            if !page.frame_scripts_pending(Some(realm)) {
                break;
            }
        }
        page.agent.host.activate_realm(realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "start,dom,load"
        );
        page.agent.host.execute("finishModule()").unwrap();
        page.agent.host.run_microtask_checkpoint().unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "start,dom,load,settled"
        );
    }

    #[test]
    fn frame_outer_dimensions_follow_native_top_window_not_frame_content_box() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe id=f style='width:240px;height:120px' srcdoc='<script>parent.childOuterAtParse=[outerWidth,outerHeight].join(String.fromCharCode(124))</script>'></iframe>",
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        ).unwrap();
        for _ in 0..16 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(page.agent.host.evaluate_string(
            "String(childOuterAtParse === [outerWidth,outerHeight].join('|') && f.contentWindow.innerWidth === 240 && f.contentWindow.outerWidth === outerWidth && f.contentWindow.outerHeight === outerHeight)"
        ).unwrap(), "true");
        page.agent.host.execute(
            "globalThis.childOuterGetter=Object.getOwnPropertyDescriptor(f.contentWindow,'outerWidth').get;globalThis.topOuterGetter=Object.getOwnPropertyDescriptor(window,'outerWidth').get;outerWidth=1;outerHeight=2;f.style.width='360px'"
        ).unwrap();
        assert_eq!(page.agent.host.evaluate_string(
            "String(f.contentWindow.innerWidth === 360 && f.contentWindow.outerWidth === topOuterGetter.call(window) && childOuterGetter.call(window) === topOuterGetter.call(f.contentWindow))"
        ).unwrap(), "true");
        page.agent
            .host
            .install_layout(oryn_js::v8_bindings::BindingLayout {
                viewport: Some((900., 650.)),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(page.agent.host.evaluate_string(
            "[f.contentWindow.outerWidth,f.contentWindow.outerHeight,outerWidth,outerHeight].join('|')"
        ).unwrap(), "900|650|1|2");
    }

    #[test]
    fn native_media_owner_refreshes_after_parent_resize_without_window_getter() {
        let mut page = RustExecutableDocument::load_with_environment(
            "<body><iframe id=f style='width:240px;height:120px' srcdoc='<body>child</body>'></iframe>",
            &BTreeMap::new(), None,
            Some(Url::parse("https://example.test/root").unwrap()),
        ).unwrap();
        for _ in 0..16 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) { break; }
        }
        let child = *page.frame_realms.keys().next().expect("child realm");
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert!(page.agent.host.native_media_matches_in_realm(child, "(width: 240px)").unwrap());
        page.agent.host.execute("f.style.width='360px';f.contentWindow.innerWidth=7").unwrap();
        assert!(page.agent.host.native_media_matches_in_realm(child, "(width: 360px)").unwrap());
        assert!(!page.agent.host.native_media_matches_in_realm(child, "(width: 240px)").unwrap());
        // Explicit owner entry must not alter the incumbent author realm.
        assert_eq!(page.agent.host.evaluate_string("String(f.contentWindow.innerWidth)").unwrap(), "7");
        page.agent.host.execute("f.remove()").unwrap();
        page.checkpoint().unwrap();
        assert!(page.agent.host.native_media_matches_in_realm(child, "all").is_err());
    }

    #[test]
    fn frame_viewport_uses_native_content_box_before_scripts_and_after_parent_resize() {
        let mut page=RustExecutableDocument::load_with_environment("<body><iframe id=f style='width:240px;height:120px;border:8px solid;padding:4px;transform:scale(2)' srcdoc='<script>parent.initialSize=innerWidth+String.fromCharCode(120)+innerHeight</script>'></iframe>",&BTreeMap::new(),None,Some(Url::parse("https://example.test/root").unwrap())).unwrap();
        for _ in 0..16 {
            page.drain_tasks().unwrap();
            if !page.frame_scripts_pending(None) {
                break;
            }
        }
        page.agent.host.activate_realm(page.top_realm).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("initialSize").unwrap(),
            "240x120"
        );
        page.agent.host.execute("globalThis.resizes=0;f.contentWindow.onresize=()=>resizes++;f.style.width='360px';globalThis.changedSize=f.contentWindow.innerWidth;globalThis.childMedia=f.contentWindow.matchMedia('(width: 360px)').matches").unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("changedSize+':'+childMedia")
                .unwrap(),
            "360:true"
        );
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("String(resizes)").unwrap(),
            "1"
        );
        page.agent.host.execute("f.contentWindow.innerWidth=7;f.style.width='400px';globalThis.authorValue=f.contentWindow.innerWidth;globalThis.nativeMedia=f.contentWindow.matchMedia('(width: 400px)').matches").unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("authorValue+':'+nativeMedia")
                .unwrap(),
            "7:true"
        );
    }
}

#[cfg(test)]
mod streamed_xml_input_tests {
    use super::*;
    #[test]
    fn sequential_xml_scripts_release_source_custody_before_document_eof() {
        let body = "globalThis.xmlRuns=(globalThis.xmlRuns||0)+1;";
        let source = format!(
            "<html xmlns='http://www.w3.org/1999/xhtml'>{}</html>",
            format!("<script>{body}</script>").repeat(1100)
        );
        let source_bytes = source.len();
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Ok(source)].into_iter(),
            )),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/many.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        let budget = page.dynamic_scripts.source_bytes.clone();
        let initial = budget.load(std::sync::atomic::Ordering::Acquire);
        let mut peak = initial;
        for _ in 0..6000 {
            page.resume_pending_parser_step().unwrap();
            peak = peak.max(budget.load(std::sync::atomic::Ordering::Acquire));
            if !page.parser_pending() {
                break;
            }
        }
        assert!(!page.parser_pending());
        assert_eq!(
            page.agent.host.evaluate_string("String(xmlRuns)").unwrap(),
            "1100"
        );
        assert!(
            peak <= initial + source_bytes + 512,
            "completed sequential script records must not accumulate source reservations: {initial} -> {peak}"
        );
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            initial,
            "EOF releases parser-owned bytes and preserves the captured bootstrap baseline"
        );
    }
    #[test]
    fn stopped_xml_input_releases_deferred_script_and_input_reservations() {
        struct Input(bool);
        impl crate::host::DocumentInput for Input {
            fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
                if self.0 {
                    return std::task::Poll::Pending;
                }
                self.0 = true;
                std::task::Poll::Ready(Some(Ok("<html xmlns='http://www.w3.org/1999/xhtml'><script defer='defer' src='data:text/javascript,globalThis.staleDeferred=true'/>".into())))
            }
        }
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(Input(false)),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/deferred.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        let budget = page.dynamic_scripts.source_bytes.clone();
        let initial = budget.load(std::sync::atomic::Ordering::Acquire);
        for _ in 0..16 {
            page.resume_pending_parser_step().unwrap();
        }
        assert!(budget.load(std::sync::atomic::Ordering::Acquire) > initial);
        assert!(page.frame_script_preparations.iter().any(|owner| {
            owner
                .xml_deferred
                .iter()
                .any(|job| job._inline_reservation.is_some())
        }));
        page.agent.host.execute("stop()").unwrap();
        page.poll_frame_script_preparations().unwrap();
        assert!(page.frame_script_preparations.is_empty());
        assert_eq!(
            budget.load(std::sync::atomic::Ordering::Acquire),
            initial,
            "stop releases only parser source custody, preserving the original bootstrap baseline"
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof staleDeferred")
                .unwrap(),
            "undefined"
        );
    }
    #[test]
    fn root_xml_executes_script_before_network_eof_and_idle_input_does_not_spin() {
        struct Input {
            phase: u8,
            release: Rc<std::cell::Cell<bool>>,
        }
        impl crate::host::DocumentInput for Input {
            fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
                if self.phase == 0 {
                    self.phase = 1;
                    return std::task::Poll::Ready(Some(Ok("<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.beforeEOF=42</script>".into())));
                }
                if !self.release.get() {
                    return std::task::Poll::Pending;
                }
                if self.phase == 1 {
                    self.phase = 2;
                    return std::task::Poll::Ready(Some(Ok(
                        "<p id='after'>after</p></html>".into()
                    )));
                }
                std::task::Poll::Ready(None)
            }
        }
        let release = Rc::new(std::cell::Cell::new(false));
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(Input {
                phase: 0,
                release: release.clone(),
            }),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/stream.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        for _ in 0..16 {
            page.resume_pending_parser_step().unwrap();
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(globalThis.beforeEOF)")
                .unwrap(),
            "42"
        );
        assert!(page.node_by_id("after").is_none());
        assert!(page.parser_pending());
        assert!(!page.frame_scripts_runnable());
        page.agent.host.execute("globalThis.independent=7").unwrap();
        release.set(true);
        for _ in 0..32 {
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
        assert!(page.node_by_id("after").is_some());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(independent)")
                .unwrap(),
            "7"
        );
    }
    #[test]
    fn idle_html_frame_input_is_pending_without_runnable_spin() {
        struct Idle;
        impl crate::host::DocumentInput for Idle {
            fn poll_next(&mut self) -> std::task::Poll<Option<Result<String, HostError>>> {
                std::task::Poll::Pending
            }
        }
        let mut page =
            RustExecutableDocument::load("<iframe srcdoc='<p>child</p>'></iframe>").unwrap();
        let owner = page.frame_realms.values().next().unwrap().clone();
        let realm = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some())
            .unwrap()
            .realm;
        page.agent.host.activate_realm(realm).unwrap();
        let dom = page.agent.host.active_dom().unwrap();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        let mut pending = FrameScriptPreparation::new(FrameScriptInput {
            load: PreparedDocumentLoad { realm, identity },
            parent: owner.parent_realm,
            frame: owner.frame,
            dom,
            records: vec![],
            lifecycle_allowed: true,
            _refresh_url: None,
            limits: &page.dynamic_scripts,
            host: &mut page.agent.host,
        })
        .unwrap();
        pending
            .start_document_stream(
                crate::host::DocumentStream {
                    content_language: None,
                    chunks: Box::new(Idle),
                    content_type: Some("text/html".into()),
                    encoding: Default::default(),
                },
                &mut page.agent.host,
                &page.dynamic_scripts,
            )
            .unwrap();
        page.frame_script_preparations.push_back(pending);
        page.poll_frame_script_preparations().unwrap();
        assert!(page.frame_scripts_pending(Some(realm)));
        assert!(!page.frame_scripts_runnable());
    }
    #[test]
    fn parser_write_is_visible_before_return_and_preserves_split_tokens() {
        let mut page=RustExecutableDocument::load("<body><script>document.write('<i id=');document.write('\"written\">value');globalThis.immediate=document.getElementById('written').textContent;document.write('</i>')</script><p id=tail>tail</p>").unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("immediate").unwrap(),
            "value"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn recoverable_html_parse_errors_preserve_deferred_classic_and_module_execution() {
        let resources = BTreeMap::from([(
            "https://example.test/defer.js".into(),
            "order.push('defer')".into(),
        )]);
        // Missing doctype and the unmatched end tag are HTML parse errors, not a
        // fatal XML completion. All accepted deferred jobs must still run once.
        let mut page=RustExecutableDocument::load_with_environment("<body></unmatched><script>globalThis.order=[]</script><script defer src=defer.js></script><script type=module>order.push('module')</script><script>order.push('classic')</script>",&resources,None,Some(Url::parse("https://example.test/root").unwrap())).unwrap();
        assert!(
            !page.document.errors.is_empty(),
            "exercise recoverable parser errors"
        );
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "classic,defer,module"
        );
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "classic,defer,module"
        );
    }
    #[test]
    fn external_defer_write_is_ignored_but_parser_blocking_write_is_inserted() {
        let resources = BTreeMap::from([
            (
                "https://example.test/block.js".into(),
                "document.write('<b id=inserted>ok</b>')".into(),
            ),
            (
                "https://example.test/defer.js".into(),
                "document.write('<b id=destructive>bad</b>');globalThis.deferredRan=true".into(),
            ),
        ]);
        let mut page=RustExecutableDocument::load_with_environment("<body><script src=block.js></script><script defer src=defer.js></script><p id=tail>tail</p>",&resources,None,Some(Url::parse("https://example.test/root").unwrap())).unwrap();
        assert!(page.node_by_id("inserted").is_some());
        assert!(page.node_by_id("tail").is_some());
        assert!(page.node_by_id("destructive").is_none());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(deferredRan)")
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn module_write_counter_covers_immediate_jobs_and_expires_before_later_tasks() {
        let mut page=RustExecutableDocument::load("<body><p id=original>original</p><script type=module>document.write('<b id=bad>bad</b>');await Promise.resolve();document.write('<b id=alsoBad>bad</b>');globalThis.afterModule=()=>{document.write('<i id=later>later</i>');document.close()}</script>").unwrap();
        assert!(page.node_by_id("original").is_some());
        assert!(page.node_by_id("bad").is_none());
        assert!(page.node_by_id("alsoBad").is_none());
        page.agent.host.execute("afterModule()").unwrap();
        page.checkpoint().unwrap();
        assert!(page.node_by_id("later").is_some());
        assert!(page.node_by_id("original").is_none());
    }
    #[test]
    fn explicit_open_during_external_execution_creates_a_writable_insertion_point() {
        let resources=BTreeMap::from([("https://example.test/defer.js".into(),"document.open();document.write('<i id=replacement>replacement</i>');globalThis.synchronous=document.getElementById('replacement').textContent;document.close()".into())]);
        let mut page = RustExecutableDocument::load_with_environment(
            "<p id=original>original</p><script defer src=defer.js></script>",
            &resources,
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("synchronous").unwrap(),
            "replacement"
        );
        assert!(page.node_by_id("replacement").is_some());
        assert!(page.node_by_id("original").is_none());
    }
    #[test]
    fn parser_written_style_discovery_preserves_synchronous_markup_before_later_script() {
        let quote = |text: &str| serde_json::to_string(text).unwrap().replace('<', "\\u003c");
        let source = format!(
            "<body><script>document.write({});globalThis.visibleBeforeReturn=!!document.getElementById('after');document.write({})</script><p id=tail>tail</p>",
            quote("<style>i{color:green}</style><i id=after>after</i>"),
            quote("<script>globalThis.laterScriptSaw=!!document.getElementById('after')</script>")
        );
        let mut page = RustExecutableDocument::load(&source).unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(visibleBeforeReturn)+':'+String(laterScriptSaw)")
                .unwrap(),
            "true:true"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn parser_written_handler_attributes_do_not_delay_following_markup() {
        let mut page=RustExecutableDocument::load("<body><script>document.write('<button id=button onclick=\"globalThis.clicked=true\">click</button><i id=after>after</i>');globalThis.sawAfter=!!document.getElementById('after');document.getElementById('button').click()</script>").unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(sawAfter)+':'+String(clicked)")
                .unwrap(),
            "true:true"
        );
    }
    #[test]
    fn nested_inline_writes_execute_before_each_caller_resumes() {
        let quote = |text: &str| serde_json::to_string(text).unwrap().replace('<', "\\u003c");
        let inner = "<script>order.push('inner');document.write('<b id=inner>value</b>')</script>";
        let middle = format!(
            "<script>document.write({});order.push(document.getElementById('inner')?'middle':'missing')</script>",
            quote(inner)
        );
        let outer = format!(
            "<body><script>globalThis.order=[];document.write({});order.push('outer')</script>",
            quote(&middle)
        );
        let mut page = RustExecutableDocument::load(&outer).unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "inner,middle,outer"
        );
    }
    #[test]
    fn parser_written_external_script_keeps_remaining_writes_before_network_suffix() {
        let quote = |text: &str| serde_json::to_string(text).unwrap().replace('<', "\\u003c");
        let source = format!(
            "<body><script>document.write({});document.write('k')</script>ed",
            quote("<script src='inner.js'></script>r")
        );
        let resources = BTreeMap::from([(
            "https://example.test/inner.js".into(),
            "document.write('wo')".into(),
        )]);
        let mut page = RustExecutableDocument::load_with_environment(
            &source,
            &resources,
            None,
            Some(Url::parse("https://example.test/root").unwrap()),
        )
        .unwrap();
        assert_eq!(page.agent.host.evaluate_string("Array.from(document.body.childNodes).filter(n=>n.nodeType===3).map(n=>n.data).join('')").unwrap(),"worked");
    }
    #[test]
    fn transferred_written_parser_reopen_retires_old_owner_and_keeps_new_stream() {
        let mut page = RustExecutableDocument::load("<body><p id=original>original</p>").unwrap();
        page.agent
            .host
            .execute(
                "document.open();document.write('<style>p{color:green}</style><p id=old>old</p>')",
            )
            .unwrap();
        page.checkpoint().unwrap();
        page.agent.host.execute("document.close();document.open();document.write('<p id=new>new</p>');document.close()").unwrap();
        page.checkpoint().unwrap();
        assert!(page.node_by_id("new").is_some());
        assert!(page.node_by_id("old").is_none());
        assert!(page.frame_script_preparations.iter().all(|owner| {
            owner.load.realm != page.top_realm
                || owner.load.identity.2
                    == page
                        .agent
                        .host
                        .document_stream_epoch(page.top_realm)
                        .unwrap()
        }));
    }
    #[test]
    fn detached_document_write_keeps_tokenizer_between_calls_and_scripts_inert() {
        let mut page = RustExecutableDocument::load("<p id=original>original</p>").unwrap();
        page.agent.host.execute("const detached=document.implementation.createHTMLDocument('');detached.open();detached.write('<i id=');detached.write('\"part\">value');globalThis.detachedImmediate=detached.getElementById('part').textContent;detached.write('</i><script>globalThis.detachedRan=true<'+ '/script>');detached.close()").unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("detachedImmediate+':'+String(globalThis.detachedRan)")
                .unwrap(),
            "value:undefined"
        );
        assert!(page.node_by_id("original").is_some());
    }
    #[test]
    fn parser_written_frame_has_initial_native_document_before_write_returns() {
        let mut page=RustExecutableDocument::load("<body><script>document.write('<iframe id=writtenFrame></iframe>');const child=document.getElementById('writtenFrame').contentDocument;globalThis.hadInitial=!!child;child.open();child.write('<b id=child>child</b>');globalThis.childBeforeReturn=child.getElementById('child').textContent;child.close()</script><p id=tail>tail</p>").unwrap();
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(hadInitial)+':'+childBeforeReturn")
                .unwrap(),
            "true:child"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn written_child_document_uses_its_own_tokenizer_and_is_visible_before_close() {
        let mut page =
            RustExecutableDocument::load("<body><p id=parent>parent</p><iframe id=f></iframe>")
                .unwrap();
        page.agent.host.execute("const d=document.getElementById('f').contentDocument;d.open();d.write('<i id=');d.write('\"child\">child');globalThis.childImmediate=d.getElementById('child').textContent;d.write('</i>');d.close()").unwrap();
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("childImmediate").unwrap(),
            "child"
        );
        assert!(page.node_by_id("parent").is_some());
    }
    #[test]
    fn xml_observers_run_before_script_preparation_and_can_make_script_inert() {
        let source = "<html xmlns='http://www.w3.org/1999/xhtml'><body><script>globalThis.ran=[];new MutationObserver(records=>{for(const record of records)for(const node of record.addedNodes)if(node.nodeType===1 &amp;&amp; node.localName==='script')node.type='text/plain'}).observe(document.body,{childList:true,subtree:true})</script><script>ran.push('classic')</script><script type='module'>ran.push('module')</script><script src='must-not-fetch.js' async=''/><script src='must-not-fetch-defer.js' defer=''/></body></html>";
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Ok(source.into())].into_iter(),
            )),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/prepare.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        for _ in 0..64 {
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
        assert_eq!(
            page.agent.host.evaluate_string("ran.join(',')").unwrap(),
            ""
        );
        assert!(
            page.frame_script_preparations
                .iter()
                .all(|owner| owner.ordered.is_empty() && owner.asynchronous.is_empty())
        );
    }
    #[test]
    fn xml_document_write_rejection_preserves_parser_suffix_and_script_order() {
        let source = "<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.order=[];try{document.write('bad')}catch(e){order.push(e.name)}order.push(document.getElementById('later')?'bad-suffix':'early')</script><p id='later'/><script>order.push(document.getElementById('later')?'later':'missing')</script></html>";
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Ok(source.into())].into_iter(),
            )),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/order.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        for _ in 0..64 {
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "InvalidStateError,early,later"
        );
        assert!(page.node_by_id("later").is_some());
    }
    #[test]
    fn malformed_xml_tail_cancels_queued_detached_deferred_scripts() {
        let source = "<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.executed=['prefix']</script><script type='module'>globalThis.executed.push('deferred')</script><broken></html>";
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Ok(source.into())].into_iter(),
            )),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/bad.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        for _ in 0..64 {
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("executed.join(',')")
                .unwrap(),
            "prefix"
        );
        assert!(
            page.agent
                .dom
                .borrow()
                .query_selector(page.agent.dom.borrow().document(), "parsererror")
                .unwrap()
                .is_some()
        );
        assert!(!page.parser_pending());
        assert!(
            page.frame_script_preparations
                .iter()
                .all(|owner| owner.xml_deferred.is_empty() && owner.ordered.is_empty())
        );
    }
    #[test]
    fn selected_frame_parser_clear_preserves_parent_document() {
        let page = RustExecutableDocument::load(
            "<p id=parent>parent</p><iframe srcdoc='<p id=child>child</p>'></iframe>",
        )
        .unwrap();
        let child = page
            .automation_realms()
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap();
        let dom = page.agent.host.automation_dom(child.realm).unwrap();
        dom.borrow_mut()
            .clear_document_in(child.document_root)
            .unwrap();
        assert!(page.node_by_id("parent").is_some());
        assert!(
            dom.borrow()
                .children(child.document_root)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn oversized_html_token_yields_without_starving_independent_script() {
        let source = format!(
            "<p>{}</p><script>globalThis.finishedToken=true</script>",
            "雪".repeat(65536)
        );
        let stream = crate::host::DocumentStream::html(Box::new(vec![Ok(source)].into_iter()));
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/large").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap();
        page.resume_pending_parser_step().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(globalThis.finishedToken)")
                .unwrap(),
            "undefined"
        );
        page.agent
            .host
            .execute("globalThis.interleaved=42")
            .unwrap();
        for _ in 0..64 {
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(finishedToken)+':'+interleaved")
                .unwrap(),
            "true:42"
        );
    }
    #[test]
    fn parser_write_input_capacity_rejects_without_losing_following_suffix() {
        let mut page=RustExecutableDocument::load("<script>globalThis.limited=false;try{document.write('x'.repeat(1024*1024+1))}catch(e){limited=true}</script><p id=after>after</p>").unwrap();
        assert_eq!(
            page.agent.host.evaluate_string("String(limited)").unwrap(),
            "true"
        );
        assert!(page.node_by_id("after").is_some());
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum XmlPublicationFailure {
    AfterHtmlTokenization,
    AfterHtmlEof,
    BeforeStylesBarrier,
    BeforeEncodingRestore,
    BeforeSetup,
    BeforeHtmlPublication,
    AfterScriptExecution,
    BeforeScriptCommit,
    BeforeStyleAdmission,
    BeforeStyleEvent,
    BeforeReadiness,
    BeforeLifecycle,
}
#[cfg(test)]
thread_local! {static XML_SETUP_FAILURE_REALM:std::cell::Cell<Option<RealmId>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
thread_local! {static XML_PUBLICATION_FAILURE:std::cell::Cell<Option<XmlPublicationFailure>>=const{std::cell::Cell::new(None)};}
#[cfg(test)]
pub(super) fn inject_stylesheet_event_failure() {
    XML_PUBLICATION_FAILURE.with(|failure|failure.set(Some(XmlPublicationFailure::BeforeStyleEvent)));
}
#[cfg(test)]
pub(super) fn xml_publication_failure(point: XmlPublicationFailure) -> Result<(), HostError> {
    if XML_PUBLICATION_FAILURE.with(|failure| {
        if failure.get() == Some(point) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        Err(HostError::Execution(
            "injected XML publication failure".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod xml_publication_recovery_tests {
    use super::*;
    fn page(source: &str) -> RustExecutableDocument {
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Ok(source.to_owned())].into_iter(),
            )),
            content_type: Some("application/xhtml+xml".into()),
            encoding: Default::default(),
        };
        RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/recovery.xhtml").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap()
    }
    fn recover(page: &mut RustExecutableDocument) {
        for _ in 0..96 {
            for owner in &mut page.frame_script_preparations {
                owner.parser_retry = None;
            }
            page.resume_pending_parser_step().unwrap();
            if !page.parser_pending() {
                break;
            }
        }
    }
    fn html_page(source: &str) -> RustExecutableDocument {
        RustExecutableDocument::load_stream_with_limits(
            crate::host::DocumentStream::html(Box::new(vec![Ok(source.to_owned())].into_iter())),
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/recovery.html").unwrap()),
            64 * 1024 * 1024,
            true,
        )
        .unwrap()
    }
    #[test]
    fn parser_constructs_before_attributes_and_insertion_and_retains_fallback_identity() {
        let mut page = html_page(
            r#"<body><script>
          globalThis.constructions=[];globalThis.callbackChildren=[];globalThis.failures=[];window.onerror=(message,url,line,column,error)=>{failures.push(error.name);return true};
          customElements.define('x-parser-native',class extends HTMLElement{static observedAttributes=['id'];connectedCallback(){callbackChildren.push(this.childNodes.length)}attributeChangedCallback(){callbackChildren.push(this.childNodes.length)}constructor(){super();constructions.push([this.parentNode===null,this.attributes.length,this.childNodes.length]);for(const operation of [()=>document.open(),()=>document.write('bad')]){try{operation();throw Error('markup was allowed')}catch(error){if(error.name!=='InvalidStateError')throw error}}}});
          customElements.define('x-parser-bad',class extends HTMLElement{constructor(){super();globalThis.retainedBad=this;this.id='constructor-owned'}});
        </script><x-parser-native id='constructed'><span id='child'></span></x-parser-native><x-parser-bad id='fallback'><b id='fallback-child'></b></x-parser-bad><p id='tail'></p>"#,
        );
        recover(&mut page);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("JSON.stringify(constructions)")
                .unwrap(),
            "[[true,0,0]]"
        );
        assert_eq!(page.agent.host.evaluate_string("String(callbackChildren.join()==='0,0' && document.getElementById('constructed').firstElementChild.id==='child' && document.getElementById('fallback') instanceof HTMLUnknownElement && retainedBad.id==='constructor-owned' && retainedBad!==document.getElementById('fallback') && document.getElementById('fallback-child').parentNode===document.getElementById('fallback') && failures.join()==='NotSupportedError')").unwrap(),"true");
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn xml_parser_uses_synchronous_custom_element_construction() {
        let mut page = page(
            r#"<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.xmlCalls=[];customElements.define('x-xml-native',class extends HTMLElement{constructor(){super();xmlCalls.push(this.parentNode===null &amp;&amp; this.attributes.length===0)}})</script><x-xml-native id='xml-result'/></html>"#,
        );
        recover(&mut page);
        assert_eq!(page.agent.host.evaluate_string("String(xmlCalls.length===1 && xmlCalls[0] && document.getElementById('xml-result') instanceof customElements.get('x-xml-native'))").unwrap(),"true");
    }
    #[cfg(unix)]
    #[test]
    fn parser_error_insertion_point_depends_on_fetch_admission() {
        struct Missing(Arc<std::sync::atomic::AtomicUsize>);
        impl crate::network::NetworkBroker for Missing {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                Ok(crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 404,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: Vec::new(),
                })
            }
        }
        for (url, expected, requests) in [
            ("missing.js", "Some text", 1),
            ("http://this is not parseable:-80/", "text", 0),
        ] {
            let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let source = format!(
                "Some <script src='{url}' onerror=\"document.write('text');document.close();globalThis.observed=document.documentElement.textContent\"></script>"
            );
            let mut page = RustExecutableDocument::load_stream_with_limits(
                crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(source)))),
                &BTreeMap::new(),
                Some(SharedNetworkBroker::new(Missing(count.clone()))),
                Some(Url::parse("https://example.test/").unwrap()),
                256 * 1024 * 1024,
                true,
            )
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while page.parser_pending() {
                page.resume_pending_parser().unwrap();
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(
                page.agent.host.evaluate_string("observed").unwrap(),
                expected
            );
            assert_eq!(count.load(std::sync::atomic::Ordering::Acquire), requests);
        }
    }

    #[test]
    fn html_publication_retry_retains_tokenizer_suffix_and_unexecuted_event() {
        let mut page = html_page(
            "<body><script>globalThis.count=(globalThis.count||0)+1</script><p id=tail>tail</p>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeHtmlPublication)));
        assert!(page.resume_pending_parser().is_err());
        assert!(page.parser_pending());
        assert_eq!(
            page.agent.host.evaluate_string("typeof count").unwrap(),
            "undefined"
        );
        recover(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("String(count)").unwrap(),
            "1"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn html_script_capacity_retry_keeps_prepared_event_and_source_suffix() {
        let mut page = html_page(
            "<body><script>globalThis.capacityRuns=(globalThis.capacityRuns||0)+1</script><i id=tail>tail</i>",
        );
        let budget = page.dynamic_scripts.source_bytes.clone();
        let before = budget.swap(
            MAX_QUEUED_SCRIPT_SOURCE_BYTES,
            std::sync::atomic::Ordering::AcqRel,
        );
        let result = page.resume_pending_parser();
        budget.store(before, std::sync::atomic::Ordering::Release);
        assert!(matches!(result, Err(HostError::LimitExceeded { .. })));
        assert!(page.parser_pending());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof capacityRuns")
                .unwrap(),
            "undefined"
        );
        recover(&mut page);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(capacityRuns)")
                .unwrap(),
            "1"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[test]
    fn html_post_script_retry_does_not_replay_body_or_nested_write() {
        let mut page = html_page(
            "<body><script>globalThis.runs=(globalThis.runs||0)+1;document.write('<b id=written>value</b>');globalThis.immediate=!!document.getElementById('written')</script><i id=tail>tail</i>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::AfterScriptExecution)));
        assert!(page.resume_pending_parser().is_err());
        assert!(page.parser_pending());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(runs)+':'+immediate")
                .unwrap(),
            "1:true"
        );
        recover(&mut page);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(runs)+':'+document.querySelectorAll('#written').length")
                .unwrap(),
            "1:1"
        );
        assert!(page.node_by_id("tail").is_some());
    }
    #[cfg(unix)]
    #[test]
    fn frame_html_post_script_retry_preserves_child_prefix() {
        let mut page = html_page(
            r#"<iframe id=f srcdoc="<body><script>globalThis.childRuns=(globalThis.childRuns||0)+1</script><p id=tail>tail</p>"></iframe>"#,
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::AfterScriptExecution)));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut failed = false;
        while std::time::Instant::now() < deadline {
            if page.poll_context_events().is_err() {
                failed = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(failed);
        for _ in 0..96 {
            for owner in &mut page.frame_script_preparations {
                owner.parser_retry = None;
            }
            page.poll_context_events().unwrap();
        }
        assert_eq!(page.agent.host.evaluate_string("String(document.getElementById('f').contentWindow.childRuns)+':'+!!document.getElementById('f').contentDocument.getElementById('tail')").unwrap(),"1:true");
    }
    #[cfg(unix)]
    #[test]
    fn stylesheet_event_admission_retry_keeps_installed_sheet_and_blocks_next_script() {
        let mut page = retained_fixture_stylesheet_stream(
            "<script>globalThis.sheetEvents=0</script><link rel=stylesheet href=style.css onload='sheetEvents++'><script>globalThis.sheetObserved=getComputedStyle(document.documentElement).marginTop</script>",
            "text/html", "https://example.test/recovery.html", "html{margin-top:19px}",
        );
        let mut page = page.unwrap();
        page.resources.insert(
            "https://example.test/style.css".into(),
            "html{margin-top:19px}".into(),
        );
        let sources = page.resources.clone();
        page.agent.host.set_module_sources(&sources);
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeStyleEvent)));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut failed = false;
        while std::time::Instant::now() < deadline {
            if page.poll_context_events().is_err() {
                failed = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(failed);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof sheetObserved+':'+sheetEvents")
                .unwrap(),
            "undefined:0"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while page.parser_pending() {
            for owner in &mut page.frame_script_preparations {
                owner.parser_retry = None;
            }
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        page.checkpoint().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("sheetObserved+':'+sheetEvents")
                .unwrap(),
            "19px:1"
        );
    }
    #[test]
    fn html_retry_owner_cannot_resume_after_stop_or_document_replacement() {
        for replacement in [false, true] {
            let mut page = html_page(
                "<body><script>globalThis.prefixRuns=(globalThis.prefixRuns||0)+1</script><script>globalThis.staleSuffixRan=true</script>",
            );
            XML_PUBLICATION_FAILURE
                .with(|failure| failure.set(Some(XmlPublicationFailure::AfterScriptExecution)));
            assert!(page.resume_pending_parser().is_err());
            if replacement {
                page.agent
                    .host
                    .execute("document.open();document.write('<p id=new>new</p>');document.close()")
                    .unwrap();
            } else {
                page.agent.host.execute("stop()").unwrap();
            }
            recover(&mut page);
            assert_eq!(
                page.agent
                    .host
                    .evaluate_string("String(prefixRuns)+':'+typeof staleSuffixRan")
                    .unwrap(),
                "1:undefined"
            );
            if replacement {
                assert!(page.node_by_id("new").is_some());
            }
        }
    }
    #[test]
    fn terminal_html_input_failure_cannot_publish_successful_document_lifecycle() {
        let mut page = RustExecutableDocument::load("<body>initial</body>").unwrap();
        page.agent.host.execute("globalThis.lateLifecycle=0;document.addEventListener('DOMContentLoaded',()=>lateLifecycle++);addEventListener('load',()=>lateLifecycle++)").unwrap();
        let dom = page.agent.dom.clone();
        let stream = crate::host::DocumentStream {
            content_language: None,
            chunks: Box::new(crate::host::ReadyDocumentInput(
                vec![Err(HostError::Execution("terminal input failure".into()))].into_iter(),
            )),
            content_type: Some("text/html".into()),
            encoding: Default::default(),
        };
        let parser = LiveParserState::new(stream, &dom, &mut page.agent.host).unwrap();
        let load = PreparedDocumentLoad {
            realm: page.top_realm,
            identity: page
                .agent
                .host
                .automation_realm_identity(page.top_realm)
                .unwrap(),
        };
        let mut owner = FrameScriptPreparation::new(FrameScriptInput {
            load,
            parent: page.top_realm,
            frame: load.identity.0,
            dom,
            records: vec![],
            lifecycle_allowed: true,
            _refresh_url: None,
            limits: &page.dynamic_scripts,
            host: &mut page.agent.host,
        })
        .unwrap();
        owner.root_document = true;
        owner.live_parser = Some(parser);
        page.frame_script_preparations.push_back(owner);
        assert!(page.poll_frame_script_preparations().is_err());
        assert!(page.frame_script_preparations.is_empty());
        assert!(page.agent.host.realm_document_load_stopped(page.top_realm));
        page.poll_frame_script_preparations().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(lateLifecycle)")
                .unwrap(),
            "0"
        );
    }
    #[test]
    fn preparation_setup_failure_preserves_owner_and_retry_deadline() {
        let mut page = page(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.setupRuns=(globalThis.setupRuns||0)+1</script></html>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeSetup)));
        // Scheduled XML starts in pending_buffered. Drive admission through
        // the production scheduler before expecting the owner's setup hook.
        let mut failure = None;
        for _ in 0..32 {
            if let Err(error) = page.resume_pending_parser_step() {
                failure = Some(error);
                break;
            }
        }
        assert!(
            failure.is_some(),
            "scheduled XML never reached injected setup failure"
        );
        assert_eq!(page.frame_script_preparations.len(), 1);
        let owner = page.frame_script_preparations.front_mut().unwrap();
        assert!(owner.xml_parser.is_some());
        assert!(owner.parser_retry.is_some());
        assert!(!owner.runnable(&mut page.agent.host));
        recover(&mut page);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(setupRuns)")
                .unwrap(),
            "1"
        );
    }
    #[cfg(unix)]
    #[test]
    fn stylesheet_admission_failure_preserves_blocking_queue_before_following_xml_script() {
        let mut page = retained_fixture_stylesheet_stream(
            "<html xmlns='http://www.w3.org/1999/xhtml'><head><link rel='stylesheet' href='style.css'/><script>globalThis.sheetReady=getComputedStyle(document.documentElement).marginTop</script></head></html>",
            "application/xhtml+xml", "https://example.test/recovery.xhtml", "html{margin-top:17px}",
        );
        let mut page = page.unwrap();
        page.resources.insert(
            "https://example.test/style.css".into(),
            "html{margin-top:17px}".into(),
        );
        let sources = page.resources.clone();
        page.agent.host.set_module_sources(&sources);
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeStyleAdmission)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        let owner = page.frame_script_preparations.front().unwrap();
        assert_eq!(owner.style_nodes.len(), 1);
        assert!(owner.styles_block_scripts(&page.agent.host).unwrap());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof sheetReady")
                .unwrap(),
            "undefined"
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while page.parser_pending() {
            for owner in &mut page.frame_script_preparations {
                owner.parser_retry = None;
            }
            page.poll_context_events().unwrap();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            page.agent.host.evaluate_string("sheetReady").unwrap(),
            "17px"
        );
    }
    #[cfg(unix)]
    #[test]
    fn stop_after_stylesheet_admission_failure_does_not_run_following_script() {
        let mut page = page(
            "<html xmlns='http://www.w3.org/1999/xhtml'><head><link rel='stylesheet' href='must-not-fetch.css'/><script>globalThis.staleScriptRan=true</script></head></html>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeStyleAdmission)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        assert_eq!(
            page.frame_script_preparations
                .front()
                .unwrap()
                .style_nodes
                .len(),
            1
        );
        page.agent.host.execute("stop()").unwrap();
        for owner in &mut page.frame_script_preparations {
            owner.parser_retry = None;
        }
        page.poll_frame_script_preparations().unwrap();
        assert!(page.frame_script_preparations.is_empty());
        assert_eq!(
            page.agent
                .host
                .evaluate_string("typeof staleScriptRan")
                .unwrap(),
            "undefined"
        );
    }
    #[test]
    fn xml_script_admission_failure_retains_parser_event_and_unstarted_script() {
        let mut page = page(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script id='first'>globalThis.order=['first']</script><p id='suffix'/><script>order.push('second')</script></html>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeScriptCommit)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        let owner = page.frame_script_preparations.front().unwrap();
        assert!(owner.xml_parser.is_some());
        assert_eq!(owner.parser_events.len(), 1);
        assert!(owner.ordered.is_empty());
        let node = page.node_by_id("first").unwrap();
        assert!(
            !page
                .agent
                .dom
                .borrow()
                .script_already_started(node)
                .unwrap()
        );
        assert!(owner.parser_retry.is_some());
        assert!(
            !page
                .frame_script_preparations
                .front_mut()
                .unwrap()
                .runnable(&mut page.agent.host)
        );
        recover(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("order.join(',')").unwrap(),
            "first,second"
        );
        assert!(page.node_by_id("suffix").is_some());
        assert!(
            page.frame_script_preparations
                .iter()
                .all(|owner| owner.parser_events.is_empty() && owner.xml_completion.is_none())
        );
    }
    #[test]
    fn xml_lifecycle_handoff_failure_retains_owner_and_recovers_once() {
        let mut page = page(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.calls=['script'];document.addEventListener('DOMContentLoaded',()=>calls.push('dom'));addEventListener('load',()=>calls.push('load'))</script></html>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeLifecycle)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        let owner = page.frame_script_preparations.front().unwrap();
        assert!(!owner.lifecycle_queued);
        assert!(!owner.lifecycle_active);
        assert!(owner.parser_retry.is_some());
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "script"
        );
        recover(&mut page);
        assert_eq!(
            page.agent.host.evaluate_string("calls.join(',')").unwrap(),
            "script,dom,load"
        );
    }
    #[test]
    fn xml_eof_failure_retains_completion_and_readiness_runs_once() {
        let mut page = page("<html xmlns='http://www.w3.org/1999/xhtml'><p id='last'/></html>");
        page.agent.host.execute("globalThis.interactive=0;document.addEventListener('readystatechange',()=>{if(document.readyState==='interactive')interactive++})").unwrap();
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeReadiness)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        let owner = page.frame_script_preparations.front().unwrap();
        assert!(owner.xml_parser.is_none());
        assert!(owner.xml_completion.is_some());
        assert!(!owner.lifecycle_queued);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("document.readyState+':'+interactive")
                .unwrap(),
            "loading:0"
        );
        recover(&mut page);
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(interactive)")
                .unwrap(),
            "1"
        );
        assert!(page.node_by_id("last").is_some());
        assert!(!page.parser_pending());
    }
    #[test]
    fn xml_completion_failure_then_stop_drops_old_owner_without_more_scripts() {
        let mut page = page(
            "<html xmlns='http://www.w3.org/1999/xhtml'><script>globalThis.oldRan=true</script></html>",
        );
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeScriptCommit)));
        let mut failed = false;
        for _ in 0..32 {
            if page.resume_pending_parser_step().is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        page.agent.host.execute("stop()").unwrap();
        for owner in &mut page.frame_script_preparations {
            owner.parser_retry = None;
        }
        page.poll_frame_script_preparations().unwrap();
        assert_eq!(
            page.agent
                .host
                .evaluate_string("String(globalThis.oldRan)")
                .unwrap(),
            "undefined"
        );
        assert!(page.frame_script_preparations.is_empty());
    }
}

#[cfg(all(test, unix))]
mod html_encoding_restart_regressions {
    use super::*;
    fn load(prefix: &str, header: &str) -> RustExecutableDocument {
        let mut bytes = format!("<!doctype html><head>{}{prefix}", " ".repeat(1100)).into_bytes();
        bytes.extend_from_slice(b"</head><body><p id=value>");
        bytes.push(0xc0);
        bytes.extend_from_slice(
            b"</p><script>globalThis.result=document.getElementById('value').textContent</script>",
        );
        let stream = crate::html_encoding_replay::BufferedDecodedInput::stream(
            bytes,
            Some(header.into()),
            None,
            None,
        );
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/late-encoding").unwrap()),
            16 * 1024 * 1024,
            true,
        )
        .unwrap();
        // External deferred preparations finish on the resource executor.
        // Yield while waiting rather than assuming a fixed number of tight polls
        // gives another thread time to deliver its completion.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while page.parser_pending() && std::time::Instant::now() < deadline {
            page.resume_pending_parser_step().unwrap();
            if page.parser_pending() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(
            !page.parser_pending(),
            "encoding replay did not finish its deferred preparations"
        );
        page
    }
    #[test]
    fn detected_encoding_is_visible_to_script_in_the_selecting_input_chunk() {
        let text = "Ελληνικά 日本語 Русский ".repeat(80);
        let bytes=format!("<!doctype html><head>{}</head><body><p id=value>{text}</p><script>globalThis.observedEncoding=document.characterSet;globalThis.observedText=document.getElementById('value').textContent</script>"," ".repeat(1100)).into_bytes();
        let stream = crate::html_encoding_replay::BufferedDecodedInput::stream(
            bytes,
            Some("text/html".into()),
            None,
            None,
        );
        let mut page = RustExecutableDocument::load_stream_with_limits(
            stream,
            &BTreeMap::new(),
            None,
            Some(Url::parse("https://example.test/detection").unwrap()),
            16 * 1024 * 1024,
            true,
        )
        .unwrap();
        for _ in 0..256 {
            if !page.parser_pending() {
                break;
            }
            page.resume_pending_parser_step().unwrap();
        }
        assert!(!page.parser_pending());
        assert_eq!(
            page.agent.host.evaluate_string("observedEncoding").unwrap(),
            "UTF-8"
        );
        assert_eq!(
            page.agent.host.evaluate_string("observedText").unwrap(),
            text
        );
    }
    #[test]
    fn original_head_late_meta_replays_native_document_and_script_observes_new_encoding() {
        let mut page = load(
            "<script type=application/json>{}</script><!--non-ascii is allowed--><meta charset=windows-1251>",
            "text/html",
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("result+':'+document.characterSet")
                .unwrap(),
            "А:windows-1251"
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("document.querySelectorAll('#value').length.toString()")
                .unwrap(),
            "1"
        );
    }
    #[test]
    fn head_exit_template_author_script_and_transport_certainty_prevent_late_switch() {
        for prefix in [
            "</head><meta charset=windows-1251>",
            "<template><meta charset=windows-1251></template>",
            "<script>globalThis.early=(globalThis.early||0)+1</script><meta charset=windows-1251>",
        ] {
            let mut page = load(prefix, "text/html");
            assert_eq!(
                page.agent.host.evaluate_string("result").unwrap(),
                "À",
                "{prefix}"
            );
            if prefix.starts_with("<script>") {
                assert_eq!(
                    page.agent.host.evaluate_string("early.toString()").unwrap(),
                    "1"
                );
            }
        }
        let mut page = load(
            "<meta charset=windows-1251>",
            "text/html; charset=windows-1252",
        );
        assert_eq!(page.agent.host.evaluate_string("result").unwrap(), "À");
    }
    #[test]
    fn srcdoc_is_certain_authored_text_and_archive_does_not_gain_decoder_authority() {
        let source = format!(
            "<head>{}<meta charset=windows-1251></head><body><p id=v>À</p>",
            " ".repeat(1100)
        );
        let stream =
            crate::host::DocumentStream::html(Box::new(std::iter::once(Ok(source.clone()))));
        assert_eq!(
            stream.chunks.encoding_confidence(),
            oryn_net::HtmlEncodingConfidence::Certain
        );
        let budget = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let archive = frame_navigation_history::ArchivedSource::capture(
            &source,
            Some(Url::parse("https://example.test/").unwrap()),
            &budget,
        )
        .unwrap();
        assert_eq!(archive.source.as_ref(), source);
    }

    #[test]
    fn buffered_data_frame_keeps_original_bytes_through_native_policy_and_parser() {
        let mut bytes = format!(
            "<head>{}<meta charset=windows-1251></head><body><p id=value>",
            " ".repeat(1100)
        )
        .into_bytes();
        bytes.push(0xc0);
        bytes.extend_from_slice(b"</p>");
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        let mut page = RustExecutableDocument::load(&format!(
            "<iframe id=child src='data:text/html;base64,{encoded}'></iframe>"
        ))
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.checkpoint().unwrap();
            let top = page.agent.host.active_realm();
            let mut ready = String::new();
            if let Some(child) = page.automation_realms().into_iter().find(|realm| {
                realm.parent_realm.is_some()
                    && realm.live
                    && page
                        .agent
                        .host
                        .automation_dom(realm.realm)
                        .is_some_and(|dom| {
                            dom.borrow()
                                .get_element_by_id(realm.document_root, "value")
                                .ok()
                                .flatten()
                                .is_some()
                        })
            }) {
                page.agent.host.activate_realm(child.realm).unwrap();
                ready=page.agent.host.evaluate_string("document.getElementById('value')?document.getElementById('value').textContent+':'+document.characterSet:''").unwrap();
                page.agent.host.activate_realm(top).unwrap();
            }
            if !ready.is_empty() {
                assert_eq!(ready, "А:windows-1251");
                assert!(
                    page.automation_realms()
                        .into_iter()
                        .filter(|realm| realm.live)
                        .all(|realm| page
                            .agent
                            .host
                            .network_broker_for_realm(realm.realm)
                            .is_none()),
                    "local data commit cannot install network authority"
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "buffered frame did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn dynamic_blob_frame_uses_local_registry_without_http_broker() {
        let mut page = RustExecutableDocument::load("<body></body>").unwrap();
        assert!(
            page.agent
                .host
                .network_broker_for_realm(page.top_realm)
                .is_none()
        );
        page.agent.host.execute("globalThis.localFrameUrl=URL.createObjectURL(new Blob(['<p id=blobValue>local blob</p>'],{type:'text/html'}));const frame=document.createElement('iframe');frame.src=localFrameUrl;document.body.append(frame)").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            page.checkpoint().unwrap();
            if page.automation_realms().into_iter().any(|realm| {
                realm.parent_realm.is_some()
                    && realm.live
                    && page
                        .agent
                        .host
                        .automation_dom(realm.realm)
                        .is_some_and(|dom| {
                            dom.borrow()
                                .get_element_by_id(realm.document_root, "blobValue")
                                .ok()
                                .flatten()
                                .is_some()
                        })
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "local blob frame did not finish: {:?}",
                page.diagnostics
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            page.automation_realms()
                .into_iter()
                .filter(|realm| realm.live)
                .all(|realm| page
                    .agent
                    .host
                    .network_broker_for_realm(realm.realm)
                    .is_none()),
            "local blob commit cannot install network authority"
        );
        page.agent
            .host
            .execute("URL.revokeObjectURL(localFrameUrl)")
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sandboxed_creator_blob_frame_preserves_inherited_origin_isolation() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let handle = context.new_page();
        let url = Url::parse("https://example.test/root.html").unwrap();
        context
            .install_document_security(handle.page_id(), url.to_string(), &[])
            .unwrap();
        let client = crate::context_service::SharedContextClient::direct(context, handle.page_id());
        let mut page = RustExecutableDocument::load_with_context_and_limits(
            r#"<body><iframe sandbox="allow-scripts" srcdoc="<body>creator</body>"></iframe>"#,
            &BTreeMap::new(),
            None,
            Some(url),
            256 * 1024 * 1024,
            (client, handle.page_id(), None),
        )
        .unwrap();
        let creator = *page.frame_realms.keys().next().unwrap();
        page.agent.host.activate_realm(creator).unwrap();
        page.agent
            .host
            .execute(
                r#"
            globalThis.result='pending';
            globalThis.blobFrame=document.createElement('iframe');
            blobFrame.src=URL.createObjectURL(new Blob([
                '<script>window.test_result="blob-frame-ok";</script>'
            ],{type:'text/html'}));
            blobFrame.onload=()=>{try {result=blobFrame.contentWindow.test_result}
                catch(e){result=e.name+':'+e.message}};
            document.body.append(blobFrame);
        "#,
            )
            .unwrap();
        page.agent.host.activate_realm(page.top_realm).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let result = loop {
            page.checkpoint().unwrap();
            page.agent.host.activate_realm(creator).unwrap();
            let result = page.agent.host.evaluate_string("result").unwrap();
            page.agent.host.activate_realm(page.top_realm).unwrap();
            if result != "pending" {
                break result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Blob frame did not load: {:?}",
                page.diagnostics
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        // HTML's sandboxing flag set includes the container document's flags.
        // Determining the origin checks the sandboxed-origin flag before the
        // Blob URL origin, so this descendant must remain inaccessible.
        assert_eq!(result, "SecurityError:SecurityError");
    }

    #[test]
    fn initial_blob_fragment_scrolls_and_notifies_retained_window_handler() {
        for url in [None, Some(Url::parse("https://example.test/root").unwrap())] {
            let mut page = RustExecutableDocument::load_with_environment(
                "<!doctype html><body>",
                &BTreeMap::new(),
                None,
                url,
            )
            .unwrap();
            page.agent
                .host
                .execute(
                    r#"
            globalThis.frame=document.createElement('iframe');
            frame.src=URL.createObjectURL(new Blob([
                '<!doctype html><style>body{margin:0}.block{height:5000px}</style>' +
                '<body><div class=block></div><a id=target></a><div class=block></div>'
            ],{type:'text/html'}))+'#target';
            document.body.append(frame);
            globalThis.events=[];
            frame.contentWindow.onscroll=()=>events.push('scroll:'+frame.contentWindow.scrollY);
            frame.onload=()=>{
                events.push('load:'+frame.contentWindow.scrollY);
                try { const w=frame.contentWindow,t=w.document.getElementById('target');
                globalThis.detail=[w.location.href, JSON.stringify(t.getBoundingClientRect()), w.document.documentElement.scrollHeight,w.innerHeight,typeof w.onscroll].join('|');
                } catch(e){ globalThis.detail=e.name+':'+e.message+':'+frame.contentWindow.document.body.innerHTML; }
            };
        "#,
                )
                .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            let result = loop {
                page.checkpoint().unwrap();
                let result = page.agent.host.evaluate_string("events.join(',')").unwrap();
                if result.contains("load:") && result.contains("scroll:") {
                    break result;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "fragment events: {result}; detail: {}; diagnostics: {:?}",
                    page.agent
                        .host
                        .evaluate_string("globalThis.detail")
                        .unwrap(),
                    page.diagnostics
                );
                std::thread::sleep(Duration::from_millis(1));
            };
            assert!(result.contains("load:5000"), "{result}");
            assert!(result.contains("scroll:5000"), "{result}");
        }
    }

    #[test]
    fn encoding_policy_retry_keeps_decoder_bytes_and_event_before_commit() {
        let mut page = RustExecutableDocument::load("<head></head>").unwrap();
        let dom = page.agent.dom.clone();
        let document = dom.borrow().document();
        dom.borrow_mut().clear_document_in(document).unwrap();
        let mut bytes = format!(
            "<head>{}<meta charset=windows-1251></head><body><p id=value>",
            " ".repeat(1100)
        )
        .into_bytes();
        bytes.push(0xc0);
        bytes.extend_from_slice(b"</p>");
        let stream = crate::html_encoding_replay::BufferedDecodedInput::stream(
            bytes,
            Some("text/html".into()),
            None,
            None,
        );
        let parser = LiveParserState::new(stream, &dom, &mut page.agent.host).unwrap();
        let realm = page.agent.host.active_realm();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        let mut owner = FrameScriptPreparation::root_html(
            PreparedDocumentLoad { realm, identity },
            dom.clone(),
            &page.dynamic_scripts,
            &mut page.agent.host,
        )
        .unwrap();
        let old_resources = std::sync::Arc::downgrade(&parser.resource_scheduler.lifetime);
        owner.live_parser = Some(parser);
        XML_PUBLICATION_FAILURE
            .with(|failure| failure.set(Some(XmlPublicationFailure::BeforeEncodingRestore)));
        let mut failed = false;
        for _ in 0..64 {
            if owner.poll_parser(&mut page).is_err() {
                failed = true;
                break;
            }
        }
        assert!(failed);
        let parser = owner.live_parser.as_ref().unwrap();
        assert_eq!(
            parser.source.encoding_confidence(),
            oryn_net::HtmlEncodingConfidence::Tentative
        );
        assert!(parser.source.encoding_restart_needed("windows-1251"));
        assert!(owner.encoding_replay.is_none());
        assert!(
            old_resources.upgrade().is_some(),
            "failed policy preflight retains provisional resource owner"
        );
        assert!(matches!(
            owner.parser_events.front().map(|event| &event.event),
            Some(html::ParserEvent::EncodingIndicator { .. })
        ));
        for _ in 0..64 {
            owner.poll_parser(&mut page).unwrap();
            if owner.live_parser.is_none() {
                break;
            }
        }
        assert!(owner.live_parser.is_none());
        assert!(
            old_resources.upgrade().is_none(),
            "accepted restart drops provisional resource owner"
        );
        assert!(owner.xml_deferred.is_empty());
        assert!(
            owner
                .ordered
                .iter()
                .chain(owner.asynchronous.iter())
                .all(|job| job._inline_reservation.is_none())
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string("document.getElementById('value').textContent")
                .unwrap(),
            "А"
        );
    }

    #[test]
    fn deferred_or_policy_denied_preparation_does_not_commit_tentative_encoding() {
        let mut deferred = load(
            "<script defer src='data:text/javascript,globalThis.deferredRuns%3D(globalThis.deferredRuns%7C%7C0)%2B1'></script><meta charset=windows-1251>",
            "text/html",
        );
        assert_eq!(
            deferred
                .agent
                .host
                .evaluate_string(
                    "document.getElementById('value').textContent+':'+document.characterSet"
                )
                .unwrap(),
            "А:windows-1251"
        );
        assert_eq!(
            deferred
                .agent
                .host
                .evaluate_string("deferredRuns.toString()")
                .unwrap(),
            "1"
        );
        let mut denied = load(
            r#"<meta http-equiv='Content-Security-Policy' content="script-src 'none'"><script>globalThis.blockedRan=true</script><meta charset=windows-1251>"#,
            "text/html",
        );
        assert_eq!(
            denied
                .agent
                .host
                .evaluate_string(
                    "document.getElementById('value').textContent+':'+document.characterSet"
                )
                .unwrap(),
            "А:windows-1251"
        );
        assert_eq!(
            denied
                .agent
                .host
                .evaluate_string("typeof blockedRan")
                .unwrap(),
            "undefined"
        );
    }

    #[test]
    fn importmap_preparation_before_late_meta_is_replayed_without_author_execution() {
        let mut page = load(
            "<script type=importmap>{\"imports\":{\"helper\":\"./helper.js\"}}</script><meta charset=windows-1251>",
            "text/html",
        );
        assert_eq!(
            page.agent
                .host
                .evaluate_string(
                    "document.getElementById('value').textContent+':'+document.characterSet"
                )
                .unwrap(),
            "А:windows-1251"
        );
    }
}

#[cfg(not(unix))]
impl FrameScriptPreparation {
    fn queue_portable_style(
        &mut self,
        page: &mut RustExecutableDocument,
        node: NodeId,
    ) -> Result<(), HostError> {
        let revision = self
            .dom
            .borrow()
            .arena()
            .stylesheet_revision(node)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        if self.portable_styles.contains(&(node, revision)) {
            return Ok(());
        }
        if self.portable_styles.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "parser stylesheet owners".into(),
            });
        }
        page.schedule_stylesheet_with_parser(self.load.realm, node, true)?;
        self.portable_styles.insert((node, revision));
        Ok(())
    }
    fn portable_styles_block_scripts(&self, host: &RawV8Host) -> Result<bool, HostError> {
        for (node, revision) in &self.portable_styles {
            if self.dom.borrow().arena().stylesheet_revision(*node).ok() == Some(*revision)
                && host.cached_stylesheet_blocks_scripts_in_realm(self.load.realm, *node)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "parser_continuation_tests.rs"]
mod parser_continuation_tests;

#[cfg(all(test, unix))]
mod parser_stylesheet_completion_routing_tests {
    use super::*;
    #[test]
    fn retained_ready_is_routed_and_stale_owner_is_terminal_without_publication() {
        let mut page = RustExecutableDocument::load("<head></head>").unwrap();
        let dom = page.agent.dom.clone();
        let realm = page.agent.host.active_realm();
        let identity = page.agent.host.automation_realm_identity(realm).unwrap();
        let node = dom.borrow_mut().create_element("style");
        let document = dom.borrow().document();
        dom.borrow_mut().append_child(document, node).unwrap();
        let revision = dom.borrow().arena().stylesheet_revision(node).unwrap();
        let mut owner = FrameScriptPreparation::root_html(
            PreparedDocumentLoad { realm, identity },
            dom.clone(),
            &page.dynamic_scripts,
            &mut page.agent.host,
        )
        .unwrap();
        let cancel = Arc::new(crate::context_service::ResourceCancellation::default());
        let completion = ParserResourceCompletion::RetainedStylesheet(RetainedStylesheetReady {
            identity: ParserDocumentIdentity {
                realm,
                document: identity.0,
                document_generation: identity.1,
                stream_epoch: identity.2,
            },
            node,
            revision,
            cancellation: cancel.clone(),
            result: Err(RetainedStylesheetFailure::Operational("unused stale response".into())),
        });
        assert!(parser_stylesheet_owner_completion(&completion).unwrap());
        dom.borrow_mut().remove_child(document, node).unwrap();
        let mut scheduler = ParserResourceScheduler::new();
        scheduler.ready_completions.push_back((completion, false));
        owner.styles = Some(scheduler);
        owner.poll_styles(&mut page).unwrap();
        assert!(cancel.flag.load(std::sync::atomic::Ordering::Acquire));
        assert!(!owner.retained_resources());
        assert!(!page.agent.host.realm_has_pending_stylesheet_events(realm));
    }
    #[test]
    fn script_completion_is_rejected_not_applied_by_stylesheet_owner() {
        let page = RustExecutableDocument::load("<script></script>").unwrap();
        let dom = page.agent.dom.clone();
        let realm = page.agent.host.active_realm();
        let document = dom.borrow().document();
        let node = dom.borrow_mut().create_element("script");
        dom.borrow_mut().append_child(document, node).unwrap();
        let record = {
            let scheduler = ParserResourceScheduler::new();
            let mut record = capture_parser_script_node(&page.agent.host, &dom, node).unwrap();
            record.3 = "throw new Error('must not run')".into();
            ParserResourceCompletion::Script {
                realm,
                stream_epoch: 0,
                document_generation: 1,
                document,
                order: 0,
                record,
                asynchronous: true,
                result: Err(ParserScriptFailure::Fetch {
                    message: "wrong kind".into(),
                    source_bytes: scheduler.source_bytes.clone(),
                }),
            }
        };
        assert!(parser_stylesheet_owner_completion(&record).is_err());
    }
}

//! Asynchronous native CSS-image lifecycle. No network operation is awaited by
//! layout getters; completion publishes dimensions and invalidates their revision.
use super::*;
const MAX_IMAGES: usize = 256;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
#[derive(Default)]
pub(super) struct State {
    backend_finalized: bool,
    wanted: BTreeSet<String>,
    layout_wanted: BTreeSet<String>,
    capture_wanted: BTreeSet<String>,
    captures: BTreeMap<u64, BTreeSet<String>>,
    entries: BTreeMap<String, Entry>,
    dependencies: BTreeMap<String, BTreeSet<String>>,
    next: u64,
}
enum Entry {
    Queued,
    Pending(u64),
    Failed,
    Ready {
        resource: oryn_common::v4::PaintImageResource,
        size: (u32, u32),
    },
}
pub(super) struct Pending {
    url: String,
    token: u64,
    document: NodeId,
    generation: u64,
    epoch: u64,
    stream: Option<u64>,
    final_url: Option<String>,
    content_type: Option<String>,
    bytes: Vec<u8>,
}
fn current(state: &V8BindingState, pending: &Pending) -> bool {
    !state.execution_retired()
        && state.document_root() == pending.document
        && state.document_generation() == pending.generation
        && state.document_stream_epoch() == pending.epoch
        && matches!(state.css_images.borrow().entries.get(&pending.url),Some(Entry::Pending(token)) if *token==pending.token)
}
pub(super) fn pending(state: &V8BindingState) -> bool {
    state
        .css_images
        .borrow()
        .entries
        .values()
        .any(|entry| matches!(entry, Entry::Queued | Entry::Pending(_)))
}
pub(super) fn clear(state: &V8BindingState) -> Result<(), V8BindingError> {
    let removed = state.css_images.borrow().entries.keys().cloned().collect();
    {
        let mut images = state.css_images.borrow_mut();
        images.wanted.clear();
        images.layout_wanted.clear();
        images.capture_wanted.clear();
        images.captures.clear();
        images.dependencies.clear();
    }
    cancel(state, &removed)
}
fn publish(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut sizes = BTreeMap::new();
    for (url, entry) in &state.css_images.borrow().entries {
        if let Entry::Ready { resource, size } = entry {
            sizes.insert(url.clone(), *size);
            sizes.insert(resource.url.clone(), *size);
        }
    }
    state
        .dom
        .borrow_mut()
        .set_css_image_intrinsics(state.document_root(), sizes)?;
    let svg = resources(state)
        .into_iter()
        .filter(|resource| resource.media_type == "image/svg+xml")
        .collect();
    state
        .dom
        .borrow_mut()
        .set_css_svg_resources(state.document_root(), svg)?;
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
pub(super) fn resources(state: &V8BindingState) -> Vec<oryn_common::v4::PaintImageResource> {
    state
        .css_images
        .borrow()
        .entries
        .values()
        .filter_map(|entry| {
            if let Entry::Ready { resource, .. } = entry {
                Some(resource.clone())
            } else {
                None
            }
        })
        .collect()
}
pub(super) fn cancel(
    state: &V8BindingState,
    removed: &BTreeSet<String>,
) -> Result<(), V8BindingError> {
    let ids = state
        .pending_platform_operations
        .borrow()
        .iter()
        .filter_map(|(id, operation)| match operation {
            PendingPlatformOperation::CssImage(pending) if removed.contains(&pending.url) => {
                Some(*id)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut abort_error = None;
    for id in ids {
        let operation = state.pending_platform_operations.borrow_mut().remove(&id);
        state.cancelled_platform_operations.borrow_mut().insert(id);
        if let Some(host) = state.platform_host.borrow().as_ref() {
            host.cancel(id);
        }
        if let Some(PendingPlatformOperation::CssImage(Pending {
            stream: Some(stream_id),
            ..
        })) = operation
            && let Err(error) = submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )
        {
            abort_error = Some(error);
        }
    }
    for url in removed {
        let mut images = state.css_images.borrow_mut();
        images.entries.remove(url);
        images.dependencies.remove(url);
    }
    if !removed.is_empty() {
        publish(state)?;
    }
    abort_error.map_or(Ok(()), Err)
}
pub(super) fn schedule(
    state: &V8BindingState,
    urls: impl IntoIterator<Item = String>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    let mut urls = urls.into_iter().collect::<BTreeSet<_>>();
    urls.extend(inline_svg_dependencies(state)?);
    {
        let images = state.css_images.borrow();
        dependency_closure(
            urls.union(&images.capture_wanted).cloned().collect(),
            &images.dependencies,
        )?;
    }
    state.css_images.borrow_mut().layout_wanted = urls;
    reconcile(state)
}
pub(super) fn capture(
    state: &V8BindingState,
    owner: u64,
    urls: &BTreeSet<String>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    {
        let mut images = state.css_images.borrow_mut();
        if !images.captures.contains_key(&owner) && images.captures.len() >= 64 {
            return Err(V8BindingError::Allocation("CSS capture owner limit"));
        }
        let captured = images
            .capture_wanted
            .union(urls)
            .cloned()
            .collect::<BTreeSet<_>>();
        dependency_closure(
            captured.union(&images.layout_wanted).cloned().collect(),
            &images.dependencies,
        )?;
        images
            .captures
            .entry(owner)
            .or_default()
            .extend(urls.iter().cloned());
        images.capture_wanted = captured;
    }
    reconcile(state)
}
pub(super) fn release_capture(state: &V8BindingState, owner: u64) -> Result<(), V8BindingError> {
    {
        let mut images = state.css_images.borrow_mut();
        if images.captures.remove(&owner).is_none() {
            return Ok(());
        }
        images.capture_wanted = images
            .captures
            .values()
            .flat_map(|urls| urls.iter().cloned())
            .collect();
    }
    reconcile(state)
}
pub(super) fn capture_pending(state: &V8BindingState, owner: u64) -> bool {
    let images = state.css_images.borrow();
    let Some(urls) = images.captures.get(&owner) else {
        return false;
    };
    match dependency_closure(urls.clone(), &images.dependencies) {
        Ok(urls) => urls.iter().any(|url| {
            matches!(
                images.entries.get(url),
                Some(Entry::Queued | Entry::Pending(_))
            )
        }),
        Err(_) => false,
    }
}
fn validate_urls<'a>(urls: impl Iterator<Item = &'a String>) -> Result<(), V8BindingError> {
    for (index, url) in urls.enumerate() {
        if index >= MAX_IMAGES || url.len() > 64 * 1024 {
            return Err(V8BindingError::Message(
                "CSS image inventory exceeds native bounds".into(),
            ));
        }
    }
    Ok(())
}
fn reconcile(state: &V8BindingState) -> Result<(), V8BindingError> {
    let wanted = {
        let images = state.css_images.borrow();
        dependency_closure(
            images
                .layout_wanted
                .union(&images.capture_wanted)
                .cloned()
                .collect(),
            &images.dependencies,
        )?
    };
    if wanted.len() > MAX_IMAGES || wanted.iter().any(|url| url.len() > 64 * 1024) {
        return Err(V8BindingError::Message(
            "CSS image inventory exceeds native bounds".into(),
        ));
    }
    let removed = state
        .css_images
        .borrow()
        .wanted
        .difference(&wanted)
        .cloned()
        .collect();
    // Native cancellation removes the old entries even if stream-abort delivery
    // fails. Commit the desired inventory before propagating that failure so a
    // later capture cannot inherit orphaned demand from an already released ID.
    let cancelled = cancel(state, &removed);
    {
        let mut images = state.css_images.borrow_mut();
        images.wanted = wanted.clone();
        for url in wanted {
            images.entries.entry(url).or_insert(Entry::Queued);
        }
    }
    cancelled?;
    pump(state)
}
impl V8BindingState {
    /// Call after selecting a native document's resource backend. A missing
    /// backend is a terminal failure, not an outstanding network request.
    /// Bindings may still queue demand before native installation completes.
    pub fn finalize_css_image_backend(&self) -> Result<(), V8BindingError> {
        self.css_images.borrow_mut().backend_finalized = true;
        pump(self)
    }
}
fn pump(state: &V8BindingState) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Ok(());
    }
    if state
        .platform_host
        .borrow()
        .as_ref()
        .is_none_or(|host| !host.supports_image_requests())
    {
        let mut images = state.css_images.borrow_mut();
        if !images.backend_finalized {
            return Ok(());
        }
        let mut changed = false;
        for entry in images.entries.values_mut() {
            if matches!(entry, Entry::Queued) {
                *entry = Entry::Failed;
                changed = true;
            }
        }
        drop(images);
        if changed {
            publish(state)?;
        }
        return Ok(());
    }
    loop {
        if state.pending_platform_operations.borrow().len()
            >= crate::platform_admission::PENDING_LIMIT.saturating_sub(2)
        {
            break;
        }
        let next = state
            .css_images
            .borrow()
            .entries
            .iter()
            .find_map(|(url, entry)| matches!(entry, Entry::Queued).then(|| url.clone()));
        let Some(url) = next else { break };
        if Url::parse(&url).is_err() {
            state
                .css_images
                .borrow_mut()
                .entries
                .insert(url, Entry::Failed);
            continue;
        }
        let initiator_url = state.request_initiator_url()?;
        let token = {
            let mut images = state.css_images.borrow_mut();
            images.next = images
                .next
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("CSS image identity exhausted"))?;
            let token = images.next;
            images.entries.insert(url.clone(), Entry::Pending(token));
            token
        };
        let request = PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: None,
            use_cors_preflight: false,
            method: "GET".into(),
            url: url.clone(),
            headers: vec![("accept".into(), "image/*,*/*;q=0.5".into())],
            body: Vec::new(),
            credentials: "include".into(),
            initiator_url: Some(initiator_url),
            referrer_policy: Some(state.document_referrer_policy()),
        };
        let pending = Pending {
            url: url.clone(),
            token,
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            stream: None,
            final_url: None,
            content_type: None,
            bytes: Vec::new(),
        };
        if let Err(error) = submit_platform_operation(
            state,
            PlatformHostRequest::ImageStart {
                request,
                cors_enabled: false,
            },
            PendingPlatformOperation::CssImage(pending),
        ) {
            state
                .css_images
                .borrow_mut()
                .entries
                .insert(url, Entry::Failed);
            publish(state)?;
            return Err(error);
        }
    }
    Ok(())
}
pub(super) fn decode(bytes: &[u8]) -> Option<((u32, u32), String)> {
    decode_with_type(bytes, None)
}
fn decode_with_type(bytes: &[u8], content_type: Option<&str>) -> Option<((u32, u32), String)> {
    let xml = oryn_common::xml_encoding::decode(bytes, content_type).ok();
    if let Some(size) = xml
        .as_ref()
        .and_then(|text| image_dimensions::svg_size(text.as_bytes()))
    {
        if u64::from(size.0) * u64::from(size.1) <= 16_777_216 {
            return Some((size, "image/svg+xml".into()));
        }
        return None;
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let size = reader.into_dimensions().ok()?;
    if size.0 == 0 || size.1 == 0 || u64::from(size.0) * u64::from(size.1) > 16_777_216 {
        return None;
    }
    image::load_from_memory(bytes).ok()?;
    let media = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        image::ImageFormat::Bmp => "image/bmp",
        image::ImageFormat::Tiff => "image/tiff",
        image::ImageFormat::Avif => "image/avif",
        _ => "application/octet-stream",
    };
    Some((size, media.into()))
}
pub(super) fn complete(
    state: &V8BindingState,
    mut pending: Pending,
    result: Result<PlatformHostResult, V8BindingError>,
) -> Result<(), V8BindingError> {
    if !current(state, &pending) {
        if let Ok(PlatformHostResult::HttpStarted { stream_id, .. })
        | Ok(PlatformHostResult::HttpChunk {
            stream_id,
            done: false,
            ..
        }) = result
        {
            submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )?;
        }
        return Ok(());
    }
    let mut finished = false;
    match result {
        Ok(PlatformHostResult::HttpStarted {
            stream_id,
            url,
            status,
            headers,
            ..
        }) => {
            pending.stream = Some(stream_id);
            if !(200..300).contains(&status) || url.len() > 64 * 1024 || Url::parse(&url).is_err() {
                finished = true;
            } else {
                pending.final_url = Some(url);
                pending.content_type = oryn_common::xml_encoding::response_content_type(&headers);
            }
        }
        Ok(PlatformHostResult::HttpChunk {
            encoded_body_size: _,
            stream_id,
            bytes,
            done,
        }) => {
            if pending.stream != Some(stream_id) || pending.final_url.is_none() {
                // A completion cannot switch the native stream established by
                // ImageStart. Retire the known stream without accepting bytes.
                return fail(state, pending);
            }
            let cached = state
                .css_images
                .borrow()
                .entries
                .values()
                .filter_map(|entry| {
                    if let Entry::Ready { resource, .. } = entry {
                        Some(resource.bytes.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>();
            let other_pending = state
                .pending_platform_operations
                .borrow()
                .values()
                .filter_map(|operation| {
                    if let PendingPlatformOperation::CssImage(pending) = operation {
                        Some(pending.bytes.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>();
            if bytes.len() > MAX_BYTES.saturating_sub(pending.bytes.len())
                || cached
                    .saturating_add(other_pending)
                    .saturating_add(pending.bytes.len())
                    .saturating_add(bytes.len())
                    > MAX_TOTAL
            {
                finished = true;
            } else {
                pending.bytes.extend_from_slice(&bytes);
                if done {
                    pending.stream = None;
                    finished = true;
                    let total = state
                        .css_images
                        .borrow()
                        .entries
                        .values()
                        .filter_map(|entry| {
                            if let Entry::Ready { resource, .. } = entry {
                                Some(resource.bytes.len())
                            } else {
                                None
                            }
                        })
                        .sum::<usize>();
                    if total.saturating_add(pending.bytes.len()) <= MAX_TOTAL
                        && let (Some((size, media_type)), Some(final_url)) = (
                            decode_with_type(&pending.bytes, pending.content_type.as_deref()),
                            pending.final_url.clone(),
                        )
                    {
                        let aliases = if final_url != pending.url {
                            vec![pending.url.clone()]
                        } else {
                            Vec::new()
                        };
                        // SVG image/resource documents use secure processing: their
                        // nested network references are failed resources, not new page requests.
                        let dependencies: Result<BTreeSet<String>, V8BindingError> =
                            Ok(BTreeSet::new());
                        if let Ok(dependencies) = dependencies {
                            let admitted = {
                                let images = state.css_images.borrow();
                                let mut graph = images.dependencies.clone();
                                graph.insert(pending.url.clone(), dependencies.clone());
                                dependency_closure(
                                    images
                                        .layout_wanted
                                        .union(&images.capture_wanted)
                                        .cloned()
                                        .collect(),
                                    &graph,
                                )
                                .is_ok()
                            };
                            if admitted {
                                let mut images = state.css_images.borrow_mut();
                                images
                                    .dependencies
                                    .insert(pending.url.clone(), dependencies);
                                images.entries.insert(
                                    pending.url.clone(),
                                    Entry::Ready {
                                        resource: oryn_common::v4::PaintImageResource {
                                            svg_document_snapshot: false,
                                            svg_instances: Vec::new(),
                                            content_type: pending.content_type.clone(),
                                            url: final_url,
                                            url_aliases: aliases,
                                            media_type,
                                            bytes: std::mem::take(&mut pending.bytes),
                                        },
                                        size,
                                    },
                                );
                            }
                        }
                    }
                }
            }
        }
        // AdmissionHost owns cleanup for errors delivered after acceptance.
        // Only successful responses that fail image validation still need an abort.
        Err(_) => {
            pending.stream = None;
            finished = true;
        }
        _ => finished = true,
    }
    if finished {
        fail(state, pending)
    } else {
        let stream_id = pending
            .stream
            .ok_or(V8BindingError::Allocation("CSS image stream missing"))?;
        let url = pending.url.clone();
        if let Err(error) = submit_platform_operation(
            state,
            PlatformHostRequest::HttpRead {
                stream_id,
                max_bytes: 64 * 1024,
            },
            PendingPlatformOperation::CssImage(pending),
        ) {
            state
                .css_images
                .borrow_mut()
                .entries
                .insert(url, Entry::Failed);
            // submit_platform_operation transferred the rejected read's stream
            // to native cleanup. Do not re-admit an abort for that same custody.
            publish(state)?;
            return Err(error);
        }
        Ok(())
    }
}

fn fail(state: &V8BindingState, pending: Pending) -> Result<(), V8BindingError> {
    // Publish terminal readiness even if abort submission itself is rejected.
    // Otherwise a failed transport can leave image readiness pending forever.
    if matches!(
        state.css_images.borrow().entries.get(&pending.url),
        Some(Entry::Pending(_))
    ) {
        state
            .css_images
            .borrow_mut()
            .entries
            .insert(pending.url, Entry::Failed);
    }
    let abort = if let Some(stream_id) = pending.stream {
        submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        )
        .map(|_| ())
    } else {
        Ok(())
    };
    publish(state)?;
    reconcile(state)?;
    abort
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<PlatformHostRequest>>,
        reject_images: Cell<bool>,
        reject_reads: Cell<bool>,
        reject_aborts: Cell<bool>,
        abandoned: RefCell<Vec<u64>>,
    }
    impl PlatformHost for Host {
        fn supports_image_requests(&self) -> bool {
            !self.reject_images.get()
        }
        fn submit(&self, _: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            if self.reject_reads.get() && matches!(request, PlatformHostRequest::HttpRead { .. }) {
                return Err("closed native transport".into());
            }
            if self.reject_aborts.get() && matches!(request, PlatformHostRequest::HttpAbort { .. })
            {
                return Err("closed native abort transport".into());
            }
            self.requests.borrow_mut().push(request);
            Ok(())
        }
        fn abandon_http_stream(&self, stream: u64) {
            self.abandoned.borrow_mut().push(stream);
        }
        fn cancel(&self, _: HostOperationId) {}
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<crate::platform::PlatformHostCompletion> {
            Vec::new()
        }
    }
    fn take(state: &V8BindingState) -> Pending {
        let id = state
            .pending_platform_operations
            .borrow()
            .iter()
            .find_map(|(id, operation)| {
                matches!(operation, PendingPlatformOperation::CssImage(_)).then_some(*id)
            })
            .unwrap();
        let Some(PendingPlatformOperation::CssImage(pending)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            unreachable!()
        };
        pending
    }
    fn started(stream_id: u64, url: &str) -> Result<PlatformHostResult, V8BindingError> {
        Ok(PlatformHostResult::HttpStarted {
            timing: None,
            internal_response: None,
            response_type: None,
            upload: None,
            cors_filtered: false,
            redirected: true,
            stream_id,
            status: 200,
            url: url.into(),
            headers: Vec::new(),
        })
    }
    #[test]
    fn completed_backend_selection_fails_unhosted_demand_without_reviving_it_on_later_install() {
        check_unhosted_demand(false);
    }
    #[test]
    fn transform_only_backend_does_not_receive_css_image_requests() {
        check_unhosted_demand(true);
    }
    fn check_unhosted_demand(transform_only: bool) {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let unavailable = Rc::new(Host::default());
        unavailable.reject_images.set(true);
        if transform_only {
            state.set_platform_host(unavailable.clone());
        }
        let before = "https://example.test/before.svg".to_owned();
        let absent = "https://example.test/absent.svg".to_owned();
        let later = "https://example.test/later.svg".to_owned();
        schedule(&state, [before.clone()]).unwrap();
        assert!(pending(&state));
        state.finalize_css_image_backend().unwrap();
        assert!(!pending(&state));
        assert!(resources(&state).is_empty());
        assert!(matches!(
            state.css_images.borrow().entries.get(&before),
            Some(Entry::Failed)
        ));
        schedule(&state, [before.clone(), absent.clone()]).unwrap();
        assert!(!pending(&state));
        assert!(state.pending_platform_operations.borrow().is_empty());
        assert!(unavailable.requests.borrow().is_empty());
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        state.finalize_css_image_backend().unwrap();
        assert!(host.requests.borrow().is_empty());
        // A real later backend may serve new demand. Previously failed demand
        // stays terminal until its owner explicitly removes and requests it again.
        schedule(&state, [before.clone(), absent.clone(), later.clone()]).unwrap();
        assert!(pending(&state));
        assert_eq!(host.requests.borrow().len(), 1);
        let operation = take(&state);
        assert_eq!(operation.url, later);
        complete(
            &state,
            operation,
            Err(V8BindingError::Message("fixture denial".into())),
        )
        .unwrap();
        assert!(!pending(&state));
        schedule(&state, Vec::<String>::new()).unwrap();
        schedule(&state, [before.clone()]).unwrap();
        assert_eq!(host.requests.borrow().len(), 2);
        let operation = take(&state);
        assert_eq!(operation.url, before);
    }
    #[test]
    fn css_svg_retains_original_bytes_and_http_charset_through_completion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host);
        schedule(&state, ["https://example.test/clip.svg".into()]).unwrap();
        let operation = take(&state);
        let mut response = started(77, "https://example.test/clip.svg").unwrap();
        if let PlatformHostResult::HttpStarted { headers, .. } = &mut response {
            headers.push((
                "Content-Type".into(),
                "image/svg+xml; charset=windows-1252".into(),
            ));
        }
        complete(&state, operation, Ok(response)).unwrap();
        let original=b"<svg xmlns='http://www.w3.org/2000/svg' width='37' height='19'><clipPath id='\xe9'/></svg>".to_vec();
        let operation = take(&state);
        complete(
            &state,
            operation,
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 77,
                bytes: original.clone(),
                done: true,
            }),
        )
        .unwrap();
        let ready = resources(&state);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].bytes, original);
        assert_eq!(ready[0].media_type, "image/svg+xml");
        assert_eq!(
            ready[0].content_type.as_deref(),
            Some("image/svg+xml; charset=windows-1252")
        );
        let decoded =
            oryn_common::xml_encoding::decode(&ready[0].bytes, ready[0].content_type.as_deref())
                .unwrap();
        assert!(decoded.contains("id='é'"));
    }
    #[test]
    fn native_css_image_lifecycle_publishes_only_complete_current_resources() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><p>content</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let url = "https://example.test/generated.svg".to_string();
        // A layout flush queues work when the native host is not installed. It
        // neither fetches synchronously nor fabricates intrinsic dimensions.
        schedule(&state, [url.clone()]).unwrap();
        assert!(pending(&state));
        assert!(resources(&state).is_empty());
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        pump(&state).unwrap();
        let operation = take(&state);
        complete(
            &state,
            operation,
            started(10, "https://cdn.test/assets/generated.svg"),
        )
        .unwrap();
        assert!(resources(&state).is_empty());
        let operation = take(&state);
        complete(
            &state,
            operation,
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 10,
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="37" height="19"/>"#
                    .to_vec(),
                done: true,
            }),
        )
        .unwrap();
        assert!(!pending(&state));
        let ready = resources(&state);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].url, "https://cdn.test/assets/generated.svg");
        assert_eq!(ready[0].url_aliases, vec![url.clone()]);
        let sizes = dom
            .borrow()
            .arena()
            .get(state.document_root())
            .unwrap()
            .document_metadata
            .as_ref()
            .unwrap()
            .css_image_intrinsics
            .clone();
        assert_eq!(sizes[&url], (37, 19));
        assert_eq!(sizes[&ready[0].url], (37, 19));
        assert_eq!(
            dom.borrow()
                .arena()
                .get(state.document_root())
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .css_svg_resources,
            ready
        );
        let requests = host.requests.borrow().len();
        schedule(&state, [url.clone()]).unwrap();
        assert_eq!(host.requests.borrow().len(), requests);
        schedule(&state, Vec::<String>::new()).unwrap();
        assert!(resources(&state).is_empty());
        assert!(
            dom.borrow()
                .arena()
                .get(state.document_root())
                .unwrap()
                .document_metadata
                .as_ref()
                .unwrap()
                .css_svg_resources
                .is_empty()
        );
        schedule(&state, [url.clone()]).unwrap();
        let obsolete = take(&state);
        schedule(&state, Vec::<String>::new()).unwrap();
        complete(&state, obsolete, started(11, &url)).unwrap();
        assert!(resources(&state).is_empty());
        assert!(!pending(&state));
        assert!(
            host.requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::HttpAbort { stream_id: 11 }))
        );
    }
    #[test]
    fn exhausted_binding_admission_retains_native_stream_abandonment() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>image</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        state.next_platform_operation_id.set(u64::MAX);
        assert!(
            submit_platform_operation(
                &state,
                PlatformHostRequest::HttpAbort { stream_id: 81 },
                PendingPlatformOperation::WebSocketCommand
            )
            .is_err()
        );
        assert_eq!(*host.abandoned.borrow(), vec![81]);
        assert!(host.requests.borrow().is_empty());
        state.next_platform_operation_id.set(1);
        for id in 0..crate::platform_admission::PENDING_LIMIT {
            state.pending_platform_operations.borrow_mut().insert(
                HostOperationId(id as u64),
                PendingPlatformOperation::WebSocketCommand,
            );
        }
        assert!(
            submit_platform_operation(
                &state,
                PlatformHostRequest::HttpRead {
                    stream_id: 82,
                    max_bytes: 1
                },
                PendingPlatformOperation::WebSocketCommand
            )
            .is_err()
        );
        assert_eq!(*host.abandoned.borrow(), vec![81, 82]);
        assert!(host.requests.borrow().is_empty());
    }
    #[test]
    fn failed_css_image_read_submission_releases_readiness_and_stream() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><p/>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        host.reject_reads.set(true);
        state.set_platform_host(host.clone());
        schedule(&state, ["https://example.test/fail.svg".into()]).unwrap();
        let operation = take(&state);
        assert!(
            complete(
                &state,
                operation,
                started(12, "https://example.test/fail.svg")
            )
            .is_err()
        );
        assert!(!pending(&state));
        assert!(resources(&state).is_empty());
        assert_eq!(
            *host.abandoned.borrow(),
            vec![12],
            "rejected read transfers exact stream cleanup once"
        );
        assert!(
            !host
                .requests
                .borrow()
                .iter()
                .any(|request| matches!(request, PlatformHostRequest::HttpAbort { stream_id: 12 })),
            "transferred cleanup is not re-admitted"
        );
    }
    #[test]
    fn fetched_filter_svg_does_not_admit_external_images_stylesheets_or_fonts() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>filter</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        schedule(&state, ["https://origin.test/filter.svg".into()]).unwrap();
        let operation = take(&state);
        complete(
            &state,
            operation,
            started(30, "https://cdn.test/assets/filter.svg"),
        )
        .unwrap();
        let operation = take(&state);
        complete(&state,operation,Ok(PlatformHostResult::HttpChunk{encoded_body_size:None,stream_id:30,bytes:br#"<svg xmlns="http://www.w3.org/2000/svg"><style>@import "external.css"; @font-face { font-family: external; src: url(external.woff2) }</style><filter id="f"><feImage href="tile.png"/></filter></svg>"#.to_vec(),done:true})).unwrap();
        assert!(!pending(&state));
        assert_eq!(resources(&state).len(), 1);
        assert!(!resources(&state)[0].svg_document_snapshot);
        assert_eq!(
            host.requests
                .borrow()
                .iter()
                .filter(|request| matches!(request, PlatformHostRequest::ImageStart { .. }))
                .count(),
            1,
            "secure resource documents cannot admit nested image, CSS, or font requests"
        );
        schedule(&state, Vec::<String>::new()).unwrap();
        assert!(!pending(&state));
        assert!(resources(&state).is_empty());
        assert!(state.css_images.borrow().dependencies.is_empty());
    }
    #[test]
    fn capture_owners_release_only_their_inventory_and_wait_only_for_their_dependencies() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>capture</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        // No platform host: queued resources remain inspectable without I/O.
        let shared = "https://origin.test/shared.svg".to_string();
        let first = "https://origin.test/first.png".to_string();
        let second = "https://origin.test/second.png".to_string();
        let layout = "https://origin.test/layout.png".to_string();
        schedule(&state, [layout.clone()]).unwrap();
        capture(&state, 1, &BTreeSet::from([shared.clone(), first.clone()])).unwrap();
        capture(&state, 2, &BTreeSet::from([shared.clone(), second.clone()])).unwrap();
        assert!(capture_pending(&state, 1));
        assert!(capture_pending(&state, 2));
        release_capture(&state, 1).unwrap();
        assert!(!capture_pending(&state, 1));
        assert!(capture_pending(&state, 2));
        assert!(!state.css_images.borrow().entries.contains_key(&first));
        assert!(state.css_images.borrow().entries.contains_key(&shared));
        state
            .css_images
            .borrow_mut()
            .entries
            .insert(shared.clone(), Entry::Failed);
        state
            .css_images
            .borrow_mut()
            .entries
            .insert(second.clone(), Entry::Failed);
        assert!(!capture_pending(&state, 2));
        assert!(pending(&state));
        release_capture(&state, 2).unwrap();
        assert_eq!(state.css_images.borrow().wanted, BTreeSet::from([layout]));
        release_capture(&state, 1).unwrap();
        assert!(state.css_images.borrow().captures.is_empty());
    }
    #[test]
    fn capture_release_commits_inventory_even_when_abort_delivery_fails() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p/>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let url = "https://origin.test/captured.svg".to_string();
        capture(&state, 1, &BTreeSet::from([url.clone()])).unwrap();
        let operation = take(&state);
        complete(&state, operation, started(71, &url)).unwrap();
        host.reject_aborts.set(true);
        assert!(release_capture(&state, 1).is_err());
        assert!(!capture_pending(&state, 1));
        assert!(state.css_images.borrow().captures.is_empty());
        assert!(state.css_images.borrow().wanted.is_empty());
        assert!(state.css_images.borrow().entries.is_empty());
        release_capture(&state, 1).unwrap();
        assert_eq!(*host.abandoned.borrow(), vec![71]);
        host.reject_aborts.set(false);
        capture(&state, 2, &BTreeSet::from([url])).unwrap();
        assert!(capture_pending(&state, 2));
    }
    #[test]
    fn capture_dependency_limit_rejection_does_not_publish_partial_owner() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p/>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let existing = "https://origin.test/existing.png".to_string();
        capture(&state, 1, &BTreeSet::from([existing.clone()])).unwrap();
        let root = "https://origin.test/filter.svg".to_string();
        state.css_images.borrow_mut().dependencies.insert(
            root.clone(),
            (0..MAX_IMAGES)
                .map(|index| format!("https://origin.test/{index}.png"))
                .collect(),
        );
        assert!(capture(&state, 2, &BTreeSet::from([root])).is_err());
        assert_eq!(state.css_images.borrow().captures.len(), 1);
        assert_eq!(state.css_images.borrow().wanted, BTreeSet::from([existing]));
        assert!(capture_pending(&state, 1));
    }
    #[test]
    fn capture_owner_admission_is_bounded_without_disturbing_existing_owners() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p/>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        for owner in 1..=64 {
            capture(&state, owner, &BTreeSet::new()).unwrap();
        }
        assert!(
            capture(
                &state,
                65,
                &BTreeSet::from(["https://origin.test/rejected.png".into()])
            )
            .is_err()
        );
        assert!(state.css_images.borrow().wanted.is_empty());
        release_capture(&state, 1).unwrap();
        capture(&state, 65, &BTreeSet::new()).unwrap();
        assert_eq!(state.css_images.borrow().captures.len(), 64);
        clear(&state).unwrap();
        assert!(state.css_images.borrow().captures.is_empty());
    }
    #[test]
    fn generated_image_decoder_rejects_invalid_and_oversized_payloads() {
        assert!(decode(b"not an image").is_none());
        assert!(
            decode(br#"<svg xmlns="http://www.w3.org/2000/svg" width="999999" height="999999"/>"#)
                .is_none()
        );
    }
}

fn dependency_closure(
    mut wanted: BTreeSet<String>,
    graph: &BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeSet<String>, V8BindingError> {
    let mut pending = wanted.iter().cloned().collect::<Vec<_>>();
    while let Some(url) = pending.pop() {
        if let Some(dependencies) = graph.get(&url) {
            for dependency in dependencies {
                if wanted.insert(dependency.clone()) {
                    if wanted.len() > MAX_IMAGES {
                        return Err(V8BindingError::Message(
                            "CSS SVG dependency inventory bound".into(),
                        ));
                    }
                    pending.push(dependency.clone());
                }
            }
        }
    }
    validate_urls(wanted.iter())?;
    Ok(wanted)
}
fn svg_dependencies(bytes: &[u8], source_url: &str) -> Result<BTreeSet<String>, V8BindingError> {
    svg_dependencies_with_type(bytes, source_url, None)
}
fn svg_dependencies_with_type(
    bytes: &[u8],
    source_url: &str,
    content_type: Option<&str>,
) -> Result<BTreeSet<String>, V8BindingError> {
    let failure = || V8BindingError::Message("invalid SVG dependency source".into());
    let source = oryn_common::xml_encoding::decode(bytes, content_type).map_err(|_| failure())?;
    let base = url::Url::parse(source_url).map_err(|_| failure())?;
    let parsed = oryn_dom::xml::parse(&source, "image/svg+xml");
    let mut pending = vec![(parsed.document, base.clone())];
    let mut seen = BTreeSet::new();
    let mut dependencies = BTreeSet::new();
    while let Some((id, mut current)) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        if seen.len() > 65_536 {
            return Err(failure());
        }
        let node = parsed.dom.get(id).map_err(|_| failure())?;
        if let Some(value) = node
            .attributes
            .namespaced(Some("http://www.w3.org/XML/1998/namespace"), "base")
        {
            current = current.join(&value.value).map_err(|_| failure())?;
        }
        if matches!(&node.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&matches!(name.local.as_ref(),"image"|"feImage"))
        {
            let href = node
                .attributes
                .get_null("href")
                .map(String::as_str)
                .or_else(|| {
                    node.attributes
                        .namespaced(Some("http://www.w3.org/1999/xlink"), "href")
                        .map(|value| value.value.as_str())
                });
            if let Some(href) = href
                && !href.trim().is_empty()
                && !href.starts_with('#')
                && !href.starts_with("data:")
            {
                let mut target = current.join(href).map_err(|_| failure())?;
                target.set_fragment(None);
                let mut own = base.clone();
                own.set_fragment(None);
                if target != own {
                    dependencies.insert(target.to_string());
                    validate_urls(dependencies.iter())?;
                }
            }
        }
        pending.extend(
            node.children
                .iter()
                .rev()
                .map(|child| (*child, current.clone())),
        );
    }
    Ok(dependencies)
}

#[cfg(test)]
mod dependency_tests {
    use super::*;
    #[test]
    fn svg_dependencies_follow_redirect_and_xml_base_but_not_script_or_local_ids() {
        let urls=svg_dependencies(br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xml:base="../images/"><filter><feImage xlink:href="a.svg#part"/></filter><g xml:base="nested/"><image href="b.png"/></g><image href="#local"/><image href="data:image/png;base64,AA=="/><script href="must-not-fetch.js"/></svg>"##,"https://cdn.test/assets/filter.svg").unwrap();
        assert_eq!(
            urls,
            BTreeSet::from([
                "https://cdn.test/images/a.svg".into(),
                "https://cdn.test/images/nested/b.png".into()
            ])
        );
    }
    #[test]
    fn dependency_cycles_terminate_and_orphans_are_not_retained() {
        let graph = BTreeMap::from([
            ("a".into(), BTreeSet::from(["b".into()])),
            ("b".into(), BTreeSet::from(["a".into()])),
            ("orphan".into(), BTreeSet::from(["hidden".into()])),
        ]);
        assert_eq!(
            dependency_closure(BTreeSet::from(["a".into()]), &graph).unwrap(),
            BTreeSet::from(["a".into(), "b".into()])
        );
        assert!(
            dependency_closure(BTreeSet::new(), &graph)
                .unwrap()
                .is_empty()
        );
    }
}

fn inline_svg_dependencies(state: &V8BindingState) -> Result<BTreeSet<String>, V8BindingError> {
    let base = state.native_document_base_url(true)?;
    let dom = state.dom.borrow();
    let mut pending = vec![state.document_root()];
    let mut seen = BTreeSet::new();
    let mut result = BTreeSet::new();
    let mut bytes = 0usize;
    while let Some(id) = pending.pop() {
        if !seen.insert(id) || seen.len() > 100_000 {
            return Err(V8BindingError::Message(
                "inline SVG dependency tree bound".into(),
            ));
        }
        let node = dom.arena().get(id)?;
        if matches!(&node.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()=="svg")
        {
            let source = dom.serialize_xml(id)?;
            bytes = bytes
                .checked_add(source.len())
                .filter(|bytes| *bytes <= MAX_BYTES)
                .ok_or_else(|| {
                    V8BindingError::Message("inline SVG dependency source bound".into())
                })?;
            result.extend(svg_dependencies(source.as_bytes(), &base)?);
            validate_urls(result.iter())?;
        } else {
            pending.extend(node.children.iter().rev().copied());
        }
    }
    Ok(result)
}

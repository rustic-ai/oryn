//! Native image preloads and link prefetches use the ordinary resource Fetch pipeline.
use super::*;
#[derive(Default)]
pub(super) struct State {
    cursor: Option<u64>,
    environment: Option<(u64, u64, u64)>,
    next: u64,
    entries: BTreeMap<NodeId, Entry>,
}
struct Entry {
    key: Key,
    token: u64,
    event: Option<bool>,
    publication: Option<u64>,
    target: oryn_dom::TrackedNodeLease,
}
#[derive(Clone, PartialEq, Eq)]
struct Key {
    prefetch: bool,
    url: String,
    cors: Option<bool>,
    referrer_policy: String,
}
pub(super) struct Pending {
    node: NodeId,
    token: u64,
    document: NodeId,
    generation: u64,
    epoch: u64,
    stream: Option<u64>,
    bytes: Vec<u8>,
}
fn environment(state: &V8BindingState) -> (u64, u64, u64) {
    let viewport = state.layout.borrow().viewport.unwrap_or((1280., 720.));
    let dpr = state.layout_provider.borrow().as_ref().map_or(1., |p| p.native_device_pixel_ratio());
    (viewport.0.to_bits(), viewport.1.to_bits(), dpr.to_bits())
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    !state.document_load_stopped()
        && (state.preload_resources.borrow().cursor != Some(state.dom.borrow().mutation_cursor().0)
            || state.preload_resources.borrow().environment != Some(environment(state))
            || state
                .preload_resources
                .borrow()
                .entries
                .values()
                .any(|e| e.event.is_some() || e.publication.is_some()))
}
fn current(state: &V8BindingState, p: &Pending) -> bool {
    !state.execution_retired()
        && p.document == state.document_root()
        && p.generation == state.document_generation()
        && p.epoch == state.document_stream_epoch()
        && state
            .preload_resources
            .borrow()
            .entries
            .get(&p.node)
            .is_some_and(|e| e.token == p.token)
}
fn selected(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<BTreeMap<NodeId, Key>, V8BindingError> {
    let dom = state.dom.borrow();
    let mut wanted = BTreeMap::new();
    for node in dom.arena().descendants_inclusive(state.document_root())? {
        if !dom.is_element(node, "http://www.w3.org/1999/xhtml", "link") {
            continue;
        }
        let rel = dom.attribute_in_null_namespace(node, "rel")?.unwrap_or("");
        let prefetch = rel
            .split_ascii_whitespace()
            .any(|v| v.eq_ignore_ascii_case("prefetch"));
        if !prefetch
            && (!rel
                .split_ascii_whitespace()
                .any(|v| v.eq_ignore_ascii_case("preload"))
                || !dom
                    .attribute_in_null_namespace(node, "as")?
                    .is_some_and(|v| v.eq_ignore_ascii_case("image")))
        {
            continue;
        }
        let media = dom
            .attribute_in_null_namespace(node, "media")?
            .unwrap_or("");
        if !media.trim().is_empty()
            && !state
                .native_media_matches(media)
                .map_err(V8BindingError::EventCallback)?
        {
            continue;
        }
        let href = dom
            .attribute_in_null_namespace(node, "href")?
            .filter(|s| !s.is_empty());
        let source = if prefetch {
            href.map(str::to_owned)
        } else {
            let candidates = responsive_images::parse(dom.attribute_in_null_namespace(node, "imagesrcset")?.unwrap_or(""));
            let sizes = dom.attribute_in_null_namespace(node, "imagesizes")?.unwrap_or("");
            let provider = state.layout_provider.borrow().clone();
            let dpr = provider.as_ref().map_or(1., |p| p.native_device_pixel_ratio());
            let base = state.native_document_base_url(false)?;
            let size = {
                let _retained = state.enter_retained_allocation(scope)?;
                provider.as_ref().and_then(|p| p.native_image_source_size(sizes, &base, None).ok())
                    .unwrap_or_else(|| state.layout.borrow().viewport.unwrap_or((1280.,720.)).0)
            };
            responsive_images::select(candidates, href, size, dpr).map(|(url,_)| url)
        };
        let Some(source) = source else { continue; };
        let Ok(url) = state.native_document_resource_url(&source) else {
            continue;
        };
        let url = url.to_string();
        let cors = dom
            .attribute_in_null_namespace(node, "crossorigin")?
            .map(|v| v.eq_ignore_ascii_case("use-credentials"));
        if wanted.len() >= 256 {
            return Err(V8BindingError::Allocation("preload inventory exceeded"));
        }
        let referrer_policy = dom
            .attribute_in_null_namespace(node, "referrerpolicy")?
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| state.document_referrer_policy());
        wanted.insert(
            node,
            Key {
                prefetch,
                url,
                cors,
                referrer_policy,
            },
        );
    }
    Ok(wanted)
}
fn cancel(state: &V8BindingState, node: NodeId) -> Result<(), V8BindingError> {
    let removed = state.preload_resources.borrow_mut().entries.remove(&node);
    if let Some(receipt) = removed.and_then(|entry| entry.publication) {
        navigation_api::retire_native_event_receipt(state, receipt);
    }
    let ids = state
        .pending_platform_operations
        .borrow()
        .iter()
        .filter_map(|(id, p)| {
            matches!(p,PendingPlatformOperation::Preload(p)if p.node==node).then_some(*id)
        })
        .collect::<Vec<_>>();
    let mut failure = None;
    for id in ids {
        let p = state.pending_platform_operations.borrow_mut().remove(&id);
        state.cancelled_platform_operations.borrow_mut().insert(id);
        if let Some(host) = state.platform_host.borrow().clone() {
            host.cancel(id)
        }
        if let Some(PendingPlatformOperation::Preload(p)) = p
            && let Some(stream_id) = p.stream
            && let Err(e) = submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )
        {
            failure = Some(e)
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(super) fn clear(state: &V8BindingState) -> Result<(), V8BindingError> {
    let nodes = state
        .preload_resources
        .borrow()
        .entries
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let mut failure = None;
    for node in nodes {
        if let Err(e) = cancel(state, node) {
            failure = Some(e)
        }
    }
    failure.map_or(Ok(()), Err)
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_retired() || state.execution_suspended() || state.document_load_stopped() {
        return Ok(0);
    }
    let cursor = state.dom.borrow().mutation_cursor().0;
    let wanted = selected(scope, state)?;
    let removed = state
        .preload_resources
        .borrow()
        .entries
        .iter()
        .filter_map(|(node, e)| (wanted.get(node) != Some(&e.key)).then_some(*node))
        .collect::<Vec<_>>();
    for node in removed {
        cancel(state, node)?;
    }
    for (node, key) in wanted {
        if state.preload_resources.borrow().entries.contains_key(&node) {
            continue;
        }
        let target = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
        let token = {
            let mut preloads = state.preload_resources.borrow_mut();
            preloads.next = preloads
                .next
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("preload identity exhausted"))?;
            let token = preloads.next;
            preloads.entries.insert(
                node,
                Entry {
                    key: key.clone(),
                    token,
                    event: None,
                    publication: None,
                    target,
                },
            );
            token
        };
        let pending = Pending {
            node,
            token,
            document: state.document_root(),
            generation: state.document_generation(),
            epoch: state.document_stream_epoch(),
            stream: None,
            bytes: Vec::new(),
        };
        let request = PlatformHttpRequest {
            upgrade_mixed_image: false,
            fetch: None,
            use_cors_preflight: false,
            method: "GET".into(),
            url: key.url,
            headers: vec![(
                "accept".into(),
                if key.prefetch {
                    "*/*"
                } else {
                    "image/avif,image/webp,image/png,image/jpeg,image/*;q=0.8,*/*;q=0.5"
                }
                .into(),
            )],
            body: Vec::new(),
            credentials: if !key.prefetch && key.cors == Some(false) {
                "same-origin"
            } else {
                "include"
            }
            .into(),
            initiator_url: Some(state.request_initiator_url()?),
            referrer_policy: Some(key.referrer_policy),
        };
        if let Err(error) = submit_platform_operation(
            state,
            if key.prefetch {
                PlatformHostRequest::PrefetchStart { request }
            } else {
                PlatformHostRequest::ImagePreloadStart {
                    request,
                    cors_enabled: key.cors.is_some(),
                }
            },
            PendingPlatformOperation::Preload(pending),
        ) {
            if let Some(e) = state.preload_resources.borrow_mut().entries.get_mut(&node) {
                e.event = Some(false)
            }
            return Err(error);
        }
    }
    state.preload_resources.borrow_mut().cursor = Some(cursor);
    state.preload_resources.borrow_mut().environment = Some(environment(state));
    let ready = state
        .preload_resources
        .borrow()
        .entries
        .iter()
        .filter_map(|(node, e)| e.event.map(|success| (*node, e.token, success)))
        .collect::<Vec<_>>();
    let mut count = 0;
    for (node, token, success) in ready {
        if state.execution_retired() || state.execution_suspended() {
            break;
        }
        let selected = selected(scope, state)?;
        let valid = state
            .preload_resources
            .borrow()
            .entries
            .get(&node)
            .is_some_and(|e| {
                e.token == token
                    && {
                        let (owner, current) = e.target.current();
                        Rc::ptr_eq(&owner, &state.dom) && current == node
                    }
                    && selected.get(&node) == Some(&e.key)
            });
        if !valid {
            continue;
        }
        let existing = state
            .preload_resources
            .borrow()
            .entries
            .get(&node)
            .and_then(|entry| entry.publication);
        let receipt = match existing {
            Some(receipt) => receipt,
            None => {
                let receipt = navigation_api::start_native_script_completion_publication(
                    scope, node, success,
                )?;
                let retained = {
                    let mut preloads = state.preload_resources.borrow_mut();
                    if let Some(entry) = preloads
                        .entries
                        .get_mut(&node)
                        .filter(|entry| entry.token == token)
                    {
                        entry.publication = Some(receipt);
                        true
                    } else {
                        false
                    }
                };
                if !retained {
                    navigation_api::retire_native_event_receipt(state, receipt);
                    continue;
                }
                receipt
            }
        };
        if navigation_api::resume_native_history_publication(scope, receipt)? {
            if let Some(entry) = state
                .preload_resources
                .borrow_mut()
                .entries
                .get_mut(&node)
                .filter(|entry| entry.token == token)
            {
                entry.publication = None;
                entry.event = None;
            }
            count += 1;
        }
    }
    Ok(count)
}
fn finish(state: &V8BindingState, p: &mut Pending, success: bool) -> Result<(), V8BindingError> {
    if let Some(e) = state
        .preload_resources
        .borrow_mut()
        .entries
        .get_mut(&p.node)
        .filter(|e| e.token == p.token)
    {
        e.event = Some(success)
    }
    if let Some(stream_id) = p.stream.take() {
        submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        )?;
    }
    Ok(())
}
pub(super) fn complete(
    state: &V8BindingState,
    mut p: Pending,
    result: Result<PlatformHostResult, V8BindingError>,
) -> Result<(), V8BindingError> {
    if !current(state, &p) {
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
    let prefetch = state
        .preload_resources
        .borrow()
        .entries
        .get(&p.node)
        .is_some_and(|entry| entry.key.prefetch);
    match result {
        Ok(PlatformHostResult::HttpStarted {
            stream_id,
            status,
            internal_response,
            ..
        }) => {
            p.stream = Some(stream_id);
            let status = internal_response.map_or(status, |r| r.status);
            if !(200..300).contains(&status) {
                return finish(state, &mut p, false);
            }
        }
        Ok(PlatformHostResult::HttpChunk {
            encoded_body_size: _,
            stream_id,
            bytes,
            done,
        }) => {
            let retained = state
                .pending_platform_operations
                .borrow()
                .values()
                .filter_map(|pending| {
                    if let PendingPlatformOperation::Preload(p) = pending {
                        Some(p.bytes.len())
                    } else {
                        None
                    }
                })
                .sum::<usize>();
            if p.stream != Some(stream_id)
                || bytes.len() > (16 * 1024 * 1024usize).saturating_sub(p.bytes.len())
                || retained
                    .saturating_add(p.bytes.len())
                    .saturating_add(bytes.len())
                    > 32 * 1024 * 1024
            {
                return finish(state, &mut p, false);
            }
            if !prefetch {
                p.bytes.extend_from_slice(&bytes);
            }
            if done {
                p.stream = None;
                let success = prefetch || css_images::decode(&p.bytes).is_some();
                return finish(state, &mut p, success);
            }
        }
        _ => return finish(state, &mut p, false),
    }
    let stream_id = p
        .stream
        .ok_or(V8BindingError::Allocation("preload stream missing"))?;
    let (node, token) = (p.node, p.token);
    if let Err(error) = submit_platform_operation(
        state,
        PlatformHostRequest::HttpRead {
            stream_id,
            max_bytes: 64 * 1024,
        },
        PendingPlatformOperation::Preload(p),
    ) {
        if let Some(e) = state
            .preload_resources
            .borrow_mut()
            .entries
            .get_mut(&node)
            .filter(|e| e.token == token)
        {
            e.event = Some(false)
        }
        let _ = submit_platform_operation(
            state,
            PlatformHostRequest::HttpAbort { stream_id },
            PendingPlatformOperation::WebSocketCommand,
        );
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Host {
        requests: RefCell<Vec<(HostOperationId, PlatformHostRequest)>>,
        cancelled: RefCell<Vec<HostOperationId>>,
    }
    impl PlatformHost for Host {
        fn submit(&self, id: HostOperationId, r: PlatformHostRequest) -> Result<(), String> {
            self.requests.borrow_mut().push((id, r));
            Ok(())
        }
        fn cancel(&self, id: HostOperationId) {
            self.cancelled.borrow_mut().push(id)
        }
        fn retire(&self) {}
        fn drain_completions(&self, _: usize) -> Vec<crate::platform::PlatformHostCompletion> {
            Vec::new()
        }
    }
    #[test]
    fn prefetch_consumes_arbitrary_response_without_script_or_image_decode_and_publishes_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><link rel=prefetch href='/future'>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/").unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let source=v8::String::new(scope,"globalThis.loads=0;document.querySelector('link').onload=e=>{if(!e.isTrusted)throw Error('untrusted');loads++}").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        poll(scope, &state).unwrap();
        let (id, request) = host.requests.borrow()[0].clone();
        let PlatformHostRequest::PrefetchStart { request } = request else {
            panic!("dedicated prefetch")
        };
        assert_eq!(request.method, "GET");
        assert_eq!(request.credentials, "include");
        assert_eq!(request.url, "https://example.test/future");
        assert!(request.fetch.is_none());
        let Some(PendingPlatformOperation::Preload(mut pending)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            panic!("prefetch")
        };
        pending.stream = Some(90);
        complete(
            &state,
            pending,
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 90,
                bytes: b"globalThis.executed=true;not-an-image".to_vec(),
                done: false,
            }),
        )
        .unwrap();
        assert!(
            state
                .preload_resources
                .borrow()
                .entries
                .values()
                .all(|entry| entry.event.is_none())
        );
        let read = host.requests.borrow().last().unwrap().0;
        let Some(PendingPlatformOperation::Preload(pending)) =
            state.pending_platform_operations.borrow_mut().remove(&read)
        else {
            panic!("body")
        };
        assert!(
            pending.bytes.is_empty(),
            "prefetch does not duplicate cache body storage"
        );
        complete(
            &state,
            pending,
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 90,
                bytes: Vec::new(),
                done: true,
            }),
        )
        .unwrap();
        poll(scope, &state).unwrap();
        poll(scope, &state).unwrap();
        let source = v8::String::new(scope, "loads===1&&typeof executed==='undefined'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    #[test]
    fn prefetch_removal_retires_request_and_late_stream_without_publication() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<!doctype html><link id=p rel=prefetch href='/future'>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/").unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        let id = host.requests.borrow()[0].0;
        let Some(PendingPlatformOperation::Preload(mut pending)) =
            state.pending_platform_operations.borrow_mut().remove(&id)
        else {
            panic!("prefetch")
        };
        let node = dom.borrow().find_by_id("p").unwrap();
        dom.borrow_mut().remove(node).unwrap();
        poll(scope, &state).unwrap();
        pending.stream = Some(91);
        complete(
            &state,
            pending,
            Ok(PlatformHostResult::HttpChunk {
                encoded_body_size: None,
                stream_id: 91,
                bytes: b"late".to_vec(),
                done: false,
            }),
        )
        .unwrap();
        assert!(state.preload_resources.borrow().entries.is_empty());
        assert!(host.requests.borrow().iter().any(|(_, request)| matches!(
            request,
            PlatformHostRequest::HttpAbort { stream_id: 91 }
        )));
    }
    #[test]
    fn preload_request_uses_document_encoding_and_reselects_after_base_change() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<base id=b href='/first/'><link rel=preload as=image href='image?å'>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/page").unwrap();
        dom.borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "windows-1252".into();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        poll(scope, &state).unwrap();
        let base = dom.borrow().find_by_id("b").unwrap();
        dom.borrow_mut()
            .set_attribute(base, "href", "/second/")
            .unwrap();
        poll(scope, &state).unwrap();
        let urls = host
            .requests
            .borrow()
            .iter()
            .filter_map(|(_, request)| {
                if let PlatformHostRequest::ImagePreloadStart { request, .. } = request {
                    Some(request.url.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            urls,
            vec![
                "https://example.test/first/image?%E5",
                "https://example.test/second/image?%E5"
            ]
        );
        assert!(!host.cancelled.borrow().is_empty());
    }
    #[test]
    fn responsive_link_preloads_select_native_candidates_and_cancel_on_reselection() {
        struct Provider(Cell<f64>);
        impl BindingLayoutProvider for Provider {
            fn compute_layout(&self, _: &BTreeMap<NodeId, BTreeSet<String>>) -> Result<BindingLayout, String> {
                Ok(BindingLayout::default())
            }
            fn native_device_pixel_ratio(&self) -> f64 { self.0.get() }
            fn native_image_source_size(&self, sizes: &str, _: &str, auto: Option<f64>) -> Result<f64, String> {
                assert!(auto.is_none(), "link cannot use rendered auto width");
                Ok(if sizes == "200px" { 200. } else { 100. })
            }
        }
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<link id=p rel=preload as=image imagesrcset='small.png 100w, large.png 200w' imagesizes=100px>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/page").unwrap();
        let provider = Rc::new(Provider(Cell::new(1.)));
        state.set_layout_provider(provider.clone());
        let host = Rc::new(Host::default());state.set_platform_host(host.clone());
        assert!(super::super::test_eval::eval(scope, "devicePixelRatio===1").is_true());
        poll(scope, &state).unwrap();
        provider.0.set(2.);assert!(ready(&state));poll(scope, &state).unwrap();
        assert!(super::super::test_eval::eval(scope, "devicePixelRatio===2").is_true());
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            let owner=window;
            while(owner&&!Object.getOwnPropertyDescriptor(owner,'devicePixelRatio'))owner=Object.getPrototypeOf(owner);
            const d=Object.getOwnPropertyDescriptor(owner,'devicePixelRatio');
            globalThis.nativeDprGetter=d.get;
            if(typeof d.get!=='function'||typeof d.set!=='function'||!d.enumerable||!d.configurable)return false;
            for(const receiver of [{},document]){
                try{d.get.call(receiver);return false}catch(e){if(!(e instanceof TypeError))return false}
            }
            window.devicePixelRatio=99;
            return window.devicePixelRatio===99&&Object.getOwnPropertyDescriptor(window,'devicePixelRatio').value===99;
        })()"#).is_true());
        provider.0.set(1.);
        let node = dom.borrow().find_by_id("p").unwrap();
        dom.borrow_mut().set_attribute(node, "imagesizes", "200px").unwrap();
        poll(scope, &state).unwrap();
        let urls = host.requests.borrow().iter().filter_map(|(_,r)| {
            if let PlatformHostRequest::ImagePreloadStart {request,..}=r {Some(request.url.clone())} else {None}
        }).collect::<Vec<_>>();
        assert_eq!(urls, ["https://example.test/small.png", "https://example.test/large.png"]);
        assert!(!host.cancelled.borrow().is_empty());
        assert!(super::super::test_eval::eval(scope, "delete window.devicePixelRatio;typeof devicePixelRatio==='undefined'&&nativeDprGetter.call(window)===1").is_true());
        assert!(super::super::test_eval::eval(scope, r#"(()=>{
            const link=document.getElementById('p');
            link.imageSrcset='\ud800';if(link.getAttribute('imagesrcset')!=='\ufffd'||link.imageSrcset!=='\ufffd')return false;
            link.setAttribute('imagesrcset','\ud800');if(link.imageSrcset!=='\ufffd')return false;
            link.imageSizes='\ud800';if(link.imageSizes!=='\ud800'||link.getAttribute('imagesizes')!=='\ud800')return false;
            return true;
        })()"#).is_true());
    }
    #[test]
    fn image_preloads_publish_real_error_tasks_and_resource_timing_entries() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<!doctype html><link rel=preload as=image href='https://foreign.test/partial'>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        let source=v8::String::new(scope,"globalThis.errors=0;document.querySelector('link').onerror=e=>{if(!e.isTrusted)throw Error('untrusted preload');errors++}").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        poll(scope, &state).unwrap();
        let (id, request) = host.requests.borrow()[0].clone();
        assert!(matches!(
            request,
            PlatformHostRequest::ImagePreloadStart {
                cors_enabled: false,
                ..
            }
        ));
        state
            .handle_platform_completion(
                scope,
                crate::platform::PlatformHostCompletion {
                    operation_id: id,
                    result: Err("opaque range replay denied".into()),
                },
            )
            .unwrap();
        poll(scope, &state).unwrap();
        let source=v8::String::new(scope,"errors===1&&performance.getEntriesByType('resource').length===1&&performance.getEntries()[0].initiatorType==='link'").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let count = host.requests.borrow().len();
        poll(scope, &state).unwrap();
        assert_eq!(
            host.requests.borrow().len(),
            count,
            "unchanged link does not restart completed preload"
        );
    }
    #[test]
    fn successful_preload_decode_waits_for_terminal_and_removed_link_suppresses_event() {
        for removed in [false, true] {
            super::super::tests::initialize_v8();
            let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<!doctype html><link rel=preload as=image href='https://example.test/image.png'>").into_runtime().unwrap()));
            let mut isolate = v8::Isolate::new(Default::default());
            v8::scope!(let scope,&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state =
                install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
            let host = Rc::new(Host::default());
            state.set_platform_host(host.clone());
            let source=v8::String::new(scope,"globalThis.loads=0;globalThis.link=document.querySelector('link');link.onload=e=>{if(!e.isTrusted)throw Error('untrusted');loads++}").unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            poll(scope, &state).unwrap();
            let id = host.requests.borrow()[0].0;
            let Some(PendingPlatformOperation::Preload(mut pending)) =
                state.pending_platform_operations.borrow_mut().remove(&id)
            else {
                panic!("preload")
            };
            pending.stream = Some(77);
            let mut png = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(1, 1)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            complete(
                &state,
                pending,
                Ok(PlatformHostResult::HttpChunk {
                    encoded_body_size: None,
                    stream_id: 77,
                    bytes: png.into_inner(),
                    done: false,
                }),
            )
            .unwrap();
            assert!(
                state
                    .preload_resources
                    .borrow()
                    .entries
                    .values()
                    .all(|e| e.event.is_none())
            );
            let read = host.requests.borrow().last().unwrap().0;
            let Some(PendingPlatformOperation::Preload(pending)) =
                state.pending_platform_operations.borrow_mut().remove(&read)
            else {
                panic!("read")
            };
            complete(
                &state,
                pending,
                Ok(PlatformHostResult::HttpChunk {
                    encoded_body_size: None,
                    stream_id: 77,
                    bytes: Vec::new(),
                    done: true,
                }),
            )
            .unwrap();
            if removed {
                let source = v8::String::new(scope, "link.remove()").unwrap();
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap();
            }
            poll(scope, &state).unwrap();
            let source =
                v8::String::new(scope, if removed { "loads===0" } else { "loads===1" }).unwrap();
            assert!(
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap()
                    .is_true()
            );
        }
    }
}

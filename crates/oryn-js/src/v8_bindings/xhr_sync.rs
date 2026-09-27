//! Blocking XHR waits only for its own native operation. It does not run a
//! nested JavaScript event loop, deliver unrelated responses or checkpoint jobs.
//! HTTP authority, CORS, redirects and cookies remain with PlatformHost.
use super::*;

pub(super) fn exception(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "XMLHttpRequest".into(),
        member: name.into(),
    }
}

struct Custody<'a> {
    state: &'a V8BindingState,
    host: Rc<dyn PlatformHost>,
    operation: Option<HostOperationId>,
    stream: Option<u64>,
}

impl Drop for Custody<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.operation.take() {
            self.host.cancel(id);
            self.state
                .pending_platform_operations
                .borrow_mut()
                .remove(&id);
            self.state
                .cancelled_platform_operations
                .borrow_mut()
                .insert(id);
        }
        if let Some(stream) = self.stream.take() {
            // Transfer to pre-reserved cleanup custody even when abort admission
            // is unavailable. This is not a claim of physical cleanup completion.
            self.host.abandon_http_stream(stream);
        }
    }
}

impl Custody<'_> {
    fn wait(
        &mut self,
        scope: &mut v8::PinScope,
        deadline: Option<Instant>,
        request: PlatformHostRequest,
        pending: PendingPlatformOperation,
        accept_value: Option<fn(&PlatformHostResult) -> bool>,
    ) -> Result<PlatformHostResult, V8BindingError> {
        if scope.is_execution_terminating() || self.state.execution_retired() {
            return Err(exception("AbortError"));
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(exception("TimeoutError"));
        }
        let id = submit_platform_operation(self.state, request, pending)
            .map_err(|_| exception("NetworkError"))?;
        self.operation = Some(id);
        loop {
            if scope.is_execution_terminating() || self.state.execution_retired() {
                return Err(exception("AbortError"));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(exception("TimeoutError"));
            }
            let ready = self
                .state
                .ready_platform_completions
                .borrow()
                .iter()
                .position(|completion| completion.operation_id == id);
            if let Some(index) = ready {
                let completion = self
                    .state
                    .ready_platform_completions
                    .borrow_mut()
                    .remove(index)
                    .expect("owned synchronous completion");
                self.state
                    .pending_platform_operations
                    .borrow_mut()
                    .remove(&id);
                self.operation = None;
                resource_timing::complete(self.state, id, &completion.result);
                let accepted = if let Some(accept) = accept_value {
                    match &completion.result {
                        // Resource-bearing replies require their explicit
                        // custody protocol, never generic value delivery.
                        Ok(
                            PlatformHostResult::HttpStarted { .. }
                            | PlatformHostResult::WebSocketOpened { .. },
                        ) => false,
                        Ok(value) => accept(value),
                        Err(_) => false,
                    }
                } else {
                    match &completion.result {
                        Ok(PlatformHostResult::HttpStarted { stream_id, .. })
                            if self.stream.is_none() =>
                        {
                            self.stream = Some(*stream_id);
                            true
                        }
                        Ok(PlatformHostResult::HttpChunk { stream_id, .. }) => {
                            self.stream == Some(*stream_id)
                        }
                        Err(_) => false,
                        _ => false,
                    }
                };
                self.host.acknowledge_completion(id, accepted);
                if !accepted {
                    return Err(exception("NetworkError"));
                }
                return completion.result.map_err(|_| exception("NetworkError"));
            }
            // Preserve order and producer custody of other operations. They are
            // delivered at the next ordinary event-loop checkpoint after send.
            let incoming = self.host.drain_completions(16);
            let empty = incoming.is_empty();
            self.state
                .ready_platform_completions
                .borrow_mut()
                .extend(incoming);
            if empty {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

/// Wait for one value response without a JavaScript event-loop checkpoint.
/// Callers must explicitly name their terminal reply category. This cannot
/// accept stream/socket resources through a generic value result.
pub(super) fn request(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    request: PlatformHostRequest,
    deadline: Option<Instant>,
    accept: fn(&PlatformHostResult) -> bool,
) -> Result<PlatformHostResult, V8BindingError> {
    let host = state
        .platform_host
        .borrow()
        .clone()
        .ok_or_else(|| exception("NetworkError"))?;
    let mut custody = Custody {
        state,
        host,
        operation: None,
        stream: None,
    };
    custody.wait(
        scope,
        deadline,
        request,
        PendingPlatformOperation::SynchronousHttp,
        Some(accept),
    )
}

/// A complete response collected through the caller's existing native authority.
pub(super) struct Response {
    pub status: u16,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub(super) fn fetch(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    request: PlatformHttpRequest,
    deadline: Option<Instant>,
    max_body_bytes: usize,
) -> Result<Response, V8BindingError> {
    fetch_request(
        scope,
        state,
        PlatformHostRequest::HttpStart { request },
        deadline,
        max_body_bytes,
    )
}

/// The caller selects an existing, parent-validated request category. This does
/// not infer a destination from author-controlled headers or bypass host policy.
pub(super) fn fetch_request(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    start: PlatformHostRequest,
    deadline: Option<Instant>,
    max_body_bytes: usize,
) -> Result<Response, V8BindingError> {
    let host = state
        .platform_host
        .borrow()
        .clone()
        .ok_or_else(|| exception("NetworkError"))?;
    let mut custody = Custody {
        state,
        host,
        operation: None,
        stream: None,
    };
    let start = custody.wait(
        scope,
        deadline,
        start,
        PendingPlatformOperation::SynchronousHttp,
        None,
    )?;
    let PlatformHostResult::HttpStarted {
        stream_id,
        status,
        url,
        headers,
        ..
    } = start
    else {
        return Err(exception("NetworkError"));
    };
    let mut body = Vec::new();
    loop {
        let chunk = custody.wait(
            scope,
            deadline,
            PlatformHostRequest::HttpRead {
                stream_id,
                max_bytes: 64 * 1024,
            },
            PendingPlatformOperation::SynchronousHttp,
            None,
        )?;
        let PlatformHostResult::HttpChunk { bytes, done, .. } = chunk else {
            return Err(exception("NetworkError"));
        };
        if bytes.len() > max_body_bytes.saturating_sub(body.len()) {
            return Err(exception("NetworkError"));
        }
        body.try_reserve(bytes.len())
            .map_err(|_| exception("NetworkError"))?;
        body.extend_from_slice(&bytes);
        if done {
            custody.stream = None;
            break;
        }
    }
    Ok(Response {
        status,
        url,
        headers,
        body,
    })
}

pub(super) fn send(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    generation: u64,
    request: PlatformHttpRequest,
) -> Result<(), V8BindingError> {
    let deadline = match &*platform_object(state, id)? {
        PlatformObjectRecord::XMLHttpRequest(xhr) => xhr.deadline,
        _ => return Err(V8BindingError::Allocation("XHR record")),
    };
    let result = (|| {
        // Blob URLs are realm-owned capabilities, using the same lookup as the
        // asynchronous path. No host request is needed for their immutable data.
        if let Some(blob) = blob_url_record(state, &request.url) {
            if request.method != "GET" || blob.bytes.len() > 16 * 1024 * 1024 {
                return Err(exception("NetworkError"));
            }
            let headers = vec![("content-type".into(), blob.media_type)];
            return Ok(Response {
                status: 200,
                url: request.url,
                headers,
                body: blob.bytes,
            });
        }
        // Match the native broker's existing collected-response limit. No
        // content-controlled size may grow an unbounded synchronous buffer.
        fetch(scope, state, request, deadline, 16 * 1024 * 1024)
    })();
    {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
            return Err(V8BindingError::Allocation("XHR record"));
        };
        xhr.ready_state = 4;
        xhr.send_invoked = false;
        xhr.upload_complete = true;
        xhr.active_operation = None;
        xhr.stream_id = None;
        xhr.deadline = None;
        xhr.send_started = None;
        match result {
            Ok(Response {
                status,
                url,
                headers,
                body,
            }) => {
                xhr.status = status;
                xhr.response_url = url;
                xhr.response_total = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse().ok());
                xhr.response_headers = visible_response_headers(headers);
                xhr.response_bytes = body;
            }
            Err(error) => {
                xhr.status = 0;
                xhr.response_url.clear();
                xhr.response_headers.clear();
                xhr.response_bytes.clear();
                xhr.response_total = None;
                // Synchronous request-error steps throw, without async error,
                // timeout, upload or progress events.
                return Err(error);
            }
        }
    }
    // WPT send-sync-no-response-event-order: OPENED (from open), DONE,
    // load, loadend. No HEADERS_RECEIVED, LOADING, loadstart or upload events.
    for event in ["readystatechange", "load", "loadend"] {
        if !xhr_current_generation(state, id, generation) {
            break;
        }
        dispatch_platform_event(scope, state, id, event, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_dom::html;

    #[derive(Default)]
    struct Host {
        ready: RefCell<VecDeque<PlatformHostCompletion>>,
        requests: RefCell<Vec<PlatformHttpRequest>>,
        cancelled: RefCell<Vec<HostOperationId>>,
        abandoned: RefCell<Vec<u64>>,
        acknowledged: RefCell<Vec<(HostOperationId, bool)>>,
        stall_start: bool,
        stall_read: bool,
        deny: bool,
        wrong_stream: bool,
        terminate_on_read: Option<v8::IsolateHandle>,
    }
    impl PlatformHost for Host {
        fn submit(&self, id: HostOperationId, request: PlatformHostRequest) -> Result<(), String> {
            let result = match request {
                PlatformHostRequest::HttpStart { request } => {
                    self.requests.borrow_mut().push(request);
                    if self.stall_start {
                        return Ok(());
                    }
                    if self.deny {
                        Err("policy denied".into())
                    } else {
                        Ok(PlatformHostResult::HttpStarted {
                            timing: None,
                            internal_response: None,
                            response_type: None,
                            upload: None,
                            cors_filtered: false,
                            redirected: false,
                            stream_id: 7,
                            status: 200,
                            url: "https://example.test/final".into(),
                            headers: vec![
                                ("content-type".into(), "text/plain".into()),
                                ("content-length".into(), "6".into()),
                                ("set-cookie".into(), "private=1".into()),
                            ],
                        })
                    }
                }
                PlatformHostRequest::HttpRead { stream_id, .. } => {
                    if let Some(handle) = &self.terminate_on_read {
                        handle.terminate_execution();
                    }
                    if self.stall_read {
                        return Ok(());
                    }
                    Ok(PlatformHostResult::HttpChunk {
                        encoded_body_size: None,
                        stream_id: if self.wrong_stream { 99 } else { stream_id },
                        bytes: b"native".to_vec(),
                        done: true,
                    })
                }
                PlatformHostRequest::HttpAbort { .. } => Ok(PlatformHostResult::HttpAborted),
                _ => return Err("unexpected request".into()),
            };
            self.ready.borrow_mut().push_back(PlatformHostCompletion {
                operation_id: id,
                result,
            });
            Ok(())
        }
        fn cancel(&self, id: HostOperationId) {
            self.cancelled.borrow_mut().push(id);
        }
        fn abandon_http_stream(&self, stream: u64) {
            self.abandoned.borrow_mut().push(stream);
        }
        fn retire(&self) {}
        fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
            let mut ready = self.ready.borrow_mut();
            let count = ready.len().min(limit);
            ready.drain(..count).collect()
        }
        fn acknowledge_completion(&self, id: HostOperationId, accepted: bool) {
            self.acknowledged.borrow_mut().push((id, accepted));
        }
    }

    fn run(global: BindingGlobal, host: Rc<Host>, script: &str) {
        super::super::tests::initialize_v8();
        let parsed = html::parse("");
        let root = parsed.document;
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(0),
            1,
            root,
            global,
        )
        .unwrap();
        state.commit_location("https://example.test/").unwrap();
        state.set_platform_host(host);
        let source = v8::String::new(scope, script).unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        assert!(state.pending_platform_operations.borrow().is_empty());
    }

    #[test]
    fn synchronous_response_events_headers_body_and_unrelated_completion() {
        let host = Rc::new(Host::default());
        host.ready.borrow_mut().push_back(PlatformHostCompletion {
            operation_id: HostOperationId(u64::MAX),
            result: Ok(PlatformHostResult::HttpAborted),
        });
        run(
            BindingGlobal::Window,
            host.clone(),
            r#"
          const events=[];const xhr=new XMLHttpRequest();
          let microtask=false;Promise.resolve().then(()=>microtask=true);
          for(const type of ['readystatechange','loadstart','progress','load','loadend','error'])
            xhr.addEventListener(type,e=>{if(type==='load'&&!(e instanceof ProgressEvent))throw Error('event');events.push(type==='readystatechange'?xhr.readyState:type)});
          xhr.upload.onload=()=>{throw Error('upload event')};
          xhr.open('POST','/body',false);xhr.withCredentials=true;xhr.setRequestHeader('X-Test','yes');xhr.send('payload');
          events.join(',')==='1,4,load,loadend' && xhr.status===200 && xhr.responseText==='native'
            && xhr.response==='native' && xhr.responseURL==='https://example.test/final'
            && xhr.getResponseHeader('set-cookie')===null && !microtask
        "#,
        );
        let request = &host.requests.borrow()[0];
        assert_eq!(request.body, b"payload");
        assert_eq!(request.credentials, "include");
        assert!(request.use_cors_preflight);
        assert_eq!(host.acknowledged.borrow().len(), 2);
        assert!(
            host.acknowledged
                .borrow()
                .iter()
                .all(|(_, accepted)| *accepted)
        );
        assert!(host.cancelled.borrow().is_empty());
        assert!(host.abandoned.borrow().is_empty());
    }

    #[test]
    fn window_sync_restrictions_preserve_open_state() {
        run(
            BindingGlobal::Window,
            Rc::new(Host::default()),
            r#"
          function rejects(f){try{f()}catch(e){return e.name==='InvalidAccessError'}return false}
          const x=new XMLHttpRequest();x.timeout=1;
          if(!rejects(()=>x.open('GET','/',false))||x.readyState!==0)throw Error('timeout open');
          x.timeout=0;x.responseType='text';
          if(!rejects(()=>x.open('GET','/',false))||x.readyState!==0)throw Error('type open');
          x.responseType='';x.open('GET','/',false);
          if(!rejects(()=>x.timeout=0)||!rejects(()=>x.responseType=''))throw Error('setters');
          x.open('GET','/',true);x.timeout=1;x.responseType='text';true
        "#,
        );
    }

    #[test]
    fn worker_sync_typed_response_and_timeout_are_supported() {
        run(
            BindingGlobal::Worker,
            Rc::new(Host::default()),
            r#"
          const x=new XMLHttpRequest();x.timeout=1000;x.responseType='arraybuffer';
          x.open('GET','/',false);x.send('discarded');
          x.status===200&&new Uint8Array(x.response).length===6&&!('responseXML' in x)
        "#,
        );
    }

    #[test]
    fn worker_timeout_cancels_owned_start_or_read_and_no_async_events() {
        for stall_read in [false, true] {
            let host = Rc::new(Host {
                stall_read,
                stall_start: !stall_read,
                ..Default::default()
            });
            run(
                BindingGlobal::Worker,
                host.clone(),
                r#"
              const x=new XMLHttpRequest();x.timeout=100;x.open('GET','/',false);
              let events=0;x.ontimeout=x.onerror=x.onloadend=()=>events++;
              let name='';try{x.send()}catch(e){name=e.name}
              name==='TimeoutError'&&x.status===0&&x.readyState===4&&events===0
            "#,
            );
            assert_eq!(host.cancelled.borrow().len(), 1);
            assert_eq!(
                *host.abandoned.borrow(),
                if stall_read { vec![7] } else { vec![] }
            );
        }
    }

    #[test]
    fn network_failure_or_wrong_stream_cannot_publish_response() {
        for wrong_stream in [false, true] {
            let host = Rc::new(Host {
                wrong_stream,
                deny: !wrong_stream,
                ..Default::default()
            });
            run(
                BindingGlobal::Window,
                host.clone(),
                r#"
              const x=new XMLHttpRequest();x.open('GET','/',false);
              let events=0;x.onerror=x.onloadend=()=>events++;
              let name='';try{x.send()}catch(e){name=e.name}
              name==='NetworkError'&&x.status===0&&x.readyState===4&&x.responseText===''&&events===0
            "#,
            );
            assert!(
                host.acknowledged
                    .borrow()
                    .iter()
                    .any(|(_, accepted)| !*accepted)
            );
            assert_eq!(
                *host.abandoned.borrow(),
                if wrong_stream { vec![7] } else { vec![] }
            );
        }
    }

    #[test]
    fn synchronous_blob_and_completion_reentrant_open_keep_new_generation() {
        run(
            BindingGlobal::Window,
            Rc::new(Host::default()),
            r#"
          const x=new XMLHttpRequest(),url=URL.createObjectURL(new Blob(['blob']));
          const events=[];x.open('GET',url,false);
          x.onload=()=>{events.push(x.responseText);x.open('GET','/new',false)};
          x.onloadend=()=>events.push('old loadend');x.send();URL.revokeObjectURL(url);
          events.join(',')==='blob'&&x.readyState===1&&x.responseText===''
        "#,
        );
    }
    #[test]
    fn collected_body_limit_and_mid_read_termination_transfer_custody() {
        for terminate in [false, true] {
            super::super::tests::initialize_v8();
            let parsed = html::parse("");
            let root = parsed.document;
            let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
            let mut isolate = v8::Isolate::new(Default::default());
            let handle = isolate.thread_safe_handle();
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let state = install_generated_bindings_with_root_for_global(
                scope,
                context,
                dom,
                RealmId(0),
                1,
                root,
                BindingGlobal::Window,
            )
            .unwrap();
            let host = Rc::new(Host {
                terminate_on_read: terminate.then_some(handle),
                ..Default::default()
            });
            state.set_platform_host(host.clone());
            let request = PlatformHttpRequest {
                upgrade_mixed_image: false,
                fetch: None,
                use_cors_preflight: false,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: vec![],
                body: vec![],
                credentials: "same-origin".into(),
                initiator_url: Some("https://example.test/".into()),
                referrer_policy: None,
            };
            let result = fetch(
                scope,
                &state,
                request,
                Some(Instant::now() + Duration::from_secs(1)),
                5,
            );
            assert!(
                matches!(result, Err(V8BindingError::Unsupported { member, .. })
                if member == if terminate { "AbortError" } else { "NetworkError" })
            );
            if terminate {
                scope.cancel_terminate_execution();
            }
            assert_eq!(*host.abandoned.borrow(), vec![7]);
            assert_eq!(host.cancelled.borrow().len(), usize::from(terminate));
            assert!(state.pending_platform_operations.borrow().is_empty());
        }
    }

    #[test]
    fn value_reply_helper_accepts_only_named_nonresource_result() {
        super::super::tests::initialize_v8();
        let parsed = html::parse("");
        let root = parsed.document;
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(0),
            1,
            root,
            BindingGlobal::Window,
        )
        .unwrap();
        let host = Rc::new(Host::default());
        state.set_platform_host(host.clone());
        assert!(matches!(
            request(
                scope,
                &state,
                PlatformHostRequest::HttpAbort { stream_id: 42 },
                Some(Instant::now() + Duration::from_secs(1)),
                |value| matches!(value, PlatformHostResult::HttpAborted)
            ),
            Ok(PlatformHostResult::HttpAborted)
        ));
        let start = PlatformHostRequest::HttpStart {
            request: PlatformHttpRequest {
                upgrade_mixed_image: false,
                fetch: None,
                use_cors_preflight: false,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: vec![],
                body: vec![],
                credentials: "same-origin".into(),
                initiator_url: Some("https://example.test/".into()),
                referrer_policy: None,
            },
        };
        // Even an overly broad caller predicate cannot publish an unowned stream.
        assert!(
            request(
                scope,
                &state,
                start,
                Some(Instant::now() + Duration::from_secs(1)),
                |_| true
            )
            .is_err()
        );
        assert_eq!(
            host.acknowledged
                .borrow()
                .iter()
                .map(|(_, accepted)| *accepted)
                .collect::<Vec<_>>(),
            vec![true, false]
        );
        assert!(state.pending_platform_operations.borrow().is_empty());
    }
}

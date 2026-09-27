// Native Window.open admission. Names use the same collision-safe DOMString
// representation as frame attributes, native history and parent selectors.
fn window_open_dom_string(scope: &mut v8::PinScope, value: v8::Local<v8::String>) -> String {
    let mut units = vec![0_u16; value.length()];
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
    oryn_dom::dom_string::from_units(&units)
}
#[derive(Default, Debug, PartialEq, Eq)]
struct WindowOpenFeatures {
    noopener: bool,
    noreferrer: bool,
}
fn window_open_features(value: &str) -> WindowOpenFeatures {
    let units = oryn_dom::dom_string::units(value);
    let scalar = String::from_utf16_lossy(&units).to_ascii_lowercase();
    let mut result = WindowOpenFeatures::default();
    // The feature tokenizer accepts ASCII whitespace around both '=' and ','.
    let bytes = scalar.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b',')
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b',' | b'=')
        {
            cursor += 1;
        }
        let name = &scalar[start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = "";
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let start = cursor;
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && bytes[cursor] != b','
            {
                cursor += 1;
            }
            value = &scalar[start..cursor];
        }
        let integer = value
            .strip_prefix('+')
            .or_else(|| value.strip_prefix('-'))
            .unwrap_or(value);
        let digits = integer
            .bytes()
            .take_while(u8::is_ascii_digit)
            .collect::<Vec<_>>();
        let enabled = value.is_empty()
            || value == "yes"
            || value == "true"
            || !digits.is_empty() && digits.iter().any(|digit| *digit != b'0');
        match name {
            "noopener" => result.noopener = enabled,
            "noreferrer" => result.noreferrer = enabled,
            _ => {}
        }
        if cursor == start && cursor < bytes.len() {
            cursor += 1;
        }
    }
    result.noopener |= result.noreferrer;
    result
}

fn standalone_window_open_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Object>>, String> {
    let top = oryn_js::v8_bindings::native_window_top(scope, window);
    match name.to_ascii_lowercase().as_str() {
        "" | "_self" => return Ok(Some(window)),
        "_parent" => {
            return Ok(Some(oryn_js::v8_bindings::native_window_parent(
                scope, window,
            )));
        }
        "_top" => return Ok(Some(top)),
        "_blank" => return Ok(None),
        _ => {}
    }
    let mut seen = BTreeSet::new();
    let mut roots = vec![window, top];
    roots.extend(standalone_auxiliary_windows(scope, window));
    for root in roots {
        let mut pending = vec![root];
        while let Some(window) = pending.pop() {
            let Some(context) = window.get_creation_context(scope) else {
                continue;
            };
            let Some(state) = context.get_slot::<V8BindingState>() else {
                continue;
            };
            if state.execution_retired()
                || state.execution_suspended()
                || !seen.insert(state.realm())
            {
                continue;
            }
            if let Some(frame) = context.get_slot::<FrameLifetime>() {
                let Some(parent) = frame.parent.upgrade() else {
                    continue;
                };
                let dom = parent.dom();
                let dom = dom.borrow();
                if frame.closed.get()
                    || parent.execution_retired()
                    || !dom
                        .is_connected(frame.frame)
                        .map_err(|error| error.to_string())?
                    || dom
                        .node_document(frame.frame)
                        .map_err(|error| error.to_string())?
                        != Some(parent.document_root())
                    || dom.frame_context_generation(frame.frame)
                        != Some(frame.connection_generation)
                {
                    continue;
                }
            }
            if seen.len() > 4096 {
                return Err("Window.open target tree capacity".into());
            }
            if state.native_window_target_name() == name {
                return Ok(Some(window));
            }
            let children = {
                let dom = state.dom();
                let dom = dom.borrow();
                let mut nodes = vec![state.document_root()];
                let mut children = Vec::new();
                let mut visited = BTreeSet::new();
                while let Some(node) = nodes.pop() {
                    if !visited.insert(node) {
                        continue;
                    }
                    if visited.len() > 65536 {
                        return Err("Window.open DOM tree capacity".into());
                    }
                    if let Some(child) = state.frame_window(node) {
                        children.push(child);
                        continue;
                    }
                    let record = dom.arena().get(node).map_err(|error| error.to_string())?;
                    if node != state.document_root()
                        && matches!(record.kind, oryn_dom::NodeKind::Document)
                    {
                        continue;
                    }
                    nodes.extend(record.children.iter().rev().copied());
                }
                children
            };
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| v8::Local::new(scope, child)),
            );
        }
    }
    Ok(None)
}

const POPUP_DENIAL_TRACE_CAPACITY: usize = 64;
const POPUP_DENIAL_REASON_BYTES: usize = 1024;

// A denial is parent-owned diagnostic data, not an author callback or a root
// for a Window. Keep it on the top context so removal of a sending child does
// not erase it and another root sharing this isolate cannot drain it.
#[derive(Default)]
struct NativePopupDenials {
    reasons: Vec<(String, bool)>,
    overflow: u64,
}
impl NativePopupDenials {
    fn record(&mut self, reason: String) {
        if self.reasons.len() == POPUP_DENIAL_TRACE_CAPACITY {
            self.overflow = self.overflow.saturating_add(1);
            return;
        }
        let truncated = reason.len() > POPUP_DENIAL_REASON_BYTES;
        let mut end = reason.len().min(POPUP_DENIAL_REASON_BYTES);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        // Copy the bounded prefix rather than retaining a large input buffer
        // whose length was truncated but whose allocation stayed large.
        self.reasons.push((reason[..end].to_owned(), truncated));
    }
    fn take(&mut self) -> Vec<crate::executable::ContextTraceEntry> {
        let mut entries = std::mem::take(&mut self.reasons).into_iter().map(|(reason, truncated)| {
            crate::executable::ContextTraceEntry {
                kind: "capability_failure".into(),
                detail: serde_json::json!({"capability":"popup.open","detail":reason,"truncated":truncated}),
            }
        }).collect::<Vec<_>>();
        let overflow = std::mem::take(&mut self.overflow);
        if overflow != 0 {
            entries.push(crate::executable::ContextTraceEntry {
                kind: "capability_failure".into(),
                detail: serde_json::json!({"capability":"popup.open","detail":"additional popup denials omitted from bounded native trace","omitted":overflow}),
            });
        }
        entries
    }
}

fn popup_trace_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Context> {
    let window = context.global(scope);
    let top = oryn_js::v8_bindings::native_window_top(scope, window);
    top.get_creation_context(scope).unwrap_or(context)
}

fn record_native_popup_denial<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
    reason: String,
) {
    let context = popup_trace_context(scope, context);
    let trace = context
        .get_slot::<RefCell<NativePopupDenials>>()
        .unwrap_or_else(|| {
            let trace = Rc::new(RefCell::new(NativePopupDenials::default()));
            context.set_slot(trace.clone());
            trace
        });
    trace.borrow_mut().record(reason);
}

impl RawV8Host {
    pub(crate) fn take_window_open_trace_entries(
        &mut self,
    ) -> Vec<crate::executable::ContextTraceEntry> {
        let Some(context) = self.contexts.get(&self.active_realm).cloned() else {
            return Vec::new();
        };
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let context = popup_trace_context(scope, context);
        context
            .get_slot::<RefCell<NativePopupDenials>>()
            .map(|trace| trace.borrow_mut().take())
            .unwrap_or_default()
    }
}

fn native_window_open_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    rv.set(v8::null(scope).into());
    let Some(context) = native_window_callback_context(scope, &args) else {
        throw_type_error(scope, "Window.open requires a Window receiver");
        return;
    };
    let window = context.global(scope);
    if !oryn_js::v8_bindings::check_native_window_access(scope, window) {
        return;
    }
    let entry = {
        let context = scope.get_entered_or_microtask_context();
        v8::Global::new(scope, context)
    };
    let Some(binding) = context.get_slot::<V8BindingState>() else {
        return;
    };
    let generation = binding.document_generation();
    let stream = binding.document_stream_epoch();
    let mut converted = Vec::with_capacity(3);
    for (index, default) in [(0, ""), (1, "_blank"), (2, "")] {
        if args.get(index).is_undefined() {
            converted.push(default.to_string());
            continue;
        }
        let Some(value) = args.get(index).to_string(scope) else {
            return;
        };
        converted.push(window_open_dom_string(scope, value));
    }
    // Conversion can navigate, close, or replace the receiver Document.
    if binding.execution_retired()
        || binding.execution_suspended()
        || binding.document_generation() != generation
        || binding.document_stream_epoch() != stream
    {
        return;
    }
    if !oryn_js::v8_bindings::check_native_window_access(scope, window) {
        return;
    }
    let result = (|| -> Result<Option<v8::Local<v8::Object>>, String> {
        let base_state = v8::Local::new(scope, &entry)
            .get_slot::<V8BindingState>()
            .unwrap_or_else(|| binding.clone());
        let url = if converted[0].is_empty() {
            None
        } else {
            let scalar = String::from_utf16_lossy(&oryn_dom::dom_string::units(&converted[0]));
            let base = base_state
                .native_document_base_url(true)
                .map_err(|error| error.to_string())?;
            Some(
                Url::parse(&base)
                    .and_then(|base| base.join(&scalar))
                    .map_err(|_| "SyntaxError: invalid Window.open URL".to_string())?
                    .to_string(),
            )
        };
        let features = window_open_features(&converted[2]);
        let name = if converted[1].is_empty() {
            "_blank"
        } else {
            converted[1].as_str()
        };
        let name = oryn_js::navigation::effective_navigation_target(name, features.noopener);
        let scope = &mut v8::ContextScope::new(scope, context);
        #[cfg(unix)]
        if let Some(endpoint) =
            endpoints_for_context(scope, context).and_then(|endpoints| endpoints.context)
        {
            if features.noopener && name.eq_ignore_ascii_case("_blank") && url.is_some() {
                reserve_blank_prerender_activation_slot(scope)
                    .map_err(|error| error.to_string())?;
            }
            let rollback = endpoint
                .client
                .reserve_auxiliary_rollback(endpoint.page_id)
                .map_err(|error| error.to_string())?;
            let response = endpoint
                .client
                .request(ContextServiceRequest::OpenPage {
                    hyperlink_audit: None,
                    local_inventory: local_navigation_inventory(scope)
                        .map_err(|error| error.to_string())?,
                    link_navigation: false,
                    user_initiated: false,
                    defer_navigation: false,
                    local_related_navigation: true,
                    selected_target: None,
                    document_source: None,
                    opener_page_id: endpoint.page_id,
                    creator: Some(
                        binding
                            .native_worker_creator()
                            .map_err(|error| error.to_string())?,
                    ),
                    noopener: features.noopener,
                    target_name: name.into(),
                    form_authority: None,
                    url,
                    post: None,
                    referrer_policy: features.noreferrer.then(|| "no-referrer".into()),
                })
                .map_err(|error| error.to_string())?;
            return match response {
                ContextServiceResponse::PageDenied { reason } => {
                    record_native_popup_denial(scope, context, reason);
                    Ok(None)
                }
                ContextServiceResponse::PageOpened {
                    selected_frame,
                    page_id,
                    created,
                    opener_page_id,
                    related_root,
                    related_navigation_url,
                    blank_prerender,
                    ..
                } => {
                    if let Some(receipt) = blank_prerender {
                        if !features.noopener || created || related_root.is_some() {
                            return Err("invalid Window.open prerender activation".into());
                        }
                        queue_blank_prerender_activation(scope, receipt)
                            .map_err(|error| error.to_string())?;
                        return Ok(None);
                    }
                    if related_navigation_url.is_some()
                        && (!created
                            || features.noopener
                            || opener_page_id.is_none()
                            || related_root.is_none())
                    {
                        return Err("invalid related initial navigation response".into());
                    }
                    if let Some(admission) = related_root {
                        if !created {
                            return Err(
                                "existing Window.open target supplied auxiliary admission".into()
                            );
                        }
                        create_admitted_auxiliary_root(
                            scope,
                            page_id,
                            opener_page_id,
                            admission,
                            related_navigation_url.as_deref(),
                            rollback,
                        )
                        .map_err(|error| error.to_string())?;
                    }
                    if features.noopener || created && opener_page_id.is_none() {
                        return Ok(None);
                    }
                    if selected_frame.is_none() && page_id == endpoint.page_id {
                        return Ok(Some(oryn_js::v8_bindings::native_window_top(scope, window)));
                    }
                    let target = match selected_frame {
                        Some(frame) => oryn_js::message_wire::WindowTarget::Frame(
                            frame.frame_token.ok_or_else(|| {
                                "Window.open selected frame token missing".to_string()
                            })?,
                        ),
                        None => oryn_js::message_wire::WindowTarget::Root,
                    };
                    remote_window_object(scope, page_id, target, false, true)
                        .map(Some)
                        .map_err(|error| error.to_string())
                }
                _ => Err("invalid Window.open response".into()),
            };
        }
        let selected = match standalone_window_open_target(scope, window, name)? {
            Some(target) => Some(target),
            None => create_standalone_auxiliary_root(
                scope,
                context,
                name,
                features.noopener,
                features.noreferrer,
            )?,
        };
        if !features.noopener
            && !["_self", "_parent", "_top"]
                .iter()
                .any(|keyword| name.eq_ignore_ascii_case(keyword))
            && let Some(target) = selected
            && let Some(frame) = target
                .get_creation_context(scope)
                .and_then(|context| context.get_slot::<FrameLifetime>())
        {
            *frame.opener.borrow_mut() = Some(v8::Global::new(scope, window));
            oryn_js::v8_bindings::set_native_window_opener(scope, target, Some(window))
                .map_err(|error| error.to_string())?;
        }

        if let (Some(target), Some(url)) = (selected, url) {
            let target_context = target
                .get_creation_context(scope)
                .ok_or_else(|| "Window.open target context missing".to_string())?;
            let url = v8::String::new(scope, &url)
                .ok_or_else(|| "Window.open URL allocation".to_string())?;
            oryn_js::v8_bindings::set_native_window_location(scope, target_context, url.into());
        }
        Ok(if features.noopener { None } else { selected })
    })();
    match result {
        Ok(Some(window)) => rv.set(window.into()),
        Ok(None) => {}
        Err(error) if error.starts_with("SyntaxError:") => {
            oryn_js::v8_bindings::window_messages::throw_external_message_error(
                scope,
                "SyntaxError",
            )
        }
        Err(error) => throw_type_error(scope, &error),
    }
}

#[cfg(test)]
mod window_open_feature_tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn open_page_is_not_an_author_json_operation() {
        let request = ContextServiceRequest::OpenPage {
            hyperlink_audit: None,
            local_inventory: Default::default(),
            link_navigation: false,
            user_initiated: false,
            defer_navigation: false,
            local_related_navigation: false,
            selected_target: None,
            document_source: None,
            opener_page_id: PageId(1),
            creator: None,
            noopener: false,
            target_name: "name".into(),
            form_authority: None,
            url: None,
            post: None,
            referrer_policy: None,
        };
        assert!(!request.author_compatibility_operation());
    }
    #[test]
    fn feature_boolean_tokens_and_duplicates() {
        for value in [
            "noopener",
            "NOOPENER = yes",
            "noopener=2",
            "noopener=99999999999999999999999",
            "noopener=1suffix",
            "noreferrer=true",
            "noopener=0,noreferrer",
        ] {
            assert!(window_open_features(value).noopener, "{value}");
        }
        for value in [
            "noopener=0",
            "noopener=false",
            "noopener=yes,noopener=0",
            "noopener=not-a-number",
        ] {
            assert!(!window_open_features(value).noopener, "{value}");
        }
        assert!(window_open_features("noopener=0,noreferrer=1").noreferrer);
    }

    #[test]
    fn native_popup_denials_bound_utf8_and_report_overflow_once() {
        let mut trace = NativePopupDenials::default();
        trace.record("🧭".repeat(512));
        for index in 1..POPUP_DENIAL_TRACE_CAPACITY {
            trace.record(format!("denial-{index}"));
        }
        for _ in 0..3 {
            trace.record("overflow".into());
        }
        assert_eq!(trace.reasons.len(), POPUP_DENIAL_TRACE_CAPACITY);
        assert!(
            trace
                .reasons
                .iter()
                .all(|(reason, _)| reason.len() <= POPUP_DENIAL_REASON_BYTES)
        );
        assert!(
            trace
                .reasons
                .iter()
                .all(|(reason, _)| reason.capacity() <= POPUP_DENIAL_REASON_BYTES)
        );
        let entries = trace.take();
        assert_eq!(entries.len(), POPUP_DENIAL_TRACE_CAPACITY + 1);
        assert_eq!(
            entries[0].detail["detail"],
            "🧭".repeat(POPUP_DENIAL_REASON_BYTES / 4)
        );
        assert_eq!(entries[0].detail["truncated"], true);
        assert_eq!(entries[1].detail["detail"], "denial-1");
        assert_eq!(entries[1].detail["truncated"], false);
        assert_eq!(entries.last().unwrap().detail["omitted"], 3);
        assert!(
            entries
                .iter()
                .all(|entry| entry.kind == "capability_failure"
                    && entry.detail["capability"] == "popup.open")
        );
        assert!(trace.take().is_empty());
        trace.record("after drain".into());
        assert_eq!(trace.take().len(), 1);
    }

    #[test]
    fn native_popup_denials_stay_with_top_window_and_do_not_cross_roots() {
        fn record(host: &mut RawV8Host, realm: RealmId, reason: &str) {
            let context = host.contexts[&realm].clone();
            v8::scope!(let scope, &mut host.isolate);
            let context = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, context);
            record_native_popup_denial(scope, context, reason.into());
        }
        let mut host = RawV8Host::new();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            1,
        )
        .unwrap();
        let first = host.active_realm();
        host.execute(
            "globalThis.frame=document.createElement('iframe');document.body.append(frame)",
        )
        .unwrap();
        host.adopt_created_realms();
        let child = host
            .realm_ids()
            .into_iter()
            .find(|realm| *realm != first)
            .unwrap();
        let second = host.create_realm("https://second.test/", 1, None).unwrap();
        host.activate_realm(second).unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>second")
                    .into_runtime()
                    .unwrap(),
            )),
            1,
        )
        .unwrap();
        record(&mut host, first, "first root");
        record(&mut host, child, "removed child");
        record(&mut host, second, "second root");
        host.activate_realm(first).unwrap();
        host.execute("frame.remove()").unwrap();
        host.activate_realm(second).unwrap();
        let entries = host.take_window_open_trace_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].detail["detail"], "second root");
        assert!(host.take_window_open_trace_entries().is_empty());
        host.activate_realm(first).unwrap();
        let entries = host.take_window_open_trace_entries();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.detail["detail"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["first root", "removed child"]
        );
        assert!(host.take_window_open_trace_entries().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn denied_popup_trace_survives_legacy_reader_and_parse_failures() {
        let context = crate::Browser::new()
            .new_context_with_options(crate::runtime::ContextOptions::default().with_popups(false));
        let handle = context.new_page();
        let client = SharedContextClient::direct(context, handle.page_id());
        let mut page = crate::executable::RustExecutableDocument::load_with_context_and_limits(
            "<body>",
            &BTreeMap::new(),
            None,
            None,
            256 * 1024 * 1024,
            (client, handle.page_id(), None),
        )
        .unwrap();
        assert_eq!(
            page.evaluate(
                r#"(() => {
            globalThis.traceReads = 0;
            Object.defineProperty(globalThis, '__oryn_context_trace', {configurable:true,
                get(){traceReads++; throw Error('author trace getter')}});
            __oryn_take_context_trace = () => {throw Error('legacy reader failed')};
            return String(open() === null) + '|' + traceReads;
        })()"#
            )
            .unwrap(),
            "true|0"
        );
        let error = page.take_context_trace_entries().unwrap_err();
        assert!(
            error.to_string().contains("legacy reader failed"),
            "{error}"
        );
        page.execute("__oryn_take_context_trace=()=>({wrong:'shape'})")
            .unwrap();
        assert!(page.take_context_trace_entries().is_err());
        page.execute("__oryn_take_context_trace=()=>[]").unwrap();
        let entries = page.take_context_trace_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "capability_failure");
        assert_eq!(entries[0].detail["capability"], "popup.open");
        assert!(!entries[0].detail["detail"].as_str().unwrap().is_empty());
        assert!(page.take_context_trace_entries().unwrap().is_empty());
        drop(page);
        handle.close().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn native_denied_popups_return_null_and_publish_one_trace_each() {
        let context = crate::Browser::new()
            .new_context_with_options(crate::runtime::ContextOptions::default().with_popups(false));
        let page = context.new_page();
        page.load_html("https://popup.example.test/app", "<body>")
            .await
            .unwrap();
        assert_eq!(
            page.evaluate("String(open()===null && open('','named','noopener')===null)")
                .await
                .unwrap(),
            "true"
        );
        for _ in 0..2 {
            let trace = page.trace().await.unwrap();
            assert_eq!(trace.iter().filter(|entry| matches!(&entry.kind,
                crate::trace::TraceEventKind::PolicyDenied { capability } if capability == "popup.open")).count(), 2, "{trace:#?}");
        }
        page.close().await.unwrap();
    }
}

fn native_window_opener_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let window = context.global(scope);
    let scope = &mut v8::ContextScope::new(scope, context);
    #[cfg(unix)]
    if let Some(endpoint) =
        endpoints_for_context(scope, context).and_then(|endpoints| endpoints.context)
    {
        let state = context
            .get_slot::<V8BindingState>()
            .ok_or_else(|| "Window.opener bindings unavailable".to_string())?;
        if state.execution_retired() || state.execution_suspended() {
            return Ok(oryn_js::v8_bindings::native_window_opener(scope, window));
        }
        if !state.document_prerendering() {
            let response = endpoint
                .client
                .request(ContextServiceRequest::NativeWindowOpener {
                    page_id: endpoint.page_id,
                    creator: state
                        .native_worker_creator()
                        .map_err(|error| error.to_string())?,
                })
                .map_err(|error| error.to_string())?;
            let ContextServiceResponse::WindowOpener { opener } = response else {
                return Err("invalid native opener response".into());
            };
            let opener = match opener {
                Some((page, oryn_js::message_wire::WindowTarget::Root))
                    if page == endpoint.page_id =>
                {
                    Some(oryn_js::v8_bindings::native_window_top(scope, window))
                }
                Some((page, target)) => Some(
                    remote_window_object(scope, page, target, false, true)
                        .map_err(|error| error.to_string())?,
                ),
                None => None,
            };
            oryn_js::v8_bindings::set_native_window_opener(scope, window, opener)
                .map_err(|error| error.to_string())?;
            return Ok(opener.map_or_else(|| v8::null(scope).into(), Into::into));
        }
    }
    if let Some(frame) = context.get_slot::<FrameLifetime>() {
        return Ok(frame
            .opener
            .borrow()
            .as_ref()
            .map(|opener| v8::Local::new(scope, opener).into())
            .unwrap_or_else(|| v8::null(scope).into()));
    }
    Ok(oryn_js::v8_bindings::native_window_opener(scope, window))
}

//! Native XHR MIME selection, decoding and cached inert document responses.
use super::*;
use crate::mime_type::MimeType;

fn invalid_state() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "XMLHttpRequest".into(),
        member: "InvalidStateError".into(),
    }
}

fn response_mime(xhr: &XhrRecord) -> MimeType {
    MimeType::from_headers(&xhr.response_headers)
        .unwrap_or_else(|| MimeType::parse("text/xml").expect("valid fallback MIME type"))
}

pub(super) fn override_mime(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let value = value.to_rust_string_lossy(scope);
    let mut record = platform_object_mut(state, id)?;
    let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record else {
        return Err(V8BindingError::Allocation("XHR record"));
    };
    if xhr.ready_state >= 3 {
        return Err(invalid_state());
    }
    xhr.override_mime = Some(MimeType::parse(&value).unwrap_or_else(|| {
        MimeType::parse("application/octet-stream").expect("valid fallback MIME type")
    }));
    Ok(())
}

fn decode(xhr: &XhrRecord) -> (String, &'static str) {
    let response = response_mime(xhr);
    let mime = xhr.override_mime.as_ref().unwrap_or(&response);
    // An absent override charset inherits the response charset. A present but
    // invalid override is still selected, so it never revives the server label.
    let label = mime
        .parameter("charset")
        .or_else(|| response.parameter("charset"));
    let encoding = label.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()));
    let essence = mime.essence();
    let xml = matches!(essence, "application/xml" | "text/xml") || essence.ends_with("+xml");
    let kind = if xml && matches!(xhr.response_type.as_str(), "" | "document") {
        oryn_net::TextResourceKind::Xml
    } else if essence == "text/html" && xhr.response_type == "document" {
        oryn_net::TextResourceKind::Html
    } else {
        oryn_net::TextResourceKind::Fetch
    };
    // Feed only the selected, recognized charset to the shared sniffing path.
    // The shared decoder handles BOM precedence and XML/HTML declarations; XHR
    // retains UTF-8 fallback rather than a navigation document's legacy fallback.
    let header = encoding.map(|encoding| format!("{};charset={}", essence, encoding.name()));
    oryn_net::decode_resource_text_with_fallback(
        &xhr.response_bytes,
        header.as_deref(),
        kind,
        Some("UTF-8"),
    )
}

pub(super) fn text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: &XhrRecord,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if !matches!(xhr.response_type.as_str(), "" | "text") {
        return Err(invalid_state());
    }
    if xhr.ready_state < 3 {
        return v8_scalar_string(scope, "");
    }
    v8_scalar_string(scope, &decode(xhr).0)
}

pub(super) fn response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    xml_only: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (kind, bytes, mime, url, text, encoding, language) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::XMLHttpRequest(xhr) = &*record else {
            return Err(V8BindingError::Allocation("XHR record"));
        };
        if xml_only && !matches!(xhr.response_type.as_str(), "" | "document") {
            return Err(invalid_state());
        }
        if !xml_only && matches!(xhr.response_type.as_str(), "" | "text") {
            return self::text(scope, xhr);
        }
        if xhr.ready_state != 4 || xhr.status == 0 {
            return Ok(v8::null(scope).into());
        }
        if let Some(value) = &xhr.response_object {
            return Ok(value.value(scope));
        }
        let mime = xhr
            .override_mime
            .clone()
            .unwrap_or_else(|| response_mime(xhr));
        let (text, encoding) = decode(xhr);
        (
            xhr.response_type.clone(),
            xhr.response_bytes.clone(),
            mime,
            xhr.response_url.clone(),
            text,
            encoding,
            oryn_dom::language::content_language_header(
                xhr.response_headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-language"))
                    .map(|(_, value)| value.as_str()),
            ),
        )
    };
    let value = if xml_only || kind == "document" {
        let essence = mime.essence();
        let html = essence == "text/html";
        let xml = matches!(essence, "text/xml" | "application/xml") || essence.ends_with("+xml");
        if (!html && !xml) || (html && kind.is_empty()) {
            return Ok(v8::null(scope).into());
        }
        let mut parsed = if html {
            oryn_dom::html::parse_inert(&text)
        } else {
            oryn_dom::xml::parse(&text, essence)
        };
        if !html && !parsed.errors.is_empty() {
            return Ok(v8::null(scope).into());
        }
        if let Some(metadata) = parsed
            .dom
            .get_mut(parsed.document)?
            .document_metadata
            .as_mut()
        {
            metadata.url = Some(url);
            metadata.content_language = language;
            metadata.encoding = encoding.into();
            metadata.content_type = essence.into();
        }
        let document = state
            .dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)?;
        wrap_node(scope, state, document)?.into()
    } else {
        xhr_response_value(scope, state, &kind, &bytes, &mime.serialize())?
    };
    let mut record = platform_object_mut(state, id)?;
    if let PlatformObjectRecord::XMLHttpRequest(xhr) = &mut *record {
        xhr.response_object = Some(platform_listener_graph::Value::new(scope, value));
    }
    drop(record);
    platform_listener_graph::refresh(scope, state, id)?;
    Ok(value)
}

#[cfg(test)]
mod decoding_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn configure(
        state: &V8BindingState,
        kind: &str,
        mime: &str,
        override_type: Option<&str>,
        bytes: Vec<u8>,
    ) {
        let mut records = state.platform_objects.borrow_mut();
        let xhr = records
            .values_mut()
            .find_map(|record| match record {
                PlatformObjectRecord::XMLHttpRequest(xhr) => Some(xhr),
                _ => None,
            })
            .unwrap();
        xhr.ready_state = 4;
        xhr.status = 200;
        xhr.response_type = kind.into();
        xhr.response_headers = if mime.is_empty() {
            Vec::new()
        } else {
            vec![("content-type".into(), mime.into())]
        };
        xhr.override_mime = override_type.and_then(MimeType::parse);
        xhr.response_bytes = bytes;
        xhr.response_object = None;
        xhr.response_url = "https://example.test/data".into();
    }
    #[test]
    fn native_xhr_dispatch_runs_ordered_handlers_and_once_listeners_once() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(95), 1).unwrap();
        eval(
            scope,
            "globalThis.xhr=new XMLHttpRequest();globalThis.order=[];xhr.addEventListener('readystatechange',()=>order.push('before'));xhr.onreadystatechange=()=>order.push('handler');xhr.addEventListener('readystatechange',()=>order.push('once'),{once:true});xhr.addEventListener('readystatechange',()=>order.push('after'));xhr.ontimeout=()=>order.push('timeout')",
        );
        eval(
            scope,
            "xhr.addEventListener('abort',e=>{order.push('abort-listener');globalThis.abortEvent=e;globalThis.abortDuring=[e instanceof ProgressEvent,e.isTrusted,e.target===xhr,e.currentTarget===xhr,e.bubbles,e.cancelable,e.eventPhase].join('|')});xhr.onabort=()=>order.push('abort-handler')",
        );
        let id = state
            .platform_objects
            .borrow()
            .iter()
            .find_map(|(id, record)| {
                matches!(record, PlatformObjectRecord::XMLHttpRequest(_)).then_some(*id)
            })
            .unwrap();
        dispatch_platform_event(scope, &state, id, "readystatechange", None).unwrap();
        dispatch_platform_event(scope, &state, id, "readystatechange", None).unwrap();
        dispatch_platform_event(scope, &state, id, "timeout", None).unwrap();
        dispatch_platform_event(scope, &state, id, "abort", None).unwrap();
        assert_eq!(
            eval(scope, "abortDuring").to_rust_string_lossy(scope),
            "true|true|true|true|false|false|2"
        );
        assert!(
            eval(
                scope,
                "abortEvent.target===xhr && abortEvent.currentTarget===null"
            )
            .boolean_value(scope)
        );
        assert_eq!(
            eval(scope, "order.join(',')").to_rust_string_lossy(scope),
            "before,handler,once,after,before,handler,after,timeout,abort-listener,abort-handler"
        );
    }
    #[test]
    fn pinned_default_text_and_document_encoding_matrix() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(93), 1).unwrap();
        eval(scope, "globalThis.xhr=new XMLHttpRequest()");
        for kind in ["", "text", "document"] {
            for (mime, prefix) in [
                (
                    "application/xml",
                    "<?xml version='1.0' encoding='windows-1252'?><x>",
                ),
                ("text/html", "<!doctype html><meta charset=windows-1252><x>"),
            ] {
                for charset in ["", ";charset=utf-8", ";charset=windows-1252"] {
                    let body = format!("{prefix}機</x>");
                    let windows = charset.ends_with("windows-1252")
                        || (charset.is_empty()
                            && (kind == "document"
                                || (kind.is_empty() && mime == "application/xml")));
                    let payload = if windows { "æ©Ÿ" } else { "機" };
                    configure(
                        &state,
                        kind,
                        &format!("{mime}{charset}"),
                        None,
                        body.into_bytes(),
                    );
                    if kind == "document" {
                        assert_eq!(
                            eval(scope, "xhr.responseXML.querySelector('x').textContent")
                                .to_rust_string_lossy(scope),
                            payload
                        );
                        assert!(
                            eval(
                                scope,
                                "xhr.response===xhr.responseXML && xhr.response===xhr.response"
                            )
                            .boolean_value(scope)
                        );
                        assert_eq!(
                            eval(scope, "xhr.response.characterSet").to_rust_string_lossy(scope),
                            if windows { "windows-1252" } else { "UTF-8" }
                        );
                    } else {
                        assert_eq!(
                            eval(scope, "xhr.responseText").to_rust_string_lossy(scope),
                            format!("{prefix}{payload}</x>")
                        );
                        assert!(
                            eval(scope, "xhr.response===xhr.responseText").boolean_value(scope)
                        );
                        if mime == "text/html" && kind.is_empty() {
                            assert!(eval(scope, "xhr.responseXML===null").boolean_value(scope));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn override_charset_bom_final_mime_and_utf8_fallback_are_distinct() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(94), 1).unwrap();
        eval(scope, "globalThis.xhr=new XMLHttpRequest()");
        for (override_type, expected) in [
            ("text/xml", "Âð"),
            ("text/plain;charset=342", "��"),
            ("text/plain;charset=UTF-8", "��"),
        ] {
            configure(
                &state,
                "",
                "text/plain;charset=windows-1252",
                Some(override_type),
                vec![0xc2, 0xf0],
            );
            assert_eq!(
                eval(scope, "xhr.responseText").to_rust_string_lossy(scope),
                expected
            );
        }
        configure(
            &state,
            "text",
            "text/plain;charset=windows-1252",
            Some("text/plain;charset=342"),
            vec![0xff, 0xfe, 0xe9, 0],
        );
        assert_eq!(
            eval(scope, "xhr.responseText").to_rust_string_lossy(scope),
            "é"
        );
        for (kind, mime, override_type, expected) in [
            ("", "text/plain", Some("application/example+xml"), "æ©Ÿ"),
            ("text", "application/xml", None, "機"),
            ("", "", None, "æ©Ÿ"),
            ("", "application/xml", Some("text/plain"), "機"),
        ] {
            let prefix = "<?xml version='1.0' encoding='windows-1252'?><x>";
            configure(
                &state,
                kind,
                mime,
                override_type,
                format!("{prefix}機</x>").into_bytes(),
            );
            assert_eq!(
                eval(scope, "xhr.responseText").to_rust_string_lossy(scope),
                format!("{prefix}{expected}</x>")
            );
        }
        configure(
            &state,
            "document",
            "text/html",
            None,
            b"<x>\xc3\xa9</x>".to_vec(),
        );
        assert_eq!(
            eval(scope, "xhr.response.querySelector('x').textContent").to_rust_string_lossy(scope),
            "é"
        );
        assert_eq!(
            eval(scope, "xhr.response.characterSet").to_rust_string_lossy(scope),
            "UTF-8"
        );
        configure(
            &state,
            "text",
            "text/plain",
            None,
            vec![0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf],
        );
        assert_eq!(
            eval(scope, "xhr.responseText").to_rust_string_lossy(scope),
            "\u{feff}"
        );
    }
}

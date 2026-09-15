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
    xhr.response_headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| MimeType::parse(value))
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
    let label = xhr
        .override_mime
        .as_ref()
        .and_then(|mime| mime.parameter("charset"))
        .or_else(|| response.parameter("charset"));
    let encoding = label
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, encoding, _) = encoding.decode(&xhr.response_bytes);
    (text.into_owned(), encoding.name())
}

pub(super) fn text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: &XhrRecord,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if !matches!(xhr.response_type.as_str(), "" | "text") {
        return Err(invalid_state());
    }
    if xhr.ready_state < 3 {
        return v8_string(scope, "");
    }
    v8_string(scope, &decode(xhr).0)
}

pub(super) fn response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    xml_only: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let (kind, bytes, mime, url, text, encoding) = {
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
            return Ok(v8::Local::new(scope, value));
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
        xhr.response_object = Some(v8::Global::new(scope, value));
    }
    Ok(value)
}

//! XSLTProcessor keeps imported nodes/parameters in traced V8 fields, never raw DOM pointers.
use super::*;

#[derive(Default)]
struct Resources(Vec<(String, String, String)>);
fn native_transform(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    style: &str,
    source: Option<&str>,
    parameters: &[(String, String, String)],
    url: &str,
    resources: Resources,
) -> Result<crate::platform::XsltResult, V8BindingError> {
    let request = crate::platform::XsltRequest {
        style: style.into(),
        source: source.map(str::to_owned),
        parameters: parameters.to_vec(),
        url: url.into(),
        resources: resources.0,
    };
    let reply = xhr_sync::request(
        scope,
        state,
        PlatformHostRequest::XsltTransform { request },
        Some(std::time::Instant::now() + std::time::Duration::from_secs(12)),
        |value| matches!(value, PlatformHostResult::XsltTransformed { .. }),
    )?;
    let PlatformHostResult::XsltTransformed { transform: result } = reply else {
        return Err(failure("invalid helper response"));
    };
    Ok(result)
}
fn failure(message: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "XSLTProcessor".into(),
        member: message.into(),
    }
}
fn own_index(
    scope: &mut v8::PinScope,
    array: v8::Local<v8::Array>,
    index: u32,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, &index.to_string())
        .ok_or(V8BindingError::Allocation("XSLT private index"))?;
    if array.create_data_property(scope, key.into(), value) != Some(true) {
        return Err(V8BindingError::Allocation("XSLT private value"));
    }
    Ok(())
}
fn storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let brand = object.internal_field_count() >= 6
        && object
            .get_internal_field(scope, 5)
            .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
            .is_some_and(|value| {
                value.is_string() && value.to_rust_string_lossy(scope) == "XSLTProcessor"
            });
    if !brand {
        return Err(failure("TypeError: incompatible receiver"));
    }
    if let Some(value) = object
        .get_internal_field(scope, 4)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.try_cast::<v8::Array>().ok())
    {
        return Ok(value);
    }
    let data = v8::Array::new(scope, 2);
    let empty = v8::null(scope);
    own_index(scope, data, 0, empty.into())?;
    let params = v8::Array::new(scope, 0);
    own_index(scope, data, 1, params.into())?;
    object.set_internal_field(4, data.into());
    Ok(data)
}
fn parameters(
    scope: &mut v8::PinScope,
    data: v8::Local<v8::Array>,
) -> Result<Vec<(String, String, String)>, V8BindingError> {
    let entries = data
        .get_index(scope, 1)
        .and_then(|v| v.try_cast::<v8::Array>().ok())
        .ok_or_else(|| failure("parameter storage"))?;
    let mut values = Vec::new();
    for index in 0..entries.length() {
        let entry = entries
            .get_index(scope, index)
            .and_then(|v| v.try_cast::<v8::Array>().ok())
            .ok_or_else(|| failure("parameter entry"))?;
        let mut text = |index| {
            let value = entry
                .get_index(scope, index)
                .and_then(|value| value.try_cast::<v8::String>().ok())
                .ok_or_else(|| failure("parameter value"))?;
            rust_string_preserving_utf16(scope, value)
        };
        values.push((text(0)?, text(1)?, text(2)?));
    }
    Ok(values)
}
fn store_parameters(
    scope: &mut v8::PinScope,
    data: v8::Local<v8::Array>,
    values: &[(String, String, String)],
) -> Result<(), V8BindingError> {
    let array = v8::Array::new(scope, values.len() as i32);
    for (index, (ns, name, value)) in values.iter().enumerate() {
        let entry = v8::Array::new(scope, 3);
        for (slot, text) in [ns, name, value].into_iter().enumerate() {
            let text = v8_string(scope, text)?;
            own_index(scope, entry, slot as u32, text)?;
        }
        own_index(scope, array, index as u32, entry.into())?;
    }
    own_index(scope, data, 1, array.into())?;
    Ok(())
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let data = storage(scope, object)?;
    match member {
        "reset" => {
            let null = v8::null(scope);
            own_index(scope, data, 0, null.into())?;
            store_parameters(scope, data, &[])?;
            return Ok(None);
        }
        "clearParameters" => {
            store_parameters(scope, data, &[])?;
            return Ok(None);
        }
        "setParameter" | "getParameter" | "removeParameter" => {
            if args.length() < if member == "setParameter" { 3 } else { 2 } {
                return Err(failure("TypeError: missing parameter arguments"));
            }
            let ns = if args.get(0).is_null_or_undefined() {
                String::new()
            } else {
                required_string(scope, args, 0)?
            };
            let name = required_string(scope, args, 1)?;
            let value = if member == "setParameter" {
                Some(required_string(scope, args, 2)?)
            } else {
                None
            };
            // Conversions may call authored code, so read storage after conversion.
            let mut values = parameters(scope, data)?;
            let index = values.iter().position(|(n, l, _)| n == &ns && l == &name);
            if member == "getParameter" {
                return Ok(Some(match index {
                    Some(index) => v8_string(scope, &values[index].2)?,
                    None => v8::null(scope).into(),
                }));
            }
            if let Some(index) = index {
                values.remove(index);
            }
            if let Some(value) = value {
                if values.len() >= 1024 {
                    return Err(failure("parameter limit exceeded"));
                }
                values.push((ns, name, value));
            }
            store_parameters(scope, data, &values)?;
            return Ok(None);
        }
        "importStylesheet" => {
            let (owner, node) = node_identity::argument(scope, args.get(0))?
                .ok_or_else(|| failure("TypeError: stylesheet must be a Node"))?;
            let stylesheet = template_links::serialize_xml(scope, &owner, node)?;
            let base = stylesheet_base(&owner, node)?;
            let resources = dependencies(scope, state, &stylesheet, &base)?;
            if !native_transform(scope, state, &stylesheet, None, &[], &base, resources)?.valid {
                return Err(failure(
                    "InvalidStateError: invalid or externally dependent stylesheet",
                ));
            }
            // Retain the stylesheet DOM strongly and serialize afresh for each transform.
            own_index(scope, data, 0, args.get(0))?;
            return Ok(None);
        }
        "transformToFragment" | "transformToDocument" => {}
        _ => return Err(failure("unknown operation")),
    }
    let stylesheet_node = data
        .get_index(scope, 0)
        .ok_or_else(|| failure("stylesheet storage"))?;
    if stylesheet_node.is_null() {
        return Ok(Some(v8::null(scope).into()));
    }
    let (stylesheet_owner, stylesheet_node) = node_identity::argument(scope, stylesheet_node)?
        .ok_or_else(|| failure("retired stylesheet"))?;
    let (source_owner, source_node) = node_identity::argument(scope, args.get(0))?
        .ok_or_else(|| failure("TypeError: source must be a Node"))?;
    let destination = if member == "transformToFragment" {
        let (owner, node) = node_identity::argument(scope, args.get(1))?
            .ok_or_else(|| failure("TypeError: output must be a Document"))?;
        if !matches!(
            owner.dom.borrow().arena().get(node)?.kind,
            NodeKind::Document
        ) {
            return Err(failure("TypeError: output must be a Document"));
        }
        Some((owner, node))
    } else {
        None
    };
    let stylesheet = template_links::serialize_xml(scope, &stylesheet_owner, stylesheet_node)?;
    let source = template_links::serialize_xml(scope, &source_owner, source_node)?;
    let params = parameters(scope, data)?;
    let base = stylesheet_base(&stylesheet_owner, stylesheet_node)?;
    let resources = dependencies(scope, state, &stylesheet, &base)?;
    let Some((output, kind)) = native_transform(
        scope,
        state,
        &stylesheet,
        Some(&source),
        &params,
        &base,
        resources,
    )?
    .output
    else {
        return Ok(Some(v8::null(scope).into()));
    };
    if let Some((owner, document)) = destination {
        let fragment = {
            let mut dom = owner.dom.borrow_mut();
            let fragment = dom.create_fragment();
            dom.set_node_document(fragment, document)?;
            if kind == 2 {
                if !output.is_empty() {
                    let text = dom.create_text(output);
                    dom.append_child(fragment, text)?;
                }
            } else if kind == 1 {
                dom.set_inner_html_with_context(fragment, &output, "body")?;
            } else {
                // XML fragment results may contain multiple roots or only text.
                let output = output.trim_start();
                let output = if output.starts_with("<?xml") {
                    output.split_once("?>").map_or(output, |(_, body)| body)
                } else {
                    output
                };
                let parsed = oryn_dom::xml::parse(
                    &format!("<oryn-xslt-fragment>{output}</oryn-xslt-fragment>"),
                    "application/xml",
                );
                if !parsed.errors.is_empty() {
                    return Err(failure("invalid XML output"));
                }
                let root = parsed
                    .dom
                    .get(parsed.document)?
                    .children
                    .iter()
                    .find(|id| {
                        parsed
                            .dom
                            .get(**id)
                            .is_ok_and(|n| matches!(n.kind, NodeKind::Element { .. }))
                    })
                    .copied()
                    .ok_or_else(|| failure("invalid XML output"))?;
                for child in parsed.dom.get(root)?.children.clone() {
                    let child = dom.import_subtree(&parsed.dom, child)?;
                    dom.append_child(fragment, child)?;
                }
            }
            dom.set_node_document(fragment, document)?;
            for node in dom.arena().descendants_inclusive(fragment)? {
                let record = dom.arena_mut().get_mut(node)?;
                // XSLT-created script elements are executable when subsequently connected.
                // The ordinary insertion pipeline retains CSP/resource/owner checks.
                record.script_already_started = false;
                record.script_parser_document = None;
                record.parser_open_document = None;
            }
            fragment
        };
        custom_elements::initialize_fragment_registry(&owner, fragment, document)?;
        return Ok(Some(wrap_node(scope, &owner, fragment)?.into()));
    }
    let mime = if kind == 1 {
        "text/html"
    } else if kind == 2 {
        "text/plain"
    } else {
        "application/xml"
    };
    let mut parsed = match kind {
        1 => oryn_dom::html::parse_inert_dom_string(&output),
        2 => oryn_dom::html::parse_text_document(&output, mime),
        _ => oryn_dom::xml::parse(&output, mime),
    };
    let source_url = {
        let dom = source_owner.dom.borrow();
        let document = dom
            .arena()
            .node_document(source_node)?
            .unwrap_or(source_owner.document_root());
        dom.arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|metadata| metadata.url.clone())
    };
    if let Some(metadata) = parsed
        .dom
        .get_mut(parsed.document)?
        .document_metadata
        .as_mut()
    {
        metadata.url = source_url;
    }
    let document = state
        .dom
        .borrow_mut()
        .import_subtree(&parsed.dom, parsed.document)?;
    state.derive_document_provenance(state.document_root(), document)?;
    Ok(Some(wrap_node(scope, state, document)?.into()))
}

fn stylesheet_base(owner: &V8BindingState, node: NodeId) -> Result<String, V8BindingError> {
    let dom = owner.dom.borrow();
    let document = dom
        .arena()
        .node_document(node)?
        .unwrap_or(owner.document_root());
    let mut base = Url::parse(
        &dom.arena()
            .get(document)?
            .document_metadata
            .as_ref()
            .and_then(|m| m.url.clone())
            .unwrap_or(owner.request_initiator_url()?),
    )
    .map_err(|_| failure("invalid stylesheet base"))?;
    // The serialized selected element keeps its own xml:base. Fold only its
    // omitted ancestors here, before libxslt resolves the selected root's base.
    let mut ancestors = Vec::new();
    let mut parent = dom.arena().get(node)?.parent;
    while let Some(node) = parent {
        ancestors.push(node);
        parent = dom.arena().get(node)?.parent;
    }
    for node in ancestors.into_iter().rev() {
        if matches!(dom.arena().get(node)?.kind, NodeKind::Element { .. })
            && let Some(value) =
                dom.attribute_ns(node, Some("http://www.w3.org/XML/1998/namespace"), "base")?
        {
            base = base
                .join(value)
                .map_err(|_| failure("invalid ancestor xml:base"))?;
        }
    }
    Ok(base.to_string())
}

fn load_stylesheet(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    url: &str,
    deadline: std::time::Instant,
) -> Result<(String, String), V8BindingError> {
    let request = crate::platform::PlatformHttpRequest {
        upgrade_mixed_image: false,
        fetch: Some(crate::platform::FetchOptions {
            keepalive: false,
            privileged_range: false,
            cache: Default::default(),
            mode: "same-origin".into(),
            redirect: "follow".into(),
            integrity: String::new(),
        }),
        use_cors_preflight: false,
        method: "GET".into(),
        url: url.into(),
        headers: vec![(
            "accept".into(),
            "application/xslt+xml,application/xml,text/xml".into(),
        )],
        body: Vec::new(),
        credentials: "same-origin".into(),
        initiator_url: Some(state.request_initiator_url()?),
        referrer_policy: Some(state.document_referrer_policy()),
    };
    let response = xhr_sync::fetch_request(
        scope,
        state,
        PlatformHostRequest::XsltStart { request },
        Some(deadline),
        4 * 1024 * 1024,
    )?;
    if !(200..300).contains(&response.status) {
        return Err(failure("stylesheet HTTP failure"));
    }
    let content_type = response
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.as_str());
    let (source, _) = oryn_net::decode_resource_text(
        &response.body,
        content_type,
        oryn_net::TextResourceKind::Xml,
    );
    Ok((response.url, source))
}
fn dependencies(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    source: &str,
    base: &str,
) -> Result<Resources, V8BindingError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut resources = Resources::default();
    let mut pending = vec![(source.to_string(), base.to_string())];
    let mut seen = std::collections::BTreeSet::from([base.to_string()]);
    let mut bytes = source.len();
    while let Some((source, base)) = pending.pop() {
        let parsed = oryn_dom::xml::parse(&source, "application/xml");
        if !parsed.errors.is_empty() {
            return Err(failure("invalid stylesheet XML"));
        }
        let dom = parsed.into_runtime()?;
        let mut nodes = vec![(
            dom.document(),
            Url::parse(&base).map_err(|_| failure("invalid stylesheet base URL"))?,
        )];
        while let Some((node, mut base)) = nodes.pop() {
            let record = dom.arena().get(node)?;
            if matches!(record.kind, NodeKind::Element { .. })
                && let Some(value) =
                    dom.attribute_ns(node, Some("http://www.w3.org/XML/1998/namespace"), "base")?
            {
                base = base.join(value).map_err(|_| failure("invalid xml:base"))?;
            }
            nodes.extend(record.children.iter().map(|node| (*node, base.clone())));
            if !matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/XSL/Transform" && matches!(name.local.as_ref(),"include"|"import"))
            {
                continue;
            }
            let href = dom
                .attribute_in_null_namespace(node, "href")?
                .ok_or_else(|| failure("stylesheet dependency has no href"))?;
            let url = base
                .join(href)
                .map_err(|_| failure("invalid stylesheet dependency URL"))?
                .to_string();
            if !seen.insert(url.clone()) {
                continue;
            }
            if seen.len() > 64 {
                return Err(failure("stylesheet dependency count exceeded"));
            }
            let (final_url, text) = load_stylesheet(scope, state, &url, deadline)?;
            bytes = bytes
                .checked_add(text.len())
                .filter(|bytes| *bytes <= 16 * 1024 * 1024)
                .ok_or_else(|| failure("stylesheet dependency bytes exceeded"))?;
            resources.0.push((url, final_url.clone(), text.clone()));
            pending.push((text, final_url));
        }
    }
    Ok(resources)
}

/// Called at native XML parser completion, before readiness. PI resources use
/// the same document-owned XSLT request path as processor dependencies.
pub fn transform_document(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<Option<(String, i32)>, V8BindingError> {
    let document = state.document_root();
    let sheet = oryn_dom::document_xslt_stylesheet(state.dom.borrow().arena(), document)?;
    let Some(sheet) = sheet else {
        return Ok(None);
    };
    let base = state.native_document_base_url(true)?;
    let url = Url::parse(&base)
        .and_then(|base| base.join(&sheet.href))
        .map_err(|_| failure("invalid XSLT PI URL"))?;
    let (base, stylesheet) = load_stylesheet(
        scope,
        state,
        url.as_str(),
        std::time::Instant::now() + std::time::Duration::from_secs(10),
    )?;
    let resources = dependencies(scope, state, &stylesheet, &base)?;
    let source = template_links::serialize_xml(scope, state, document)?;
    native_transform(
        scope,
        state,
        &stylesheet,
        Some(&source),
        &[],
        &base,
        resources,
    )?
    .output
    .map(Some)
    .ok_or_else(|| failure("XSLT PI transform failed"))
}

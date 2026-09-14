//! Shared native entry-list construction for FormData and form navigation.
use super::*;

/// Normalize string fields at serialization, not when constructing FormData.
pub(super) fn normalize_newlines(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

pub(super) fn escape_disposition(value: &str) -> String {
    value
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace('"', "%22")
}

pub(super) fn multipart_boundary() -> Result<String, V8BindingError> {
    let mut random = [0u8; 18];
    getrandom::fill(&mut random)
        .map_err(|_| error("OperationError: multipart entropy unavailable"))?;
    Ok(format!(
        "----oryn-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
    ))
}

fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, name)
        .ok_or(V8BindingError::Allocation("FormDataEvent private key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "FormData".into(),
        member: name.into(),
    }
}

pub(super) fn validate(
    state: &V8BindingState,
    form: Option<NodeId>,
    submitter: Option<NodeId>,
) -> Result<(), V8BindingError> {
    let Some(form) = form else { return Ok(()) };
    let dom = state.dom.borrow();
    if !dom.is_element(form, "http://www.w3.org/1999/xhtml", "form") {
        return Err(error("TypeError: expected form"));
    }
    if let Some(submitter) = submitter {
        if !validation::is_submit_button(&dom, submitter)? {
            return Err(error("TypeError: expected submit button"));
        }
        if dom.form_owner(submitter)? != Some(form) {
            return Err(error("NotFoundError"));
        }
    }
    Ok(())
}

struct Construction<'a> {
    state: &'a V8BindingState,
    form: NodeId,
    _lease: submissions::RootedTarget,
}
impl Drop for Construction<'_> {
    fn drop(&mut self) {
        self.state
            .entry_lists_in_progress
            .borrow_mut()
            .remove(&self.form);
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    form: Option<NodeId>,
    submitter: Option<NodeId>,
    object: Option<v8::Local<'s, v8::Object>>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let _guard = if let Some(form) = form {
        let lease = submissions::RootedTarget::new(state, form)?;
        if !state.entry_lists_in_progress.borrow_mut().insert(form) {
            return Err(error("InvalidStateError"));
        }
        Some(Construction {
            state,
            form,
            _lease: lease,
        })
    } else {
        None
    };
    let entries = match form {
        Some(form) => construct_entry_list(scope, state, form, submitter)?,
        None => Vec::new(),
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::FormData(FormDataRecord { entries }),
    );
    let object = match object {
        Some(object) => {
            bind_platform_object(scope, state, id, object);
            object
        }
        None => new_platform_object(scope, state, "FormData", id)?,
    };
    if let Some(form) = form {
        let event = create_event_object_with_interface(
            scope,
            state,
            "FormDataEvent",
            "formdata",
            true,
            false,
        )?;
        set_event_data(scope, event, object.into())?;
        let id = event_id(scope, event)?;
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
        dispatch_event(scope, state, form, event)?;
    }
    Ok(object)
}

type Entries = Vec<(String, v8::Global<v8::Value>)>;

pub(super) fn name_value_pairs(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<Vec<(String, String)>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let entries = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::FormData(form) = &*record else {
            return Err(error("TypeError: expected FormData"));
        };
        form.entries.clone()
    };
    entries
        .into_iter()
        .map(|(name, value)| {
            let value = v8::Local::new(scope, value);
            let text = if value.is_string() {
                let value = v8::Local::<v8::String>::try_from(value).unwrap();
                oryn_dom::dom_string::usv(&rust_string_preserving_utf16(scope, value)?)
            } else {
                let object = v8::Local::<v8::Object>::try_from(value)
                    .map_err(|_| error("TypeError: expected File"))?;
                let owner = binding_state_for_receiver(scope, object)
                    .ok_or_else(|| error("TypeError: expected File"))?;
                let id = stateful_id(scope, object)?;
                let record = platform_object(&owner, id)?;
                let PlatformObjectRecord::Blob(blob) = &*record else {
                    return Err(error("TypeError: expected File"));
                };
                blob.file
                    .as_ref()
                    .ok_or_else(|| error("TypeError: expected File"))?
                    .name
                    .clone()
            };
            Ok((normalize_newlines(&name), normalize_newlines(&text)))
        })
        .collect()
}
fn text_entry(
    scope: &mut v8::PinScope,
    entries: &mut Entries,
    name: String,
    value: &str,
) -> Result<(), V8BindingError> {
    let value = v8_string(scope, &oryn_dom::dom_string::usv(value))?;
    entries.push((
        oryn_dom::dom_string::usv(&name),
        v8::Global::new(scope, value),
    ));
    Ok(())
}

fn construct_entry_list(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<Entries, V8BindingError> {
    let controls = state.dom.borrow().submittable_form_controls(form)?;
    let mut entries = Vec::new();
    for control in controls {
        let dom = state.dom.borrow();
        if !["button", "input", "textarea", "select"]
            .iter()
            .any(|tag| dom.is_element(control, "http://www.w3.org/1999/xhtml", tag))
            || dom.control_disabled(control)?
            || dom
                .arena()
                .ancestors(control)?
                .into_iter()
                .any(|node| dom.is_element(node, "http://www.w3.org/1999/xhtml", "datalist"))
        {
            continue;
        }
        let input = dom.is_element(control, "http://www.w3.org/1999/xhtml", "input");
        let kind = dom
            .attribute_in_null_namespace(control, "type")?
            .unwrap_or("")
            .to_ascii_lowercase();
        let button = dom.is_element(control, "http://www.w3.org/1999/xhtml", "button")
            || (input && matches!(kind.as_str(), "submit" | "reset" | "button" | "image"));
        if button && submitter != Some(control) {
            continue;
        }
        let name = dom
            .attribute_in_null_namespace(control, "name")?
            .unwrap_or("")
            .to_owned();
        if input && kind == "image" {
            let (x, y) = dom.arena().get(control)?.image_coordinates;
            let prefix = if name.is_empty() {
                String::new()
            } else {
                format!("{name}.")
            };
            drop(dom);
            text_entry(scope, &mut entries, format!("{prefix}x"), &x.to_string())?;
            text_entry(scope, &mut entries, format!("{prefix}y"), &y.to_string())?;
            continue;
        }
        if name.is_empty() {
            continue;
        }
        if dom.is_element(control, "http://www.w3.org/1999/xhtml", "select") {
            let mut values = Vec::new();
            for option in dom.select_options(control)? {
                if dom.option_selected(option)? && !dom.option_disabled(option)? {
                    values.push(dom.option_value(option)?);
                }
            }
            drop(dom);
            for value in values {
                text_entry(scope, &mut entries, name.clone(), &value)?;
            }
            continue;
        }
        if input && matches!(kind.as_str(), "checkbox" | "radio") && !dom.checked(control)? {
            continue;
        }
        if input && kind == "file" {
            let selected = dom.arena().get(control)?.selected_files.clone();
            drop(dom);
            if !selected.is_empty() {
                let input = wrap_node(scope, state, control)?;
                let list = file_lists::input(scope, state, input, None)?;
                let list = v8::Local::<v8::Object>::try_from(list)
                    .map_err(|_| error("TypeError: expected FileList"))?;
                for value in file_lists::entries(scope, list)?
                    .ok_or_else(|| error("TypeError: expected FileList"))?
                {
                    entries.push((
                        oryn_dom::dom_string::usv(&name),
                        v8::Global::new(scope, value),
                    ));
                }
            } else {
                let file = BlobRecord {
                    bytes: Vec::new(),
                    media_type: "application/octet-stream".into(),
                    file: Some(FileRecord {
                        name: String::new(),
                        last_modified: 0,
                    }),
                };
                let id = register_platform_object(state, PlatformObjectRecord::Blob(file));
                let value = new_platform_object(scope, state, "File", id)?;
                entries.push((
                    oryn_dom::dom_string::usv(&name),
                    v8::Global::new(scope, v8::Local::<v8::Value>::from(value)),
                ));
            }
            continue;
        }
        let value = if input && kind == "hidden" && name.eq_ignore_ascii_case("_charset_") {
            "UTF-8".into()
        } else {
            dom.control_value(control)?
        };
        let dirname = if (input
            && !matches!(
                kind.as_str(),
                "date"
                    | "datetime-local"
                    | "month"
                    | "week"
                    | "time"
                    | "number"
                    | "range"
                    | "color"
                    | "checkbox"
                    | "radio"
                    | "file"
                    | "image"
            ))
            || dom.is_element(control, "http://www.w3.org/1999/xhtml", "textarea")
        {
            dom.attribute_in_null_namespace(control, "dirname")?
                .filter(|name| !name.is_empty())
                .map(|name| {
                    (
                        name.to_owned(),
                        oryn_dom::directionality::directionality(dom.arena(), control),
                    )
                })
        } else {
            None
        };
        drop(dom);
        text_entry(scope, &mut entries, name, &value)?;
        if let Some((name, direction)) = dirname {
            text_entry(scope, &mut entries, name, direction)?;
        }
    }
    Ok(entries)
}

pub(super) fn validate_event_data(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| error("TypeError: expected FormData"))?;
    if object.is_proxy() {
        return Err(error("TypeError: expected FormData"));
    }
    let owner = binding_state_for_receiver(scope, object)
        .ok_or_else(|| error("TypeError: expected FormData"))?;
    let id = stateful_id(scope, object)?;
    if !matches!(
        &*platform_object(&owner, id)?,
        PlatformObjectRecord::FormData(_)
    ) {
        return Err(error("TypeError: expected FormData"));
    }
    Ok(())
}

pub(super) fn set_event_data(
    scope: &mut v8::PinScope,
    event: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = key(scope, "oryn.formdata-event.data")?;
    event
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}

pub(super) fn event_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = key(scope, "oryn.formdata-event.data")?;
    if event.is_proxy() || !event.has_private(scope, key).unwrap_or(false) {
        return Err(error("TypeError: invalid FormDataEvent receiver"));
    }
    event
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}

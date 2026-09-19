//! Shared native entry-list construction for FormData and form navigation.
use super::*;

/// Normalize string fields at serialization, not when constructing FormData.
pub(super) fn normalize_newlines(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

pub(super) fn encoded_disposition(
    value: &str,
    encoding: &'static encoding_rs::Encoding,
) -> Vec<u8> {
    let encoded = encoding.encode(value).0;
    let mut output = Vec::with_capacity(encoded.len());
    for byte in encoded.iter().copied() {
        match byte {
            b'\r' => output.extend_from_slice(b"%0D"),
            b'\n' => output.extend_from_slice(b"%0A"),
            b'"' => output.extend_from_slice(b"%22"),
            _ => output.push(byte),
        }
    }
    output
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

/// The constructor's realm supplies the result prototype, while the form's
/// actual native owner supplies controls, custom internals and event delivery.
pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    let form = if args.get(0).is_undefined() {
        None
    } else {
        Some(
            node_identity::argument(scope, args.get(0))?
                .ok_or_else(|| error("TypeError: expected form"))?,
        )
    };
    // Complete interface conversion before checking submit-button/form ownership.
    let submitter = if args.get(1).is_null_or_undefined() {
        None
    } else {
        validation::convert_submitter(scope, args.get(1))?;
        node_identity::argument(scope, args.get(1))?
    };
    let owner = form.as_ref().map(|(owner, _)| owner).unwrap_or(state);
    let form_node = form.as_ref().map(|(_, node)| *node);
    let submitter_node = if let Some((submitter_owner, node)) = submitter {
        if form_node.is_some() {
            if !validation::is_submit_button(&submitter_owner.dom.borrow(), node)? {
                return Err(error("TypeError: expected submit button"));
            }
            if !Rc::ptr_eq(&submitter_owner.dom, &owner.dom) {
                return Err(error("NotFoundError"));
            }
            Some(node)
        } else {
            None
        }
    } else {
        None
    };
    validate(owner, form_node, submitter_node)?;
    create(
        scope,
        owner,
        form_node,
        submitter_node,
        Some(args.this()),
        encoding_rs::UTF_8,
    )?;
    Ok(())
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

pub(super) struct Creation {
    object: v8::Global<v8::Object>,
    event: Option<retained_dom_event::Event>,
    guard: Option<submissions::FormOperationGuard>,
    done: bool,
}
impl Creation {
    pub(super) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        state: &Rc<V8BindingState>,
        form: Option<NodeId>,
        submitter: Option<NodeId>,
        object: Option<v8::Local<'s, v8::Object>>,
        encoding: &'static encoding_rs::Encoding,
    ) -> Result<Self, V8BindingError> {
        let guard = if let Some(form) = form {
            Some(
                submissions::FormOperationGuard::begin(
                    scope,
                    state,
                    form,
                    submissions::FormOperation::EntryList,
                )?
                .ok_or_else(|| error("InvalidStateError"))?,
            )
        } else {
            None
        };
        let entries = match form {
            Some(form) => construct_entry_list(scope, state, form, submitter, encoding)?,
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
        let event = if let Some(form) = form {
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
            state
                .events
                .borrow_mut()
                .get_mut(&id)
                .ok_or(V8BindingError::Allocation("formdata event"))?
                .is_trusted = true;
            Some(retained_dom_event::Event::new(
                scope, state, form, "formdata", event,
            )?)
        } else {
            None
        };
        Ok(Self {
            object: v8::Global::new(scope, object),
            event,
            guard,
            done: false,
        })
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
    ) -> Result<Option<v8::Global<v8::Object>>, V8BindingError> {
        if !self.done {
            if let Some(event) = self.event.as_mut() {
                let owner = event.owner();
                if event.poll(scope, &owner)?.is_none() {
                    return Ok(None);
                }
            }
            self.event = None;
            self.guard = None;
            self.done = true;
        }
        Ok(Some(self.object.clone()))
    }
}
/// Script FormData construction remains synchronous; native submission keeps
/// Creation itself and resumes its callback suffix between actor polls.
pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    form: Option<NodeId>,
    submitter: Option<NodeId>,
    object: Option<v8::Local<'s, v8::Object>>,
    encoding: &'static encoding_rs::Encoding,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let mut creation = Creation::new(scope, state, form, submitter, object, encoding)?;
    let object = creation
        .poll(scope)?
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(v8::Local::new(scope, &object))
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
            // Entries retain collision-safe UTF-16 storage internally. Decode
            // once at the serialization boundary, as multipart already does.
            let name = String::from_utf16_lossy(&oryn_dom::dom_string::units(&name));
            let text = String::from_utf16_lossy(&oryn_dom::dom_string::units(&text));
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
    encoding: &'static encoding_rs::Encoding,
) -> Result<Entries, V8BindingError> {
    let controls = state.dom.borrow().submittable_form_controls(form)?;
    let mut entries = Vec::new();
    for control in controls {
        if state
            .dom
            .borrow()
            .is_form_associated_custom_element(control)
        {
            let behavior = state
                .dom
                .borrow()
                .arena()
                .get(control)?
                .submit_behavior
                .clone();
            if let Some(behavior) = behavior {
                let eligible = {
                    let dom = state.dom.borrow();
                    submitter == Some(control)
                        && !dom.control_disabled(control)?
                        && !dom.has_datalist_ancestor(control)?
                };
                if eligible && !behavior.name.is_empty() {
                    text_entry(scope, &mut entries, behavior.name, &behavior.value)?;
                }
                continue;
            }
            let eligible = {
                let dom = state.dom.borrow();
                !dom.control_disabled(control)? && !dom.has_datalist_ancestor(control)?
            };
            if eligible {
                entries.extend(custom_elements::submission_entries(scope, state, control)?);
            }
            continue;
        }
        let dom = state.dom.borrow();
        if !["button", "input", "textarea", "select"]
            .iter()
            .any(|tag| dom.is_element(control, "http://www.w3.org/1999/xhtml", tag))
            || dom.control_disabled(control)?
            || dom.has_datalist_ancestor(control)?
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
            encoding.name().into()
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

#[cfg(test)]
mod directionality_regressions {
    use super::*;
    #[test]
    fn serialized_pairs_decode_lossless_storage_once_for_names_values_and_files() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let script=v8::String::new(scope,r#"(()=>{
            const data=new FormData(),literal='\u{f0000}oryn-wtf16:[55296]';
            data.append(literal,literal+'\nend');
            data.append('surrogate\ud800','value\udc00');
            const file=new File(['body'],literal);if(file.name!==literal)throw Error('literal filename prefix collision');if(new File([], '\ud800').name!=='\ufffd')throw Error('filename USVString conversion');data.append('file',file);
            return data;
        })()"#).unwrap();
        let object = v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let object = v8::Local::<v8::Object>::try_from(object).unwrap();
        let literal = format!("{}[55296]", oryn_dom::dom_string::PREFIX);
        assert_eq!(
            name_value_pairs(scope, &state, object).unwrap(),
            vec![
                (literal.clone(), format!("{literal}\r\nend")),
                ("surrogate\u{fffd}".into(), "value\u{fffd}".into()),
                ("file".into(), literal),
            ]
        );
    }

    #[test]
    fn entry_directionality_matches_pinned_input_state_matrix() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f dir=rtl><textarea name=area dirname=area.dir></textarea></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        // Expected applicability follows pinned dirname-only-if-applies.html,
        // with unknown/empty Text states and inherited direction added.
        let script=v8::String::new(scope,r#"(()=>{
            const yes=['hidden','text','search','tel','url','email','password','submit'];
            const no=['date','month','week','time','datetime-local','number','range','color','checkbox','radio','file','image','reset','button'];
            const form=document.getElementById('f');let submitter;
            for(const type of [...yes,...no,'unknown','']){
                const input=document.createElement('input');input.setAttribute('type',type);
                input.name=type||'empty';input.setAttribute('dirname',input.name+'.dir');input.checked=true;
                form.appendChild(input);if(type==='submit')submitter=input;
            }
            const data=new FormData(form,submitter);
            for(const type of [...yes,'unknown','empty','area']){
                if(data.get(type+'.dir')!==(type==='tel'?'ltr':'rtl'))throw Error('missing direction: '+type);
            }
            for(const type of no){if(data.has(type+'.dir'))throw Error('unexpected direction: '+type);}
            const tel=form.querySelector('input[type=tel]');tel.dir='rtl';if(new FormData(form,submitter).get('tel.dir')!=='rtl')throw Error('explicit tel direction');tel.removeAttribute('dir');if(new FormData(form,submitter).get('tel.dir')!=='ltr')throw Error('missing tel direction overrides inherited RTL');
            return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod retained_creation_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn formdata_creation_keeps_mutated_entries_and_guard_through_callback_failure() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><input name=x value=one></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(228), 1).unwrap();
        eval(
            scope,
            "globalThis.log=[];f.addEventListener('formdata',e=>{globalThis.original=e;log.push('first');e.formData.append('y','two')},{once:true});f.addEventListener('formdata',e=>{log.push(e===original?'second':'wrong');e.formData.append('z','three')})",
        );
        let form = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "f")
            .unwrap()
            .unwrap();
        let mut creation =
            Creation::new(scope, &state, Some(form), None, None, encoding_rs::UTF_8).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(creation.poll(scope).is_err());
        assert!(submissions::FormOperation::EntryList.active(scope, &state, form));
        assert!(eval(scope, "log.join()==='first'").is_true());
        let object = creation.poll(scope).unwrap().unwrap();
        let object = v8::Local::new(scope, &object);
        assert_eq!(
            name_value_pairs(scope, &state, object).unwrap(),
            vec![
                ("x".into(), "one".into()),
                ("y".into(), "two".into()),
                ("z".into(), "three".into())
            ]
        );
        assert!(!submissions::FormOperation::EntryList.active(scope, &state, form));
        assert!(eval(scope, "log.join()==='first,second'").is_true());
    }
}

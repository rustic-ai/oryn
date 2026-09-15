//! Live, owner-traced validation state and constraint-validation algorithms.
use super::*;

pub(super) fn convert_submitter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if value.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    let invalid = || V8BindingError::Allocation("SubmitEvent submitter requires an HTMLElement");
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.is_proxy() {
        return Err(invalid());
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let node = argument_node(scope, &owner, value)?;
    if !matches!(&owner.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { name } if name.ns.as_ref() == "http://www.w3.org/1999/xhtml")
    {
        return Err(invalid());
    }
    Ok(value)
}

pub(super) fn set_submitter(
    scope: &mut v8::PinScope<'_, '_>,
    event: v8::Local<v8::Object>,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = key(scope, "oryn.submit-event.submitter")?;
    event
        .set_private(scope, key, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}

pub(super) fn submitter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Allocation("SubmitEvent receiver");
    if event.is_proxy() {
        return Err(invalid());
    }
    let key = key(scope, "oryn.submit-event.submitter")?;
    if !event.has_private(scope, key).unwrap_or(false) {
        return Err(invalid());
    }
    event.get_private(scope, key).ok_or_else(invalid)
}

pub(super) fn is_submit_button(dom: &DomRuntime, button: NodeId) -> Result<bool, DomError> {
    dom.is_submit_button(button)
}

pub(super) fn is_control_interface(interface: &str) -> bool {
    matches!(
        interface,
        "HTMLInputElement"
            | "HTMLTextAreaElement"
            | "HTMLSelectElement"
            | "HTMLButtonElement"
            | "HTMLFieldSetElement"
            | "HTMLOutputElement"
            | "HTMLObjectElement"
    )
}

pub(super) fn request_submit(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Result<bool, V8BindingError> {
    let skip_validation = {
        let dom = state.dom.borrow();
        if !dom.is_element(form, "http://www.w3.org/1999/xhtml", "form") {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLFormElement".into(),
                member: "TypeError: expected form".into(),
            });
        }
        if let Some(button) = submitter {
            let is_submit = is_submit_button(&dom, button)?;
            if !is_submit {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLFormElement".into(),
                    member: "requestSubmit requires a submit button".into(),
                });
            }
            if dom.form_owner(button)? != Some(form) {
                return Err(V8BindingError::Unsupported {
                    interface: "HTMLFormElement".into(),
                    member: "NotFoundError".into(),
                });
            }
        }
        if !state.forms_allowed.get()
            || state.entry_lists_in_progress.borrow().contains(&form)
            || !dom.is_connected(form)?
        {
            return Ok(false);
        }
        dom.attribute_in_null_namespace(form, "novalidate")?
            .is_some()
            || match submitter {
                Some(button) => dom
                    .attribute_in_null_namespace(button, "formnovalidate")?
                    .is_some(),
                None => false,
            }
    };
    if !state
        .submission_events_in_progress
        .borrow_mut()
        .insert(form)
    {
        return Ok(false);
    }
    let result = (|| -> Result<bool, V8BindingError> {
        if !skip_validation && !validate_form(scope, state, form, true)? {
            return Ok(false);
        }
        let event =
            create_event_object_with_interface(scope, state, "SubmitEvent", "submit", true, true)?;
        let value = match submitter {
            Some(node) => wrap_node(scope, state, node)?.into(),
            None => v8::null(scope).into(),
        };
        set_submitter(scope, event, value)?;
        let id = event_id(scope, event)?;
        state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
        let accepted = dispatch_event(scope, state, form, event)?;
        Ok(accepted && state.dom.borrow().is_connected(form)?)
    })();
    state
        .submission_events_in_progress
        .borrow_mut()
        .remove(&form);
    result
}

/// Snapshot invalid controls before any event callback can change ownership,
/// eligibility or validity. Canceling invalid events does not make the form valid.
pub(super) fn check_form(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    form: NodeId,
) -> Result<bool, V8BindingError> {
    validate_form(scope, state, form, false)
}

pub(super) fn validate_form(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    form: NodeId,
    report: bool,
) -> Result<bool, V8BindingError> {
    let invalid = state.dom.borrow().invalid_form_controls(form)?;
    let valid = invalid.is_empty();
    let mut unhandled = Vec::new();
    for node in invalid {
        if fire_invalid(scope, state, node)? {
            unhandled.push(node);
        }
    }
    if report {
        report_controls(scope, state, &unhandled)?;
    }
    Ok(valid)
}

pub(super) fn validate_control(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    report: bool,
) -> Result<bool, V8BindingError> {
    let invalid = {
        let dom = state.dom.borrow();
        dom.control_will_validate(node)? && !dom.control_validity(node)?
    };
    if invalid && fire_invalid(scope, state, node)? && report {
        report_controls(scope, state, &[node])?;
    }
    Ok(!invalid)
}

fn fire_invalid(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    let event = create_event_object(scope, state, "invalid", false, true)?;
    let id = event_id(scope, event)?;
    state.events.borrow_mut().get_mut(&id).unwrap().is_trusted = true;
    dispatch_event(scope, state, node, event)
}

fn report_controls(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    controls: &[NodeId],
) -> Result<(), V8BindingError> {
    // Headless reporting brings the first available unhandled control into
    // focus; its live validationMessage describes the problem. Do not select
    // a new invalid control introduced by a callback, or report canceled ones.
    for &node in controls {
        let available = {
            let dom = state.dom.borrow();
            dom.is_connected(node)?
                && dom.control_will_validate(node)?
                && !dom.control_validity(node)?
                && !dom.is_inert(node)?
        };
        if !available || css_suppresses_focus(state, node)? {
            continue;
        }
        focus_html_element(scope, state, node, false)?;
        // Focus handlers may intentionally move focus elsewhere. Do not
        // override that decision by attempting the next invalid control.
        break;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submit_event_traces_detached_submitter_without_rooting_unreachable_cycles() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let baseline = state.objects.borrow().len();
        {
            v8::scope!(let inner, scope);
            let source = v8::String::new(inner, r#"(()=>{
                for(let i=0;i<128;i++) {
                    const b=document.createElement('button');b.event=new SubmitEvent('submit',{submitter:b});
                }
                const b=document.createElement('button');b.textContent='retained';
                globalThis.savedSubmitEvent=new SubmitEvent('submit',{submitter:b});
            })()"#).unwrap();
            v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.objects.borrow().len(), baseline + 1);
        {
            v8::scope!(let inner, scope);
            let source =
                v8::String::new(inner, "savedSubmitEvent.submitter.textContent==='retained'")
                    .unwrap();
            assert!(
                v8::Script::compile(inner, source, None)
                    .unwrap()
                    .run(inner)
                    .unwrap()
                    .is_true()
            );
            let source = v8::String::new(inner, "delete globalThis.savedSubmitEvent").unwrap();
            v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.objects.borrow().len(), baseline);
    }

    #[test]
    fn validity_retains_detached_owner_and_unreachable_cycles_collect() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let baseline = state.objects.borrow().len();
        {
            v8::scope!(let inner, scope);
            let source = v8::String::new(inner, r#"(()=>{
                for(let i=0;i<128;i++) {
                    const s=document.createElement('select');s.savedValidity=s.validity;
                }
                const s=document.createElement('select');s.required=true;s.setCustomValidity('retained');
                globalThis.savedValidity=s.validity;
            })()"#).unwrap();
            v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.objects.borrow().len(), baseline + 1);
        {
            v8::scope!(let inner, scope);
            let source = v8::String::new(
                inner,
                "savedValidity.valueMissing&&savedValidity.customError&&!savedValidity.valid",
            )
            .unwrap();
            assert!(
                v8::Script::compile(inner, source, None)
                    .unwrap()
                    .run(inner)
                    .unwrap()
                    .is_true()
            );
            let source = v8::String::new(inner, "delete globalThis.savedValidity").unwrap();
            v8::Script::compile(inner, source, None)
                .unwrap()
                .run(inner)
                .unwrap();
        }
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        assert_eq!(state.objects.borrow().len(), baseline);
    }
}

fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("validation key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

pub(super) fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let owner = wrap_node(scope, state, node)?;
    let cache = key(scope, "oryn.validity")?;
    if let Some(value) = owner.get_private(scope, cache)
        && let Ok(value) = v8::Local::<v8::Object>::try_from(value)
    {
        return Ok(value);
    }
    let object = new_stateful_object(scope, state, "ValidityState", 0)?;
    let owner_key = key(scope, "oryn.associated.owner")?;
    object
        .set_private(scope, owner_key, owner.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    owner
        .set_private(scope, cache, object.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(object)
}

pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let invalid = || V8BindingError::Unsupported {
        interface: "ValidityState".into(),
        member: "incompatible receiver".into(),
    };
    if object.is_proxy() {
        return Err(invalid());
    }
    let owner_key = key(scope, "oryn.associated.owner")?;
    let owner = object
        .get_private(scope, owner_key)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(invalid)?;
    let cache = key(scope, "oryn.validity")?;
    if !owner
        .get_private(scope, cache)
        .is_some_and(|value| value.strict_equals(object.into()))
    {
        return Err(invalid());
    }
    let node = argument_node(scope, state, owner.into())?;
    let dom = state.dom.borrow();
    if ![
        "select", "input", "textarea", "button", "fieldset", "output", "object",
    ]
    .iter()
    .any(|tag| dom.is_element(node, "http://www.w3.org/1999/xhtml", tag))
    {
        return Err(invalid());
    }
    let value = match member {
        "valueMissing" => dom.control_value_missing(node)?,
        "customError" => !dom.custom_validity_message(node)?.is_empty(),
        "valid" => dom.control_validity(node)?,
        "typeMismatch" => dom.control_type_mismatch(node)?,
        "rangeUnderflow" => dom.control_numeric_validity(node)?.range_underflow,
        "rangeOverflow" => dom.control_numeric_validity(node)?.range_overflow,
        "stepMismatch" => dom.control_numeric_validity(node)?.step_mismatch,
        "patternMismatch" => dom.control_pattern_mismatch(node)?,
        // Remaining input constraint algorithms are not implemented yet.
        "tooLong" => dom.control_length_validity(node)?.1,
        "tooShort" => dom.control_length_validity(node)?.0,
        "badInput" => false,
        _ => return Err(invalid()),
    };
    Ok(v8::Boolean::new(scope, value).into())
}

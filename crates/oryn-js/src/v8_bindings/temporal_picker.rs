//! Trusted temporal picker lifecycle. Private paint candidates are never DOM
//! nodes and author-dispatched events cannot select or open a picker.
use super::*;
fn eligible(state: &V8BindingState, node: NodeId) -> Result<bool, V8BindingError> {
    let dom = state.dom.borrow();
    Ok(dom.has_native_input_picker(node)
        && dom.is_connected(node)?
        && !dom.control_disabled(node)?
        && !dom.is_inert(node)?
        && (!dom.is_editable_text_control(node)
            || dom.attribute_in_null_namespace(node, "readonly")?.is_none()))
}
pub(super) fn show(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let error = |name: &str| V8BindingError::Unsupported {
        interface: "HTMLInputElement".into(),
        member: name.into(),
    };
    let kind = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "type")?
        .unwrap_or("text")
        .to_ascii_lowercase();
    {
        let dom = state.dom.borrow();
        if dom.control_disabled(node)?
            || (dom.attribute_in_null_namespace(node, "readonly")?.is_some()
                && dom.is_editable_text_control(node))
        {
            return Err(error("InvalidStateError"));
        }
    }
    if !matches!(kind.as_str(), "file" | "color") {
        let window = state
            .global_object
            .to_local(scope)
            .ok_or_else(|| error("InvalidStateError"))?;
        let top = native_window_top(scope, window);
        let top_state =
            binding_state_for_receiver(scope, top).ok_or_else(|| error("SecurityError"))?;
        if !Rc::ptr_eq(state, &top_state) {
            let origin = current_platform_origin(state);
            if origin == "null" || origin != current_platform_origin(&top_state) {
                return Err(error("SecurityError"));
            }
        }
    }
    if !state.has_transient_user_activation() {
        return Err(error("NotAllowedError"));
    }
    if !state.consume_transient_user_activation() {
        return Err(error("NotAllowedError"));
    }
    if kind == "file" {
        file_picker::open(scope, state, node)?;
    } else if state.dom.borrow().has_native_input_picker(node) {
        open(scope, state, node)?;
    }
    Ok(())
}
fn open(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
) -> Result<(), V8BindingError> {
    native_platform::ensure_ui_locale(state);
    if !eligible(state, node)? || css_suppresses_focus_in_scope(scope, state, node)? {
        return Ok(());
    }
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis() as f64)
        .unwrap_or(0.);
    state.dom.borrow_mut().open_temporal_picker(node, today)?;
    state.layout_cache_key.borrow_mut().take();
    Ok(())
}
fn close(state: &V8BindingState, node: NodeId) {
    state.dom.borrow_mut().remove_from_top_layer(node);
    state.layout_cache_key.borrow_mut().take();
}
#[path = "temporal_picker_job.rs"]
mod job;
pub(super) use job::Job;
pub(super) fn prepare_keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    alt: bool,
) -> Result<(bool, Option<Job>), V8BindingError> {
    if !state.dom.borrow().has_native_input_picker(node) {
        return Ok((false, None));
    }
    let picker = state.dom.borrow().temporal_picker_snapshot(node)?;
    let Some(picker) = picker else {
        if key == "ArrowDown" && (alt || state.dom.borrow().input_datalist(node)?.is_some()) {
            open(scope, state, node)?;
            return Ok((true, None));
        }
        return Ok((false, None));
    };
    if !eligible(state, node)? {
        close(state, node);
        return Ok((false, None));
    }
    if picker.kind == "datalist" && matches!(key, " " | "Home" | "End" | "ArrowLeft" | "ArrowRight")
    {
        return Ok((false, None));
    }
    match key {
        "Escape" | "Tab" => {
            close(state, node);
            return Ok((key != "Tab", None));
        }
        "Enter" | " " => {
            return Ok((true, Some(Job::choose(state, node, picker.active)?)));
        }
        "Home" | "End" => {
            let column = picker.active % picker.columns;
            let delta = if key == "Home" {
                -(column as i32)
            } else {
                picker.columns as i32 - 1 - column as i32
            };
            state
                .dom
                .borrow_mut()
                .move_temporal_picker(node, delta, false)?;
        }
        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
            let delta = match key {
                "ArrowLeft" => -1,
                "ArrowRight" => 1,
                "ArrowUp" => -(picker.columns as i32),
                _ => picker.columns as i32,
            };
            state
                .dom
                .borrow_mut()
                .move_temporal_picker(node, delta, false)?;
        }
        "PageUp" | "PageDown" => state.dom.borrow_mut().move_temporal_picker(
            node,
            if key == "PageUp" { -1 } else { 1 },
            true,
        )?,
        _ => return Ok((picker.kind != "datalist", None)),
    }
    state.layout_cache_key.borrow_mut().take();
    Ok((true, None))
}
pub(super) fn keyboard(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    key: &str,
    alt: bool,
) -> Result<bool, V8BindingError> {
    let (handled, job) = prepare_keyboard(scope, state, node, key, alt)?;
    if let Some(mut job) = job
        && !job.poll(scope)?
    {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(handled)
}
pub(super) fn prepare_click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<(bool, Option<Job>), V8BindingError> {
    if !state.dom.borrow().has_native_input_picker(node) {
        return Ok((false, None));
    }
    let id = event_id(scope, event)?;
    if !state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.is_trusted)
    {
        return Ok((false, None));
    }
    let picker = state.dom.borrow().temporal_picker_snapshot(node)?;
    let Some(_picker) = picker else {
        open(scope, state, node)?;
        return Ok((true, None));
    };
    let x = native_input_property(scope, event, "MouseEvent", "clientX")?
        .number_value(scope)
        .unwrap_or(f64::NAN);
    let y = native_input_property(scope, event, "MouseEvent", "clientY")?
        .number_value(scope)
        .unwrap_or(f64::NAN);
    refresh_layout_in_scope(scope, state)?;
    let action = {
        let layout = state.layout.borrow();
        layout
            .generated_nodes
            .iter()
            .rev()
            .find(|item| {
                item.parent_id == Some(node_key(node))
                    && item.tag.starts_with("::temporal-")
                    && item.geometry.box_model.as_ref().is_some_and(|model| {
                        let r = model.border_box;
                        x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
                    })
            })
            .map(|item| item.tag.clone())
    };
    if let Some(action) = action {
        match action.as_str() {
            "::temporal-previous" => state
                .dom
                .borrow_mut()
                .move_temporal_picker(node, -1, true)?,
            "::temporal-next" => state.dom.borrow_mut().move_temporal_picker(node, 1, true)?,
            "::temporal-clear" => return Ok((true, Some(Job::value(state, node, "")?))),
            _ => {
                if let Some(index) = action
                    .strip_prefix("::temporal-item-")
                    .and_then(|index| index.parse::<usize>().ok())
                {
                    return Ok((true, Some(Job::choose(state, node, index)?)));
                }
            }
        }
        state.layout_cache_key.borrow_mut().take();
        return Ok((true, None));
    }
    Ok((false, None))
}

pub(super) fn click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    event: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    let (handled, job) = prepare_click(scope, state, node, event)?;
    if let Some(mut job) = job
        && !job.poll(scope)?
    {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(handled)
}

pub(super) fn fixup(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    let opened = state
        .dom
        .borrow()
        .top_layer()
        .filter(|entry| entry.kind == oryn_dom::TopLayerKind::TemporalPicker)
        .map(|entry| entry.node)
        .collect::<Vec<_>>();
    for node in opened {
        if !eligible(state, node)? || css_suppresses_focus_in_scope(scope, state, node)? {
            close(state, node);
        }
    }
    Ok(())
}

pub(super) fn prepare_accessibility(
    state: &Rc<V8BindingState>,
    node: NodeId,
    expected: &oryn_dom::TemporalPicker,
    action: &crate::accessibility::PickerAction,
) -> Result<Option<Job>, V8BindingError> {
    if state.dom.borrow().temporal_picker_snapshot(node)?.as_ref() != Some(expected)
        || !eligible(state, node)?
    {
        return Err(V8BindingError::Message(
            "stale native picker accessibility target".into(),
        ));
    }
    match action {
        crate::accessibility::PickerAction::Choose { index } => {
            if !expected.items.get(*index).is_some_and(|item| item.enabled) {
                return Err(V8BindingError::Message(
                    "disabled native picker candidate".into(),
                ));
            }
            return Ok(Some(Job::choose(state, node, *index)?));
        }
        crate::accessibility::PickerAction::Previous => state
            .dom
            .borrow_mut()
            .move_temporal_picker(node, -1, true)?,
        crate::accessibility::PickerAction::Next => {
            state.dom.borrow_mut().move_temporal_picker(node, 1, true)?
        }
        crate::accessibility::PickerAction::Clear => return Ok(Some(Job::value(state, node, "")?)),
        crate::accessibility::PickerAction::Accept => {
            if !state
                .dom
                .borrow()
                .temporal_candidate_selectable(node, &expected.draft)?
            {
                return Err(V8BindingError::Message(
                    "invalid native picker draft".into(),
                ));
            }
            return Ok(Some(Job::value(state, node, &expected.draft)?));
        }
    }
    state.layout_cache_key.borrow_mut().take();
    Ok(None)
}

// Private native UI consumes its own wheel input before author event dispatch.
pub(super) fn wheel(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    x: f64,
    y: f64,
    delta: f64,
) -> Result<bool, V8BindingError> {
    if delta == 0. {
        return Ok(false);
    }
    refresh_layout_in_scope(scope, state)?;
    let owner = {
        let layout = state.layout.borrow();
        layout
            .generated_nodes
            .iter()
            .rev()
            .find(|item| {
                item.tag == "::picker(temporal)"
                    && item.geometry.box_model.as_ref().is_some_and(|model| {
                        let r = model.border_box;
                        x >= r.x && x < r.right() && y >= r.y && y < r.bottom()
                    })
            })
            .and_then(|item| item.parent_id)
    };
    let Some(owner) = owner else { return Ok(false) };
    let node = NodeId {
        slot: owner as u32,
        generation: (owner >> 32) as u32,
    };
    if !eligible(state, node)? {
        return Ok(false);
    }
    let rows = (delta.abs() / 32.).ceil().clamp(1., 256.) as i32 * if delta < 0. { -1 } else { 1 };
    state.dom.borrow_mut().scroll_temporal_picker(node, rows)?;
    state.layout_cache_key.borrow_mut().take();
    Ok(true)
}

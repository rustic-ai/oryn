//! WebDriver option selection uses the select container, including collapsed
//! options that have no independently painted box. No author DOM method is called.
use super::*;
const HTML: &str = "http://www.w3.org/1999/xhtml";
pub(super) fn container(
    state: &V8BindingState,
    option: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    if !dom.is_element(option, HTML, "option") {
        return Ok(None);
    }
    let mut cursor = dom.parent(option)?;
    while let Some(node) = cursor {
        if dom.is_element(node, HTML, "select") {
            return Ok(Some(node));
        }
        cursor = dom.parent(node)?;
    }
    Ok(None)
}
fn mouse(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    select: NodeId,
    kind: &str,
) -> Result<(), V8BindingError> {
    let event = create_native_input_event(
        scope,
        state,
        NativeInputEventInputs {
            interface: "MouseEvent",
            event_type: kind,
            bubbles: true,
            cancelable: true,
            composed: true,
            detail: 1,
            related: None,
            fields: &[
                ("button", NativeInputValue::Number(0.)),
                (
                    "buttons",
                    NativeInputValue::Number(if kind == "mousedown" { 1. } else { 0. }),
                ),
            ],
        },
    )?;
    // The WebDriver algorithm owns selectedness; a second HTML activation would
    // reopen the picker or toggle the selection again.
    let id = event_id(scope, event)?;
    let dispatch = begin_event_dispatch(state, id)?;
    let result = dispatch_event_listeners(scope, state, select, event, || {});
    clear_event_path(scope, event);
    drop(dispatch);
    result.map(|_| ())
}
pub(super) fn click(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    option: NodeId,
    select: NodeId,
) -> Result<(), V8BindingError> {
    let generation = state.document_generation();
    let epoch = state.document_stream_epoch();
    let authority = state.document_authority_epoch();
    let live = || -> Result<bool, V8BindingError> {
        Ok(!state.execution_retired()
            && state.document_generation() == generation
            && state.document_stream_epoch() == epoch
            && state.document_authority_epoch() == authority
            && state.dom.borrow().is_connected(select)?
            && state.dom.borrow().node_document(select)? == Some(state.document_root())
            && container(state, option)? == Some(select))
    };
    if !live()? {
        return Ok(());
    }
    state.notify_user_activation();
    for kind in ["mouseover", "mousemove", "mousedown"] {
        mouse(scope, state, select, kind)?;
        if !live()? {
            return Ok(());
        }
    }
    focus_html_element(scope, state, select, true)?;
    if !live()? {
        return Ok(());
    }
    let enabled = {
        let dom = state.dom.borrow();
        !dom.option_disabled(option)? && !dom.control_disabled(select)? && !dom.is_inert(select)?
    };
    if enabled {
        let input = create_event_object(scope, state, "input", true, false)?;
        let id = event_id(scope, input)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = true;
        }
        dispatch_event(scope, state, select, input)?;
        if !live()? {
            return Ok(());
        }
        let previous = {
            let mut dom = state.dom.borrow_mut();
            if dom.option_disabled(option)?
                || dom.control_disabled(select)?
                || dom.is_inert(select)?
            {
                return Ok(());
            }
            let previous = dom.option_selected(option)?;
            let multiple = dom
                .attribute_in_null_namespace(select, "multiple")?
                .is_some();
            dom.set_option_selected(option, if multiple { !previous } else { true })?;
            dom.set_user_validity(select, true)?;
            dom.remove_from_top_layer(select);
            previous
        };
        state.layout_cache_key.borrow_mut().take();
        // WebDriver element-click specifies change when previous selectedness
        // was false; this differs from an arbitrary listbox pointer gesture.
        if !previous {
            let event = create_event_object(scope, state, "change", true, false)?;
            let id = event_id(scope, event)?;
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.is_trusted = true;
            }
            dispatch_event(scope, state, select, event)?;
            if !live()? {
                return Ok(());
            }
        }
    }
    for kind in ["mouseup", "click"] {
        mouse(scope, state, select, kind)?;
        if !live()? {
            break;
        }
    }
    Ok(())
}

//! Native clipboard defaults require a one-use host grant, never script globals.

pub(super) struct CompositionNotificationInputs<'a> {
    pub(super) target: NodeId,
    pub(super) name: &'a str,
    pub(super) input_type: &'a str,
    pub(super) text: &'a str,
    pub(super) composing: bool,
    pub(super) cancelable: bool,
}

/// Inputs for copy selected.
struct ClipboardCopyInputs<'a> {
    dom: &'a mut DomRuntime,
    layout: &'a BindingLayout,
    node: NodeId,
    parent: NodeId,
    range: &'a RangeRecord,
    selected: &'a BTreeMap<NodeId, bool>,
    whole: bool,
    top: bool,
    plain: &'a mut Vec<u16>,
    budget: &'a mut usize,
    base: Option<&'a url::Url>,
}
use super::*;
use crate::clipboard::{
    ClipboardAction, ClipboardActionResult, ClipboardData, ClipboardError, ClipboardItem,
    ClipboardPayload,
};
use crate::data_transfer::{Data, Mode, Store};
mod job;
pub(super) use job::ClipboardJob;

pub(super) fn creator(
    state: &V8BindingState,
) -> Result<crate::worker_wire::Creator, V8BindingError> {
    if state.execution_retired() || state.execution_tasks_stopped() {
        return Err(error(ClipboardError::Retired));
    }
    if state.message_transport.borrow().is_some() {
        return state.native_worker_creator();
    }
    Ok(state.worker_creator.borrow().clone().unwrap_or_else(|| {
        crate::worker_wire::Creator::Window {
            target: state.message_target.borrow().clone(),
            document_generation: state.document_generation(),
            stream_epoch: state.document_authority_epoch(),
        }
    }))
}
pub struct NativeClipboardGrant {
    instance: u64,
    document: NodeId,
    generation: u64,
    epoch: u64,
    authority: u64,
    target: NodeId,
    action: ClipboardAction,
    used: bool,
    job: Option<Box<ClipboardJob>>,
}
pub(super) fn grant(
    state: &Rc<V8BindingState>,
    target: NodeId,
    action: ClipboardAction,
) -> Result<NativeClipboardGrant, V8BindingError> {
    if state.execution_retired()
        || state.dom.borrow().arena().node_document(target)? != Some(state.document_root())
        || !state.dom.borrow().is_document_connected(target)?
    {
        return Err(error(ClipboardError::Retired));
    }
    Ok(NativeClipboardGrant {
        instance: state.instance_id,
        document: state.document_root(),
        generation: state.document_generation(),
        epoch: state.document_stream_epoch(),
        authority: state.document_authority_epoch(),
        target,
        action,
        used: false,
        job: None,
    })
}
fn error(error: ClipboardError) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Clipboard".into(),
        member: match error {
            ClipboardError::NotAllowed => "NotAllowedError",
            ClipboardError::Retired => "InvalidStateError",
            ClipboardError::QuotaExceeded => "QuotaExceededError",
            ClipboardError::Unavailable => "NotSupportedError",
            ClipboardError::InvalidData => "DataError",
        }
        .into(),
    }
}
fn valid(state: &V8BindingState, grant: &NativeClipboardGrant) -> bool {
    !state.execution_retired()
        && !state.execution_tasks_stopped()
        && state.instance_id == grant.instance
        && state.document_root() == grant.document
        && state.document_generation() == grant.generation
        && state.document_stream_epoch() == grant.epoch
        && state.document_authority_epoch() == grant.authority
        && state
            .dom
            .borrow()
            .arena()
            .node_document(grant.target)
            .ok()
            .flatten()
            == Some(grant.document)
        && state
            .dom
            .borrow()
            .is_document_connected(grant.target)
            .unwrap_or(false)
}
fn from_data(data: &ClipboardData) -> Result<Store, V8BindingError> {
    data.validate().map_err(error)?;
    let mut store = Store::default();
    for item in &data.items {
        match &item.payload {
            ClipboardPayload::Text(value) => {
                store
                    .add_string(value.clone(), item.media_type.clone())
                    .map_err(|_| error(ClipboardError::InvalidData))?;
            }
            ClipboardPayload::File {
                name,
                last_modified,
                bytes,
            } => {
                store.add_file(oryn_dom::SelectedFile {
                    name: name.clone(),
                    last_modified: *last_modified,
                    media_type: item.media_type.clone(),
                    bytes: bytes.clone().into(),
                });
            }
        }
    }
    Ok(store)
}
pub(super) fn to_data(store: &Store) -> Result<ClipboardData, V8BindingError> {
    let mut data = ClipboardData::default();
    let mut bytes = 0usize;
    for item in store.items() {
        let size = match &item.kind {
            Data::String(value) => value.len().checked_mul(2),
            Data::File(file) => file.bytes.len().checked_add(file.name.len()),
        }
        .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
        bytes = bytes
            .checked_add(size)
            .and_then(|bytes| bytes.checked_add(item.media_type.len()))
            .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
        if bytes > crate::clipboard::MAX_CLIPBOARD_BYTES
            || data.items.len() >= crate::clipboard::MAX_CLIPBOARD_ITEMS
        {
            return Err(error(ClipboardError::QuotaExceeded));
        }
        let payload = match &item.kind {
            Data::String(value) => ClipboardPayload::Text(value.clone()),
            Data::File(file) => ClipboardPayload::File {
                name: file.name.clone(),
                last_modified: file.last_modified,
                bytes: file.bytes.to_vec(),
            },
        };
        data.items.push(ClipboardItem {
            media_type: item.media_type.clone(),
            payload,
        });
    }
    data.validate().map_err(error)?;
    Ok(data)
}
fn control(state: &V8BindingState, target: NodeId) -> bool {
    state.dom.borrow().is_editable_text_control(target)
}
pub(super) fn control_selection(
    state: &V8BindingState,
    target: NodeId,
) -> Result<(u32, u32, String), V8BindingError> {
    let value = state.dom.borrow().control_edit_value(target)?;
    let length = oryn_dom::dom_string::len(&value) as u32;
    let (start, end, _) = state
        .text_control_selections
        .borrow()
        .get(&target)
        .cloned()
        .unwrap_or_else(|| {
            let (start, end) = if state.dom.borrow().is_temporal_control(target) {
                state
                    .dom
                    .borrow()
                    .temporal_selection(target)
                    .unwrap_or((length, length))
            } else {
                (length, length)
            };
            (start, end, "none".into())
        });
    Ok((
        start.min(length),
        end.min(length).max(start.min(length)),
        value,
    ))
}
fn password(state: &V8BindingState, target: NodeId) -> bool {
    state
        .dom
        .borrow()
        .attribute_in_null_namespace(target, "type")
        .ok()
        .flatten()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
}
pub(super) fn editable(state: &V8BindingState, target: NodeId) -> Result<bool, V8BindingError> {
    if !state.dom.borrow().is_document_connected(target)? {
        return Ok(false);
    }
    if control(state, target) {
        let dom = state.dom.borrow();
        return Ok(!dom.control_disabled(target)?
            && dom
                .attribute_in_null_namespace(target, "readonly")?
                .is_none());
    }
    let Some(host) = editing_host(state)? else {
        return Ok(false);
    };
    Ok(target == host
        || state
            .dom
            .borrow()
            .arena()
            .ancestors(target)?
            .contains(&host))
}
// Clone selected light-tree content while retaining exact source ownership for style.
// Never pair cloned querySelector results with the original tree by list index.
pub(super) fn rich_selection(
    state: &V8BindingState,
    range: &RangeRecord,
) -> Result<(Vec<u16>, String), V8BindingError> {
    refresh_layout(state)?;
    let base = url::Url::parse(&state.native_document_base_url(true)?).ok();
    let layout = state.layout.borrow();
    let mut dom = state.dom.borrow_mut();
    let common = common_ancestor(&dom, range.start_container, range.end_container)?;
    let mut selected = BTreeMap::new();
    if matches!(
        dom.arena().get(common)?.kind,
        NodeKind::Text { .. } | NodeKind::CDataSection { .. }
    ) {
        selected.insert(common, false);
    } else {
        for child in light_children(&dom, common)? {
            collect_range_content_selection(&dom, child, range, &mut selected)?;
        }
    }
    let fragment = dom.create_fragment();
    dom.retain_native(fragment)?;
    let mut plain = Vec::new();
    let mut budget = crate::clipboard::MAX_CLIPBOARD_BYTES;
    let result = (|| {
        let roots = if selected.contains_key(&common) {
            vec![common]
        } else {
            light_children(&dom, common)?
        };
        for node in roots {
            copy_selected(ClipboardCopyInputs {
                dom: &mut dom,
                layout: &layout,
                node,
                parent: fragment,
                range,
                selected: &selected,
                whole: false,
                top: true,
                plain: &mut plain,
                budget: &mut budget,
                base: base.as_ref(),
            })?;
        }
        let html = dom.serialize(fragment)?;
        if html.len().saturating_add(plain.len().saturating_mul(2))
            > crate::clipboard::MAX_CLIPBOARD_BYTES
        {
            return Err(V8BindingError::Allocation(
                "clipboard formatted selection budget",
            ));
        }
        Ok((plain, html))
    })();
    dom.release_native(fragment)?;
    result
}
// Snapshot selected shells without changing the source Range or DOM. Whole
// subtrees are copied recursively by copy_selected; only partial shells need
// their children represented individually in this map.
fn collect_range_content_selection(
    dom: &DomRuntime,
    node: NodeId,
    range: &RangeRecord,
    selected: &mut BTreeMap<NodeId, bool>,
) -> Result<(), V8BindingError> {
    let start = (range.start_container, range.start_offset);
    let end = (range.end_container, range.end_offset);
    if start == end {
        return Ok(());
    }
    let mut pending = vec![node];
    let mut seen = BTreeSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            return Err(DomError::Cycle.into());
        }
        let before = relative_boundary(dom, node, false)?;
        let after = relative_boundary(dom, node, true)?;
        if compare_boundary_points(dom, end, before)? <= 0
            || compare_boundary_points(dom, start, after)? >= 0
        {
            continue;
        }
        let fully = compare_boundary_points(dom, start, before)? <= 0
            && compare_boundary_points(dom, after, end)? <= 0;
        selected.insert(node, fully);
        if !fully {
            pending.extend(light_children(dom, node)?.into_iter().rev());
        }
    }
    Ok(())
}
fn copy_selected(params: ClipboardCopyInputs<'_>) -> Result<(), V8BindingError> {
    let ClipboardCopyInputs {
        dom,
        layout,
        node,
        parent,
        range,
        selected,
        whole,
        top,
        plain,
        budget,
        base,
    } = params;

    let Some(full) = whole
        .then_some(true)
        .or_else(|| selected.get(&node).copied())
    else {
        return Ok(());
    };
    // Bound both node count and copied bytes before allocation, including markup overhead.
    *budget = budget
        .checked_sub(128)
        .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
    let kind = dom.arena().get(node)?.kind.clone();
    let clone = match &kind {
        NodeKind::Text { data } | NodeKind::CDataSection { data } => {
            let length = oryn_dom::dom_string::len(data);
            let start = if !full && node == range.start_container {
                range.start_offset as usize
            } else {
                0
            };
            let end = if !full && node == range.end_container {
                range.end_offset as usize
            } else {
                length
            };
            *budget = budget
                .checked_sub(end.saturating_sub(start).saturating_mul(8))
                .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
            let units = oryn_dom::dom_string::units(data);
            let units =
                &units[start.min(units.len())..end.min(units.len()).max(start.min(units.len()))];
            plain.extend_from_slice(units);
            let text = dom.create_text(oryn_dom::dom_string::from_units(units));
            if top {
                let shell = dom.create_element("span");
                dom.append_child(parent, shell)?;
                dom.append_child(shell, text)?;
                if let Some(source) = dom.parent(node)? {
                    copy_formatting(dom, layout, source, shell, budget)?;
                    clipboard_markup::rewrite_from_source(dom, source, shell, base)?;
                }
                return Ok(());
            } else {
                text
            }
        }
        NodeKind::Element { .. } => {
            let size = dom
                .arena()
                .get(node)?
                .attributes
                .records()
                .map(|attribute| {
                    attribute
                        .value
                        .len()
                        .saturating_add(attribute.local_name.len())
                })
                .sum::<usize>();
            *budget = budget
                .checked_sub(size)
                .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
            let shell = dom.clone_node(node, false)?;
            // Attach before recursion so a failing bounded copy is reclaimed with its fragment.
            dom.append_child(parent, shell)?;
            copy_formatting(dom, layout, node, shell, budget)?;
            clipboard_markup::rewrite_from_source(dom, node, shell, base)?;
            for child in light_children(dom, node)? {
                copy_selected(ClipboardCopyInputs {
                    dom,
                    layout,
                    node: child,
                    parent: shell,
                    range,
                    selected,
                    whole: full,
                    top: false,
                    plain,
                    budget,
                    base,
                })?;
            }
            return Ok(());
        }
        _ => return Ok(()),
    };
    dom.append_child(parent, clone)?;
    Ok(())
}
fn copy_formatting(
    dom: &mut DomRuntime,
    layout: &BindingLayout,
    source: NodeId,
    clone: NodeId,
    budget: &mut usize,
) -> Result<(), V8BindingError> {
    const PROPERTIES: &[&str] = &[
        "color",
        "background-color",
        "font-family",
        "font-size",
        "font-style",
        "font-weight",
        "font-variant",
        "line-height",
        "letter-spacing",
        "word-spacing",
        "text-decoration-line",
        "text-decoration-style",
        "text-decoration-color",
        "text-transform",
        "white-space",
        "direction",
        "unicode-bidi",
        "vertical-align",
        "text-align",
        "writing-mode",
        "text-orientation",
    ];
    let Some(styles) = layout.styles.get(&node_key(source)) else {
        return Ok(());
    };
    let mut css = dom
        .attribute_in_null_namespace(clone, "style")?
        .unwrap_or_default()
        .to_string();
    for property in PROPERTIES {
        if let Some(value) = styles.get(*property) {
            let size = property.len().saturating_add(value.len()).saturating_add(2);
            *budget = budget
                .checked_sub(size)
                .ok_or_else(|| error(ClipboardError::QuotaExceeded))?;
            css.push(';');
            css.push_str(property);
            css.push(':');
            css.push_str(value);
        }
    }
    dom.set_attribute(clone, "style", css)?;
    Ok(())
}
fn selected_data(
    state: &V8BindingState,
    target: NodeId,
) -> Result<Option<ClipboardData>, V8BindingError> {
    if control(state, target) {
        if password(state, target) {
            return Ok(None);
        }
        let (start, end, value) = control_selection(state, target)?;
        if start == end {
            return Ok(None);
        }
        let value = oryn_dom::dom_string::units(&value);
        return Ok(Some(ClipboardData::text(
            value[start as usize..end as usize].to_vec(),
        )));
    }
    let Some(range) = state
        .selection_range
        .get()
        .and_then(|id| state.ranges.borrow().get(&id).cloned())
    else {
        return Ok(None);
    };
    if range.start_container == range.end_container && range.start_offset == range.end_offset {
        return Ok(None);
    }
    if state
        .dom
        .borrow()
        .arena()
        .node_document(range.start_container)?
        != Some(state.document_root())
    {
        return Err(error(ClipboardError::Retired));
    }
    let (text, html) = rich_selection(state, &range)?;
    let data = ClipboardData {
        items: vec![
            ClipboardItem {
                media_type: "text/plain".into(),
                payload: ClipboardPayload::Text(text),
            },
            ClipboardItem {
                media_type: "text/html".into(),
                payload: ClipboardPayload::Text(oryn_dom::dom_string::units(&html)),
            },
        ],
    };
    data.validate().map_err(error)?;
    Ok(Some(data))
}
fn private<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, &format!("oryn.native.input.{name}"))
        .ok_or(V8BindingError::Allocation("native InputEvent key"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
pub(super) fn input_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = private(scope, name).ok()?;
    object
        .get_private(scope, key)
        .filter(|value| !value.is_undefined())
}
fn input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    target: NodeId,
    kind: &str,
    input_type: &str,
    text: Option<&str>,
    store: Option<data_transfer::SharedStore>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    // Input/textarea beforeinput and every input event expose empty ranges.
    // Do not construct then discard a snapshot of an unrelated DOM selection:
    // it may have been removed by the completed SPA action.
    let include_target_ranges = !control(state, target) && kind != "input";
    let event = create_input_event_object_with_ranges(
        scope,
        state,
        kind,
        input_type,
        text,
        kind == "beforeinput",
        include_target_ranges,
    )?;
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
        record.composed = true;
    }
    for name in ["data", "inputType", "isComposing"] {
        let authored = v8_string(scope, &format!("__oryn_{name}"))?;
        let value = event
            .get(scope, authored)
            .ok_or(V8BindingError::JavaScriptException)?;
        let key = private(scope, name)?;
        event
            .set_private(scope, key, value)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    let transfer = match store {
        Some(store) => data_transfer::wrap(scope, state, store)?.into(),
        None => v8::null(scope).into(),
    };
    let key = private(scope, "dataTransfer")?;
    event
        .set_private(scope, key, transfer)
        .ok_or(V8BindingError::JavaScriptException)?;
    let ranges = if control(state, target) || kind == "input" {
        v8::Array::new(scope, 0).into()
    } else {
        let key = v8_string(scope, "__oryn_target_ranges")?;
        event
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?
    };
    let key = private(scope, "targetRanges")?;
    event
        .set_private(scope, key, ranges)
        .ok_or(V8BindingError::JavaScriptException)?;
    for name in [
        "data",
        "inputType",
        "isComposing",
        "dataTransfer",
        "target_ranges",
    ] {
        let key = v8_string(scope, &format!("__oryn_{name}"))?;
        event
            .delete(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(event)
}

#[cfg(test)]
pub(super) fn sanitized_html(markup: &str) -> Result<String, V8BindingError> {
    clipboard_markup::sanitize(markup, None)
}
pub(super) fn sanitized_html_for_state(
    state: &V8BindingState,
    markup: &str,
) -> Result<String, V8BindingError> {
    let base = url::Url::parse(&state.native_document_base_url(true)?).ok();
    clipboard_markup::sanitize(markup, base.as_ref())
}

/// One event in a retained clipboard default. The event and its data store stay
/// owned across infrastructure failure; resumption never restarts listeners.
pub(super) struct ClipboardNotification {
    owner: RcWeak<V8BindingState>,
    identity: (NodeId, u64, u64, u64),
    event: v8::Global<v8::Object>,
    notification: native_platform::publication::Notification,
    store: Option<data_transfer::SharedStore>,
    source: Option<data_transfer::SharedStore>,
    result: Option<bool>,
}
impl ClipboardNotification {
    fn create(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        name: &str,
        event: v8::Local<v8::Object>,
        store: Option<data_transfer::SharedStore>,
    ) -> Result<Self, V8BindingError> {
        let target = wrap_node(scope, state, target)?;
        let notification =
            native_platform::publication::pending_dom_dispatch(scope, state, target, name, event)
                .for_parser_stream(state.document_stream_epoch());
        Ok(Self {
            owner: Rc::downgrade(state),
            identity: (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            ),
            event: v8::Global::new(scope, event),
            notification,
            store,
            source: None,
            result: None,
        })
    }
    fn clipboard(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        name: &str,
        store: data_transfer::SharedStore,
    ) -> Result<Self, V8BindingError> {
        if !matches!(name, "copy" | "cut" | "paste") {
            return Err(error(ClipboardError::InvalidData));
        }
        let source = store;
        let store = Rc::new(RefCell::new(source.borrow().clone()));
        let event =
            create_event_object_with_interface(scope, state, "ClipboardEvent", name, true, true)?;
        let transfer = data_transfer::wrap(scope, state, store.clone())?;
        data_transfer::attach_event(scope, event, "ClipboardEvent", transfer.into())?;
        let id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.is_trusted = true;
            record.composed = true;
        }
        store.borrow_mut().set_mode(if name == "paste" {
            Mode::ReadOnly
        } else {
            Mode::ReadWrite
        });
        let mut notification = Self::create(scope, state, target, name, event, Some(store))?;
        source.borrow_mut().set_mode(Mode::Protected);
        notification.source = Some(source);
        Ok(notification)
    }
    pub(super) fn editing(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        target: NodeId,
        name: &str,
        input_type: &str,
        text: Option<&str>,
        data: Option<&ClipboardData>,
    ) -> Result<Self, V8BindingError> {
        let store = if control(state, target) {
            None
        } else {
            data.map(|data| {
                from_data(data).map(|mut store| {
                    store.set_mode(Mode::ReadOnly);
                    Rc::new(RefCell::new(store))
                })
            })
            .transpose()?
        };
        let event = input_event(scope, state, target, name, input_type, text, store.clone())?;
        let target = if control(state, target) {
            target
        } else {
            editing_host_for_node(
                &state.dom.borrow(),
                Some(target),
                state.design_mode_enabled(),
                state.document_root(),
            )?
            .unwrap_or(target)
        };
        Self::create(scope, state, target, name, event, store)
    }
    pub(super) fn composition(
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
        params: CompositionNotificationInputs<'_>,
    ) -> Result<Self, V8BindingError> {
        let CompositionNotificationInputs {
            target,
            name,
            input_type,
            text,
            composing,
            cancelable,
        } = params;

        let event = input_event(scope, state, target, name, input_type, Some(text), None)?;
        let key = private(scope, "isComposing")?;
        let value = v8::Boolean::new(scope, composing);
        event
            .set_private(scope, key, value.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        let id = event_id(scope, event)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.cancelable = cancelable;
        }
        Self::create(scope, state, target, name, event, None)
    }
    fn current(&self, state: &Rc<V8BindingState>) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|owner| Rc::ptr_eq(&owner, state))
            && !state.execution_retired()
            && !state.execution_tasks_stopped()
            && self.identity
                == (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
    }
    pub(super) fn poll(
        &mut self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<Option<bool>, V8BindingError> {
        if let Some(result) = self.result {
            return Ok(Some(result));
        }
        if !self.current(state) {
            self.cancel();
            return Ok(Some(false));
        }
        if !self.notification.resume(scope, state)? {
            return Ok(None);
        }
        if !self.current(state) {
            self.cancel();
            return Ok(Some(false));
        }
        let event = v8::Local::new(scope, &self.event);
        let id = event_id(scope, event)?;
        let accepted = state
            .events
            .borrow()
            .get(&id)
            .is_some_and(|record| !record.default_prevented);
        self.notification.finish(scope, state);
        if let (Some(source), Some(store)) = (&self.source, &self.store) {
            let mut completed = store.borrow().clone();
            completed.set_mode(Mode::Protected);
            *source.borrow_mut() = completed;
        }
        self.detach();
        self.result = Some(accepted);
        Ok(Some(accepted))
    }
    fn detach(&self) {
        if let Some(store) = &self.store {
            store.borrow_mut().set_mode(Mode::Detached);
        }
    }
    pub(super) fn cancel(&mut self) {
        if self.result.is_none() {
            if let Some(owner) = self.owner.upgrade() {
                self.notification.cancel(&owner);
            }
            self.result = Some(false);
        }
        self.detach();
    }
}
impl Drop for ClipboardNotification {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(super) fn fire_editing(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    kind: &str,
    input_type: &str,
    text: Option<&str>,
    data: Option<&ClipboardData>,
) -> Result<bool, V8BindingError> {
    let store = if control(state, target) {
        None
    } else {
        data.map(|data| {
            from_data(data).map(|mut store| {
                store.set_mode(Mode::ReadOnly);
                Rc::new(RefCell::new(store))
            })
        })
        .transpose()?
    };
    let event = input_event(scope, state, target, kind, input_type, text, store.clone())?;
    struct Detach(Option<data_transfer::SharedStore>);
    impl Drop for Detach {
        fn drop(&mut self) {
            if let Some(store) = &self.0 {
                store.borrow_mut().set_mode(Mode::Detached);
            }
        }
    }
    let _detach = Detach(store);
    let target = if control(state, target) {
        target
    } else {
        editing_host_for_node(
            &state.dom.borrow(),
            Some(target),
            state.design_mode_enabled(),
            state.document_root(),
        )?
        .unwrap_or(target)
    };
    dispatch_event(scope, state, target, event)
}
pub(super) fn action(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    grant: &mut NativeClipboardGrant,
) -> Result<ClipboardActionResult, V8BindingError> {
    if grant.job.is_none() {
        if grant.used || !valid(state, grant) {
            return Err(error(ClipboardError::NotAllowed));
        }
        let token = NativeClipboardGrant {
            instance: grant.instance,
            document: grant.document,
            generation: grant.generation,
            epoch: grant.epoch,
            authority: grant.authority,
            target: grant.target,
            action: grant.action,
            used: false,
            job: None,
        };
        let job = ClipboardJob::new(state, token)?;
        grant.used = true;
        grant.job = Some(Box::new(job));
    }
    let result = grant.job.as_mut().unwrap().poll(scope, state)?;
    if let Some(result) = result {
        grant.job.take();
        Ok(result)
    } else {
        Err(V8BindingError::Allocation("clipboard operation suspended"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    // A real DOM range survives removal and rebases, while the binding cache
    // still names the reclaimed link text (the cart failure's state).
    fn removed_selection_cache(state: &V8BindingState) -> (u64, RangeRecord) {
        let old = state.dom.borrow().find_by_id("old").unwrap();
        let parent = state.dom.borrow().parent(old).unwrap().unwrap();
        let text = state.dom.borrow().children(old).unwrap()[0];
        let point = BoundaryPoint {
            container: text,
            offset: 15,
        };
        let id = set_selection_boundaries(state, point, point).unwrap();
        let cached = state.ranges.borrow()[&id].clone();
        state.dom.borrow_mut().retain_native(old).unwrap();
        state.dom.borrow_mut().remove_child(parent, old).unwrap();
        state.dom.borrow_mut().release_native(old).unwrap();
        // Removal records retain their own native lease until observer delivery
        // completes. Model that real checkpoint before expecting reclamation;
        // the live range has already rebased, but its binding cache has not.
        let delivered = state.dom.borrow_mut().drain_mutations();
        state
            .dom
            .borrow_mut()
            .release_mutation_roots(&delivered)
            .unwrap();
        let live = state.dom.borrow().live_range(LiveRangeId(id)).unwrap();
        assert_eq!(live.start.container, parent);
        assert_eq!(live.end.container, parent);
        assert_eq!(state.ranges.borrow()[&id].start_container, text);
        assert!(state.dom.borrow().arena().get(text).is_err());
        // This is the old unconditional event-construction operation. It must
        // fail on the obsolete cache, independently of the repaired path.
        assert!(register_static_range(state, cached.clone()).is_err());
        (id, cached)
    }

    #[test]
    fn generic_editing_input_dispatch_never_snapshots_target_ranges() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<span id=old>Review checkout</span><input id=t value=old>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(237), 3).unwrap();
        eval(
            scope,
            "document.getElementById('t').focus();globalThis.inputCalls=0;globalThis.inputRangeCount=-1;document.getElementById('t').addEventListener('input',e=>{inputCalls++;inputRangeCount=e.getTargetRanges().length})",
        );
        let (id, _) = removed_selection_cache(&state);
        // Endpoint cells are empty: focus resolves a valid input; only the
        // unrelated cached range is invalid, isolating event construction.
        state.selection_anchor.set(None);
        state.selection_focus.set(None);
        dispatch_editing_input(scope, &state, "insertText", Some("x")).unwrap();
        eval(
            scope,
            "if(inputCalls !== 1 || inputRangeCount !== 0 || document.getElementById('t').value !== 'old') throw Error('generic input dispatch changed effects')",
        );
        assert_eq!(state.selection_range.get(), Some(id));
        assert!(state.dom.borrow().live_range(LiveRangeId(id)).is_ok());
    }

    #[test]
    fn control_beforeinput_and_input_ignore_unrelated_stale_selection() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<span id=old>Review checkout</span><input id=i><textarea id=t></textarea><div id=rich contenteditable>abc</div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(234), 3).unwrap();
        let (range_id, cached) = removed_selection_cache(&state);
        for (id, kind) in [
            ("i", "beforeinput"),
            ("t", "beforeinput"),
            ("rich", "input"),
        ] {
            state.ranges.borrow_mut().insert(range_id, cached.clone());
            let target = state.dom.borrow().find_by_id(id).unwrap();
            let event =
                input_event(scope, &state, target, kind, "insertText", Some("x"), None).unwrap();
            // Event construction must not read/rebase this irrelevant cache.
            assert_eq!(
                state.ranges.borrow()[&range_id].start_container,
                cached.start_container
            );
            let key = v8_string(scope, "eventUnderTest").unwrap();
            context
                .global(scope)
                .set(scope, key.into(), event.into())
                .unwrap();
            eval(
                scope,
                "if(eventUnderTest.getTargetRanges().length !== 0) throw Error('event exposed unrelated ranges')",
            );
            // The JS method boundary may legitimately synchronize the cache.
            assert_eq!(state.selection_range.get(), Some(range_id));
            assert!(state.dom.borrow().live_range(LiveRangeId(range_id)).is_ok());
        }
    }

    #[test]
    fn rich_beforeinput_rebases_live_range_after_spa_subtree_removal() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div id=rich contenteditable><span id=old>abc</span><span>def</span></div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(235), 3).unwrap();
        let host = state.dom.borrow().find_by_id("rich").unwrap();
        let old = state.dom.borrow().find_by_id("old").unwrap();
        let text = state.dom.borrow().children(old).unwrap()[0];
        let id = set_selection_boundaries(
            &state,
            BoundaryPoint {
                container: text,
                offset: 0,
            },
            BoundaryPoint {
                container: text,
                offset: 2,
            },
        )
        .unwrap();
        let event = input_event(
            scope,
            &state,
            host,
            "beforeinput",
            "insertText",
            Some("x"),
            None,
        )
        .unwrap();
        let key = v8_string(scope, "eventUnderTest").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), event.into())
            .unwrap();
        eval(
            scope,
            "if(eventUnderTest.getTargetRanges().length !== 1 || eventUnderTest.getTargetRanges()[0].endOffset !== 2) throw Error('rich range missing')",
        );
        state.dom.borrow_mut().remove_child(host, old).unwrap();
        assert_eq!(state.ranges.borrow()[&id].start_container, text);
        let event = input_event(
            scope,
            &state,
            host,
            "beforeinput",
            "insertText",
            Some("x"),
            None,
        )
        .unwrap();
        let key = v8_string(scope, "eventUnderTest").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), event.into())
            .unwrap();
        eval(
            scope,
            "const range=eventUnderTest.getTargetRanges()[0]; if(!range || range.startContainer !== document.getElementById('rich') || range.startOffset !== 0 || range.endOffset !== 0) throw Error('rich live range not rebased')",
        );
    }

    #[test]
    fn rich_beforeinput_does_not_swallow_invalid_live_range() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=rich contenteditable>abc</div>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(236), 3).unwrap();
        state.selection_range.set(Some(u64::MAX));
        let target = state.dom.borrow().find_by_id("rich").unwrap();
        let result = input_event(
            scope,
            &state,
            target,
            "beforeinput",
            "insertText",
            Some("x"),
            None,
        );
        assert!(matches!(result, Err(V8BindingError::Dom(_))));
    }

    #[test]
    fn retained_clipboard_event_resumes_suffix_and_detaches_only_on_completion() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(212), 1).unwrap();
        let target = state.dom.borrow().find_by_id("t").unwrap();
        let source=v8::String::new(scope,r#"globalThis.trace=[];const target=document.getElementById('t');target.addEventListener('copy',e=>{trace.push('first');globalThis.savedTransfer=e.clipboardData;e.clipboardData.setData('text/plain','a')},{once:true});target.addEventListener('copy',e=>{trace.push('second');e.clipboardData.setData('text/plain',e.clipboardData.getData('text/plain')+'b')});"#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let store = Rc::new(RefCell::new(Store::default()));
        let mut pending =
            ClipboardNotification::clipboard(scope, &state, target, "copy", store.clone()).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(pending.poll(scope, &state).is_err());
        assert_eq!(store.borrow().mode, Mode::Protected);
        assert_eq!(
            pending.store.as_ref().unwrap().borrow().mode,
            Mode::ReadWrite
        );
        assert_eq!(pending.poll(scope, &state).unwrap(), Some(true));
        assert_eq!(pending.poll(scope, &state).unwrap(), Some(true));
        assert_eq!(store.borrow().mode, Mode::Protected);
        assert_eq!(
            pending.store.as_ref().unwrap().borrow().mode,
            Mode::Detached
        );
        assert_eq!(to_data(&store.borrow()).unwrap().plain_text(), &[97, 98]);
        let source=v8::String::new(scope,"if(trace.join(',')!=='first,second'||savedTransfer.getData('text/plain')!=='')throw Error('clipboard listener replay or retained data access');").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let store = Rc::new(RefCell::new(Store::default()));
        let mut canceled =
            ClipboardNotification::clipboard(scope, &state, target, "copy", store.clone()).unwrap();
        native_platform::publication::fail_after_next_callback();
        assert!(canceled.poll(scope, &state).is_err());
        canceled.cancel();
        assert_eq!(canceled.poll(scope, &state).unwrap(), Some(false));
        assert_eq!(store.borrow().mode, Mode::Protected);
        assert!(store.borrow().items().is_empty());
        assert_eq!(
            canceled.store.as_ref().unwrap().borrow().mode,
            Mode::Detached
        );
        let event = v8::Local::new(scope, &canceled.event);
        let id = event_id(scope, event).unwrap();
        assert!(!state.events.borrow()[&id].dispatching);
        let mut editing = ClipboardNotification::editing(
            scope,
            &state,
            target,
            "beforeinput",
            "insertFromPaste",
            Some("x"),
            None,
        )
        .unwrap();
        editing.cancel();
        assert_eq!(editing.poll(scope, &state).unwrap(), Some(false));
    }
    #[derive(Default)]
    struct Memory(Mutex<ClipboardData>);
    #[test]
    fn trusted_backward_text_target_matches_native_code_point_deletion() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse(
            "<div id=editor contenteditable><p>abc</p></div>"
        ).into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(236), 4).unwrap();
        let target = state.dom.borrow().find_by_id("editor").unwrap();
        // The three Indic/jamo cases retain the pinned fixture's code-point
        // deletion granularity; the algorithm does not invent grapheme deletion.
        for (data, caret, start, expected) in [
            ("abc", 1, 0, "bc"),
            ("abc", 2, 1, "ac"),
            ("abc", 3, 2, "ab"),
            ("a😀b", 3, 1, "ab"),
            ("ที่", 3, 2, "ที"),
            ("क्षि", 4, 3, "क्ष"),
            ("각", 3, 2, "가"),
        ] {
            let setup = format!(
                "globalThis.editor=document.getElementById('editor');editor.innerHTML='<p></p>';globalThis.text=document.createTextNode({data:?});editor.firstChild.appendChild(text);editor.focus();getSelection().collapse(text,{caret});globalThis.savedRange=null;globalThis.beforeCount=0;globalThis.before=e=>{{beforeCount++;savedRange=e.getTargetRanges()[0];if(!e.isTrusted||!(savedRange instanceof StaticRange)||savedRange!==e.getTargetRanges()[0]||savedRange.startContainer!==text||savedRange.endContainer!==text||savedRange.startOffset!=={start}||savedRange.endOffset!=={caret})throw Error('target extent');if(text.data!=={data:?}||getSelection().anchorNode!==text||getSelection().anchorOffset!=={caret})throw Error('early mutation')}};editor.addEventListener('beforeinput',before);"
            );
            eval(scope, &setup);
            let mut notification = ClipboardNotification::editing(scope, &state, target,
                "beforeinput", "deleteContentBackward", None, None).unwrap();
            let mut outcome = None;
            for _ in 0..64 {
                outcome = notification.poll(scope, &state).unwrap();
                if outcome.is_some() { break; }
            }
            assert_eq!(outcome, Some(true));
            // execCommand uses the same native Delete primitive as keyboard
            // Command::Delete { forward:false, unit:None }; not a JS substitute.
            eval(scope, &format!("editor.removeEventListener('beforeinput',before);document.execCommand('delete');if(beforeCount!==1||text.data!=={expected:?}||savedRange.startOffset!=={start}||savedRange.endOffset!=={caret}||getSelection().anchorNode!==text||getSelection().anchorOffset!=={start})throw Error('deletion/snapshot');"));
            let event = input_event(scope, &state, target, "input", "deleteContentBackward", None, None).unwrap();
            let key = v8_string(scope, "inputUnderTest").unwrap();
            context.global(scope).set(scope, key.into(), event.into()).unwrap();
            eval(scope, "if(inputUnderTest.getTargetRanges().length!==0)throw Error('input ranges')");
        }
        eval(scope, "editor.innerHTML='<p>a😀b</p>';text=editor.firstChild.firstChild;getSelection().collapse(text,3);globalThis.cancelBefore=e=>{savedRange=e.getTargetRanges()[0];e.preventDefault()};editor.addEventListener('beforeinput',cancelBefore);");
        let mut notification = ClipboardNotification::editing(scope, &state, target,
            "beforeinput", "deleteContentBackward", None, None).unwrap();
        let mut outcome = None;
        for _ in 0..64 {
            outcome = notification.poll(scope, &state).unwrap();
            if outcome.is_some() { break; }
        }
        assert_eq!(outcome, Some(false));
        eval(scope, "if(text.data!=='a😀b'||getSelection().anchorNode!==text||getSelection().anchorOffset!==3||savedRange.startOffset!==1||savedRange.endOffset!==3)throw Error('cancellation');editor.removeEventListener('beforeinput',cancelBefore);");
    }
    #[test]
    fn trusted_delete_target_range_cancellation_keeps_dom_and_selection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::html::parse("<main id=editor contenteditable>abc<br><br><div id=child>def<br>ghi</div></main>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(235), 4).unwrap();
        eval(scope, "globalThis.editor=document.getElementById('editor');globalThis.child=document.getElementById('child');getSelection().collapse(child.firstChild,0);globalThis.originalHTML=editor.innerHTML;globalThis.originalNode=getSelection().anchorNode;editor.addEventListener('beforeinput',e=>{if(!e.isTrusted)throw Error('untrusted');globalThis.savedRange=e.getTargetRanges()[0];if(!savedRange||savedRange.startContainer!==editor||savedRange.startOffset!==2||savedRange.endContainer!==editor||savedRange.endOffset!==3)throw Error('wrong nested BR extent');if(savedRange!==e.getTargetRanges()[0])throw Error('unstable StaticRange identity');e.preventDefault()});");
        let target = state.dom.borrow().find_by_id("editor").unwrap();
        let mut notification = ClipboardNotification::editing(scope, &state, target, "beforeinput", "deleteContentBackward", None, None).unwrap();
        let mut outcome = None;
        for _ in 0..64 {
            outcome = notification.poll(scope, &state).unwrap();
            if outcome.is_some() { break; }
        }
        assert_eq!(outcome, Some(false));
        eval(scope, "if(editor.innerHTML!==originalHTML||getSelection().anchorNode!==originalNode||getSelection().anchorOffset!==0)throw Error('canceled notification mutated state');child.firstChild.data='changed';if(savedRange.startContainer!==editor||savedRange.startOffset!==2||savedRange.endContainer!==editor||savedRange.endOffset!==3)throw Error('snapshot changed after mutation');");
        let event = input_event(scope, &state, target, "input", "deleteContentBackward", None, None).unwrap();
        let key = v8_string(scope, "inputUnderTest").unwrap();
        context.global(scope).set(scope, key.into(), event.into()).unwrap();
        eval(scope, "if(inputUnderTest.getTargetRanges().length!==0)throw Error('input exposed ranges')");
    }
    impl crate::clipboard::ClipboardBackend for Memory {
        fn read(&self) -> Result<ClipboardData, ClipboardError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn write(&self, data: ClipboardData) -> Result<(), ClipboardError> {
            data.validate()?;
            *self.0.lock().unwrap() = data;
            Ok(())
        }
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, code: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, code)
    }
    #[test]
    fn native_clipboard_requires_one_use_grant_and_dispatches_cancellable_defaults() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<input id=t value=abcd>")
                .into_runtime()
                .unwrap(),
        ));
        let target = {
            let dom = dom.borrow();
            dom.get_element_by_id(dom.document(), "t").unwrap().unwrap()
        };
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let backend = Arc::new(Memory::default());
        state.set_clipboard_backend(backend.clone());
        eval(
            scope,
            r#"globalThis.events=[];t.setSelectionRange(1,3);t.addEventListener('copy',e=>{events.push(e.isTrusted);globalThis.saved=e.clipboardData});"#,
        );
        let mut permission = grant(&state, target, ClipboardAction::Copy).unwrap();
        let result = action(scope, &state, &mut permission).unwrap();
        assert!(result.clipboard_changed);
        assert!(!result.content_changed);
        assert_eq!(backend.0.lock().unwrap().plain_text(), &[98, 99]);
        assert!(action(scope, &state, &mut permission).is_err());
        assert!(
            eval(
                scope,
                "events[0]===true && saved.types.length===0 && saved.getData('text')===''"
            )
            .is_true()
        );
        eval(
            scope,
            r#"t.addEventListener('cut',e=>{e.clipboardData.setData('text','override');e.preventDefault()},{once:true});"#,
        );
        let mut permission = grant(&state, target, ClipboardAction::Cut).unwrap();
        let result = action(scope, &state, &mut permission).unwrap();
        assert!(result.event_cancelled && result.clipboard_changed && !result.content_changed);
        assert_eq!(
            backend.0.lock().unwrap().plain_text(),
            &"override".encode_utf16().collect::<Vec<_>>()
        );
        assert!(eval(scope, "t.value==='abcd'").is_true());
        eval(
            scope,
            r#"t.addEventListener('beforeinput',e=>{events.push(e.inputType);if(e.dataTransfer!==null||e.getTargetRanges().length)throw Error('control input data');e.preventDefault()},{once:true});"#,
        );
        let mut permission = grant(&state, target, ClipboardAction::Paste).unwrap();
        let result = action(scope, &state, &mut permission).unwrap();
        assert!(!result.content_changed);
        assert!(
            eval(
                scope,
                "t.value==='abcd' && events.includes('insertFromPaste')"
            )
            .is_true()
        );
        let mut permission = grant(&state, target, ClipboardAction::Paste).unwrap();
        state.retire_execution();
        assert!(action(scope, &state, &mut permission).is_err());
        assert!(grant(&state, target, ClipboardAction::Copy).is_err());
    }
    #[test]
    fn rich_copy_uses_source_styles_for_partial_text_without_neighbor_leakage() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<div id=t contenteditable><i id=outside>excluded</i><b id=chosen>abcdef</b></div>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let (target, chosen) = {
            let dom = dom.borrow();
            (
                dom.get_element_by_id(dom.document(), "t").unwrap().unwrap(),
                dom.get_element_by_id(dom.document(), "chosen")
                    .unwrap()
                    .unwrap(),
            )
        };
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.layout.borrow_mut().styles.insert(
            node_key(chosen),
            Arc::new(BTreeMap::from([
                ("color".into(), "rgb(1, 2, 3)".into()),
                ("font-weight".into(), "700".into()),
            ])),
        );
        eval(
            scope,
            "const r=document.createRange();r.setStart(chosen.firstChild,1);r.setEnd(chosen.firstChild,4);getSelection().removeAllRanges();getSelection().addRange(r)",
        );
        let data = selected_data(&state, target).unwrap().unwrap();
        assert_eq!(data.plain_text(), &[98, 99, 100]);
        let html = data
            .items
            .iter()
            .find(|item| item.media_type == "text/html")
            .unwrap();
        let ClipboardPayload::Text(units) = &html.payload else {
            panic!("HTML payload")
        };
        let html = oryn_dom::dom_string::from_units(units);
        assert!(
            html.contains("rgb(1, 2, 3)")
                && html.contains("font-weight:700")
                && html.contains("bcd")
        );
        assert!(!html.contains("excluded") && !html.contains("abcdef"));
    }
    #[test]
    fn clipboard_wire_store_roundtrip_preserves_files_and_lone_surrogates() {
        let data = ClipboardData {
            items: vec![
                ClipboardItem {
                    media_type: "text/plain".into(),
                    payload: ClipboardPayload::Text(vec![0xd800, 0, 10]),
                },
                ClipboardItem {
                    media_type: "application/octet-stream".into(),
                    payload: ClipboardPayload::File {
                        name: "exact.bin".into(),
                        last_modified: 7,
                        bytes: vec![0, 255, 1],
                    },
                },
            ],
        };
        assert_eq!(to_data(&from_data(&data).unwrap()).unwrap(), data);
    }
    #[test]
    fn sanitizer_removes_active_content_but_retains_rich_formatting() {
        let html=sanitized_html("<b style='color: red' onclick='steal()'>safe</b><script>steal()</script><a href='java&#10;script:steal()'>link</a><iframe srcdoc='bad'></iframe>").unwrap();
        assert!(
            html.contains("<b")
                && html.contains("color: red")
                && html.contains("safe")
                && html.contains("link")
        );
        assert!(
            !html.contains("onclick")
                && !html.contains("script")
                && !html.contains("iframe")
                && !html.contains("srcdoc")
        );
    }
}

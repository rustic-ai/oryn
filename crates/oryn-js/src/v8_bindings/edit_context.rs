//! Authoritative native EditContext platform object. Never stores JS text algorithms.
use super::*;
use super::edit_context_buffer::EditContextBuffer;
mod prepared_bounds { include!("edit_context_bounds.rs"); }
pub(super) struct Record {
    pub buffer: EditContextBuffer,
    text_lease: Option<std::sync::Arc<dyn Send + Sync>>,
    bounds: prepared_bounds::PreparedBounds,
    marked: Option<crate::composition::TextRange>,
    composing: bool,
    revision: u64,
    composition_before: Option<(Vec<u16>, u32, u32, Option<std::sync::Arc<dyn Send + Sync>>)>,
    composition_revision: Option<u64>,
}

fn reserve(owner: &V8BindingState, bytes: usize) -> Result<Option<std::sync::Arc<dyn Send + Sync>>, V8BindingError> {
    let allocation = owner.retained_allocation.borrow().clone();
    allocation.map(|allocation|
        allocation.reserve(bytes).map_err(V8BindingError::EventCallback)).transpose()
}

fn incompatible() -> V8BindingError {
    V8BindingError::Unsupported { interface: "EditContext".into(), member: "TypeError".into() }
}
fn resolve(scope: &mut v8::PinScope, object: v8::Local<v8::Object>)
    -> Result<Rc<RefCell<Record>>, V8BindingError> {
    if object.is_proxy() || object.internal_field_count() < 2 { return Err(incompatible()); }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
    let id = stateful_id(scope, object)?;
    let wrapper = owner.platform_wrappers.borrow().get(&id).cloned();
    if !wrapper.and_then(|weak| weak.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(object.into())) { return Err(incompatible()); }
    let entry = platform_object(&owner, id)?;
    match &*entry { PlatformObjectRecord::EditContext(record) => Ok(record.clone()), _ => Err(incompatible()) }
}
fn member<'s>(scope: &mut v8::PinScope<'s, '_>, options: Option<v8::Local<v8::Object>>, name: &str)
    -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match options { Some(options) => {
        let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("EditContext dictionary key"))?;
        options.get(scope, key.into()).ok_or(V8BindingError::JavaScriptException)
    }, None => Ok(v8::undefined(scope).into()) }
}
fn units(scope: &mut v8::PinScope, owner: &V8BindingState, value: v8::Local<v8::Value>) -> Result<(Vec<u16>, Option<std::sync::Arc<dyn Send + Sync>>), V8BindingError> {
    let string = value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?;
    if owner.execution_retired() || scope.is_execution_terminating() { return Err(incompatible()); }
    let _entry = owner.enter_retained_allocation(scope)?;
    let lease = reserve(owner, string.length().checked_mul(2).ok_or(V8BindingError::Allocation("EditContext text size"))?)?;
    let mut units = Vec::new();
    units.try_reserve_exact(string.length()).map_err(|_| V8BindingError::Allocation("EditContext text"))?;
    units.resize(string.length(), 0);
    string.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
    Ok((units, lease))
}
pub(super) fn construct(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, args: &v8::FunctionCallbackArguments)
    -> Result<(), V8BindingError> {
    let value = args.get(0);
    let options = if value.is_null_or_undefined() { None } else {
        Some(v8::Local::<v8::Object>::try_from(value).map_err(|_| incompatible())?)
    };
    // WebIDL dictionary member order is lexical, not the order of the Rust fields.
    let end = member(scope, options, "selectionEnd")?;
    let end = if end.is_undefined() { 0 } else { end.uint32_value(scope).ok_or(V8BindingError::JavaScriptException)? };
    let start = member(scope, options, "selectionStart")?;
    let start = if start.is_undefined() { 0 } else { start.uint32_value(scope).ok_or(V8BindingError::JavaScriptException)? };
    let text = member(scope, options, "text")?;
    let (text, text_lease) = if text.is_undefined() { (Vec::new(), None) } else { units(scope, owner, text)? };
    if owner.next_platform_object_id.get() > 9_007_199_254_740_991 {
        return Err(V8BindingError::Allocation("EditContext identity exhausted"));
    }
    let id = register_platform_object(owner, PlatformObjectRecord::EditContext(Rc::new(RefCell::new(Record {
        buffer: EditContextBuffer::new(text, start, end),
        text_lease,
        bounds: prepared_bounds::PreparedBounds::default(),
        marked: None,
        composing: false,
        revision: 0,
        composition_before: None,
        composition_revision: None,
    }))));
    bind_platform_object(scope, owner, id, args.this());
    Ok(())
}
pub(super) fn get<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<v8::Object>, name: &str)
    -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = resolve(scope, object)?;
    let record = record.borrow();
    if name == "text" {
        return v8::String::new_from_two_byte(scope, record.buffer.text(), v8::NewStringType::Normal)
            .map(Into::into).ok_or(V8BindingError::Allocation("EditContext text result"));
    }
    let value = match name {
        "selectionStart" => record.buffer.selection_start,
        "selectionEnd" => record.buffer.selection_end,
        "characterBoundsRangeStart" => record.buffer.character_bounds_start,
        _ => return Err(incompatible()),
    };
    Ok(v8::Integer::new_from_unsigned(scope, value).into())
}
pub(super) fn call<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<v8::Object>, name: &str,
    args: &v8::FunctionCallbackArguments<'s>) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let record = resolve(scope, object)?;
    if name == "attachedElements" {
        let key = private_key(scope, "oryn.ec.host")?;
        let attached = object.get_private(scope, key).filter(|value| value.is_object());
        let array = v8::Array::new(scope, i32::from(attached.is_some()));
        if let Some(host) = attached {
            if array.set_index(scope, 0, host) != Some(true) {
                return Err(V8BindingError::Allocation("EditContext attachment result publication"));
            }
        }
        return Ok(Some(array.into()));
    }
    if name == "characterBounds" {
        let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
        let length = record.borrow().bounds.values().len();
        let array = v8::Array::new(scope, i32::try_from(length)
            .map_err(|_| V8BindingError::Allocation("EditContext bounds result length"))?);
        for index in 0..length {
            if owner.execution_retired() || scope.is_execution_terminating() {
                return Err(V8BindingError::Allocation("EditContext bounds result cancelled"));
            }
            let [x,y,width,height] = record.borrow().bounds.values()[index];
            let rect = dom_rect_object(scope, BindingRect { x, y, width, height })?;
            if array.set_index(scope, index as u32, rect.into()) != Some(true) {
                return Err(V8BindingError::Allocation("EditContext bounds result publication"));
            }
        }
        return Ok(Some(array.into()));
    }
    if matches!(name, "updateControlBounds" | "updateSelectionBounds") {
        if args.length() < 1 { return Err(incompatible()); }
        let bounds = bounds(scope, args.get(0))?;
        let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
        if owner.execution_retired() || scope.is_execution_terminating() { return Err(incompatible()); }
        if name == "updateControlBounds" { record.borrow_mut().buffer.control_bounds = bounds; }
        else { record.borrow_mut().buffer.selection_bounds = bounds; }
        return Ok(Some(v8::undefined(scope).into()));
    }
    if args.length() < 2 { return Err(incompatible()); }
    let start = args.get(0).uint32_value(scope).ok_or(V8BindingError::JavaScriptException)?;
    if name == "updateCharacterBounds" {
        let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
        let sequence = args.get(1).to_object(scope).ok_or_else(incompatible)?;
        let key = v8::Symbol::get_iterator(scope);
        let method = sequence.get(scope, key.into()).ok_or(V8BindingError::JavaScriptException)?;
        let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| incompatible())?;
        let mut converted = prepared_bounds::PreparedBounds::default();
        iterable_conversion::consume(scope, sequence, method, |scope, value| {
            if owner.execution_retired() || scope.is_execution_terminating() { return Err(V8BindingError::Allocation("EditContext bounds owner cancelled")); }
            let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| incompatible())?;
            if object.is_proxy() { return Err(incompatible()); }
            let components = rect_components(scope, object).ok_or_else(incompatible)?;
            if components.get_index(scope, 4).ok_or(V8BindingError::JavaScriptException)?.boolean_value(scope) { return Err(incompatible()); }
            let mut value = [0.; 4];
            for (index, component) in value.iter_mut().enumerate() {
                *component = components.get_index(scope, index as u32).and_then(|value| value.number_value(scope)).ok_or_else(incompatible)?;
            }
            if owner.execution_retired() || scope.is_execution_terminating() {
                return Err(V8BindingError::Allocation("EditContext bounds conversion cancelled"));
            }
            let _entry = owner.enter_retained_allocation(scope)?;
            converted.push(value, |bytes| reserve(&owner,bytes))?;
            Ok(())
        })?;
        if owner.execution_retired() || scope.is_execution_terminating() { return Err(V8BindingError::Allocation("EditContext bounds owner retired")); }
        let mut record = record.borrow_mut();
        record.buffer.character_bounds_start = start;
        let old_bounds = std::mem::replace(&mut record.bounds, converted);
        drop(record);
        drop(old_bounds);
        return Ok(Some(v8::undefined(scope).into()));
    }
    let end = args.get(1).uint32_value(scope).ok_or(V8BindingError::JavaScriptException)?;
    match name {
        "updateSelection" => {
            let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
            if owner.execution_retired() || scope.is_execution_terminating() { return Err(incompatible()); }
            let mut record = record.borrow_mut();
            let revision = record.revision.checked_add(1).ok_or(V8BindingError::Allocation("EditContext revision exhausted"))?;
            record.buffer.update_selection(start, end);
            record.revision = revision;
            let obsolete_before = record.composition_before.take();
            record.composition_revision = None;
            drop(record);
            drop(obsolete_before);
        }
        "updateText" => {
            if args.length() < 3 { return Err(incompatible()); }
            let owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
            let (text, _scratch) = units(scope, &owner, args.get(2))?;
            if owner.execution_retired() || scope.is_execution_terminating() { return Err(incompatible()); }
            let _entry = owner.enter_retained_allocation(scope)?;
            let (expected, size) = { let record = record.borrow(); (record.revision, record.buffer.text().len().checked_add(text.len())
                .and_then(|length| length.checked_mul(2)).ok_or(V8BindingError::Allocation("EditContext replacement size"))?) };
            let revision = expected.checked_add(1).ok_or(V8BindingError::Allocation("EditContext revision exhausted"))?;
            let lease = reserve(&owner, size)?;
            let mut record = record.borrow_mut();
            if record.revision != expected || owner.execution_retired() { return Err(incompatible()); }
            record.buffer.update_text(start, end, &text)
                .map_err(|_| V8BindingError::Allocation("EditContext replacement"))?;
            let old_lease = std::mem::replace(&mut record.text_lease, lease);
            record.revision = revision;
            let obsolete_before = record.composition_before.take();
            record.composition_revision = None;
            drop(record);
            drop(old_lease);
            drop(obsolete_before);
        }
        _ => return Err(incompatible()),
    }
    Ok(Some(v8::undefined(scope).into()))
}

fn bounds(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<[f64; 4], V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| incompatible())?;
    if object.is_proxy() { return Err(incompatible()); }
    let components = rect_components(scope, object).ok_or_else(incompatible)?;
    if components.get_index(scope, 4).ok_or(V8BindingError::JavaScriptException)?.boolean_value(scope) { return Err(incompatible()); }
    let mut result = [0.; 4];
    for (index, value) in result.iter_mut().enumerate() {
        *value = components.get_index(scope, index as u32).and_then(|value| value.number_value(scope)).ok_or_else(incompatible)?;
    }
    Ok(result)
}

fn private_key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str) -> Result<v8::Local<'s, v8::Private>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("EditContext attachment key"))?;
    Ok(v8::Private::for_api(scope, Some(key)))
}
pub(super) fn attached<'s>(scope: &mut v8::PinScope<'s, '_>, host: v8::Local<v8::Object>) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = private_key(scope, "oryn.ec.context")?;
    let value = host.get_private(scope, key).ok_or(V8BindingError::JavaScriptException)?;
    Ok(if value.is_undefined() { v8::null(scope).into() } else { value })
}
pub(super) fn attach(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, node: NodeId, value: v8::Local<v8::Value>) -> Result<(), V8BindingError> {
    let host = wrap_node(scope, owner, node)?;
    let context = if value.is_null() { None } else {
        let allowed = {
            let dom = owner.dom.borrow();
            matches!(&dom.arena().get(node)?.kind, NodeKind::Element { name }
                if name.ns.as_ref() == "http://www.w3.org/1999/xhtml"
                && (matches!(name.local.as_ref(), "article" | "aside" | "blockquote" | "body" | "canvas" | "div" | "footer" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "main" | "nav" | "p" | "section" | "span")
                    || oryn_dom::valid_custom_element_name(name.local.as_ref())))
        };
        if !allowed { return Err(V8BindingError::Unsupported { interface: "HTMLElement".into(), member: "NotSupportedError".into() }); }
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| incompatible())?;
        resolve(scope, object)?;
        let key = private_key(scope, "oryn.ec.host")?;
        let previous = object.get_private(scope, key).ok_or(V8BindingError::JavaScriptException)?;
        if previous.is_object() && !previous.strict_equals(host.into()) {
            return Err(V8BindingError::Unsupported { interface: "HTMLElement".into(), member: "NotSupportedError".into() });
        }
        Some(object)
    };
    let old = attached(scope, host)?;
    if old.strict_equals(value) { return Ok(()); }
    // Prepare all allocating keys and roots before touching existing edges.
    let host_key = private_key(scope, "oryn.ec.host")?;
    let context_key = private_key(scope, "oryn.ec.context")?;
    let old_object = v8::Local::<v8::Object>::try_from(old).ok();
    if let Some(context) = context {
        if context.set_private(scope, host_key, host.into()) != Some(true) {
            return Err(V8BindingError::Allocation("EditContext attachment edge"));
        }
    }
    if host.set_private(scope, context_key, value) != Some(true) {
        if let Some(context) = context {
            if context.delete_private(scope, host_key) != Some(true) {
                return Err(V8BindingError::Allocation("EditContext attachment rollback"));
            }
        }
        return Err(V8BindingError::Allocation("EditContext host edge"));
    }
    if let Some(old_object) = old_object {
        if old_object.delete_private(scope, host_key) != Some(true) {
            if host.set_private(scope, context_key, old) != Some(true) {
                return Err(V8BindingError::Allocation("EditContext host rollback"));
            }
            if let Some(context) = context {
                if context.delete_private(scope, host_key) != Some(true) {
                    return Err(V8BindingError::Allocation("EditContext attachment rollback"));
                }
            }
            return Err(V8BindingError::Allocation("EditContext previous attachment edge"));
        }
    }
    Ok(())
}

fn effective<'s>(scope: &mut v8::PinScope<'s, '_>, owner: &Rc<V8BindingState>, target: NodeId)
    -> Result<Option<(NodeId, v8::Local<'s, v8::Object>)>, V8BindingError> {
    let mut candidate = Some(target);
    let mut effective = None;
    while let Some(node) = candidate {
        let (element, parent, barrier) = {
            let dom = owner.dom.borrow();
            let record = dom.arena().get(node)?;
            (matches!(record.kind, NodeKind::Element { .. }), dom.flat_tree_parent(node)?,
                record.attributes.get_null("contenteditable").is_some_and(|value| value.eq_ignore_ascii_case("false")))
        };
        if element {
            let control = { let dom = owner.dom.borrow();
                ["input", "textarea", "select", "button"].into_iter().any(|name| dom.is_element(node, "http://www.w3.org/1999/xhtml", name)) };
            if control { return Ok(None); }
            if barrier { break; }
            let host = wrap_node(scope, owner, node)?;
            let context = attached(scope, host)?;
            if let Ok(context) = v8::Local::<v8::Object>::try_from(context) { effective = Some((node, context)); }
        }
        candidate = parent;
    }
    Ok(effective)
}

/// Existing authenticated OS operation route calls this before DOM composition.
/// Session state is owned by the native EditContext record, never an author grant.
pub(super) struct CompositionCursor {
    phase: CompositionPhase,
    host: NodeId,
    object: v8::Global<v8::Object>,
    context_owner: Rc<V8BindingState>,
    notification: Option<native_platform::publication::Notification>,
    revision: u64,
    format_origin: Option<u32>,
    command: Option<native_keyboard::TextCommandReceipt>,
    command_done: bool,
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CompositionPhase { Start, Edit, Format, AfterEdit, Snapshot }
impl CompositionCursor {
    pub(super) fn new(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId) -> Result<Option<Self>, V8BindingError> {
        let Some((host, object)) = effective(scope, owner, target)? else { return Ok(None); };
        let context_owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
        let revision = resolve(scope, object)?.borrow().revision;
        Ok(Some(Self { phase: CompositionPhase::Start, host, object: v8::Global::new(scope, object), context_owner, notification: None, revision, format_origin: None, command: None, command_done: false }))
    }
    fn resume(&mut self, scope: &mut v8::PinScope) -> Result<bool, V8BindingError> {
        if let Some(notification) = self.notification.as_mut() {
            if !notification.resume(scope, &self.context_owner)? { return Ok(false); }
            notification.finish(scope, &self.context_owner);
            self.notification = None;
        }
        Ok(true)
    }
}
impl Drop for CompositionCursor {
    fn drop(&mut self) { if let Some(notification) = &self.notification { notification.cancel(&self.context_owner); } }
}
pub(super) fn composition(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId,
    operation: &crate::composition::Operation, formats: Option<&[crate::composition::NativeTextFormat]>, cursor: &mut CompositionCursor) -> Result<Option<(crate::composition::StateSnapshot, Option<std::sync::Arc<dyn Send + Sync>>)>, V8BindingError> {
    use crate::composition::{Operation, StateSnapshot, TextRange};
    let host = cursor.host;
    let object = v8::Local::new(scope, &cursor.object);
    let fresh = effective(scope, owner, target)?;
    if owner.execution_retired() || cursor.context_owner.execution_retired()
        || !owner.dom.borrow().is_connected(host)?
        || !fresh.is_some_and(|(fresh_host, fresh_context)| fresh_host == host && fresh_context.strict_equals(object.into())) {
        return Err(incompatible());
    }
    if let Some(revision) = cursor.command.as_ref().and_then(|command|command.edit_context_revision()) { cursor.revision = revision; }
    if resolve(scope, object)?.borrow().revision != cursor.revision { return Err(incompatible()); }
    if !cursor.resume(scope)? { return Ok(None); }
    let read = Operation::Read;
    let operation = if let Operation::Command(command) = operation {
        if !cursor.command_done {
            if cursor.command.is_none() { cursor.command = Some(native_keyboard::TextCommandReceipt::new(owner.clone(), target, command.clone())?); }
            if !cursor.command.as_mut().unwrap().poll(scope)? { return Ok(None); }
            cursor.command_done = true;
            cursor.revision = resolve(scope, object)?.borrow().revision;
        }
        &read
    } else { operation };
    if !operation.validate() { return Err(incompatible()); }
    let record = resolve(scope, object)?;
    let context_owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
    let expected = cursor.revision;
    if record.borrow().revision != expected { return Err(incompatible()); }
    let was_composing = record.borrow().composing;
    let (replacement, selection, marked) = match operation {
        Operation::SetMarked { text, selected, replacement } => (Some((text.as_slice(), *replacement)), Some(*selected), true),
        Operation::SetMarkedWithFormats { .. } => return Err(incompatible()),
        Operation::Commit { text, replacement } => (Some((text.as_slice(), *replacement)), None, false),
        Operation::Read => (None, None, record.borrow().composing),
        Operation::Finish { keep_marked: true } => (None, None, record.borrow().composing),
        Operation::Unmark | Operation::Cancel | Operation::Finish { keep_marked: false } => (None, None, false),
        Operation::Command(_) => return Err(V8BindingError::Allocation("EditContext command receipt normalization")),
    };
    if cursor.phase == CompositionPhase::Start && marked && !record.borrow().composing {
        let _entry = context_owner.enter_retained_allocation(scope)?;
        let (expected_before, length) = { let record = record.borrow(); (record.revision, record.buffer.text().len()) };
        let lease = reserve(&context_owner, length.checked_mul(2).ok_or(V8BindingError::Allocation("EditContext composition before size"))?)?;
        let before = {
            let record = record.borrow();
            if record.revision != expected_before || context_owner.execution_retired() { return Err(incompatible()); }
            (record.buffer.text().to_vec(), record.buffer.selection_start, record.buffer.selection_end, lease)
        };
        let event = create_event_object_with_interface(scope, &context_owner, "Event", "compositionstart", false, false)?;
        let event_identity = event_id(scope, event)?;
        context_owner.events.borrow_mut().get_mut(&event_identity).ok_or_else(incompatible)?.is_trusted = true;
        let receiver_id = stateful_id(scope, object)?;
        let notification = native_platform::publication::pending_platform_dispatch(scope, &context_owner, receiver_id, "compositionstart", event);
        if record.borrow().revision != expected_before || context_owner.execution_retired() { return Err(incompatible()); }
        let old_before = { let mut record = record.borrow_mut();
            record.composing = true;
            record.composition_before.replace(before) };
        drop(old_before);
        cursor.notification = Some(notification);
        cursor.phase = CompositionPhase::Edit;
        if !cursor.resume(scope)? { return Ok(None); }
    }
    cursor.phase = cursor.phase.max(CompositionPhase::Edit);
    let fresh = effective(scope, owner, target)?;
    if owner.execution_retired() || !owner.dom.borrow().is_connected(host)? || record.borrow().revision != expected
        || !fresh.is_some_and(|(fresh_host, context)| fresh_host == host && context.strict_equals(object.into())) {
        return Err(incompatible());
    }
    if marked { record.borrow_mut().composing = true; }
    let context_owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
    let _retained_scope = context_owner.enter_retained_allocation(scope)?;
    if let Some((text, range)) = replacement.filter(|_| cursor.phase == CompositionPhase::Edit) {
        let range = range.or(record.borrow().marked).unwrap_or_else(|| {
            let (start, end) = record.borrow().buffer.input_range(None);
            TextRange { start, length: end - start }
        });
        let end = range.end().ok_or_else(incompatible)?;
        let (range_start, end) = { let record = record.borrow(); let limit = record.buffer.text().len() as u32; (range.start.min(limit), end.min(limit)) };
        let (revision, bytes) = { let record = record.borrow(); (
            record.revision.checked_add(1).ok_or(V8BindingError::Allocation("EditContext revision exhausted"))?,
            record.buffer.text().len().checked_add(text.len()).and_then(|value| value.checked_mul(2)).ok_or(V8BindingError::Allocation("EditContext composition size"))?) };
        let lease = reserve(&context_owner, bytes)?;
        if context_owner.execution_retired() || record.borrow().revision != expected { return Err(incompatible()); }
        let selected = selection.unwrap_or(TextRange { start: text.len() as u32, length: 0 });
        let start = range_start.checked_add(selected.start).ok_or_else(incompatible)?;
        let end_selection = start.checked_add(selected.length).ok_or_else(incompatible)?;
        let context_owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
        let event = create_event_object_with_interface(scope, &context_owner, "TextUpdateEvent", "textupdate", false, false)?;
        set_update_payload(scope, event, range_start, end, text, start, end_selection)?;
        let event_identity = event_id(scope, event)?;
        context_owner.events.borrow_mut().get_mut(&event_identity).ok_or_else(incompatible)?.is_trusted = true;
        let receiver_id = stateful_id(scope, object)?;
        let notification = native_platform::publication::pending_platform_dispatch(scope, &context_owner, receiver_id, "textupdate", event);
        let mut record_mut = record.borrow_mut();
        if record_mut.revision != expected || context_owner.execution_retired() { return Err(incompatible()); }
        record_mut.buffer.update_text(range_start, end, text).map_err(|_| V8BindingError::Allocation("EditContext composition replacement"))?;
        record_mut.buffer.update_selection(start, end_selection);
        let old_lease = std::mem::replace(&mut record_mut.text_lease, lease);
        record_mut.marked = marked.then_some(TextRange { start: range_start, length: text.len() as u32 });
        record_mut.revision = revision;
        cursor.revision = revision;
        cursor.format_origin = Some(range_start);
        record_mut.composition_revision = Some(revision);
        drop(record_mut);
        drop(old_lease);
        cursor.notification = Some(notification);
        cursor.phase = CompositionPhase::Format;
        if !cursor.resume(scope)? { return Ok(None); }
        if record.borrow().revision != revision || context_owner.execution_retired() {
            return Err(incompatible());
        }
    }
    cursor.phase = cursor.phase.max(CompositionPhase::Format);
    if cursor.phase == CompositionPhase::Format {
        if let Some(formats) = formats {
            let fresh=effective(scope,owner,target)?;
            if owner.execution_retired() || context_owner.execution_retired()
                || !owner.dom.borrow().is_connected(host)?
                || record.borrow().revision!=cursor.revision
                || !fresh.is_some_and(|(fresh_host,fresh_object)|fresh_host==host&&fresh_object.strict_equals(object.into())) { return Err(incompatible()) }
            let origin = cursor.format_origin.ok_or(V8BindingError::Allocation("native format origin"))?;
            let event = create_event_object_with_interface(scope,&context_owner,"TextFormatUpdateEvent","textformatupdate",false,false)?;
            edit_context_events::publish_native_formats(scope,&context_owner,event,origin,formats)?;
            let identity = event_id(scope,event)?;
            context_owner.events.borrow_mut().get_mut(&identity).ok_or_else(incompatible)?.is_trusted=true;
            let receiver_id=stateful_id(scope,object)?;
            let notification=native_platform::publication::pending_platform_dispatch(scope,&context_owner,receiver_id,"textformatupdate",event);
            if record.borrow().revision!=cursor.revision || context_owner.execution_retired(){return Err(incompatible())}
            cursor.notification=Some(notification);
        }
        cursor.phase=CompositionPhase::AfterEdit;
        if !cursor.resume(scope)? { return Ok(None); }
    }
    if cursor.phase == CompositionPhase::AfterEdit && matches!(operation, Operation::Cancel) {
        let length = { let record = record.borrow();
            if record.composition_revision == Some(record.revision) { record.composition_before.as_ref().map(|before| before.0.len()) } else { None } };
        let restoration_lease = length.map(|length| reserve(&context_owner, length.checked_mul(4).ok_or(V8BindingError::Allocation("EditContext cancel restoration and scratch"))?)).transpose()?;
        let before = {
            let record = record.borrow();
            if record.composition_revision == Some(record.revision) { record.composition_before.clone() }
            else { None }
        };
        if let Some((text, start, end, lease)) = before {
            let mut record = record.borrow_mut();
            let revision = record.revision.checked_add(1).ok_or(V8BindingError::Allocation("EditContext revision exhausted"))?;
            let old_length = record.buffer.text().len() as u32;
            record.buffer.update_text(0, old_length, &text).map_err(|_| V8BindingError::Allocation("EditContext cancel restoration"))?;
            record.buffer.update_selection(start, end);
            let old_lease = std::mem::replace(&mut record.text_lease, restoration_lease.flatten());
            record.revision = revision;
            cursor.revision = revision;
            let obsolete_before = record.composition_before.take();
            drop(record);
            drop(old_lease);
            drop(lease);
            drop(obsolete_before);
        }
    }
    if cursor.phase == CompositionPhase::AfterEdit && was_composing && !marked {
        let event = create_event_object_with_interface(scope, &context_owner, "Event", "compositionend", false, false)?;
        let event_identity = event_id(scope, event)?;
        context_owner.events.borrow_mut().get_mut(&event_identity).ok_or_else(incompatible)?.is_trusted = true;
        let receiver_id = stateful_id(scope, object)?;
        let notification = native_platform::publication::pending_platform_dispatch(scope, &context_owner, receiver_id, "compositionend", event);
        let obsolete_before = { let mut record = record.borrow_mut();
            record.composing = false;
            record.marked = None;
            record.composition_revision = None;
            record.composition_before.take() };
        drop(obsolete_before);
        cursor.notification = Some(notification);
        cursor.phase = CompositionPhase::Snapshot;
        if !cursor.resume(scope)? { return Ok(None); }
    }
    if cursor.phase == CompositionPhase::AfterEdit { record.borrow_mut().composing = marked; }
    cursor.phase = cursor.phase.max(CompositionPhase::Snapshot);
    let fresh = effective(scope, owner, target)?;
    if owner.execution_retired() || context_owner.execution_retired()
        || !fresh.is_some_and(|(fresh_host, fresh_context)| fresh_host == host && fresh_context.strict_equals(object.into())) {
        return Err(incompatible());
    }
    let (snapshot_revision, snapshot_length) = { let record = record.borrow(); (record.revision, record.buffer.text().len()) };
    let _entry = context_owner.enter_retained_allocation(scope)?;
    let lease = reserve(&context_owner, snapshot_length.checked_mul(4).ok_or(V8BindingError::Allocation("EditContext composition snapshot and receipt copy"))?)?;
    let record = record.borrow();
    if record.revision != snapshot_revision || context_owner.execution_retired() { return Err(incompatible()); }
    Ok(Some((StateSnapshot { mutation_revision: record.revision, aborted: false, protected: false,
        text: record.buffer.text().to_vec(), selection: TextRange { start: record.buffer.selection_start.min(record.buffer.selection_end), length: record.buffer.selection_start.abs_diff(record.buffer.selection_end) }, marked: record.marked }, lease)))
}

/// Called after trusted keypress and before DOM/control editing. Callback changes
/// to attachment/focus/selection are observed before the buffer mutation.
pub(super) struct KeyboardJob {
    target: NodeId,
    host: NodeId,
    object: v8::Global<v8::Object>,
    key: String,
    before: Option<clipboard_actions::ClipboardNotification>,
    checked: bool,
    committed: bool,
    notification: Option<native_platform::publication::Notification>,
    context_owner: Option<Rc<V8BindingState>>,
    deletion_unit: Option<crate::composition::TextUnit>,
    committed_revision: Option<u64>,
    done: bool,
}
impl KeyboardJob {
    pub(super) fn new(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId, key: &str) -> Result<Option<Self>, V8BindingError> {
        let Some((host, object)) = effective(scope, owner, target)? else { return Ok(None); };
        Ok(Some(Self { target, host, object: v8::Global::new(scope, object), key: key.into(), before: None, checked: false, committed: false, notification: None, context_owner: None, deletion_unit: None, committed_revision: None, done: false }))
    }
    pub(super) fn with_delete_unit(mut self, unit: crate::composition::TextUnit) -> Self { self.deletion_unit = Some(unit); self }
    pub(super) fn committed_revision(&self) -> Option<u64> { self.committed_revision }
    fn current(&self, scope: &mut v8::PinScope, owner: &Rc<V8BindingState>) -> Result<bool,V8BindingError> {
        let object=v8::Local::new(scope,&self.object);
        Ok(!owner.execution_retired() && owner.dom.borrow().is_connected(self.host)?
            && !binding_state_for_receiver(scope,object).ok_or_else(incompatible)?.execution_retired()
            && effective(scope,owner,self.target)?.is_some_and(|(host,context)|host==self.host&&context.strict_equals(object.into())))
    }
    pub(super) fn cancel(&mut self, _owner: &V8BindingState) {
        self.done=true;
        if let Some(event) = self.before.as_mut() { event.cancel(); }
        if let (Some(notification), Some(context)) = (&self.notification, &self.context_owner) { notification.cancel(context); }
    }
    pub(super) fn poll(&mut self, scope: &mut v8::PinScope, owner: &Rc<V8BindingState>) -> Result<Option<bool>, V8BindingError> {
        if self.done { return Ok(Some(true)); }
        if !self.current(scope,owner)? { self.cancel(owner); return Ok(Some(true)); }
        if self.committed {
            let object=v8::Local::new(scope,&self.object);
            if self.committed_revision!=Some(resolve(scope,object)?.borrow().revision) { self.cancel(owner); return Ok(Some(true)); }
            if let (Some(notification), Some(context)) = (self.notification.as_mut(), self.context_owner.as_ref()) {
                if !notification.resume(scope, context)? { return Ok(None); }
                notification.finish(scope, context);
                self.notification = None;
            }
            return Ok(Some(true));
        }
        let Some((host, _)) = effective(scope, owner, self.target)? else { return Ok(Some(true)); };
        let (kind, data) = match self.key.as_str() { "Backspace" => ("deleteContentBackward", ""), "Delete" => ("deleteContentForward", ""), "Enter" => ("insertLineBreak", "\n"), _ => ("insertText", self.key.as_str()) };
        if !self.checked {
            if self.before.is_none() { self.before = Some(clipboard_actions::ClipboardNotification::editing(scope, owner, host, "beforeinput", kind, Some(data), None)?); }
            let Some(accepted) = self.before.as_mut().unwrap().poll(scope, owner)? else { return Ok(None); };
            self.checked = true;
            if !accepted { self.done=true; return Ok(Some(true)); }
        }
        if !self.current(scope,owner)? { self.cancel(owner); return Ok(Some(true)); }
        let key = std::mem::take(&mut self.key);
        let result = keyboard_commit(scope, owner, self.target, &key, Some(self));
        self.key = key;
        result
    }
}
pub(super) fn keyboard(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId, key: &str)
    -> Result<bool, V8BindingError> {
    let Some((host, object)) = effective(scope, owner, target)? else { return Ok(false); };
    let (kind, replacement) = match key {
        "Backspace" => ("deleteContentBackward", ""),
        "Delete" => ("deleteContentForward", ""),
        "Enter" => ("insertLineBreak", "\n"),
        _ => ("insertText", key),
    };
    if !clipboard_actions::fire_editing(scope, owner, host, "beforeinput", kind, Some(replacement), None)? { return Ok(true); }
    if owner.execution_retired() || !owner.dom.borrow().is_connected(host)? { return Ok(true); }
    let Some((fresh_host, fresh)) = effective(scope, owner, target)? else { return Ok(true); };
    if fresh_host != host || !fresh.strict_equals(object.into()) { return Ok(true); }
    keyboard_commit(scope, owner, target, key, None).map(|result| result.unwrap_or(true))
}
fn keyboard_commit(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId, key: &str, job: Option<&mut KeyboardJob>) -> Result<Option<bool>, V8BindingError> {
    let Some((_, fresh)) = effective(scope, owner, target)? else { return Ok(Some(true)); };
    let (replacement, deletion) = match key { "Backspace" => ("", Some(true)), "Delete" => ("", Some(false)), "Enter" => ("\n", None), _ => (key, None) };
    let context_owner = binding_state_for_receiver(scope, fresh).ok_or_else(incompatible)?;
    let _retained_scope = context_owner.enter_retained_allocation(scope)?;
    let record = resolve(scope, fresh)?;
    let expected_input_revision = record.borrow().revision;
    let replacement_size = replacement.encode_utf16().count().checked_mul(2)
        .ok_or(V8BindingError::Allocation("EditContext input size"))?;
    let _scratch = reserve(&context_owner, replacement_size)?;
    let segmentation_bytes = record.borrow().buffer.text().len().checked_mul(3).ok_or_else(incompatible)?;
    let _segmentation = reserve(&context_owner, segmentation_bytes)?;
    let replacement: Vec<u16> = replacement.encode_utf16().collect();
    let (mut start, mut end) = record.borrow().buffer.input_range(deletion);
    if let (Some(backward), Some(unit)) = (deletion, job.as_ref().and_then(|job| job.deletion_unit)) {
        let record = record.borrow();
        if record.buffer.selection_start == record.buffer.selection_end {
            let caret = record.buffer.selection_start.min(record.buffer.text().len() as u32);
            let boundary = if unit == crate::composition::TextUnit::Paragraph { paragraph_step(&record,caret,backward)? } else if unit == crate::composition::TextUnit::Line { geometry_next_line(&record, caret, backward)? } else if unit == crate::composition::TextUnit::LineBoundary { geometry_line_boundary(&record, caret, backward)? } else { command_boundary(&record.buffer, caret, backward, unit)? };
            start = caret.min(boundary); end = caret.max(boundary);
        }
    }
    let context_owner = binding_state_for_receiver(scope, fresh).ok_or_else(incompatible)?;
    let caret = start.checked_add(replacement.len() as u32).ok_or(V8BindingError::Allocation("EditContext input selection"))?;
    let event = create_event_object_with_interface(scope, &context_owner, "TextUpdateEvent", "textupdate", false, false)?;
    set_update_payload(scope, event, start, end, &replacement, caret, caret)?;
    let id = event_id(scope, event)?;
    context_owner.events.borrow_mut().get_mut(&id).ok_or_else(incompatible)?.is_trusted = true;
    let receiver_id = stateful_id(scope, fresh)?;
    let notification = native_platform::publication::pending_platform_dispatch(scope, &context_owner, receiver_id, "textupdate", event);
    let (expected_revision, size) = { let record = record.borrow(); (record.revision,
        record.buffer.text().len().checked_add(replacement.len()).and_then(|value| value.checked_mul(2)).ok_or(V8BindingError::Allocation("EditContext input allocation size"))?) };
    let lease = reserve(&context_owner, size)?;
    if let Some(job)=job.as_ref() { if !job.current(scope,owner)? { return Ok(Some(true)); } }
    let committed_revision;
    {
        let mut record = record.borrow_mut();
        if record.revision != expected_revision || record.revision != expected_input_revision || context_owner.execution_retired() { return Err(incompatible()); }
        let revision = record.revision.checked_add(1).ok_or(V8BindingError::Allocation("EditContext revision exhausted"))?;
        let caret = start.checked_add(replacement.len() as u32).ok_or(V8BindingError::Allocation("EditContext input selection"))?;
        record.buffer.update_text(start, end, &replacement).map_err(|_| V8BindingError::Allocation("EditContext native input"))?;
        let old_lease = std::mem::replace(&mut record.text_lease, lease);
        record.buffer.update_selection(caret, caret);
        record.revision = revision;
        committed_revision = Some(revision);
        let obsolete_before = record.composition_before.take();
        record.composition_revision = None;
        drop(record);
        drop(old_lease);
        drop(obsolete_before);
    }
    if let Some(job) = job {
        job.notification = Some(notification);
        job.context_owner = Some(context_owner);
        job.committed = true;
        job.committed_revision = committed_revision;
        return job.poll(scope, owner);
    }
    let mut notification = notification;
    if !notification.resume(scope, &context_owner)? { return Ok(None); }
    notification.finish(scope, &context_owner);
    Ok(Some(true))
}

fn command_boundary(buffer: &EditContextBuffer, caret: u32, backward: bool, unit: crate::composition::TextUnit) -> Result<u32, V8BindingError> {
    use crate::composition::TextUnit;
    use unicode_segmentation::UnicodeSegmentation;
    let bytes = buffer.text().len().checked_mul(3).ok_or_else(incompatible)?;
    let mut text = String::new();
    text.try_reserve_exact(bytes).map_err(|_| V8BindingError::Allocation("EditContext segmentation scratch"))?;
    for scalar in char::decode_utf16(buffer.text().iter().copied()) { text.push(scalar.unwrap_or(char::REPLACEMENT_CHARACTER)); }
    let limit = buffer.text().len() as u32;
    if unit == TextUnit::DocumentBoundary { return Ok(if backward { 0 } else { limit }); }
    if unit == TextUnit::ParagraphBoundary {
        let (start,end)=paragraph_edges(buffer.text(),caret);
        return Ok(if backward {start}else{end});
    }
    if unit == TextUnit::Character {
        let mut offset=0u32;
        for cluster in text.graphemes(true) {
            let next=offset.checked_add(cluster.encode_utf16().count() as u32).ok_or_else(incompatible)?;
            if backward && next>=caret { return Ok(offset); }
            if !backward && next>caret { return Ok(next); }
            offset=next;
        }
        return Ok(if backward { offset.min(caret) } else { limit });
    }
    let mut offset = 0u32;
    let mut previous = 0u32;
    for segment in text.split_word_bounds() {
        let segment_start=offset;
        let length = segment.encode_utf16().count() as u32;
        offset = offset.checked_add(length).ok_or_else(incompatible)?;
        let eligible = match unit {
            TextUnit::Word => segment.chars().any(|value| !value.is_whitespace()),
            TextUnit::Paragraph | TextUnit::ParagraphBoundary => segment.contains('\n') || segment.contains('\r'),
            TextUnit::Character => true,
            TextUnit::Line | TextUnit::LineBoundary => return Err(V8BindingError::Unsupported { interface: "EditContext".into(), member: "visual line requires authored character geometry".into() }),
            TextUnit::DocumentBoundary => unreachable!(),
        };
        if eligible {
            if backward && offset >= caret { return Ok(segment_start.min(caret)); }
            if !backward && offset > caret { return Ok(offset); }
            previous = segment_start;
        }
    }
    Ok(if backward { previous.min(caret) } else { limit })
}
fn geometry_caret(record: &Record, caret: u32) -> Result<(usize,[f64;4]), V8BindingError> {
    let start=record.buffer.character_bounds_start;
    let values=record.bounds.values();
    let index=caret.checked_sub(start).ok_or(V8BindingError::Allocation("EditContext uncovered caret"))? as usize;
    if values.is_empty() || index>values.len() { return Err(V8BindingError::Allocation("EditContext uncovered caret")); }
    let mut rect=*values.get(index.min(values.len()-1)).ok_or_else(incompatible)?;
    if index==values.len() {
        let rtl=values.len()>1 && values[values.len()-2][1]==rect[1] && values[values.len()-2][0]>rect[0];
        if !rtl { rect[0]+=rect[2]; }
    }
    if !rect.iter().all(|value|value.is_finite()) { return Err(incompatible()); }
    Ok((index,rect))
}
fn geometry_line_boundary(record: &Record, caret: u32, backward: bool) -> Result<u32, V8BindingError> {
    let start = record.buffer.character_bounds_start;
    let values = record.bounds.values();
    let (index,current) = geometry_caret(record,caret)?;
    if !current.iter().all(|value| value.is_finite()) { return Err(incompatible()); }
    let mut selected = index.min(values.len()-1);
    if backward {
        while selected > 0 && values[selected-1][1] == current[1] { selected -= 1; }
    } else {
        while selected+1 < values.len() && values[selected+1][1] == current[1] { selected += 1; }
        selected += 1;
    }
    start.checked_add(selected as u32).ok_or_else(incompatible)
}
fn geometry_next_line(record: &Record, caret: u32, backward: bool) -> Result<u32, V8BindingError> {
    let start = record.buffer.character_bounds_start;
    let values = record.bounds.values();
    let (_,current) = geometry_caret(record,caret)?;
    let mut row: Option<f64> = None;
    for rect in values {
        if !rect.iter().all(|value| value.is_finite()) { return Err(incompatible()); }
        if (backward && rect[1] < current[1]) || (!backward && rect[1] > current[1]) {
            if row.is_none_or(|old| (rect[1]-current[1]).abs() < (old-current[1]).abs()) { row=Some(rect[1]); }
        }
    }
    let Some(row) = row else { return Ok(caret); };
    let mut selected = None;
    for (index, rect) in values.iter().enumerate().filter(|(_, rect)| rect[1] == row) {
        let distance=(rect[0]-current[0]).abs();
        if selected.is_none_or(|(_, old)| distance < old) { selected=Some((index,distance)); }
    }
    start.checked_add(selected.ok_or_else(incompatible)?.0 as u32).ok_or_else(incompatible)
}
fn geometry_horizontal(record: &Record, caret: u32, left: bool) -> Result<u32, V8BindingError> {
    let start=record.buffer.character_bounds_start;
    let values=record.bounds.values();
    let (_,current)=geometry_caret(record,caret)?;
    if !current.iter().all(|value| value.is_finite()) { return Err(incompatible()); }
    let mut selected=None;
    for (index, rect) in values.iter().enumerate() {
        if !rect.iter().all(|value| value.is_finite()) { return Err(incompatible()); }
        if rect[1]!=current[1] || (left && rect[0]>=current[0]) || (!left && rect[0]<=current[0]) { continue; }
        let distance=(rect[0]-current[0]).abs();
        if selected.is_none_or(|(_,old)|distance<old){selected=Some((index,distance));}
    }
    let end=start.checked_add(values.len() as u32).ok_or_else(incompatible)?;
    let (_,edge)=geometry_caret(record,end)?;
    if edge[1]==current[1] && ((left && edge[0]<current[0]) || (!left && edge[0]>current[0])) {
        let distance=(edge[0]-current[0]).abs();
        if selected.is_none_or(|(_,old)|distance<old){return Ok(end);}
    }
    selected.map_or(Ok(caret),|(index,_)|start.checked_add(index as u32).ok_or_else(incompatible))
}
pub(super) fn command_move(scope: &mut v8::PinScope, owner: &Rc<V8BindingState>, target: NodeId, direction: crate::composition::TextDirection, unit: crate::composition::TextUnit, extend: bool) -> Result<bool, V8BindingError> {
    let Some((_, object)) = effective(scope, owner, target)? else { return Ok(false); };
    let record = resolve(scope, object)?;
    let context_owner = binding_state_for_receiver(scope, object).ok_or_else(incompatible)?;
    let _entry = context_owner.enter_retained_allocation(scope)?;
    let (expected, bytes) = { let record = record.borrow(); (record.revision, record.buffer.text().len().checked_mul(3).ok_or_else(incompatible)?) };
    let _scratch = reserve(&context_owner, bytes)?;
    let backward = matches!(direction, crate::composition::TextDirection::Left | crate::composition::TextDirection::Backward);
    let mut record = record.borrow_mut();
    if record.revision != expected || context_owner.execution_retired() { return Err(incompatible()); }
    let caret = record.buffer.selection_end.min(record.buffer.text().len() as u32);
    let backward = if matches!(direction, crate::composition::TextDirection::Left | crate::composition::TextDirection::Right) && unit != crate::composition::TextUnit::Character {
        let (_, rect) = geometry_caret(&record, caret)?;
        let index = caret.checked_sub(record.buffer.character_bounds_start).ok_or_else(incompatible)? as usize;
        let values = record.bounds.values();
        let next = values.get(index+1).filter(|next| next[1] == rect[1]).or_else(|| index.checked_sub(1).and_then(|previous|values.get(previous)).filter(|previous|previous[1]==rect[1]));
        let next = next.ok_or(V8BindingError::Allocation("EditContext visual direction bounds missing"))?;
        let forward_is_right = if index+1 < values.len() && values[index+1][1]==rect[1] { next[0]>rect[0] } else { rect[0]>next[0] };
        if direction == crate::composition::TextDirection::Left { forward_is_right } else { !forward_is_right }
    } else { backward };
    let next = if unit == crate::composition::TextUnit::Paragraph { paragraph_step(&record,caret,backward)? } else if matches!(direction, crate::composition::TextDirection::Left | crate::composition::TextDirection::Right) && unit == crate::composition::TextUnit::Character { geometry_horizontal(&record, caret, backward)? } else if unit == crate::composition::TextUnit::Line { geometry_next_line(&record, caret, backward)? } else if unit == crate::composition::TextUnit::LineBoundary { geometry_line_boundary(&record, caret, backward)? } else { command_boundary(&record.buffer, caret, backward, unit)? };
    let revision = record.revision.checked_add(1).ok_or_else(incompatible)?;
    let anchor = if extend { record.buffer.selection_start } else { next };
    record.buffer.update_selection(anchor, next);
    record.revision = revision;
    let obsolete = record.composition_before.take();
    record.composition_revision = None;
    drop(record); drop(obsolete);
    Ok(true)
}
fn set_update_payload(scope: &mut v8::PinScope, event: v8::Local<v8::Object>, start: u32, end: u32,
    text: &[u16], selection_start: u32, selection_end: u32) -> Result<(), V8BindingError> {
    let key = private_key(scope, "oryn.ec.textupdate")?;
    let payload = v8::Object::new(scope);
    let null = v8::null(scope);
    if payload.set_prototype(scope, null.into()) != Some(true) { return Err(V8BindingError::Allocation("TextUpdateEvent payload")); }
    for (name, value) in [("updateRangeStart", start), ("updateRangeEnd", end), ("selectionStart", selection_start), ("selectionEnd", selection_end)] {
        let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("TextUpdateEvent key"))?;
        let value = v8::Integer::new_from_unsigned(scope, value);
        if payload.create_data_property(scope, name.into(), value.into()) != Some(true) { return Err(V8BindingError::Allocation("TextUpdateEvent field")); }
    }
    let name = v8::String::new(scope, "text").ok_or(V8BindingError::Allocation("TextUpdateEvent text key"))?;
    let text = v8::String::new_from_two_byte(scope, text, v8::NewStringType::Normal).ok_or(V8BindingError::Allocation("TextUpdateEvent text"))?;
    if payload.create_data_property(scope, name.into(), text.into()) != Some(true)
        || event.set_private(scope, key, payload.into()) != Some(true) { return Err(V8BindingError::Allocation("TextUpdateEvent payload publication")); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
#[test]
fn edit_context_native_buffer_attachment_and_trusted_keyboard_controls() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, "globalThis.editor=document.getElementById('editor');globalThis.ec=new EditContext({text:'abcd',selectionStart:2,selectionEnd:3});editor.editContext=ec;editor.focus();globalThis.updates=[];ec.addEventListener('textupdate',e=>updates.push([e.text,e.updateRangeStart,e.updateRangeEnd,e.isTrusted]));").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let input = NativeKeyboardInput { key: "Z".encode_utf16().collect(), down: true, ..Default::default() };
    assert!(dispatch_native_keyboard(scope, &state, &input).unwrap());
    let source = v8::String::new(scope, "ec.text==='abZd'&&editor.textContent===''&&JSON.stringify(updates)==='[[\"Z\",2,3,true]]'").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_native_composition_receipt_preserves_geometry_and_callback_edits() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, "globalThis.ec=new EditContext({text:'abcd',selectionStart:1,selectionEnd:3});const host=document.getElementById('editor');host.editContext=ec;host.focus();ec.updateCharacterBounds(1,[new DOMRect(1,2,3,4)]);globalThis.events=[];ec.addEventListener('textupdate',e=>events.push(e.isTrusted));").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let target = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "editor").unwrap().unwrap() };
    let marked = crate::composition::Operation::SetMarked { text: vec![b'X' as u16], selected: crate::composition::TextRange { start: 1, length: 0 }, replacement: None };
    let mut receipt = native_composition::NativeComposition::new(&state, target, &marked).unwrap();
    let result = receipt.poll(scope).unwrap().unwrap();
    assert_eq!(result.text, vec![b'a' as u16, b'X' as u16, b'd' as u16]);
    drop(receipt);
    let mut cancel = native_composition::NativeComposition::new(&state, target, &crate::composition::Operation::Cancel).unwrap();
    assert_eq!(cancel.poll(scope).unwrap().unwrap().text, "abcd".encode_utf16().collect::<Vec<_>>());
    drop(cancel);
    let source = v8::String::new(scope, "ec.characterBounds()[0].width===3&&events.every(v=>v===true)").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_native_inheritance_uses_outer_context_and_false_barriers() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=outer><div id=inner tabindex=0></div><div contenteditable=false><div id=isolated tabindex=0></div></div><input id=control></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, "globalThis.outer=document.getElementById('outer');globalThis.inner=document.getElementById('inner');globalThis.isolated=document.getElementById('isolated');outer.editContext=new EditContext();inner.editContext=new EditContext();isolated.editContext=new EditContext();inner.focus();").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let input = NativeKeyboardInput { key: vec![b'a' as u16], down: true, ..Default::default() };
    assert!(dispatch_native_keyboard(scope, &state, &input).unwrap());
    let source = v8::String::new(scope, "outer.editContext.text==='a'&&inner.editContext.text===''").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
    let source = v8::String::new(scope, "isolated.focus()").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    assert!(dispatch_native_keyboard(scope, &state, &input).unwrap());
    let source = v8::String::new(scope, "isolated.editContext.text==='a'&&outer.editContext.text==='a'").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
    let control = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "control").unwrap().unwrap() };
    assert!(effective(scope, &state, control).unwrap().is_none());
}

#[test]
fn edit_context_composition_cursor_rejects_callback_revision_without_replaying_text() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, "globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.count=0;ec.addEventListener('textupdate',()=>{count++;ec.updateSelection(0,0)});").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let target = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "editor").unwrap().unwrap() };
    let operation = crate::composition::Operation::Commit { text: vec![b'X' as u16], replacement: Some(crate::composition::TextRange { start: 1, length: 0 }) };
    let mut receipt = native_composition::NativeComposition::new(&state, target, &operation).unwrap();
    assert!(receipt.poll(scope).is_err());
    assert!(receipt.poll(scope).is_err());
    let source = v8::String::new(scope, "ec.text==='aXb'&&count===1&&ec.selectionStart===0&&ec.selectionEnd===0").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_composition_event_allocation_failure_keeps_start_for_retry() {
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let source = v8::String::new(scope, "globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.starts=0;globalThis.updates=0;ec.addEventListener('compositionstart',()=>starts++);ec.addEventListener('textupdate',()=>updates++);").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let target = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "editor").unwrap().unwrap() };
    let operation = crate::composition::Operation::SetMarked { text: vec![b'X' as u16], selected: crate::composition::TextRange { start: 1, length: 0 }, replacement: None };
    let mut receipt = native_composition::NativeComposition::new(&state, target, &operation).unwrap();
    // Exercise the actual native factory's missing-template allocation boundary.
    let template = state.instance_templates.borrow_mut().remove("Event").unwrap();
    assert!(receipt.poll(scope).is_err());
    let source = v8::String::new(scope, "ec.text==='ab'&&starts===0&&updates===0").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
    state.instance_templates.borrow_mut().insert("Event".into(), template);
    state.set_execution_suspended(true);
    assert!(receipt.poll(scope).unwrap().is_none());
    state.set_execution_suspended(false);
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text, "aXb".encode_utf16().collect::<Vec<_>>());
    let source = v8::String::new(scope, "starts===1&&updates===1").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_composition_suspension_after_commit_resumes_notification_suffix_once() {
    fn park(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
        scope.get_current_context().get_slot::<V8BindingState>().unwrap().set_execution_suspended(true);
    }
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let callback = v8::Function::new(scope, park).unwrap();
    let global = context.global(scope);
    set_object_property(scope, global, "park", callback.into()).unwrap();
    let source = v8::String::new(scope, "globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.calls=[];ec.addEventListener('textupdate',()=>{calls.push('first');park()});ec.addEventListener('textupdate',()=>calls.push('suffix'));").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let target = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "editor").unwrap().unwrap() };
    let operation = crate::composition::Operation::Commit { text: vec![b'X' as u16], replacement: Some(crate::composition::TextRange { start: 1, length: 0 }) };
    let mut receipt = native_composition::NativeComposition::new(&state, target, &operation).unwrap();
    assert!(receipt.poll(scope).unwrap().is_none());
    state.set_execution_suspended(false);
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text, "aXb".encode_utf16().collect::<Vec<_>>());
    let source = v8::String::new(scope, "ec.text==='aXb'&&calls.join('|')==='first|suffix'").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_command_boundaries_preserve_utf16_and_authored_geometry_coverage() {
    use crate::composition::TextUnit;
    let buffer=EditContextBuffer::new(vec![b'a' as u16,0x301,0xd83d,0xde00,0xd800,b' ' as u16,b'b' as u16],0,0);
    assert_eq!(command_boundary(&buffer,1,false,TextUnit::Character).unwrap(),2);
    assert_eq!(command_boundary(&buffer,4,true,TextUnit::Character).unwrap(),2);
    assert_eq!(command_boundary(&buffer,4,false,TextUnit::Character).unwrap(),5);
    let mut bounds=prepared_bounds::PreparedBounds::default();
    bounds.push([20.,10.,5.,8.], |_|Ok(None)).unwrap();
    bounds.push([10.,10.,5.,8.], |_|Ok(None)).unwrap();
    bounds.push([20.,20.,5.,8.], |_|Ok(None)).unwrap();
    let mut buffer=EditContextBuffer::new("abcde".encode_utf16().collect(),3,3);
    buffer.character_bounds_start=2;
    let record=Record { buffer,text_lease:None,bounds,marked:None,composing:false,revision:0,composition_before:None,composition_revision:None };
    assert!(geometry_caret(&record,1).is_err());
    assert!(geometry_caret(&record,6).is_err());
    assert_eq!(geometry_caret(&record,5).unwrap().1[0],25.);
    assert_eq!(geometry_horizontal(&record,2,true).unwrap(),3);
    assert_eq!(geometry_next_line(&record,2,false).unwrap(),4);
}

#[test]
fn native_composition_command_delete_uses_edit_context_not_dom() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,context);
    let shared=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,context,shared,RealmId(0),1).unwrap();
    let source=v8::String::new(scope,"globalThis.ec=new EditContext({text:'abc',selectionStart:3,selectionEnd:3});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.updates=0;ec.addEventListener('textupdate',()=>updates++);").unwrap();
    v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap();
    let target={let dom=state.dom.borrow();dom.get_element_by_id(dom.document(),"editor").unwrap().unwrap()};
    let operation=crate::composition::Operation::Command(crate::composition::TextCommand::Delete{backward:true,unit:crate::composition::TextUnit::Character});
    let mut receipt=native_composition::NativeComposition::new(&state,target,&operation).unwrap();
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text,"ab".encode_utf16().collect::<Vec<_>>());
    let source=v8::String::new(scope,"updates===1&&document.getElementById('editor').textContent===''").unwrap();
    assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
}

#[test]
fn edit_context_keyboard_beforeinput_replacement_does_not_edit_either_context() {
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,context);
    let shared=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,context,shared,RealmId(0),1).unwrap();
    let source=v8::String::new(scope,"globalThis.oldEC=new EditContext({text:'old'});globalThis.newEC=new EditContext({text:'new'});const h=document.getElementById('editor');h.editContext=oldEC;h.focus();h.addEventListener('beforeinput',()=>h.editContext=newEC);globalThis.updates=0;oldEC.addEventListener('textupdate',()=>updates++);newEC.addEventListener('textupdate',()=>updates++);").unwrap();
    v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap();
    let input=NativeKeyboardInput {key:vec![b'X' as u16],down:true,..Default::default()};
    let mut keyboard=begin_native_keyboard_receipt(state.clone(),&input).unwrap();
    assert_eq!(poll_native_keyboard_receipt(scope,&mut keyboard).unwrap(),Some(true));
    let source=v8::String::new(scope,"oldEC.text==='old'&&newEC.text==='new'&&updates===0&&document.getElementById('editor').textContent===''").unwrap();
    assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
}

}

#[cfg(test)]
mod command_park_controls { use super::*;
#[test]
fn edit_context_composition_command_suspension_resumes_owned_revision() {
    fn park(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
        scope.get_current_context().get_slot::<V8BindingState>().unwrap().set_execution_suspended(true);
    }
    super::super::tests::initialize_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    v8::scope!(let scope, &mut isolate);
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let shared = Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state = install_generated_bindings(scope, context, shared, RealmId(0), 1).unwrap();
    let callback = v8::Function::new(scope, park).unwrap();
    let global = context.global(scope);
    set_object_property(scope, global, "park", callback.into()).unwrap();
    let source = v8::String::new(scope, "globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.calls=[];ec.addEventListener('textupdate',()=>{calls.push('first');park()});ec.addEventListener('textupdate',()=>calls.push('suffix'));").unwrap();
    v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
    let target = { let dom = state.dom.borrow(); dom.get_element_by_id(dom.document(), "editor").unwrap().unwrap() };
    let operation = crate::composition::Operation::Command(crate::composition::TextCommand::Delete { backward: false, unit: crate::composition::TextUnit::Character });
    let mut receipt = native_composition::NativeComposition::new(&state, target, &operation).unwrap();
    assert!(receipt.poll(scope).unwrap().is_none());
    state.set_execution_suspended(false);
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text, "a".encode_utf16().collect::<Vec<_>>());
    let source = v8::String::new(scope, "ec.text==='a'&&calls.join('|')==='first|suffix'").unwrap();
    assert!(v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap().boolean_value(scope));
}

}

#[cfg(test)]
mod event_controls {
use super::*;
#[test]
fn native_edit_context_events_validate_dictionary_brand_and_foreign_lifetime(){
super::super::tests::initialize_v8();
let mut isolate=v8::Isolate::new(Default::default());
v8::scope!(let scope,&mut isolate);
let context=v8::Context::new(scope,Default::default());
let scope=&mut v8::ContextScope::new(scope,context);
let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body></body></html>").into_runtime().unwrap()));
let _state=install_generated_bindings(scope,context,dom,RealmId(0),1).unwrap();
let foreign_context=v8::Context::new(scope,Default::default());
let foreign_state={let child=&mut v8::ContextScope::new(scope,foreign_context);let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body></body></html>").into_runtime().unwrap()));install_generated_bindings(child,foreign_context,dom,RealmId(1),1).unwrap()};
let token=v8::String::new(scope,"https://example.test").unwrap();context.set_security_token(token.into());foreign_context.set_security_token(token.into());let foreign=foreign_context.global(scope);let global=context.global(scope);set_object_property(scope,global,"foreign",foreign.into()).unwrap();
let _foreign_state=foreign_state;
let source=v8::String::new(scope,r####"(() => {
  const check = (value, label) => { if (!value) throw Error(label); };
  const throws = (fn, type) => { try { fn(); } catch (e) { return e instanceof type; } return false; };
  check(throws(() => new TextFormat({underlineStyle:'Solid'}), TypeError), 'enum case');
  const format = new TextFormat({rangeStart:1, rangeEnd:3, underlineStyle:'wavy'});
  const original = [format];
  const event = new TextFormatUpdateEvent('textformatupdate', {textFormats:original});
  original.length = 0;
  const copy = event.getTextFormats();
  check(copy.length===1 && copy[0]===format, 'sequence copy identity');
  copy.length = 0;
  check(event.getTextFormats()[0]===format, 'fresh sequence output');
  for (const fake of [{}, Object.create(TextFormat.prototype), new Proxy(format,{})]) {
    check(throws(() => new TextFormatUpdateEvent('x',{textFormats:[fake]}),TypeError), 'format native brand');
  }
  let closed=0;
  const iterable={ [Symbol.iterator](){return {next(){return {done:false,value:{}}},return(){closed++;throw Error('close')}}} };
  check(throws(()=>new TextFormatUpdateEvent('x',{textFormats:iterable}),TypeError)&&closed===1,'close preserves conversion error');
  const order=[];const init={};
  for(const name of ['bubbles','cancelable','composed','selectionEnd','selectionStart','text','updateRangeEnd','updateRangeStart']) {
    Object.defineProperty(init,name,{get(){order.push(name);return undefined}});
  }
  const update=new TextUpdateEvent('textupdate',init);
  check(order.join(',')==='bubbles,cancelable,composed,selectionEnd,selectionStart,text,updateRangeEnd,updateRangeStart','lexical dictionary');
  check(update.text===''&&update.updateRangeStart===0&&!update.isTrusted,'defaults');
  const getter=Object.getOwnPropertyDescriptor(TextFormat.prototype,'rangeStart').get;
  check(throws(()=>getter.call(Object.create(TextFormat.prototype)),TypeError),'forged getter');
  check(throws(()=>getter.call(new Proxy(format,{})),TypeError),'proxy getter');
  const frame={contentWindow:globalThis.foreign};
  const foreign=new frame.contentWindow.TextFormat({rangeStart:7});
  const cross=new TextFormatUpdateEvent('x',{textFormats:[foreign]});
  check(cross.getTextFormats()[0]===foreign,'foreign native identity');

  check(cross.getTextFormats()[0].rangeStart===7,'detached foreign retention');
  return true;
})()
"####).unwrap();
assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
}
}

// Native hard paragraph boundaries. CRLF is one separator; offsets are UTF16.
fn paragraph_edges(units: &[u16], caret: u32) -> (u32, u32) {
    let caret=(caret as usize).min(units.len());
    let mut start=0;
    let mut index=0;
    while index<units.len() {
        if matches!(units[index],10|13) {
            let end=index;
            let after=if units[index]==13&&units.get(index+1)==Some(&10){index+2}else{index+1};
            if caret<=end { return (start as u32,end as u32); }
            if caret<after { return (start as u32,end as u32); }
            start=after; index=after;
        } else { index+=1; }
    }
    (start as u32,units.len() as u32)
}

fn paragraph_step(record: &Record, caret:u32, backward:bool)->Result<u32,V8BindingError>{
    let units=record.buffer.text();
    let (start,end)=paragraph_edges(units,caret);
    let next=if backward {
        if start==0{return Ok(caret)}
        paragraph_edges(units,start.saturating_sub(1))
    } else {
        if end as usize==units.len(){return Ok(caret)}
        let after=end+if units[end as usize]==13&&units.get(end as usize+1)==Some(&10){2}else{1};
        paragraph_edges(units,after)
    };
    // Paragraph movement preserves native inline position into the adjacent
    // paragraph. Bounds must cover both endpoints; never guess a wrapped line.
    let (_,current)=geometry_caret(record,caret)?;
    let mut chosen=None;
    let bytes=units.len().checked_mul(3).ok_or_else(incompatible)?;
    let mut text=String::new();
    text.try_reserve_exact(bytes).map_err(|_|V8BindingError::Allocation("EditContext paragraph grapheme projection"))?;
    for scalar in char::decode_utf16(units.iter().copied()){text.push(scalar.unwrap_or(char::REPLACEMENT_CHARACTER));}
    use unicode_segmentation::UnicodeSegmentation;
    let mut offset=0u32;
    // Same block-first ordering as selection_movement::next_line: the nearest
    // line in a different paragraph wins before inline-distance comparison.
    for cluster in text.graphemes(true).chain(std::iter::once("")) {
        let boundary=offset;
        offset=offset.checked_add(cluster.encode_utf16().count() as u32).ok_or_else(incompatible)?;
        if boundary<next.0||boundary>next.1 {continue;}
        let offset=boundary;
        let (_,rect)=geometry_caret(record,offset)?;
        let block=(rect[1]-current[1])*if backward{-1.}else{1.};
        if block<=0.5 {continue;}
        let inline=(rect[0]-current[0]).abs();
        if chosen.is_none_or(|(_,old_block,old_inline)|block<old_block||(block==old_block&&inline<old_inline)){chosen=Some((offset,block,inline));}
    }
    chosen.map(|(offset,_,_)|offset).ok_or_else(incompatible)
}

#[cfg(test)]
mod paragraph_controls {
    use super::*;
    #[test]
    fn hard_paragraph_edges_distinguish_separator_and_empty_paragraph(){
        let units="ab\r\n\ncd".encode_utf16().collect::<Vec<_>>();
        assert_eq!(paragraph_edges(&units,1),(0,2));
        assert_eq!(paragraph_edges(&units,3),(0,2));
        assert_eq!(paragraph_edges(&units,4),(4,4));
        assert_eq!(paragraph_edges(&units,6),(5,7));
    }
    #[test]
    fn paragraph_step_chooses_nearest_line_and_only_grapheme_boundaries(){
        let mut buffer=EditContextBuffer::new("a\n😀xy".encode_utf16().collect(),0,0);
        buffer.character_bounds_start=0;
        let mut bounds=prepared_bounds::PreparedBounds::default();
        for rect in [[8.,0.,4.,8.],[12.,0.,0.,8.],[0.,10.,8.,8.],[3.,10.,0.,8.],[9.,10.,4.,8.],[8.,20.,4.,8.]] {
            bounds.push(rect, |_|Ok(None)).unwrap();
        }
        let record=Record{buffer,text_lease:None,bounds,marked:None,composing:false,revision:0,composition_before:None,composition_revision:None};
        assert_eq!(paragraph_step(&record,0,false).unwrap(),4);
        assert_ne!(paragraph_step(&record,0,false).unwrap(),3);
        assert_eq!(paragraph_edges(record.buffer.text(),0),(0,1));
        assert_eq!(paragraph_edges(record.buffer.text(),4),(2,6));
    }
    #[test]
    fn paragraph_step_respects_nonzero_coverage_and_combining_cluster(){
        let mut buffer=EditContextBuffer::new("skip\na\ne\u{301}x".encode_utf16().collect(),5,5);
        buffer.character_bounds_start=5;
        let mut bounds=prepared_bounds::PreparedBounds::default();
        for rect in [[8.,0.,4.,8.],[12.,0.,0.,8.],[0.,10.,8.,8.],[8.,10.,0.,8.],[9.,10.,4.,8.]] {
            bounds.push(rect, |_|Ok(None)).unwrap();
        }
        let record=Record{buffer,text_lease:None,bounds,marked:None,composing:false,revision:0,composition_before:None,composition_revision:None};
        assert_eq!(paragraph_step(&record,5,false).unwrap(),9);
        assert_ne!(paragraph_step(&record,5,false).unwrap(),8);
        assert!(paragraph_step(&record,0,false).is_err());
        assert_eq!(paragraph_step(&record,9,true).unwrap(),5);
    }
}

#[test]
fn edit_context_native_format_factory_retry_does_not_replay_text_update(){
    super::super::tests::initialize_v8();
    let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);
    let context=v8::Context::new(scope,Default::default());
    let scope=&mut v8::ContextScope::new(scope,context);
    let shared=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,context,shared,RealmId(0),1).unwrap();
    let source=v8::String::new(scope,"globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.updates=0;globalThis.formats=0;globalThis.observed=null;ec.addEventListener('textupdate',()=>updates++);ec.addEventListener('textformatupdate',e=>{formats++;const f=e.getTextFormats()[0];observed=[e.isTrusted,e.target===ec,f.rangeStart,f.rangeEnd,f.underlineStyle,f.underlineThickness];});").unwrap();
    v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap();
    let target={let dom=state.dom.borrow();dom.get_element_by_id(dom.document(),"editor").unwrap().unwrap()};
    let operation=crate::composition::Operation::SetMarkedWithFormats{text:vec![b'X' as u16],selected:crate::composition::TextRange{start:1,length:0},replacement:None,formats:vec![crate::composition::NativeTextFormat{range:crate::composition::TextRange{start:0,length:1},style:crate::composition::NativeUnderlineStyle::Dotted,thickness:crate::composition::NativeUnderlineThickness::Thin}]};
    let mut receipt=native_composition::NativeComposition::new_scoped(scope,&state,target,&operation).unwrap();
    let template=state.instance_templates.borrow_mut().remove("TextFormatUpdateEvent").unwrap();
    assert!(receipt.poll(scope).is_err());
    let source=v8::String::new(scope,"ec.text==='aXb'&&updates===1&&formats===0").unwrap();
    assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
    state.instance_templates.borrow_mut().insert("TextFormatUpdateEvent".into(),template);
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text,"aXb".encode_utf16().collect::<Vec<_>>());
    let source=v8::String::new(scope,"updates===1&&formats===1&&JSON.stringify(observed)==='[true,true,1,2,\"dotted\",\"thin\"]'").unwrap();
    assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
}
#[test]
fn edit_context_native_format_callback_suspension_resumes_suffix_without_replacement(){
    fn park(scope:&mut v8::PinScope,_:v8::FunctionCallbackArguments,_:v8::ReturnValue){scope.get_current_context().get_slot::<V8BindingState>().unwrap().set_execution_suspended(true);}
    super::super::tests::initialize_v8();let mut isolate=v8::Isolate::new(Default::default());
    v8::scope!(let scope,&mut isolate);let context=v8::Context::new(scope,Default::default());let scope=&mut v8::ContextScope::new(scope,context);
    let shared=Rc::new(RefCell::new(oryn_dom::html::parse("<html><body><div id=editor tabindex=0></div></body></html>").into_runtime().unwrap()));
    let state=install_generated_bindings(scope,context,shared,RealmId(0),1).unwrap();
    let callback=v8::Function::new(scope,park).unwrap();let key=v8::String::new(scope,"park").unwrap();let global=context.global(scope);assert_eq!(global.set(scope,key.into(),callback.into()),Some(true));
    let source=v8::String::new(scope,"globalThis.ec=new EditContext({text:'ab',selectionStart:1,selectionEnd:1});const h=document.getElementById('editor');h.editContext=ec;h.focus();globalThis.updates=0;globalThis.first=0;globalThis.second=0;ec.addEventListener('textupdate',()=>updates++);ec.addEventListener('textformatupdate',()=>{first++;park()});ec.addEventListener('textformatupdate',()=>second++);").unwrap();v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap();
    let target={let dom=state.dom.borrow();dom.get_element_by_id(dom.document(),"editor").unwrap().unwrap()};
    let operation=crate::composition::Operation::SetMarkedWithFormats{text:vec![b'X' as u16],selected:crate::composition::TextRange{start:1,length:0},replacement:None,formats:vec![crate::composition::NativeTextFormat{range:crate::composition::TextRange{start:0,length:1},style:crate::composition::NativeUnderlineStyle::Solid,thickness:crate::composition::NativeUnderlineThickness::Thin}]};
    let mut receipt=native_composition::NativeComposition::new_scoped(scope,&state,target,&operation).unwrap();
    assert!(receipt.poll(scope).unwrap().is_none());state.set_execution_suspended(false);
    assert_eq!(receipt.poll(scope).unwrap().unwrap().text,"aXb".encode_utf16().collect::<Vec<_>>());
    let source=v8::String::new(scope,"updates===1&&first===1&&second===1&&ec.text==='aXb'").unwrap();assert!(v8::Script::compile(scope,source,None).unwrap().run(scope).unwrap().boolean_value(scope));
}

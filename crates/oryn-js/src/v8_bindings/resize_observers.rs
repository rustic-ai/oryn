//! Native ResizeObserver observations and depth-limited rendering broadcasts.
//! Callback batches are retained before invocation; author exceptions never replay them.
type ResizeMeasurement = (Entry, (f64, f64), usize, std::rc::Weak<V8BindingState>, u64);

type PendingResizeEntry = (Rc<RefCell<Target>>, Entry, (f64, f64));

use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxKind {
    Content,
    Border,
    Device,
}
struct Target {
    object: v8::Global<v8::Object>,
    kind: BoxKind,
    last: (f64, f64),
}
pub(super) struct Observer {
    object: Option<v8::Global<v8::Object>>,
    targets: Vec<Rc<RefCell<Target>>>,
}
#[derive(Clone)]
pub(super) struct Entry {
    target: Option<v8::Global<v8::Object>>,
    rect: BindingRect,
    content: (f64, f64),
    border: (f64, f64),
    device: (f64, f64),
}
struct Batch {
    observer: Rc<RefCell<Observer>>,
    entries: Vec<PendingResizeEntry>,
}

fn entry_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, "oryn.observer_entry.target")
        .ok_or(V8BindingError::Allocation("entry target key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    object
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)
}
fn set_entry_target(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    target: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, "oryn.observer_entry.target")
        .ok_or(V8BindingError::Allocation("entry target key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if object.set_private(scope, key, target.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
#[derive(Default)]
pub(super) struct State {
    observers: BTreeMap<u64, Rc<RefCell<Observer>>>,
    dirty: bool,
    dependencies: Vec<(std::rc::Weak<V8BindingState>, u64)>,
    pending: VecDeque<Batch>,
    identity: Option<(NodeId, u64, u64, u64)>,
    depth: usize,
    active: bool,
    skipped: bool,
    loop_error: Option<v8::Global<v8::Value>>,
    #[cfg(test)]
    fail_prepare: bool,
}
fn fail(message: &str) -> V8BindingError {
    V8BindingError::Message(format!("TypeError: {message}"))
}
pub(super) fn retire(state: &V8BindingState) {
    let mut native = state.resize_observers.borrow_mut();
    for observer in native.observers.values() {
        let mut observer = observer.borrow_mut();
        observer.targets.clear();
        observer.object = None;
    }
    native.observers.clear();
    native.pending.clear();
    native.dependencies.clear();
    native.loop_error = None;
    native.active = false;
    native.dirty = false;
    native.skipped = false;
    native.identity = None;
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let callback = v8::Local::<v8::Function>::try_from(args.get(0))
        .map_err(|_| fail("ResizeObserver callback must be callable"))?;
    // A callback may capture its observer. Retain it through a private JS edge,
    // allowing an inactive unreachable cycle to be collected with its wrapper.
    let key = v8::String::new(scope, "oryn.resize_observers.callback")
        .ok_or(V8BindingError::Allocation("observer callback key"))?;
    let key = v8::Private::for_api(scope, Some(key));
    if args.this().set_private(scope, key, callback.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    let record = Rc::new(RefCell::new(Observer {
        object: None,
        targets: Vec::new(),
    }));
    let id = register_platform_object(state, PlatformObjectRecord::ResizeObserver(record));
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn owner(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    let state = binding_state_for_receiver(scope, object)
        .ok_or_else(|| fail("ResizeObserver target must be an Element"))?;
    let node = argument_node(scope, &state, object.into())?;
    if !matches!(
        state.dom.borrow().arena().get(node)?.kind,
        NodeKind::Element { .. }
    ) {
        return Err(fail("ResizeObserver target must be an Element"));
    }
    Ok((state, node))
}
pub(super) fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    // Neither author option getters nor wrapper allocation may run while the
    // shared platform registry is borrowed.
    let observer = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::ResizeObserver(observer) = &*record else {
            return Err(fail("ResizeObserver receiver"));
        };
        observer.clone()
    };
    if member == "disconnect" {
        observer.borrow_mut().targets.clear();
        observer.borrow_mut().object = None;
        {
            let mut observers = state.resize_observers.borrow_mut();
            observers.observers.remove(&id);
            observers
                .pending
                .retain(|batch| !Rc::ptr_eq(&batch.observer, &observer));
        }
        return Ok(Some(v8::undefined(scope).into()));
    }
    let target = v8::Local::<v8::Object>::try_from(args.get(0))
        .map_err(|_| fail("ResizeObserver target must be an Element"))?;
    owner(scope, target)?;
    let mut kind = BoxKind::Content;
    if member == "observe" && !args.get(1).is_null_or_undefined() {
        let options = v8::Local::<v8::Object>::try_from(args.get(1))
            .map_err(|_| fail("ResizeObserver options must be a dictionary"))?;
        let key = v8::String::new(scope, "box")
            .ok_or(V8BindingError::Allocation("ResizeObserver option"))?;
        let value = options
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_undefined() {
            let value = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope);
            kind = match value.as_str() {
                "content-box" => BoxKind::Content,
                "border-box" => BoxKind::Border,
                "device-pixel-content-box" => BoxKind::Device,
                _ => return Err(fail("invalid ResizeObserver box")),
            };
        }
    }
    // Author conversion may adopt the target; resolve its current private owner.
    owner(scope, target)?;
    let existing = observer
        .borrow()
        .targets
        .iter()
        .position(|item| v8::Local::new(scope, &item.borrow().object) == target);
    if member == "unobserve" {
        if let Some(index) = existing {
            let removed = observer.borrow_mut().targets.remove(index);
            remove_active(state, &removed);
        }
        if observer.borrow().targets.is_empty() {
            observer.borrow_mut().object = None;
            state.resize_observers.borrow_mut().observers.remove(&id);
        }
        return Ok(Some(v8::undefined(scope).into()));
    }
    if member != "observe" {
        return Ok(None);
    }
    if let Some(index) = existing {
        let item = observer.borrow().targets[index].clone();
        if item.borrow().kind == kind {
            return Ok(Some(v8::undefined(scope).into()));
        }
    }
    {
        let observers = state.resize_observers.borrow();
        if !observers.observers.contains_key(&id) && observers.observers.len() >= 1024 {
            return Err(V8BindingError::Allocation("ResizeObserver capacity"));
        }
    }
    {
        let mut record = observer.borrow_mut();
        if existing.is_none() && record.targets.len() >= 4096 {
            return Err(V8BindingError::Allocation("ResizeObserver target capacity"));
        }
        record
            .targets
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("ResizeObserver target"))?;
    }
    let replacement = Rc::new(RefCell::new(Target {
        object: v8::Global::new(scope, target),
        kind,
        last: (0., 0.),
    }));
    let wrapper = v8::Global::new(scope, object);
    let removed = {
        let mut record = observer.borrow_mut();
        let removed = existing.map(|index| record.targets.remove(index));
        record.object = Some(wrapper);
        record.targets.push(replacement);
        removed
    };
    if let Some(removed) = removed {
        remove_active(state, &removed);
    }
    if state.execution_retired() {
        let mut observer = observer.borrow_mut();
        observer.targets.clear();
        observer.object = None;
        return Ok(Some(v8::undefined(scope).into()));
    }
    {
        let mut observers = state.resize_observers.borrow_mut();
        observers.observers.insert(id, observer.clone());
        observers.dirty = true;
    }
    Ok(Some(v8::undefined(scope).into()))
}
fn remove_active(state: &V8BindingState, target: &Rc<RefCell<Target>>) {
    let mut observers = state.resize_observers.borrow_mut();
    for batch in &mut observers.pending {
        batch
            .entries
            .retain(|(entry, _, _)| !Rc::ptr_eq(entry, target));
    }
    observers.pending.retain(|batch| !batch.entries.is_empty());
}
fn sample(scope: &mut v8::PinScope, target: &Target) -> Result<ResizeMeasurement, V8BindingError> {
    let object = v8::Local::new(scope, &target.object);
    let (state, node) = owner(scope, object)?;
    let mut rect = BindingRect::default();
    let mut content = (0., 0.);
    let mut border = (0., 0.);
    let mut physical = BindingRect::default();
    let mut vertical = false;
    let mut depth = 0;
    let mut svg_zoom = 1.;
    // Depth remains meaningful for a retained detached/retired target. A zero
    // box must pass the initial depth threshold to publish disappearance once.
    {
        let dom = state.dom.borrow();
        let mut cursor = Some(node);
        while let Some(node) = cursor {
            depth += 1;
            cursor = dom.flat_tree_parent(node)?;
            if depth > 65536 {
                return Err(V8BindingError::Allocation("ResizeObserver depth"));
            }
        }
    }
    if !state.execution_retired() && !state.execution_suspended() {
        let dom = state.dom.borrow();
        if dom.is_connected(node)? {
            let layout = state.layout.borrow();
            let style = layout.styles.get(&node_key(node));
            let replaced = [
                "img", "input", "textarea", "select", "button", "video", "audio", "canvas",
                "iframe", "object", "embed",
            ]
            .into_iter()
            .any(|name| dom.is_element(node, "http://www.w3.org/1999/xhtml", name));
            let html = matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml");
            let svg = matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg");
            let svg_without_css_box=svg&&(!dom.is_element(node,"http://www.w3.org/2000/svg","svg")||dom.flat_tree_parent(node)?.is_some_and(|parent|dom.arena().get(parent).is_ok_and(|node|matches!(&node.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()!="foreignObject"))));
            if svg_without_css_box {
                svg_zoom = style
                    .and_then(|style| style.get("oryn-effective-zoom"))
                    .and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value > 0.)
                    .unwrap_or(1.);
            }
            let inline = html
                && !replaced
                && style
                    .and_then(|style| style.get("display"))
                    .is_some_and(|display| display == "inline");
            if !inline {
                if let Some(projection) = layout.hit_projections.get(&node_key(node)) {
                    let model = &projection.box_model;
                    physical = BindingRect {
                        x: model.content_box.x,
                        y: model.content_box.y,
                        width: model.content_box.width,
                        height: model.content_box.height,
                    };
                    rect = BindingRect {
                        x: if svg_without_css_box {
                            model.content_box.x
                        } else {
                            model.content_box.x - model.padding_box.x
                        },
                        y: if svg_without_css_box {
                            model.content_box.y
                        } else {
                            model.content_box.y - model.padding_box.y
                        },
                        width: model.content_box.width,
                        height: model.content_box.height,
                    };
                    content = (rect.width, rect.height);
                    border = (model.border_box.width, model.border_box.height);
                } else if let Some(BindingGeometry::Available(value)) =
                    layout.geometry.get(&node_key(node))
                {
                    physical = *value;
                    rect = *value;
                    if !svg_without_css_box {
                        rect.x = 0.;
                        rect.y = 0.;
                    }
                    content = (rect.width, rect.height);
                    border = content;
                }
            }
            if style
                .and_then(|style| style.get("writing-mode"))
                .is_some_and(|mode| mode.starts_with("vertical") || mode.starts_with("sideways"))
            {
                vertical = true;
                content = (content.1, content.0);
                border = (border.1, border.0);
            }
        }
    }
    let ratio = state
        .layout_provider
        .borrow()
        .as_ref()
        .map_or(1., |provider| provider.native_device_pixel_ratio());
    let ratio = ratio * svg_zoom;
    let device = (
        ((physical.x + physical.width) * ratio).round() - (physical.x * ratio).round(),
        ((physical.y + physical.height) * ratio).round() - (physical.y * ratio).round(),
    );
    let device = if vertical {
        (device.1, device.0)
    } else {
        device
    };
    let selected = match target.kind {
        BoxKind::Content => content,
        BoxKind::Border => border,
        BoxKind::Device => device,
    };
    Ok((
        Entry {
            target: Some(target.object.clone()),
            rect,
            content,
            border,
            device,
        },
        selected,
        depth,
        Rc::downgrade(&state),
        state.native_paint_revision()?,
    ))
}
fn platform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    name: &'static str,
    record: PlatformObjectRecord,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = register_platform_object(state, record);
    match new_platform_object(scope, state, name, id) {
        Ok(value) => Ok(value),
        Err(error) => {
            state.platform_objects.borrow_mut().remove(&id);
            state.platform_wrappers.borrow_mut().remove(&id);
            Err(error)
        }
    }
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = stateful_id(scope, object)?;
    let entry = {
        let record = platform_object(state, id)?;
        match &*record {
            PlatformObjectRecord::ResizeObserverSize(size) => {
                let value = if member == "inlineSize" {
                    size.0
                } else {
                    size.1
                };
                drop(record);
                return Ok(v8::Number::new(scope, value).into());
            }
            PlatformObjectRecord::ResizeObserverEntry(entry) => entry.clone(),
            _ => return Err(fail("ResizeObserver entry receiver")),
        }
    };
    match member {
        "target" => entry_target(scope, object),
        "contentRect" => {
            let template = state
                .instance_templates
                .borrow()
                .get("DOMRectReadOnly")
                .and_then(|template| template.to_global(scope))
                .ok_or(V8BindingError::MissingInterface("DOMRectReadOnly"))?;
            let object = v8::Local::new(scope, &template)
                .new_instance(scope)
                .ok_or(V8BindingError::Allocation("ResizeObserver rect"))?;
            initialize_rect_components(
                scope,
                object,
                [
                    entry.rect.x,
                    entry.rect.y,
                    entry.rect.width,
                    entry.rect.height,
                ],
                true,
            )?;
            Ok(object.into())
        }
        "contentBoxSize" | "borderBoxSize" | "devicePixelContentBoxSize" => {
            let size = match member {
                "contentBoxSize" => entry.content,
                "borderBoxSize" => entry.border,
                _ => entry.device,
            };
            let object = platform(
                scope,
                state,
                "ResizeObserverSize",
                PlatformObjectRecord::ResizeObserverSize(size),
            )?;
            let values = [object.into()];
            let array = v8::Array::new_with_elements(scope, &values);
            array
                .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                .ok_or(V8BindingError::Allocation("ResizeObserver sizes"))?;
            Ok(array.into())
        }
        _ => Err(fail("ResizeObserverEntry member")),
    }
}
/// Returns true when a broadcast ran and layout must be recomputed. A false
/// result completes this rendering opportunity, including skipped-target error.
pub(super) fn broadcast(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    if !state.document_rendering_allowed() || scope.is_execution_terminating() {
        return Ok(false);
    }
    let identity = (
        state.document_root(),
        state.document_generation(),
        state.document_stream_epoch(),
        state.document_authority_epoch(),
    );
    {
        let mut observers = state.resize_observers.borrow_mut();
        if observers.identity != Some(identity) {
            observers.pending.clear();
            observers.loop_error = None;
            observers.active = false;
            observers.identity = Some(identity);
        }
    }
    if !state.resize_observers.borrow().active {
        let mut observers = state.resize_observers.borrow_mut();
        observers.depth = 0;
        observers.skipped = false;
        observers.active = true;
    }
    if state.resize_observers.borrow().pending.is_empty() {
        let (threshold, observers) = {
            let state = state.resize_observers.borrow();
            (
                state.depth,
                state.observers.values().cloned().collect::<Vec<_>>(),
            )
        };
        let mut pending = VecDeque::new();
        let mut minimum = usize::MAX;
        let mut skipped = false;
        let mut dependencies = Vec::new();
        for observer in observers {
            let targets = observer.borrow().targets.clone();
            let mut entries = Vec::new();
            for target in targets {
                let (entry, size, depth, owner, revision) = sample(scope, &target.borrow())?;
                dependencies.push((owner, revision));
                if size != target.borrow().last {
                    if depth > threshold {
                        minimum = minimum.min(depth);
                        entries.push((target, entry, size))
                    } else {
                        skipped = true
                    }
                }
            }
            if !entries.is_empty() {
                pending.push_back(Batch { observer, entries });
            }
        }
        let mut observers = state.resize_observers.borrow_mut();
        observers.pending = pending;
        observers.dependencies = dependencies;
        observers.skipped = skipped;
        if minimum != usize::MAX {
            observers.depth = minimum;
        }
    }
    let mut delivered = false;
    loop {
        if (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        ) != identity
        {
            let mut observers = state.resize_observers.borrow_mut();
            observers.pending.clear();
            observers.active = false;
            return Ok(delivered);
        }
        if !state.document_rendering_allowed() || scope.is_execution_terminating() {
            return Ok(delivered);
        }
        let batch = state.resize_observers.borrow_mut().pending.pop_front();
        let Some(batch) = batch else { break };
        let mut prepared_ids = Vec::new();
        let prepared = (|| {
            #[cfg(test)]
            if std::mem::take(&mut state.resize_observers.borrow_mut().fail_prepare) {
                return Err(V8BindingError::Allocation(
                    "injected ResizeObserver preparation",
                ));
            }
            let mut values = Vec::new();
            values
                .try_reserve(batch.entries.len())
                .map_err(|_| V8BindingError::Allocation("ResizeObserver entries"))?;
            for (_, entry, _) in &batch.entries {
                let mut public = entry.clone();
                let target = v8::Local::new(
                    scope,
                    public
                        .target
                        .as_ref()
                        .ok_or(V8BindingError::Allocation("pending entry target"))?,
                );
                public.target = None;
                let id = register_platform_object(
                    state,
                    PlatformObjectRecord::ResizeObserverEntry(public),
                );
                prepared_ids.push(id);
                let object = new_platform_object(scope, state, "ResizeObserverEntry", id)?;
                set_entry_target(scope, object, target)?;
                values.push(object.into());
            }
            let array = v8::Array::new_with_elements(scope, &values);
            let observer = batch.observer.borrow();
            let object = v8::Local::new(
                scope,
                observer
                    .object
                    .as_ref()
                    .ok_or(V8BindingError::Allocation("observer owner"))?,
            );
            let key = v8::String::new(scope, "oryn.resize_observers.callback")
                .ok_or(V8BindingError::Allocation("observer callback key"))?;
            let key = v8::Private::for_api(scope, Some(key));
            let callback = object
                .get_private(scope, key)
                .and_then(|value| value.try_cast::<v8::Function>().ok())
                .ok_or(V8BindingError::Allocation("observer callback"))?;
            Ok((array, callback, object))
        })();
        let (array, callback, object) = match prepared {
            Ok(values) => values,
            Err(error) => {
                for id in prepared_ids {
                    state.platform_objects.borrow_mut().remove(&id);
                    state.platform_wrappers.borrow_mut().remove(&id);
                }
                state
                    .resize_observers
                    .borrow_mut()
                    .pending
                    .push_front(batch);
                return Err(error);
            }
        };
        for (target, _, size) in &batch.entries {
            target.borrow_mut().last = *size;
        }
        delivered = true;
        global_events::invoke(
            scope,
            callback.into(),
            object.into(),
            &[array.into(), object.into()],
        );
    }
    if !delivered {
        if state.resize_observers.borrow().skipped {
            if state.resize_observers.borrow().loop_error.is_none() {
                let message = v8::String::new(
                    scope,
                    "ResizeObserver loop completed with undelivered notifications.",
                )
                .ok_or(V8BindingError::Allocation("ResizeObserver loop error"))?;
                let exception = v8::Exception::error(scope, message);
                state.resize_observers.borrow_mut().loop_error =
                    Some(v8::Global::new(scope, exception));
            }
            let exception = state.resize_observers.borrow().loop_error.clone().unwrap();
            let exception = v8::Local::new(scope, &exception);
            global_events::report_script_exception(scope, exception, None, false)?;
        }
        let mut observers = state.resize_observers.borrow_mut();
        observers.active = false;
        observers.dirty = observers.skipped;
        observers.skipped = false;
        observers.loop_error = None;
    }
    Ok(delivered)
}

pub(super) fn needs_render(state: &V8BindingState) -> bool {
    let observers = state.resize_observers.borrow();
    if observers.observers.is_empty() && observers.pending.is_empty() {
        return false;
    }
    observers.dirty
        || observers.active
        || !observers.pending.is_empty()
        || observers.dependencies.iter().any(|(owner, revision)| {
            owner.upgrade().is_some_and(|owner| {
                owner
                    .native_paint_revision()
                    .is_ok_and(|current| current != *revision)
            })
        })
}

/// An explicit native layout handoff is a new geometry sample opportunity.
pub(super) fn layout_changed(state: &V8BindingState) {
    let mut observers = state.resize_observers.borrow_mut();
    if !observers.observers.is_empty() {
        observers.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval(scope: &mut v8::PinScope, source: &str) -> bool {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    }
    fn layout(state: &V8BindingState, width: f64) {
        let node = state.dom.borrow().find_by_id("target").unwrap();
        let mut layout = BindingLayout::default();
        layout.geometry.insert(
            node_key(node),
            BindingGeometry::Available(BindingRect {
                x: 0.,
                y: 0.,
                width,
                height: 20.,
            }),
        );
        state.set_layout(layout);
    }
    #[test]
    fn delivered_entry_and_detached_target_cycles_are_collectable() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 20.);
        let weak = {
            v8::scope!(let inner,scope);
            assert!(eval(
                inner,
                "globalThis.o=new ResizeObserver(entries=>{globalThis.saved=entries[0];saved.target.saved=saved;saved.target.remove();o.disconnect()});o.observe(document.getElementById('target'));true"
            ));
            assert!(broadcast(inner, &state).unwrap());
            assert!(eval(
                inner,
                "saved.target.saved===saved&&!saved.target.isConnected"
            ));
            let key = v8::String::new(inner, "saved").unwrap();
            let entry = context
                .global(inner)
                .get(inner, key.into())
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap();
            let weak = v8::Weak::new(inner, entry);
            assert!(eval(
                inner,
                "delete globalThis.saved;delete globalThis.o;true"
            ));
            weak
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(weak.to_local(scope).is_none());
    }
    #[test]
    fn inactive_callback_cycles_collect_and_active_observation_keeps_callback_alive() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        fn value<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Object> {
            let source = v8::String::new(scope, source).unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .try_cast::<v8::Object>()
                .unwrap()
        }
        let idle = {
            v8::scope!(let inner,scope);
            let object = value(
                inner,
                "(()=>{const observer=new ResizeObserver(()=>observer);return observer})()",
            );
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(idle.to_local(scope).is_none());
        let active = {
            v8::scope!(let inner,scope);
            let object = value(
                inner,
                "(()=>{const observer=new ResizeObserver(()=>observer);observer.observe(document.getElementById('target'));return observer})()",
            );
            v8::Weak::new(inner, object)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        {
            v8::scope!(let inner,scope);
            let object = active
                .to_local(inner)
                .expect("native observation owns observer");
            let key = v8::String::new(inner, "disconnect").unwrap();
            let method = object
                .get(inner, key.into())
                .unwrap()
                .try_cast::<v8::Function>()
                .unwrap();
            method.call(inner, object.into(), &[]).unwrap();
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(active.to_local(scope).is_none());
    }
    #[test]
    fn native_brand_private_targets_and_retained_preparation() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        assert!(eval(
            scope,
            "globalThis.calls=0;globalThis.observer=new ResizeObserver(entries=>{calls++;globalThis.entry=entries[0]});observer.targets=new Map();observer.callback=()=>{throw Error('forged')};observer.observe(document.getElementById('target'));true"
        ));
        assert!(needs_render(&state));
        state.resize_observers.borrow_mut().fail_prepare = true;
        assert!(broadcast(scope, &state).is_err());
        assert!(eval(scope, "calls===0"));
        assert!(broadcast(scope, &state).unwrap());
        assert!(!broadcast(scope, &state).unwrap());
        assert!(!needs_render(&state));
        assert!(eval(
            scope,
            "calls===1&&entry instanceof ResizeObserverEntry&&entry.contentRect instanceof DOMRectReadOnly&&entry.contentBoxSize[0] instanceof ResizeObserverSize&&Object.isFrozen(entry.contentBoxSize)&&entry.contentBoxSize[0].inlineSize===40"
        ));
        assert!(eval(
            scope,
            "observer.disconnect();observer.observe(document.getElementById('target'));true"
        ));
        assert!(broadcast(scope, &state).unwrap());
        assert!(eval(scope, "calls===2"));
    }
    #[test]
    fn options_reentry_and_all_entry_boxes_release_platform_registry_borrows() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        assert!(eval(
            scope,
            r#"(()=>{globalThis.boxReads=0;globalThis.observer=new ResizeObserver(entries=>globalThis.saved=entries[0]);observer.observe(document.getElementById('target'),{get box(){boxReads++;new ResizeObserver(()=>{}).disconnect();new DOMException('during conversion');return {toString(){new Blob(['nested']);observer.disconnect();return 'border-box'}}}});return boxReads===1})()"#
        ));
        assert!(broadcast(scope, &state).unwrap());
        assert!(!broadcast(scope, &state).unwrap());
        assert!(eval(
            scope,
            r#"(()=>{for(const name of ['contentBoxSize','borderBoxSize','devicePixelContentBoxSize']){const boxes=saved[name];if(!Object.isFrozen(boxes)||boxes.length!==1||!(boxes[0] instanceof ResizeObserverSize)||boxes[0].inlineSize!==40||boxes[0].blockSize!==20)throw Error(name)}if(saved.target!==document.getElementById('target')||!(saved.contentRect instanceof DOMRectReadOnly))throw Error('entry identity');const marker={};try{observer.observe(saved.target,{get box(){new Blob();throw marker}});throw Error('missing exception')}catch(error){if(error!==marker)throw error}observer.disconnect();return true})()"#
        ));
    }
    #[test]
    fn native_observer_environment_tracks_provider_scale_lifecycle_and_direct_handoff() {
        struct Provider {
            revision: std::cell::Cell<u64>,
            ratio: std::cell::Cell<f64>,
        }
        impl BindingLayoutProvider for Provider {
            fn layout_revision(&self) -> Option<u64> {
                Some(self.revision.get())
            }
            fn native_device_pixel_ratio(&self) -> f64 {
                self.ratio.get()
            }
            fn compute_layout(
                &self,
                _: &BTreeMap<NodeId, BTreeSet<String>>,
            ) -> Result<BindingLayout, String> {
                Ok(BindingLayout {
                    viewport: Some((100., 100.)),
                    ..Default::default()
                })
            }
        }
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        assert!(eval(
            scope,
            "globalThis.o=new ResizeObserver(()=>{});o.observe(document.getElementById('target'));globalThis.io=new IntersectionObserver(()=>{});io.observe(document.getElementById('target'));true"
        ));
        assert!(broadcast(scope, &state).unwrap());
        assert!(!broadcast(scope, &state).unwrap());
        assert!(!needs_render(&state));
        let before = state.native_paint_revision().unwrap();
        layout(&state, 50.);
        assert!(needs_render(&state));
        assert!(intersection_observers::needs_render(&state));
        assert_ne!(state.native_paint_revision().unwrap(), before);
        let provider = Rc::new(Provider {
            revision: std::cell::Cell::new(1),
            ratio: std::cell::Cell::new(1.),
        });
        state.set_layout_provider(provider.clone());
        let first = state.native_paint_revision().unwrap();
        assert_eq!(state.native_paint_revision().unwrap(), first);
        provider.revision.set(2);
        let second = state.native_paint_revision().unwrap();
        assert_ne!(first, second);
        provider.ratio.set(2.);
        let third = state.native_paint_revision().unwrap();
        assert_ne!(second, third);
        state.execution_suspended.set(true);
        let parked = state.native_paint_revision().unwrap();
        assert_ne!(third, parked);
        assert_eq!(state.native_paint_revision().unwrap(), parked);
        state.execution_suspended.set(false);
        assert_ne!(state.native_paint_revision().unwrap(), parked);
    }
    #[test]
    fn svg_graphics_content_rect_keeps_object_space_origin() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><svg><circle id=target cx=30 cy=40 r=10></circle></svg></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = state.dom.borrow().find_by_id("target").unwrap();
        state.layout.borrow_mut().geometry.insert(
            node_key(node),
            BindingGeometry::Available(BindingRect {
                x: 20.,
                y: 30.,
                width: 20.,
                height: 20.,
            }),
        );
        assert!(eval(
            scope,
            "globalThis.o=new ResizeObserver(entries=>{globalThis.entry=entries[0]});o.observe(document.getElementById('target'));true"
        ));
        assert!(broadcast(scope, &state).unwrap());
        assert!(eval(
            scope,
            "entry.contentRect.x===20&&entry.contentRect.y===30&&entry.contentRect.width===20&&entry.contentRect.height===20"
        ));
    }
    #[test]
    fn svg_device_pixels_follow_native_effective_zoom_without_changing_object_dimensions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><svg><rect id=target width=50 height=30></rect></svg></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let node = state.dom.borrow().find_by_id("target").unwrap();
        state.layout.borrow_mut().geometry.insert(
            node_key(node),
            BindingGeometry::Available(BindingRect {
                x: 0.,
                y: 0.,
                width: 50.,
                height: 30.,
            }),
        );
        assert!(eval(
            scope,
            "globalThis.o=new ResizeObserver(()=>{});o.observe(document.getElementById('target'),{box:'device-pixel-content-box'});true"
        ));
        let observer = state
            .resize_observers
            .borrow()
            .observers
            .values()
            .next()
            .unwrap()
            .clone();
        let target = observer.borrow().targets[0].clone();
        for (zoom, expected) in [("0.1", (5., 3.)), ("10", (500., 300.))] {
            state.layout.borrow_mut().styles.insert(
                node_key(node),
                std::sync::Arc::new(BTreeMap::from([(
                    "oryn-effective-zoom".into(),
                    zoom.into(),
                )])),
            );
            let (entry, selected, _, _, _) = sample(scope, &target.borrow()).unwrap();
            assert_eq!(entry.content, (50., 30.));
            assert_eq!(entry.border, (50., 30.));
            assert_eq!(selected, expected);
        }
    }
    #[test]
    fn retained_retired_target_keeps_depth_and_reports_zero_geometry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        assert!(eval(
            scope,
            "globalThis.o=new ResizeObserver(()=>{});o.observe(document.getElementById('target'));true"
        ));
        let observer = state
            .resize_observers
            .borrow()
            .observers
            .values()
            .next()
            .unwrap()
            .clone();
        let target = observer.borrow().targets[0].clone();
        let (_, before, depth, _, _) = sample(scope, &target.borrow()).unwrap();
        assert_eq!(before, (40., 20.));
        assert!(depth > 0);
        state.retire_execution();
        let (_, after, retired_depth, _, _) = sample(scope, &target.borrow()).unwrap();
        assert_eq!(after, (0., 0.));
        assert_eq!(retired_depth, depth);
    }
    #[test]
    fn retirement_releases_scheduler_roots_without_invalidating_observer_methods() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        assert!(eval(
            scope,
            "globalThis.target=document.getElementById('target');globalThis.n=0;globalThis.o=new ResizeObserver(()=>n++);o.observe(target);true"
        ));
        let observer = state
            .resize_observers
            .borrow()
            .observers
            .values()
            .next()
            .unwrap()
            .clone();
        state.retire_execution();
        assert!(observer.borrow().object.is_none());
        assert!(!needs_render(&state));
        assert!(!broadcast(scope, &state).unwrap());
        assert!(eval(scope, "o.disconnect();o.observe(target);n===0"));
        assert!(state.resize_observers.borrow().observers.is_empty());
        assert!(observer.borrow().object.is_none());
    }
    #[test]
    fn earlier_callback_disconnects_later_observer_before_its_broadcast() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        eval(
            scope,
            "globalThis.calls=[];globalThis.a=new ResizeObserver(()=>{calls.push('a');b.disconnect()});globalThis.b=new ResizeObserver(()=>calls.push('b'));a.observe(document.getElementById('target'));b.observe(document.getElementById('target'));true",
        );
        assert!(broadcast(scope, &state).unwrap());
        assert!(eval(scope, "calls.join()==='a'"));
        assert!(!broadcast(scope, &state).unwrap());
    }
    #[test]
    fn same_depth_resize_is_skipped_until_next_rendering_opportunity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><div id=target></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        layout(&state, 40.);
        eval(
            scope,
            "globalThis.calls=0;new ResizeObserver(()=>calls++).observe(document.getElementById('target'));true",
        );
        assert!(broadcast(scope, &state).unwrap());
        layout(&state, 50.);
        assert!(!broadcast(scope, &state).unwrap());
        assert!(eval(scope, "calls===1"));
        assert!(broadcast(scope, &state).unwrap());
        assert!(eval(scope, "calls===2"));
    }
}

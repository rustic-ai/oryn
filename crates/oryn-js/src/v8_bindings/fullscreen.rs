//! Native fullscreen state shared by script requests and invoker commands.
//! Event delivery is a separate retained task from the committed top-layer change.
use super::*;
struct Change {
    target: DeferredDomTarget,
    name: &'static str,
    epoch: u64,
    event: Option<retained_dom_event::Event>,
}
#[derive(Default)]
pub(super) struct Registry {
    iframe_flags: Vec<NodeId>,
    active: Vec<DeferredDomTarget>,
    changes: VecDeque<Change>,
    retry: Option<Instant>,
    polling: bool,
}
pub(super) fn enabled(state: &V8BindingState, document: NodeId) -> bool {
    document == state.document_root()
        && !state.execution_retired()
        && state.binding_global == BindingGlobal::Window
        && state.native_permissions_policy_allows("fullscreen")
}
fn reserve(state: &V8BindingState) -> Result<(), V8BindingError> {
    let mut registry = state.fullscreen.borrow_mut();
    if registry.changes.len() >= 1024 {
        return Err(V8BindingError::Allocation("fullscreen task limit"));
    }
    registry
        .iframe_flags
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("fullscreen iframe flags"))?;
    registry
        .active
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("fullscreen active targets"))?;
    registry
        .changes
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("fullscreen task"))
}
fn enqueue(state: &V8BindingState, target: DeferredDomTarget, name: &'static str) {
    state.fullscreen.borrow_mut().changes.push_back(Change {
        target,
        name,
        epoch: state.document_generation(),
        event: None,
    });
}
/// This entry accepts only native node authority; attributes cannot grant state.
pub(super) fn request(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    state.refresh_native_container_policy()?;
    reconcile_removed(scope, state)?;
    let valid = {
        let dom = state.dom.borrow();
        enabled(state, state.document_root())
            && !state.execution_suspended()
            && dom.node_document(node)? == Some(state.document_root())
            && dom.is_connected(node)?
            && matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"||(name.ns.as_ref()=="http://www.w3.org/2000/svg"&&name.local.as_ref()=="svg")||(name.ns.as_ref()=="http://www.w3.org/1998/Math/MathML"&&name.local.as_ref()=="math"))
            && !dom.is_element(node, "http://www.w3.org/1999/xhtml", "dialog")
            && !dom.is_popover_open(node)
    };
    if !valid || !state.has_transient_user_activation() {
        return reject_request(state, node);
    }
    let mut participants = vec![(state.clone(), node)];
    let mut owner = state.clone();
    loop {
        let window = owner
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("fullscreen Window"))?;
        let parent = window_document::native_parent(scope, window);
        if parent == window {
            break;
        }
        let Some(frame) = window_document::native_frame(scope, window) else {
            return reject_request(state, node);
        };
        let Some((parent_owner, frame)) = node_identity::argument(scope, frame.into())? else {
            return reject_request(state, node);
        };
        let Some(native_parent) = binding_state_for_receiver(scope, parent) else {
            return reject_request(state, node);
        };
        if !Rc::ptr_eq(&native_parent, &parent_owner)
            || parent_owner.execution_suspended()
            || parent_owner.execution_retired()
            || parent_owner.dom.borrow().node_document(frame)? != Some(parent_owner.document_root())
            || !parent_owner.dom.borrow().is_connected(frame)?
        {
            return reject_request(state, node);
        }
        if participants
            .iter()
            .any(|(seen, _)| Rc::ptr_eq(seen, &parent_owner))
        {
            return Err(V8BindingError::Allocation("fullscreen ancestor cycle"));
        }
        participants
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("fullscreen ancestor chain"))?;
        participants.push((parent_owner.clone(), frame));
        owner = parent_owner;
    }
    // Reserve every native mutation/event destination before consuming activation.
    let mut changes = Vec::new();
    changes
        .try_reserve(participants.len())
        .map_err(|_| V8BindingError::Allocation("fullscreen changes"))?;
    for (owner, node) in participants {
        if owner.dom.borrow().fullscreen_element(owner.document_root()) == Some(node) {
            continue;
        }
        let target = DeferredDomTarget::new(&owner.dom, node, owner.document_root())?;
        let active = DeferredDomTarget::new(&owner.dom, node, owner.document_root())?;
        reserve(&owner)?;
        owner.dom.borrow_mut().reserve_top_layer(1)?;
        changes.push((owner, node, target, active));
    }
    reserve(state)?;
    if !state.consume_transient_user_activation() {
        return reject_request(state, node);
    }
    if state
        .dom
        .borrow()
        .is_element(node, "http://www.w3.org/1999/xhtml", "iframe")
    {
        let mut registry = state.fullscreen.borrow_mut();
        if !registry.iframe_flags.contains(&node) {
            registry.iframe_flags.push(node);
        }
    }
    for (owner, node, target, active) in changes {
        if owner.dom.borrow().is_fullscreen(node) {
            owner.dom.borrow_mut().remove_from_top_layer(node);
        }
        owner
            .dom
            .borrow_mut()
            .add_to_top_layer(node, oryn_dom::TopLayerKind::Fullscreen)?;
        owner.layout_cache_key.borrow_mut().take();
        {
            let mut registry = owner.fullscreen.borrow_mut();
            registry.active.retain(|old| old.node != node);
            registry.active.push(active);
            if Rc::ptr_eq(&owner, state)
                && owner
                    .dom
                    .borrow()
                    .is_element(node, "http://www.w3.org/1999/xhtml", "iframe")
                && !registry.iframe_flags.contains(&node)
            {
                registry.iframe_flags.push(node);
            }
        }
        enqueue(&owner, target, "fullscreenchange");
    }
    Ok(true)
}
fn reject_request(state: &V8BindingState, node: NodeId) -> Result<bool, V8BindingError> {
    let target = DeferredDomTarget::new(&state.dom, node, state.document_root())?;
    reserve(state)?;
    enqueue(state, target, "fullscreenerror");
    Ok(false)
}

pub(super) fn exit(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    document: NodeId,
) -> Result<bool, V8BindingError> {
    if document != state.document_root() || state.execution_retired() || state.execution_suspended()
    {
        return Ok(false);
    }
    let Some(node) = state.dom.borrow().fullscreen_element(document) else {
        return Ok(false);
    };
    exit_from(scope, state, node, false)
}
fn exit_from(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    fully: bool,
) -> Result<bool, V8BindingError> {
    let initial = if fully {
        let mut nodes = state
            .fullscreen
            .borrow()
            .active
            .iter()
            .map(|active| active.node)
            .collect::<Vec<_>>();
        nodes.retain(|candidate| *candidate != node);
        nodes.push(node);
        nodes
    } else {
        vec![node]
    };
    let mut exits = vec![(state.clone(), initial)];
    let mut owner = state.clone();
    loop {
        let count = owner
            .dom
            .borrow()
            .top_layer()
            .filter(|entry| {
                entry.kind == oryn_dom::TopLayerKind::Fullscreen
                    && owner.dom.borrow().node_document(entry.node).ok().flatten()
                        == Some(owner.document_root())
            })
            .count();
        if count != 1 && !(fully && Rc::ptr_eq(&owner, state)) {
            break;
        }
        let window = owner
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("fullscreen exit Window"))?;
        let Some(frame) = window_document::native_frame(scope, window) else {
            break;
        };
        let Some((parent, frame)) = node_identity::argument(scope, frame.into())? else {
            break;
        };
        if parent.execution_retired()
            || parent.execution_suspended()
            || parent.fullscreen.borrow().iframe_flags.contains(&frame)
            || parent
                .dom
                .borrow()
                .fullscreen_element(parent.document_root())
                != Some(frame)
            || exits.iter().any(|(seen, _)| Rc::ptr_eq(seen, &parent))
        {
            break;
        }
        exits
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("fullscreen exit ancestors"))?;
        exits.push((parent.clone(), vec![frame]));
        owner = parent;
    }
    let candidates = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for candidate in candidates {
        if candidate.execution_retired()
            || exits.iter().any(|(seen, _)| Rc::ptr_eq(seen, &candidate))
        {
            continue;
        }
        let mut descendant = candidate.clone();
        let mut seen = BTreeSet::new();
        let mut inside = false;
        loop {
            if !seen.insert(descendant.instance_id) {
                break;
            }
            let Some(window) = descendant.global_object.to_local(scope) else {
                break;
            };
            let Some(frame) = window_document::native_frame(scope, window) else {
                break;
            };
            let Some((parent, _)) = node_identity::argument(scope, frame.into())? else {
                break;
            };
            if Rc::ptr_eq(&parent, state) {
                inside = true;
                break;
            }
            descendant = parent;
        }
        if inside {
            let nodes = candidate
                .dom
                .borrow()
                .top_layer()
                .filter(|entry| {
                    entry.kind == oryn_dom::TopLayerKind::Fullscreen
                        && candidate
                            .dom
                            .borrow()
                            .node_document(entry.node)
                            .ok()
                            .flatten()
                            == Some(candidate.document_root())
                })
                .map(|entry| entry.node)
                .collect::<Vec<_>>();
            if !nodes.is_empty() {
                exits
                    .try_reserve(1)
                    .map_err(|_| V8BindingError::Allocation("fullscreen exit descendants"))?;
                exits.push((candidate, nodes));
            }
        }
    }
    let mut changes = Vec::new();
    changes
        .try_reserve(exits.len())
        .map_err(|_| V8BindingError::Allocation("fullscreen exit changes"))?;
    for (owner, nodes) in exits {
        let node = *nodes.last().unwrap();
        let target = DeferredDomTarget::new(&owner.dom, node, owner.document_root())?;
        reserve(&owner)?;
        changes.push((owner, nodes, target));
    }
    for (owner, nodes, target) in changes {
        for node in nodes {
            owner.dom.borrow_mut().remove_from_top_layer(node);
            {
                let mut registry = owner.fullscreen.borrow_mut();
                registry.active.retain(|active| active.node != node);
                registry.iframe_flags.retain(|flag| *flag != node);
            }
        }
        owner.layout_cache_key.borrow_mut().take();
        enqueue(&owner, target, "fullscreenchange");
    }
    Ok(true)
}

pub(super) fn command(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: &str,
) -> Result<bool, V8BindingError> {
    match name {
        "request-fullscreen" => {
            if !state.dom.borrow().is_fullscreen(node) {
                request(scope, state, node)?;
            }
            Ok(true)
        }
        "exit-fullscreen" => {
            if state.dom.borrow().is_fullscreen(node) {
                exit(scope, state, state.document_root())?;
            }
            Ok(true)
        }
        "toggle-fullscreen" => {
            if state.dom.borrow().is_fullscreen(node) {
                exit(scope, state, state.document_root())?;
            } else {
                request(scope, state, node)?;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
pub(super) fn promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    exit_request: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("fullscreen promise"))?;
    let accepted = if exit_request {
        exit(scope, state, node)?
    } else {
        request(scope, state, node)?
    };
    if accepted {
        let undefined = v8::undefined(scope);
        resolver
            .resolve(scope, undefined.into())
            .ok_or(V8BindingError::JavaScriptException)?;
    } else {
        let message = v8::String::new(scope, "Fullscreen request is not allowed for this document")
            .ok_or(V8BindingError::Allocation("fullscreen rejection"))?;
        let error = v8::Exception::type_error(scope, message);
        resolver
            .reject(scope, error)
            .ok_or(V8BindingError::JavaScriptException)?;
    }
    Ok(resolver.get_promise(scope).into())
}
pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    let registry = state.fullscreen.borrow();
    ((!registry.changes.is_empty()
        || registry
            .active
            .iter()
            .any(|active| !state.dom.borrow().is_fullscreen(active.node)))
        && !state.execution_suspended())
    .then(|| {
        registry.retry.map_or(Duration::ZERO, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
    })
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_suspended()
        || state.fullscreen.borrow().polling
        || !delay(state).is_some_and(|delay| delay.is_zero())
    {
        return Ok(0);
    }
    if let Err(error) = reconcile_removed(scope, state) {
        state.fullscreen.borrow_mut().retry = Some(Instant::now() + Duration::from_millis(10));
        return Err(error);
    }
    let Some(mut change) = state.fullscreen.borrow_mut().changes.pop_front() else {
        return Ok(0);
    };
    state.fullscreen.borrow_mut().polling = true;
    struct Guard<'a>(&'a V8BindingState);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.fullscreen.borrow_mut().polling = false;
        }
    }
    let _guard = Guard(state);
    let result = (|| -> Result<bool, V8BindingError> {
        if state.execution_retired() || change.epoch != state.document_generation() {
            return Ok(true);
        }
        if change.event.is_none() {
            let target = if state
                .dom
                .borrow()
                .node_document(change.target.node)
                .ok()
                .flatten()
                == Some(change.target.document)
                && state
                    .dom
                    .borrow()
                    .is_connected(change.target.node)
                    .unwrap_or(false)
            {
                change.target.node
            } else {
                change.target.document
            };
            let event = create_event_object(scope, state, change.name, true, false)?;
            let id = event_id(scope, event)?;
            {
                let mut events = state.events.borrow_mut();
                let record = events
                    .get_mut(&id)
                    .ok_or(V8BindingError::Allocation("fullscreen event"))?;
                record.is_trusted = true;
                record.composed = true;
            }
            change.event = Some(retained_dom_event::Event::new(
                scope,
                state,
                target,
                change.name,
                event,
            )?);
        }
        Ok(change.event.as_mut().unwrap().poll(scope, state)?.is_some())
    })();
    match result {
        Ok(true) => {
            state.fullscreen.borrow_mut().retry = None;
            Ok(1)
        }
        result => {
            let mut registry = state.fullscreen.borrow_mut();
            registry.changes.push_front(change);
            registry.retry = Some(Instant::now() + Duration::from_millis(10));
            result.map(|_| 0)
        }
    }
}
fn reconcile_removed(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() || state.execution_suspended() {
        return Ok(());
    }
    let removed_top = {
        let registry = state.fullscreen.borrow();
        registry
            .active
            .last()
            .filter(|active| !state.dom.borrow().is_fullscreen(active.node))
            .map(|active| active.node)
    };
    if let Some(node) = removed_top {
        exit_from(scope, state, node, true)?;
    } else {
        let mut registry = state.fullscreen.borrow_mut();
        registry
            .active
            .retain(|active| state.dom.borrow().is_fullscreen(active.node));
        registry
            .iframe_flags
            .retain(|node| state.dom.borrow().is_fullscreen(*node));
    }
    Ok(())
}
pub(super) fn retire(state: &V8BindingState) {
    let old = std::mem::take(&mut *state.fullscreen.borrow_mut());
    drop(old);
    let nodes = state
        .dom
        .borrow()
        .top_layer()
        .filter(|entry| {
            entry.kind == oryn_dom::TopLayerKind::Fullscreen
                && state.dom.borrow().node_document(entry.node).ok().flatten()
                    == Some(state.document_root())
        })
        .map(|entry| entry.node)
        .collect::<Vec<_>>();
    for node in nodes {
        state.dom.borrow_mut().remove_from_top_layer(node);
    }
}

pub(super) fn element(
    state: &V8BindingState,
    root: NodeId,
) -> Result<Option<NodeId>, V8BindingError> {
    let dom = state.dom.borrow();
    let document = dom.node_document(root)?.unwrap_or(root);
    let Some(element) = dom.fullscreen_element(document) else {
        return Ok(None);
    };
    let element = dom.retarget_node(element, Some(root))?;
    Ok((dom.control_tree_root(element)? == root).then_some(element))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Activation(AtomicBool);
    impl crate::user_activation::ActivationBackend for Activation {
        fn notify(&self, _: crate::worker_wire::Creator) -> Result<(), String> {
            self.0.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn transient(&self, _: crate::worker_wire::Creator, consume: bool) -> Result<bool, String> {
            Ok(if consume {
                self.0.swap(false, Ordering::SeqCst)
            } else {
                self.0.load(Ordering::SeqCst)
            })
        }
    }
    use super::super::native_user_activation::test_transport::Messages;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn native_fullscreen_publication_retains_listener_suffix_and_state() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(230), 1).unwrap();
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_activation_backend(Arc::new(Activation(AtomicBool::new(false))));
        eval(
            scope,
            "globalThis.log=[];t.addEventListener('fullscreenchange',e=>{log.push('first');globalThis.original=e},{once:true});t.addEventListener('fullscreenchange',e=>log.push(e===original&&e.isTrusted?'second':'wrong'))",
        );
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "t")
            .unwrap()
            .unwrap();
        state.notify_user_activation();
        assert!(request(scope, &state, node).unwrap());
        assert!(!state.has_transient_user_activation());
        assert_eq!(element(&state, state.document_root()).unwrap(), Some(node));
        assert!(eval(scope, "log.length===0").is_true());
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "log.join()==='first'").is_true());
        state.fullscreen.borrow_mut().retry = None;
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "log.join()==='first,second'").is_true());
        assert!(exit(scope, &state, state.document_root()).unwrap());
        assert_eq!(element(&state, state.document_root()).unwrap(), None);
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn fullscreen_event_attributes_dispatch_and_clear_on_all_elements() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(231), 1).unwrap();
        assert!(eval(scope, r#"(()=>{
            for(const node of [document, t, document.createElementNS('http://www.w3.org/2000/svg','svg')]) {
                for(const name of ['fullscreenchange','fullscreenerror']) {
                    const property='on'+name;let calls=0;
                    if(node[property]!==null)throw Error('default handler');
                    node[property]=function(event){if(this!==node||event.type!==name)throw Error('handler context');calls++};
                    node.dispatchEvent(new Event(name));
                    if(calls!==1)throw Error('handler dispatch');
                    node[property]=null;node.dispatchEvent(new Event(name));
                    if(calls!==1||node[property]!==null)throw Error('handler clear');
                }
            }
            return true;
        })()"#).is_true());
    }

    #[test]
    fn denied_policy_does_not_consume_activation_or_set_native_flag() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t fullscreen></div>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(231), 1).unwrap();
        let messages = Arc::new(Messages::new(
            state.document_generation(),
            state.document_authority_epoch(),
        ));
        state.set_message_transport(messages.clone());
        state.set_activation_backend(Arc::new(Activation(AtomicBool::new(false))));
        state.set_native_permissions_policy(Rc::new(|_| false));
        state.notify_user_activation();
        let node = state
            .dom
            .borrow()
            .get_element_by_id(state.document_root(), "t")
            .unwrap()
            .unwrap();
        assert!(!request(scope, &state, node).unwrap());
        assert!(state.has_transient_user_activation());
        assert!(!state.dom.borrow().is_fullscreen(node));
        assert_eq!(state.fullscreen.borrow().changes.len(), 1);
        messages.assert_lifecycle(0);
        state.retire_execution();
        messages.assert_lifecycle(1);
    }
    #[test]
    fn native_frame_chain_enters_and_exits_without_authored_parent_authority() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let parent_context = v8::Context::new(scope, Default::default());
        let (parent, parent_window, frame) = {
            let scope = &mut v8::ContextScope::new(scope, parent_context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<iframe id=f></iframe>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state =
                install_generated_bindings(scope, parent_context, dom, RealmId(232), 1).unwrap();
            let frame = state
                .dom
                .borrow()
                .get_element_by_id(state.document_root(), "f")
                .unwrap()
                .unwrap();
            let window = parent_context.global(scope);
            (state, v8::Global::new(scope, window), frame)
        };
        let child_context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, child_context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<div id=t></div>")
                .into_runtime()
                .unwrap(),
        ));
        let child = install_generated_bindings(scope, child_context, dom, RealmId(233), 1).unwrap();
        let messages = Arc::new(Messages::new(
            child.document_generation(),
            child.document_authority_epoch(),
        ));
        child.set_message_transport(messages.clone());
        child.set_activation_backend(Arc::new(Activation(AtomicBool::new(false))));
        let window = child_context.global(scope);
        let parent_window = v8::Local::new(scope, parent_window);
        let frame_object = wrap_node(scope, &parent, frame).unwrap();
        window_document::set_parent(scope, window, parent_window).unwrap();
        window_document::set_embedding_frame(scope, window, frame_object).unwrap();
        window_document::set_frame_element(scope, window, None).unwrap();
        assert!(eval(scope, "frameElement===null").is_true());
        eval(
            scope,
            "Object.defineProperty(globalThis,'parent',{value:globalThis,configurable:true})",
        );
        let target = child
            .dom
            .borrow()
            .get_element_by_id(child.document_root(), "t")
            .unwrap()
            .unwrap();
        child.notify_user_activation();
        assert!(request(scope, &child, target).unwrap());
        assert_eq!(
            child.dom.borrow().fullscreen_element(child.document_root()),
            Some(target)
        );
        assert_eq!(
            parent
                .dom
                .borrow()
                .fullscreen_element(parent.document_root()),
            Some(frame)
        );
        assert!(exit(scope, &child, child.document_root()).unwrap());
        assert_eq!(
            child.dom.borrow().fullscreen_element(child.document_root()),
            None
        );
        assert_eq!(
            parent
                .dom
                .borrow()
                .fullscreen_element(parent.document_root()),
            None
        );
        messages.assert_lifecycle(0);
        child.retire_execution();
        messages.assert_lifecycle(1);
    }
}

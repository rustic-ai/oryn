//! Retained hover boundary publication. Keep the previous rendered ancestry
//! even when the original target is removed before the next native sample.
use super::*;
use std::collections::VecDeque;
struct Step {
    node: NodeId,
    name: &'static str,
    bubbles: bool,
    related: Option<NodeId>,
}
pub(super) struct Pending {
    owner: std::rc::Weak<V8BindingState>,
    source: Rc<()>,
    identity: (NodeId, u64, u64, u64),
    input: NativePointerInput,
    new: Option<NodeId>,
    _leases: Vec<Rc<oryn_dom::TrackedNodeLease>>,
    steps: VecDeque<Step>,
    notification: Option<native_platform::publication::Notification>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let (Some(state), Some(notification)) =
            (self.owner.upgrade(), self.notification.as_ref())
        {
            notification.cancel(&state);
        }
    }
}
fn path(state: &V8BindingState, node: Option<NodeId>) -> Result<Vec<NodeId>, V8BindingError> {
    let mut path = state.dom.borrow().pointer_boundary_targets(None, node)?.1;
    path.reverse();
    Ok(path)
}
fn source_current(state: &V8BindingState, source: &Rc<()>) -> bool {
    Rc::ptr_eq(&state.native_pointers.borrow().boundary_owner, source)
}
fn current(state: &V8BindingState, identity: (NodeId, u64, u64, u64)) -> bool {
    !state.execution_retired()
        && (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        ) == identity
}
pub(super) fn drain(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: i32,
) -> Result<(), V8BindingError> {
    if state
        .native_pointers
        .borrow()
        .pointers
        .get(&id)
        .is_some_and(|p| p.boundary_claim.is_some())
    {
        return Ok(());
    }
    drain_owned(scope, state, id)
}
pub(super) fn drain_owned(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: i32,
) -> Result<(), V8BindingError> {
    if state.native_pointers.borrow().boundary_draining {
        return Ok(());
    }
    let pending = state
        .native_pointers
        .borrow_mut()
        .pointers
        .get_mut(&id)
        .and_then(|pointer| {
            pointer.boundary_retry = None;
            pointer.boundary_pending.take()
        });
    let Some(mut pending) = pending else {
        return Ok(());
    };
    if !current(state, pending.identity) || !source_current(state, &pending.source) {
        return Ok(());
    }
    state.native_pointers.borrow_mut().boundary_draining = true;
    let result = (|| {
        while let Some(step) = pending.steps.front() {
            if !current(state, pending.identity) || !source_current(state, &pending.source) {
                return Ok(());
            }
            if pending.notification.is_none() {
                if !state.dom.borrow().is_connected(step.node).unwrap_or(false)
                    || state.dom.borrow().node_document(step.node).ok().flatten()
                        != Some(state.document_root())
                {
                    pending.steps.pop_front();
                    continue;
                }
                let related = step.related.filter(|node| {
                    state.dom.borrow().node_document(*node).ok().flatten()
                        == Some(state.document_root())
                });
                let target = wrap_node(scope, state, step.node)?;
                let event = pointer_event(
                    scope,
                    state,
                    &pending.input,
                    step.name,
                    step.bubbles,
                    step.bubbles,
                    related,
                )?;
                pending.notification = Some(
                    native_platform::publication::pending_dom_dispatch(
                        scope, state, target, step.name, event,
                    )
                    .for_parser_stream(pending.identity.2),
                );
            }
            let notification = pending.notification.as_mut().unwrap();
            if !notification.resume(scope, state)? {
                return Err(V8BindingError::Allocation(
                    "pointer boundary publication suspended",
                ));
            }
            notification.finish(scope, state);
            pending.notification = None;
            pending.steps.pop_front();
        }
        if current(state, pending.identity)
            && source_current(state, &pending.source)
            && pending.input.primary
            && pending.input.device != NativePointerDevice::Touch
        {
            interest::pointer(scope, state, pending.new)?;
        }
        Ok(())
    })();
    if !source_current(state, &pending.source) {
        return Err(V8BindingError::Allocation(
            "pointer input source reset during boundary publication",
        ));
    }
    if source_current(state, &pending.source) {
        state.native_pointers.borrow_mut().boundary_draining = false;
        if result.is_err()
            && current(state, pending.identity)
            && let Some(pointer) = state.native_pointers.borrow_mut().pointers.get_mut(&id)
        {
            pointer.boundary_retry =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
            pointer.boundary_pending = Some(pending);
        }
    }
    result
}
pub(super) fn dispatch(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    old: Option<NodeId>,
    new: Option<NodeId>,
) -> Result<(), V8BindingError> {
    if state
        .native_pointers
        .borrow()
        .pointers
        .get(&input.id)
        .is_some_and(|p| p.boundary_claim.is_some())
    {
        return Err(V8BindingError::Allocation("pointer boundary already owned"));
    }
    dispatch_owned(scope, state, input, old, new)
}
pub(super) fn dispatch_owned(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    input: &NativePointerInput,
    old: Option<NodeId>,
    new: Option<NodeId>,
) -> Result<(), V8BindingError> {
    let source = state.native_pointers.borrow().boundary_owner.clone();
    drain_owned(scope, state, input.id)?;
    if !source_current(state, &source) {
        return Ok(());
    }
    if state.native_pointers.borrow().boundary_draining {
        return Ok(());
    }
    let new = new.filter(|node| state.dom.borrow().is_connected(*node).unwrap_or(false));
    let next = path(state, new)?;
    let previous = state
        .native_pointers
        .borrow()
        .pointers
        .get(&input.id)
        .map(|pointer| pointer.boundary_path.clone())
        .filter(|path| !path.is_empty());
    let previous = match previous {
        Some(path) => path,
        None => path(
            state,
            old.filter(|node| state.dom.borrow().is_connected(*node).unwrap_or(false)),
        )?,
    };
    if old == new && previous == next {
        return Ok(());
    }
    let next_leases = next
        .iter()
        .map(|node| oryn_dom::TrackedNodeLease::new(&state.dom, *node).map(Rc::new))
        .collect::<Result<Vec<_>, _>>()?;
    let mut leases = {
        let mut registry = state.native_pointers.borrow_mut();
        let pointer = registry.pointers.entry(input.id).or_default();
        pointer.boundary_path = next.clone();
        std::mem::replace(&mut pointer.boundary_leases, next_leases.clone())
    };
    leases.extend(next_leases);
    let leaves = previous
        .iter()
        .copied()
        .filter(|node| !next.contains(node))
        .collect::<Vec<_>>();
    let mut enters = next
        .iter()
        .copied()
        .filter(|node| !previous.contains(node))
        .collect::<Vec<_>>();
    enters.reverse();
    let mut steps = VecDeque::new();
    let mut add = |node, name, bubbles, related| {
        steps.push_back(Step {
            node,
            name,
            bubbles,
            related,
        });
    };
    if old != new
        && let Some(old) = old
    {
        add(old, "pointerout", true, new);
        if input.primary {
            add(old, "mouseout", true, new);
        }
    }
    for node in leaves {
        add(node, "pointerleave", false, new);
        if input.primary {
            add(node, "mouseleave", false, new);
        }
    }
    if old != new
        && let Some(new) = new
    {
        add(new, "pointerover", true, old);
        if input.primary {
            add(new, "mouseover", true, old);
        }
    }
    for node in enters {
        add(node, "pointerenter", false, old);
        if input.primary {
            add(node, "mouseenter", false, old);
        }
    }
    state
        .native_pointers
        .borrow_mut()
        .pointers
        .entry(input.id)
        .or_default()
        .boundary_pending = Some(Pending {
        owner: Rc::downgrade(state),
        source,
        identity: (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        ),
        input: input.clone(),
        new,
        _leases: leases,
        steps,
        notification: None,
    });
    drain_owned(scope, state, input.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn boundary_publication_retains_same_event_prefix_and_reset_cancels_dispatch() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            html::parse("<div id=a></div><div id=b></div>")
                .into_runtime()
                .unwrap(),
        ));
        let a = dom.borrow().find_by_id("a").unwrap();
        let b = dom.borrow().find_by_id("b").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let input = NativePointerInput::default();
        transition_native_pointer_boundary(scope, &state, &input, Some(a)).unwrap();
        eval(
            scope,
            "globalThis.seen=[];a.addEventListener('mouseout',e=>{globalThis.original=e;seen.push('once')},{once:true});a.onmouseout=e=>seen.push(e===original?'handler':'wrong');b.onmouseover=()=>seen.push('over')",
        );
        native_platform::publication::fail_after_next_callback();
        assert!(transition_native_pointer_boundary(scope, &state, &input, Some(b)).is_err());
        assert!(eval(scope, "seen.join()==='once'").is_true());
        assert!(native_input_delay(&state, std::time::Instant::now()).is_some());
        assert!(native_pointer_hover_inputs(&state).is_empty());
        transition_native_pointer_boundary(scope, &state, &input, Some(b)).unwrap();
        assert!(eval(scope, "seen.join()==='once,handler,over'").is_true());
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        eval(scope, "b.onmouseout=()=>seen.push('out')");
        native_platform::publication::fail_after_next_callback();
        assert!(transition_native_pointer_boundary(scope, &state, &input, Some(a)).is_err());
        assert!(
            state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        reset_native_pointer_sources(&state);
        assert!(
            !state
                .events
                .borrow()
                .values()
                .any(|event| event.dispatching)
        );
        assert!(native_pointer_hover_inputs(&state).is_empty());
        assert_eq!(native_input_delay(&state, std::time::Instant::now()), None);
    }
}

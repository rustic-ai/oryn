//! Native custody of distinct decode observers, independent of image-owner life.
//! API exposure waits for request-selection integration; no synthetic decode.
use super::*;

const MAX_OBSERVERS: usize = 256;

fn rejected<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str)
    -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("image decode rejected promise"))?;
    let error = dom_exception_value(scope, name)?;
    if resolver.reject(scope, error) != Some(true) { return Err(V8BindingError::JavaScriptException); }
    Ok(resolver.get_promise(scope))
}

pub(super) fn decode<'s>(scope: &mut v8::PinScope<'s, '_>, receiver: v8::Local<'s, v8::Object>)
    -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let creation = binding_state_for_receiver(scope, receiver)
        .ok_or_else(|| V8BindingError::Unsupported { interface: "HTMLImageElement".into(), member: "TypeError".into() })?;
    let node = resolve_receiver(scope, &creation, receiver, "HTMLImageElement")?;
    let Some(owner) = active_document_state_for_node(scope, &creation, node) else {
        return rejected(scope, "EncodingError");
    };
    if owner.document_load_stopped() { return rejected(scope, "EncodingError"); }
    let context = owner.realm_context.to_local(scope)
        .ok_or(V8BindingError::Allocation("image decode owner context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let _entry = owner.enter_retained_allocation(scope)?;
    let Some(selection) = selected_native_image_request(scope, &owner, node)? else {
        return rejected(scope, "EncodingError");
    };
    if selection.source.is_empty() || selection.url.is_none() {
        return rejected(scope, "EncodingError");
    }
    prepare_dynamic_image_subtree(scope, &owner, node)?;
    let generation = owner.image_sources.borrow().get(&node)
        .filter(|key| same_request(key, &selection.key))
        .and_then(|_| owner.image_generations.borrow().get(&node).copied());
    admit_selected(scope, &owner, node, generation, Some(selection.key), receiver)
}

struct Observer {
    owner: RcWeak<V8BindingState>,
    owner_id: u64,
    generation: Option<HostOperationId>,
    selection: Option<ImageRequestKey>,
    invalidated: bool,
    _selection_allocation: Option<std::sync::Arc<dyn Send + Sync>>,
    stream_epoch: u64,
    target: submissions::RootedTarget,
    resolver: v8::Weak<v8::PromiseResolver>,
    serial: u64,
    ready: bool,
    outcome: Option<bool>,
}

#[derive(Default)]
struct Queue {
    observers: Vec<Observer>,
    _capacity: Option<std::sync::Arc<dyn Send + Sync>>,
    next_serial: u64,
    in_flight: usize,
    #[cfg(test)]
    fail_next_settlement: bool,
}

fn queue(scope: &mut v8::PinScope) -> Rc<RefCell<Queue>> {
    if let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>() {
        return queue.clone();
    }
    let queue = Rc::new(RefCell::new(Queue::default()));
    scope.set_slot(queue.clone());
    queue
}

pub(super) fn admit<'s>(
    scope: &mut v8::PinScope<'s, '_>, owner: &Rc<V8BindingState>,
    node: NodeId, generation: HostOperationId, receiver: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    admit_selected(scope, owner, node, Some(generation), None, receiver)
}

fn admit_selected<'s>(
    scope: &mut v8::PinScope<'s, '_>, owner: &Rc<V8BindingState>, node: NodeId,
    generation: Option<HostOperationId>, selection: Option<ImageRequestKey>,
    receiver: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let queue = queue(scope);
    queue.borrow_mut().observers.retain(|observer| observer.resolver.to_local(scope).is_some());
    if queue.borrow().observers.len() + queue.borrow().in_flight >= MAX_OBSERVERS {
        return Err(V8BindingError::Unsupported { interface: "HTMLImageElement".into(), member: "QuotaExceededError".into() });
    }
    let _entry = owner.enter_retained_allocation(scope)?;
    if queue.borrow().observers.capacity() == 0 {
        let bytes = MAX_OBSERVERS.checked_mul(std::mem::size_of::<Observer>())
            .ok_or(V8BindingError::Allocation("image decode observer bytes"))?;
        let allocation = owner.retained_allocation.borrow().clone();
        let lease = allocation.map(|allocation| allocation.reserve(bytes))
            .transpose().map_err(V8BindingError::EventCallback)?;
        let mut observers = Vec::new();
        observers.try_reserve_exact(MAX_OBSERVERS)
            .map_err(|_| V8BindingError::Allocation("image decode observers"))?;
        if observers.capacity() > MAX_OBSERVERS {
            return Err(V8BindingError::Allocation("image decode observer admission"));
        }
        let mut state = queue.borrow_mut();
        if !state.observers.is_empty() || state.observers.capacity() != 0 {
            return Err(V8BindingError::Allocation("image decode capacity changed"));
        }
        state.observers = observers;
        state._capacity = lease;
    }
    let target = submissions::RootedTarget::new(owner, node)?;
    let bytes = selection.as_ref().map_or(Some(0), |key|
        key.source.capacity().checked_add(key.base_url.capacity())
            .and_then(|bytes| bytes.checked_add(key.referrer_policy.capacity())))
        .ok_or(V8BindingError::Allocation("image decode selection bytes"))?;
    let allocation = owner.retained_allocation.borrow().clone();
    let selection_allocation = allocation.map(|allocation| allocation.reserve(bytes))
        .transpose().map_err(V8BindingError::EventCallback)?;
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("image decode promise"))?;
    let promise = resolver.get_promise(scope);
    for (name, value) in [
        ("oryn.image.decode.resolver", resolver.into()),
        ("oryn.image.decode.receiver", receiver.into()),
    ] {
        let name = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("image decode private key"))?;
        let key = v8::Private::for_api(scope, Some(name));
        if promise.set_private(scope, key, value) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
    }
    if owner.execution_retired() || generation.is_some_and(|generation|
        !owner.image_request_is_current(node, generation)) {
        return Err(V8BindingError::Unsupported { interface: "HTMLImageElement".into(), member: "EncodingError".into() });
    }
    let serial = queue.borrow().next_serial.checked_add(1)
        .ok_or(V8BindingError::Allocation("image decode serial overflow"))?;
    let data = v8::BigInt::new_from_u64(scope, serial);
    let callback = v8::Function::builder(ready_microtask).data(data.into()).build(scope)
        .ok_or(V8BindingError::Allocation("image decode microtask"))?;
    queue.borrow_mut().next_serial = serial;
    queue.borrow_mut().observers.push(Observer {
        owner: Rc::downgrade(owner), owner_id: owner.instance_id,
        generation, stream_epoch: owner.document_stream_epoch(), target,
        selection, invalidated: false, _selection_allocation: selection_allocation,
        resolver: v8::Weak::new(scope, resolver),
        serial, ready: false, outcome: None,
    });
    scope.enqueue_microtask(callback);
    Ok(promise)
}

fn settle(scope: &mut v8::PinScope, observer: &Observer, success: bool)
    -> Result<(), V8BindingError> {
    #[cfg(test)]
    {
        let queue = queue(scope);
        let fail = std::mem::take(&mut queue.borrow_mut().fail_next_settlement);
        if fail { return Err(V8BindingError::Allocation("injected image settlement failure")); }
    }
    let Some(resolver) = observer.resolver.to_local(scope) else { return Ok(()); };
    let promise = resolver.get_promise(scope);
    let receiver_name = v8::String::new(scope, "oryn.image.decode.receiver")
        .ok_or(V8BindingError::Allocation("image decode settled key"))?;
    let receiver_key = v8::Private::for_api(scope, Some(receiver_name));
    let resolver_name = v8::String::new(scope, "oryn.image.decode.resolver")
        .ok_or(V8BindingError::Allocation("image decode settled key"))?;
    let resolver_key = v8::Private::for_api(scope, Some(resolver_name));
    // Resolving the native observer does not call retired image handlers or
    // restore fetch authority. V8 routes its reactions to their own realms.
    let value = if success { v8::undefined(scope).into() }
        else { dom_exception_value(scope, "EncodingError")? };
    let result = if success { resolver.resolve(scope, value) }
        else { resolver.reject(scope, value) };
    if result != Some(true) { return Err(V8BindingError::JavaScriptException); }
    // Once settled, an observed promise no longer needs to pin the image or
    // its resolver. Remove private custody edges without running author code.
    for key in [receiver_key, resolver_key] { promise.delete_private(scope, key); }
    Ok(())
}

fn same_request(a: &ImageRequestKey, b: &ImageRequestKey) -> bool {
    // Density-only adjustments reuse the same native bytes/request.
    a.source == b.source && a.base_url == b.base_url && a.cors_mode == b.cors_mode
        && a.referrer_policy == b.referrer_policy && a.upgrade_mixed_image == b.upgrade_mixed_image
}

pub(super) fn selected(scope: &mut v8::PinScope, owner: &V8BindingState,
    node: NodeId, selection: Option<&ImageRequestKey>) {
    let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>().cloned() else { return; };
    for observer in &mut queue.borrow_mut().observers {
        if observer.owner_id == owner.instance_id && observer.target.node == node
            && let Some(previous) = observer.selection.as_ref()
            && selection.is_none_or(|selection| !same_request(previous, selection))
        {
            observer.invalidated = true;
        }
    }
}

pub(super) fn admitted(scope: &mut v8::PinScope, owner: &V8BindingState,
    node: NodeId, generation: HostOperationId, selection: &ImageRequestKey) {
    let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>().cloned() else { return; };
    for observer in &mut queue.borrow_mut().observers {
        if observer.owner_id == owner.instance_id && observer.target.node == node
            && observer.generation.is_none() && !observer.invalidated
            && observer.selection.as_ref().is_some_and(|previous| same_request(previous, selection))
        {
            observer.generation = Some(generation);
        }
    }
}

pub(super) fn terminal(scope: &mut v8::PinScope, owner: &V8BindingState,
    node: NodeId, success: bool) -> Result<(), V8BindingError> {
    let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>().cloned() else { return Ok(()); };
    {
        let mut queue = queue.borrow_mut();
        for observer in &mut queue.observers {
            if observer.owner_id == owner.instance_id && observer.target.node == node
                && observer.generation.is_some_and(|generation|
                    owner.image_generations.borrow().get(&node) == Some(&generation))
            {
                observer.outcome = Some(success && observer.generation.is_some_and(|generation|
                    owner.image_request_is_current(node, generation)));
            }
        };
    }
    checkpoint(scope)
}

fn ready_microtask(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let Ok(serial) = args.data().try_cast::<v8::BigInt>() else { return; };
    let serial = serial.u64_value().0;
    let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>().cloned() else { return; };
    {
        let mut queue = queue.borrow_mut();
        for observer in &mut queue.observers {
            if observer.serial != serial { continue; }
            observer.ready = true;
            if let Some(owner) = observer.owner.upgrade() {
                let node = observer.target.node;
                if observer.generation.is_some_and(|generation| owner.image_request_is_current(node, generation)) {
                    if owner.failed_images.borrow().contains(&node) {
                        observer.outcome = Some(false);
                    } else if owner.decoded_images.borrow().contains_key(&node)
                        || owner.decoded_svg_images.borrow().contains_key(&node)
                    {
                        observer.outcome = Some(true);
                    }
                }
            }
        }
    }
    if let Err(error) = checkpoint(scope) { throw_binding_error(scope, &error); }
}

pub(super) fn checkpoint(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let Some(queue) = scope.get_slot::<Rc<RefCell<Queue>>>().cloned() else { return Ok(()); };
    loop {
        let selected = {
            let mut queue = queue.borrow_mut();
            let selected = queue.observers.iter().enumerate().find_map(|(index, observer)| {
                let (dom, node) = observer.target.native_lease().current();
                let invalid = observer.invalidated || observer.resolver.to_local(scope).is_none()
                    || observer.owner.upgrade().is_none_or(|owner|
                        owner.execution_retired() || owner.document_load_stopped()
                        || !Rc::ptr_eq(&dom, &owner.dom) || node != observer.target.node
                        || dom.borrow().node_document(node).ok().flatten() != Some(owner.document_root())
                        || owner.document_stream_epoch() != observer.stream_epoch
                        || observer.generation.is_some_and(|generation|
                            !owner.image_request_is_current(node, generation)));
                if invalid { Some((index, false)) }
                else if observer.ready { observer.outcome.map(|success| (index, success)) }
                else { None }
            });
            selected.map(|(index, success)| {
                queue.in_flight += 1;
                (index, queue.observers.remove(index), success)
            })
        };
        let Some((index, observer, success)) = selected else { break; };
        let result = settle(scope, &observer, success);
        {
            let mut queue = queue.borrow_mut();
            queue.in_flight -= 1;
            if result.is_err() {
                let index = index.min(queue.observers.len());
                queue.observers.insert(index, observer);
            }
        }
        result?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_observers_are_distinct_and_native_terminal_settles_all() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let generation = HostOperationId(41);
        owner.image_generations.borrow_mut().insert(node, generation);
        let receiver = wrap_node(scope, &owner, node).unwrap();
        let first = admit(scope, &owner, node, generation, receiver).unwrap();
        let second = admit(scope, &owner, node, generation, receiver).unwrap();
        assert!(!first.strict_equals(second.into()));
        assert_eq!(first.state(), v8::PromiseState::Pending);
        terminal(scope, &owner, node, true).unwrap();
        assert_eq!(first.state(), v8::PromiseState::Pending);
        scope.perform_microtask_checkpoint();
        assert_eq!(first.state(), v8::PromiseState::Fulfilled);
        assert_eq!(second.state(), v8::PromiseState::Fulfilled);
        assert!(first.result(scope).is_undefined());
        assert!(queue(scope).borrow().observers.is_empty());
    }

    #[test]
    fn replacement_and_owner_retirement_reject_observed_decode_promises() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
        let receiver = wrap_node(scope, &owner, node).unwrap();
        let replaced = admit(scope, &owner, node, HostOperationId(41), receiver).unwrap();
        owner.image_generations.borrow_mut().insert(node, HostOperationId(42));
        checkpoint(scope).unwrap();
        assert_eq!(replaced.state(), v8::PromiseState::Rejected);
        let retired = admit(scope, &owner, node, HostOperationId(42), receiver).unwrap();
        owner.retire_execution();
        checkpoint(scope).unwrap();
        assert_eq!(retired.state(), v8::PromiseState::Rejected);
        assert!(queue(scope).borrow().observers.is_empty());
    }

    #[test]
    fn already_decoded_observers_wait_for_microtask_and_reject_replaced_request() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        owner.decoded_images.borrow_mut().insert(node, DecodedImage {
            origin_clean: true, width: 1, height: 1, rgba: vec![0, 0, 0, 255],
        });
        owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
        let receiver = wrap_node(scope, &owner, node).unwrap();
        let replaced = admit(scope, &owner, node, HostOperationId(41), receiver).unwrap();
        checkpoint(scope).unwrap();
        assert_eq!(replaced.state(), v8::PromiseState::Pending);
        owner.image_generations.borrow_mut().insert(node, HostOperationId(42));
        scope.perform_microtask_checkpoint();
        assert_eq!(replaced.state(), v8::PromiseState::Rejected);
        let stable = admit(scope, &owner, node, HostOperationId(42), receiver).unwrap();
        checkpoint(scope).unwrap();
        assert_eq!(stable.state(), v8::PromiseState::Pending);
        scope.perform_microtask_checkpoint();
        assert_eq!(stable.state(), v8::PromiseState::Fulfilled);
        assert!(stable.result(scope).is_undefined());
    }

    #[test]
    fn deferred_decode_observer_binds_only_matching_admitted_request() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let receiver = wrap_node(scope, &owner, node).unwrap();
        let key = ImageRequestKey { source: "https://example.test/image.png".into(), ..Default::default() };
        let pending = admit_selected(scope, &owner, node, None, Some(key.clone()), receiver).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(pending.state(), v8::PromiseState::Pending);
        let replacement = ImageRequestKey { source: "https://example.test/other.png".into(), ..key.clone() };
        admitted(scope, &owner, node, HostOperationId(41), &replacement);
        assert!(queue(scope).borrow().observers[0].generation.is_none());
        owner.image_generations.borrow_mut().insert(node, HostOperationId(42));
        admitted(scope, &owner, node, HostOperationId(42), &key);
        terminal(scope, &owner, node, true).unwrap();
        assert_eq!(pending.state(), v8::PromiseState::Fulfilled);
        let changed = admit_selected(scope, &owner, node, None, Some(key), receiver).unwrap();
        selected(scope, &owner, node, Some(&replacement));
        checkpoint(scope).unwrap();
        assert_eq!(changed.state(), v8::PromiseState::Rejected);
    }

    #[test]
    fn public_image_decode_rejects_missing_source_and_bad_receiver_and_uses_cached_native_pixels() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image src='cached.png'></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        owner.commit_location("https://example.test/page").unwrap();
        let selection = selected_native_image_request(scope, &owner, node).unwrap().unwrap();
        owner.image_sources.borrow_mut().insert(node, selection.key);
        owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
        owner.decoded_images.borrow_mut().insert(node, DecodedImage {
            origin_clean: true, width: 1, height: 1, rgba: vec![0, 0, 0, 255],
        });
        let source = v8::String::new(scope, "document.getElementById('image').decode()").unwrap();
        let cached = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap()
            .try_cast::<v8::Promise>().unwrap();
        assert_eq!(cached.state(), v8::PromiseState::Pending);
        scope.perform_microtask_checkpoint();
        assert_eq!(cached.state(), v8::PromiseState::Fulfilled);
        for (script, expected) in [
            ("new Image().decode()", "EncodingError"),
            ("document.implementation.createHTMLDocument('').createElement('img').decode()", "EncodingError"),
            ("HTMLImageElement.prototype.decode.call({})", "TypeError"),
        ] {
            let source = v8::String::new(scope, script).unwrap();
            let promise = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap()
                .try_cast::<v8::Promise>().unwrap();
            scope.perform_microtask_checkpoint();
            assert_eq!(promise.state(), v8::PromiseState::Rejected);
            let result = promise.result(scope).to_object(scope).unwrap();
            let name = v8::String::new(scope, "name").unwrap();
            assert_eq!(result.get(scope, name.into()).unwrap().to_rust_string_lossy(scope), expected);
        }
    }

    #[test]
    fn decode_observer_limit_rejects_with_native_quota_exception() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image src='cached.png'></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        owner.commit_location("https://example.test/page").unwrap();
        let selection = selected_native_image_request(scope, &owner, node).unwrap().unwrap();
        owner.image_sources.borrow_mut().insert(node, selection.key);
        owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
        let receiver = wrap_node(scope, &owner, node).unwrap();
        for _ in 0..MAX_OBSERVERS { admit(scope, &owner, node, HostOperationId(41), receiver).unwrap(); }
        assert!(matches!(admit(scope, &owner, node, HostOperationId(41), receiver),
            Err(V8BindingError::Unsupported { member, .. }) if member == "QuotaExceededError"));
        let source = v8::String::new(scope, "document.getElementById('image').decode()").unwrap();
        let promise = v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap()
            .try_cast::<v8::Promise>().unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        let error = promise.result(scope).to_object(scope).unwrap();
        let name = v8::String::new(scope, "name").unwrap();
        assert_eq!(error.get(scope, name.into()).unwrap().to_rust_string_lossy(scope), "QuotaExceededError");
        assert_eq!(queue(scope).borrow().observers.len(), MAX_OBSERVERS);
    }

    #[test]
    fn surviving_realm_observes_retired_image_rejection_and_private_custody_is_released() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let handles, &mut isolate);
        let parent = v8::Context::new(handles, Default::default());
        let child = v8::Context::new(handles, Default::default());
        child.set_security_token(parent.get_security_token(handles));
        let (owner, promise) = {
            let scope = &mut v8::ContextScope::new(handles, child);
            let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
            let node = dom.borrow().find_by_id("image").unwrap();
            let owner = install_generated_bindings(scope, child, dom, RealmId(1), 1).unwrap();
            owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
            let receiver = wrap_node(scope, &owner, node).unwrap();
            let promise = admit(scope, &owner, node, HostOperationId(41), receiver).unwrap();
            (owner, v8::Global::new(scope, promise))
        };
        let scope = &mut v8::ContextScope::new(handles, parent);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let _parent_owner = install_generated_bindings(scope, parent, dom, RealmId(0), 1).unwrap();
        let observed = v8::Local::new(scope, &promise);
        let key = v8::String::new(scope, "childDecode").unwrap();
        parent.global(scope).set(scope, key.into(), observed.into());
        let source = v8::String::new(scope, "globalThis.seen='pending';childDecode.catch(e=>seen=e.name)").unwrap();
        v8::Script::compile(scope, source, None).unwrap().run(scope).unwrap();
        owner.retire_execution();
        checkpoint(scope).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(observed.state(), v8::PromiseState::Rejected);
        let seen = v8::String::new(scope, "seen").unwrap();
        assert_eq!(parent.global(scope).get(scope, seen.into()).unwrap().to_rust_string_lossy(scope), "EncodingError");
        for name in ["oryn.image.decode.receiver", "oryn.image.decode.resolver"] {
            let name = v8::String::new(scope, name).unwrap();
            let key = v8::Private::for_api(scope, Some(name));
            assert!(observed.get_private(scope, key).unwrap().is_undefined());
        }
        assert!(queue(scope).borrow().observers.is_empty());
    }

    #[test]
    fn failed_settlement_retains_original_observer_for_retry_without_capacity_loss() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body><img id=image></body>").into_runtime().unwrap()));
        let node = dom.borrow().find_by_id("image").unwrap();
        let owner = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        owner.image_generations.borrow_mut().insert(node, HostOperationId(41));
        let receiver = wrap_node(scope, &owner, node).unwrap();
        let promise = admit(scope, &owner, node, HostOperationId(41), receiver).unwrap();
        let second = admit(scope, &owner, node, HostOperationId(41), receiver).unwrap();
        {
            let queue = queue(scope);
            let mut queue = queue.borrow_mut();
            for observer in &mut queue.observers { observer.ready = true; }
            queue.fail_next_settlement = true;
        }
        assert!(terminal(scope, &owner, node, true).is_err());
        assert_eq!(promise.state(), v8::PromiseState::Pending);
        assert_eq!(second.state(), v8::PromiseState::Pending);
        assert_eq!(queue(scope).borrow().observers.len(), 2);
        assert_eq!(queue(scope).borrow().observers[0].serial, 1, "failure preserves observer order");
        assert_eq!(queue(scope).borrow().in_flight, 0);
        checkpoint(scope).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        assert_eq!(second.state(), v8::PromiseState::Fulfilled);
        assert!(queue(scope).borrow().observers.is_empty());
    }
}

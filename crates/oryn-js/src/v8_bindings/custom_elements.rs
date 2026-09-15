//! Native definition, construction and reaction ownership for autonomous elements.
use super::*;

struct Definition {
    constructor: v8::Global<v8::Function>,
    callbacks: [Option<v8::Global<v8::Function>>; 4],
    observed: Vec<String>,
}
struct Element {
    definition: Rc<Definition>,
    connected: bool,
    document: NodeId,
    failed: bool,
}
enum Reaction {
    Upgrade,
    Callback {
        target: v8::Global<v8::Object>,
        callback: v8::Global<v8::Function>,
        arguments: Vec<v8::Global<v8::Value>>,
    },
}
#[derive(Default)]
pub(super) struct Registry {
    definitions: RefCell<BTreeMap<String, Rc<Definition>>>,
    elements: RefCell<BTreeMap<NodeId, Element>>,
    reactions: RefCell<BTreeMap<NodeId, VecDeque<Reaction>>>,
    pending_elements: RefCell<VecDeque<NodeId>>,
    pending_upgrades: RefCell<BTreeSet<NodeId>>,
    promises: RefCell<BTreeMap<String, v8::Global<v8::PromiseResolver>>>,
    object: RefCell<Option<v8::Weak<v8::Object>>>,
    cursor: Cell<oryn_dom::MutationCursor>,
    processing: Cell<bool>,
    constructing: Cell<usize>,
    defining: Cell<bool>,
    invocation_depth: Cell<usize>,
    remaining_reactions: Cell<usize>,
}

struct DefinitionGuard<'a>(&'a Cell<bool>);
impl Drop for DefinitionGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl Registry {
    pub(super) fn has_pending(&self, state: &V8BindingState) -> bool {
        !self.definitions.borrow().is_empty()
            && (!self.reactions.borrow().is_empty()
                || self.cursor.get() != state.dom.borrow().mutation_cursor())
    }
}

pub(super) fn construction_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Value>,
) -> Result<Option<(WrapperId, NodeId)>, V8BindingError> {
    let definition = state
        .custom_elements
        .definitions
        .borrow()
        .iter()
        .find(|(_, definition)| {
            v8::Local::new(scope, &definition.constructor).strict_equals(target)
        })
        .map(|(name, definition)| (name.clone(), definition.clone()));
    let Some((name, definition)) = definition else {
        return Ok(None);
    };
    let node = state.dom.borrow_mut().create_element(&name);
    state
        .dom
        .borrow_mut()
        .set_node_document(node, state.document)?;
    let object = wrap_node(scope, state, node)?;
    let wrapper =
        receiver_wrapper_id(scope, object).ok_or(V8BindingError::Allocation("custom wrapper"))?;
    state.custom_elements.elements.borrow_mut().insert(
        node,
        Element {
            definition,
            connected: false,
            document: state.document,
            failed: false,
        },
    );
    Ok(Some((wrapper, node)))
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let object = v8::Object::new(scope);
    for name in ["define", "get", "whenDefined", "upgrade"] {
        let key = v8_string(scope, name)?;
        let function = v8::Function::builder(registry_callback)
            .data(key)
            .build(scope)
            .ok_or(V8BindingError::Allocation("custom element registry method"))?;
        object.set(scope, key, function.into());
    }
    *state.custom_elements.object.borrow_mut() = Some(v8::Weak::new(scope, object));
    state
        .custom_elements
        .cursor
        .set(state.dom.borrow().mutation_cursor());
    let global = scope.get_current_context().global(scope);
    let key = v8_string(scope, "customElements")?;
    global.set(scope, key, object.into());
    let key = v8_string(scope, "__oryn_run_custom_reactions")?;
    let function = v8::Function::new(scope, checkpoint_callback)
        .ok_or(V8BindingError::Allocation("custom reaction checkpoint"))?;
    global.set(scope, key, function.into());
    Ok(())
}

fn registry_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    if !state
        .custom_elements
        .object
        .borrow()
        .as_ref()
        .and_then(|object| object.to_local(scope))
        .is_some_and(|object| object.strict_equals(args.this().into()))
    {
        throw_type_error(scope, "CustomElementRegistry requires its native receiver");
        return;
    }
    let operation = args.data().to_rust_string_lossy(scope);
    let result = (|| -> Result<(), V8BindingError> {
        if operation == "upgrade" {
            let node = argument_node(scope, &state, args.get(0))?;
            for node in subtree(&state, node) {
                enqueue_upgrade(scope, &state, node, true)?;
            }
            return finish(scope, &state);
        }
        let Some(name) = args.get(0).to_string(scope) else {
            return Ok(());
        };
        let name = name.to_rust_string_lossy(scope);
        if operation == "get" {
            if let Some(definition) = state.custom_elements.definitions.borrow().get(&name) {
                rv.set(v8::Local::new(scope, &definition.constructor).into());
            }
            return Ok(());
        }
        if !is_valid_custom_element_name(&name) {
            let error = dom_exception_value(scope, "SyntaxError")?;
            if operation == "whenDefined" {
                let resolver = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("whenDefined resolver"))?;
                resolver.reject(scope, error);
                rv.set(resolver.get_promise(scope).into());
            } else {
                scope.throw_exception(error);
            }
            return Ok(());
        }
        if operation == "whenDefined" {
            let resolver =
                if let Some(resolver) = state.custom_elements.promises.borrow().get(&name) {
                    v8::Local::new(scope, resolver)
                } else {
                    v8::PromiseResolver::new(scope)
                        .ok_or(V8BindingError::Allocation("whenDefined resolver"))?
                };
            if let Some(definition) = state.custom_elements.definitions.borrow().get(&name) {
                let constructor = v8::Local::new(scope, &definition.constructor);
                resolver.resolve(scope, constructor.into());
            } else {
                state
                    .custom_elements
                    .promises
                    .borrow_mut()
                    .insert(name, v8::Global::new(scope, resolver));
            }
            rv.set(resolver.get_promise(scope).into());
            return Ok(());
        }
        let Ok(constructor) = v8::Local::<v8::Function>::try_from(args.get(1)) else {
            throw_type_error(scope, "custom element constructor must be callable");
            return Ok(());
        };
        if state
            .custom_elements
            .definitions
            .borrow()
            .contains_key(&name)
            || state
                .custom_elements
                .definitions
                .borrow()
                .values()
                .any(|definition| {
                    v8::Local::new(scope, &definition.constructor).strict_equals(constructor.into())
                })
        {
            let error = dom_exception_value(scope, "NotSupportedError")?;
            scope.throw_exception(error);
            return Ok(());
        }
        if state.custom_elements.defining.replace(true) {
            let error = dom_exception_value(scope, "NotSupportedError")?;
            scope.throw_exception(error);
            return Ok(());
        }
        let guard = DefinitionGuard(&state.custom_elements.defining);
        let key = v8_string(scope, "prototype")?;
        let Some(prototype) = constructor
            .get(scope, key)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            throw_type_error(
                scope,
                "custom element constructor requires an object prototype",
            );
            return Ok(());
        };
        let mut callbacks = [None, None, None, None];
        for (index, name) in [
            "connectedCallback",
            "disconnectedCallback",
            "adoptedCallback",
            "attributeChangedCallback",
        ]
        .iter()
        .enumerate()
        {
            let key = v8_string(scope, name)?;
            let Some(value) = prototype.get(scope, key) else {
                return Ok(());
            };
            if !value.is_undefined() {
                let Ok(callback) = v8::Local::<v8::Function>::try_from(value) else {
                    throw_type_error(scope, "custom element lifecycle callback must be callable");
                    return Ok(());
                };
                callbacks[index] = Some(v8::Global::new(scope, callback));
            }
        }
        let mut observed = Vec::new();
        if callbacks[3].is_some() {
            let key = v8_string(scope, "observedAttributes")?;
            let Some(value) = constructor.get(scope, key) else {
                return Ok(());
            };
            if !value.is_undefined() {
                observed = dom_string_sequence_from_iterable(scope, value)?;
            }
        }
        drop(guard);
        finish(scope, &state)?;
        let definition = Rc::new(Definition {
            constructor: v8::Global::new(scope, constructor),
            callbacks,
            observed,
        });
        state
            .custom_elements
            .definitions
            .borrow_mut()
            .insert(name.clone(), definition);
        if let Some(resolver) = state.custom_elements.promises.borrow_mut().remove(&name) {
            v8::Local::new(scope, &resolver).resolve(scope, constructor.into());
        }
        enqueue_upgrade_subtree(scope, &state, state.document)?;
        finish(scope, &state)
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn checkpoint_callback(
    scope: &mut v8::PinScope,
    _: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    checkpoint(scope);
}
pub(super) fn checkpoint(scope: &mut v8::PinScope) {
    let Some(current) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    for state in agent_states(scope, &current) {
        if !state.custom_elements.has_pending(&state) {
            continue;
        }
        let Some(context) = state
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
        else {
            continue;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Err(error) = finish(scope, &state) {
            throw_binding_error(scope, &error);
            return;
        }
    }
}

fn agent_states(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Vec<Rc<V8BindingState>> {
    scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .filter(|candidate| Rc::ptr_eq(&candidate.dom, &state.dom))
                .collect()
        })
        .unwrap_or_else(|| vec![state.clone()])
}

fn subtree(state: &V8BindingState, root: NodeId) -> Vec<NodeId> {
    let dom = state.dom.borrow();
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Ok(record) = dom.arena().get(node) {
            result.push(node);
            stack.extend(record.children.iter().rev().copied());
        }
    }
    result
}

fn enqueue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    index: usize,
    arguments: &[v8::Local<v8::Value>],
) -> Result<(), V8BindingError> {
    let callback = state
        .custom_elements
        .elements
        .borrow()
        .get(&node)
        .filter(|element| !element.failed)
        .and_then(|element| element.definition.callbacks[index].clone());
    if let Some(callback) = callback {
        let target = wrap_node(scope, state, node)?;
        state
            .custom_elements
            .reactions
            .borrow_mut()
            .entry(node)
            .or_default()
            .push_back(Reaction::Callback {
                target: v8::Global::new(scope, target),
                callback,
                arguments: arguments
                    .iter()
                    .map(|argument| v8::Global::new(scope, *argument))
                    .collect(),
            });
        state
            .custom_elements
            .pending_elements
            .borrow_mut()
            .push_back(node);
    }
    Ok(())
}

fn connection(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    connected: bool,
) -> Result<(), V8BindingError> {
    let changed = {
        let mut elements = state.custom_elements.elements.borrow_mut();
        elements.get_mut(&node).is_some_and(|element| {
            let changed = element.connected != connected && !element.failed;
            element.connected = connected;
            changed
        })
    };
    if changed {
        enqueue(scope, state, node, if connected { 0 } else { 1 }, &[])?;
    }
    Ok(())
}

pub(super) fn enqueue_upgrade_subtree(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    if state.custom_elements.definitions.borrow().is_empty() {
        return Ok(());
    }
    for node in subtree(state, root) {
        enqueue_upgrade(scope, state, node, false)?;
    }
    Ok(())
}

fn enqueue_upgrade(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    explicit: bool,
) -> Result<(), V8BindingError> {
    {
        let dom = state.dom.borrow();
        if !explicit && dom.arena().node_document(node)? != Some(state.document) {
            return Ok(());
        }
        let NodeKind::Element { name } = &dom.arena().get(node)?.kind else {
            return Ok(());
        };
        if name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
            || !state
                .custom_elements
                .definitions
                .borrow()
                .contains_key(name.local.as_ref())
        {
            return Ok(());
        }
    }
    if agent_states(scope, state).iter().any(|owner| {
        owner.custom_elements.elements.borrow().contains_key(&node)
            || owner
                .custom_elements
                .pending_upgrades
                .borrow()
                .contains(&node)
    }) {
        return Ok(());
    }
    state
        .custom_elements
        .pending_upgrades
        .borrow_mut()
        .insert(node);
    state
        .custom_elements
        .reactions
        .borrow_mut()
        .entry(node)
        .or_default()
        .push_back(Reaction::Upgrade);
    state
        .custom_elements
        .pending_elements
        .borrow_mut()
        .push_back(node);
    Ok(())
}

pub(super) fn upgrade_for_document(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let document = state.dom.borrow().arena().node_document(root)?;
    if let Some(owner) = agent_states(scope, state)
        .into_iter()
        .find(|owner| Some(owner.document) == document)
    {
        enqueue_upgrade_subtree(scope, &owner, root)?;
    }
    Ok(())
}
pub(super) fn upgrade(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    if state.dom.borrow().arena().node_document(node)? != Some(state.document) {
        return Ok(());
    }
    upgrade_with_registry(scope, state, node)
}

fn upgrade_with_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    // Adoption does not change a custom element's definition. Another realm
    // in this agent may observe the same insertion journal, but must not
    // construct the already-custom element again using its own registry.
    if agent_states(scope, state)
        .iter()
        .any(|owner| owner.custom_elements.elements.borrow().contains_key(&node))
    {
        return Ok(());
    }
    let (name, document) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Ok(());
        };
        if name.ns.as_ref() != "http://www.w3.org/1999/xhtml" {
            return Ok(());
        }
        (
            name.local.to_string(),
            dom.arena().node_document(node)?.unwrap_or(state.document),
        )
    };
    let Some(definition) = state
        .custom_elements
        .definitions
        .borrow()
        .get(&name)
        .cloned()
    else {
        return Ok(());
    };
    let original = wrap_node(scope, state, node)?;
    let initial_attributes = state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .records()
        .cloned()
        .collect::<Vec<_>>();
    let wrapper =
        receiver_wrapper_id(scope, original).ok_or(V8BindingError::Allocation("custom wrapper"))?;
    state.custom_elements.elements.borrow_mut().insert(
        node,
        Element {
            definition: definition.clone(),
            connected: false,
            document,
            failed: true,
        },
    );
    let previous = state.pending_custom_upgrade.replace(Some((wrapper, node)));
    state
        .custom_elements
        .constructing
        .set(state.custom_elements.constructing.get() + 1);
    let success = {
        v8::tc_scope!(let tc, scope);
        let constructor = v8::Local::new(tc, &definition.constructor);
        let result = constructor.new_instance(tc, &[]);
        let success = result.is_some_and(|object| {
            embedded_node_identity(tc, object).is_some_and(|(id, _, _)| id == node)
        });
        if let Some(exception) = tc.exception() {
            let message = tc.message();
            let _ = report_exception(tc, exception, message);
        }
        success
    };
    state
        .custom_elements
        .constructing
        .set(state.custom_elements.constructing.get() - 1);
    state.pending_custom_upgrade.set(previous);
    if success {
        state
            .custom_elements
            .elements
            .borrow_mut()
            .get_mut(&node)
            .unwrap()
            .failed = false;
        for attribute in initial_attributes {
            attribute_changed(
                scope,
                state,
                node,
                &attribute.local_name,
                None,
                Some(&attribute.value),
                attribute.namespace.as_deref(),
            )?;
        }
        let connected = state.dom.borrow().is_document_connected(node)?;
        connection(scope, state, node, connected)?;
    }
    Ok(())
}

pub(super) fn finish(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.custom_elements.definitions.borrow().is_empty() {
        state
            .custom_elements
            .cursor
            .set(state.dom.borrow().mutation_cursor());
        return Ok(());
    }
    if state.custom_elements.constructing.get() > 0
        || state.custom_elements.processing.replace(true)
    {
        return Ok(());
    }
    let depth = state.custom_elements.invocation_depth.get();
    if depth >= 64 {
        state.custom_elements.processing.set(false);
        return Err(V8BindingError::EventCallback(
            "custom-element reaction nesting limit exceeded".into(),
        ));
    }
    state.custom_elements.invocation_depth.set(depth + 1);
    if depth == 0 {
        state.custom_elements.remaining_reactions.set(1024);
    }
    let result = (|| {
        for _ in 0..1024 {
            let batch = state
                .dom
                .borrow()
                .mutations_since(state.custom_elements.cursor.get());
            state.custom_elements.cursor.set(batch.next);
            if batch.truncated {
                return Err(V8BindingError::EventCallback(
                    "custom-element mutation journal overflow".into(),
                ));
            }
            for mutation in batch.mutations {
                match mutation {
                    Mutation::Removed { node, .. } => {
                        for node in subtree(state, node) {
                            connection(scope, state, node, false)?;
                        }
                    }
                    // Allocation records use parent=None; they are not DOM
                    // insertions and must not observe later connectedness.
                    Mutation::Inserted {
                        node,
                        parent: Some(_),
                        ..
                    } => {
                        enqueue_upgrade_subtree(scope, state, node)?;
                        for node in subtree(state, node) {
                            let connected = state.dom.borrow().is_document_connected(node)?;
                            connection(scope, state, node, connected)?;
                        }
                    }
                    Mutation::Attribute {
                        node,
                        name,
                        old_value,
                        new_value,
                        namespace,
                    } => {
                        attribute_changed(
                            scope,
                            state,
                            node,
                            name.rsplit(':').next().unwrap_or(&name),
                            old_value.as_deref(),
                            new_value.as_deref(),
                            namespace.as_deref(),
                        )?;
                    }
                    Mutation::DocumentOwnership { node } => {
                        let document = state.dom.borrow().arena().node_document(node)?;
                        if let Some(document) = document {
                            let old = {
                                let mut elements = state.custom_elements.elements.borrow_mut();
                                elements.get_mut(&node).and_then(|element| {
                                    let old = element.document;
                                    element.document = document;
                                    (old != document).then_some(old)
                                })
                            };
                            if let Some(old) = old {
                                let old = wrap_node(scope, state, old)?;
                                let new = wrap_node(scope, state, document)?;
                                enqueue(scope, state, node, 2, &[old.into(), new.into()])?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            let queue = std::mem::take(&mut *state.custom_elements.pending_elements.borrow_mut());
            if queue.is_empty() {
                return Ok(());
            }
            // Invocation queues are separate; an element's reactions are shared.
            // Nested DOM operations flush only elements they enqueue, including
            // any reactions already pending for those elements in an outer call.
            state.custom_elements.processing.set(false);
            for node in queue {
                loop {
                    let reaction = state
                        .custom_elements
                        .reactions
                        .borrow_mut()
                        .get_mut(&node)
                        .and_then(VecDeque::pop_front);
                    let Some(reaction) = reaction else {
                        state.custom_elements.reactions.borrow_mut().remove(&node);
                        break;
                    };
                    let remaining = state.custom_elements.remaining_reactions.get();
                    if remaining == 0 {
                        return Err(V8BindingError::EventCallback(
                            "custom-element reaction limit exceeded".into(),
                        ));
                    }
                    state.custom_elements.remaining_reactions.set(remaining - 1);
                    let Reaction::Callback {
                        target,
                        callback,
                        arguments,
                    } = reaction
                    else {
                        state
                            .custom_elements
                            .pending_upgrades
                            .borrow_mut()
                            .remove(&node);
                        upgrade_with_registry(scope, state, node)?;
                        continue;
                    };
                    v8::tc_scope!(let tc, scope);
                    let callback = v8::Local::new(tc, &callback);
                    let target = v8::Local::new(tc, &target);
                    let args = arguments
                        .iter()
                        .map(|value| v8::Local::new(tc, value))
                        .collect::<Vec<_>>();
                    callback.call(tc, target.into(), &args);
                    if let Some(exception) = tc.exception() {
                        let message = tc.message();
                        let _ = report_exception(tc, exception, message);
                    }
                }
            }
            state.custom_elements.processing.set(true);
        }
        Err(V8BindingError::EventCallback(
            "custom-element reaction limit exceeded".into(),
        ))
    })();
    state.custom_elements.processing.set(false);
    state.custom_elements.invocation_depth.set(depth);
    result
}

fn attribute_changed(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    name: &str,
    old: Option<&str>,
    new: Option<&str>,
    namespace: Option<&str>,
) -> Result<(), V8BindingError> {
    let observed = state
        .custom_elements
        .elements
        .borrow()
        .get(&node)
        .is_some_and(|element| {
            element
                .definition
                .observed
                .iter()
                .any(|candidate| candidate == name)
        });
    if !observed {
        return Ok(());
    }
    let name = v8_string(scope, name)?;
    let old = match old {
        Some(value) => v8_string(scope, value)?,
        None => v8::null(scope).into(),
    };
    let new = match new {
        Some(value) => v8_string(scope, value)?,
        None => v8::null(scope).into(),
    };
    let namespace = match namespace {
        Some(value) => v8_string(scope, value)?,
        None => v8::null(scope).into(),
    };
    enqueue(scope, state, node, 3, &[name, old, new, namespace])
}

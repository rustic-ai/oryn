//! Native definition, construction and reaction ownership for autonomous elements.
type QueuedReaction = (NodeId, Option<Reaction>, Rc<ReactionTarget>, Option<u64>);

type FormStateValue = (
    Vec<(Option<String>, v8::Global<v8::Value>)>,
    v8::Global<v8::Value>,
);

type InitialReaction = (String, String, Option<String>);

type PendingRestoration = (
    std::sync::Arc<[crate::form_state::SavedFormControl]>,
    BTreeSet<usize>,
);

use super::*;

struct Definition {
    name: String,
    local_name: String,
    registry: u64,
    constructor: platform_listener_graph::Callback,
    callbacks: [Option<platform_listener_graph::Callback>; 9],
    form_associated: bool,
    observed: Vec<String>,
    disable_shadow: bool,
    disable_internals: bool,
}
#[derive(Clone)]
struct Element {
    internals_revision: Rc<()>,
    definition: Rc<Definition>,
    connected: bool,
    document: NodeId,
    failed: bool,
    upgrading: bool,
    internals_attached: bool,
    form_owner: Option<NodeId>,
    form_disabled: bool,
    submission: Vec<(Option<String>, v8::Global<v8::Value>)>,
    restoration: Option<v8::Global<v8::Value>>,
}
#[derive(Clone)]
enum Reaction {
    Upgrade {
        definition: Rc<Definition>,
        registry_root: v8::Global<v8::Object>,
    },
    Callback {
        target: v8::Global<v8::Object>,
        callback: v8::Global<v8::Function>,
        arguments: Vec<v8::Global<v8::Value>>,
        // Retried parser reactions retain the original document's dynamic
        // markup prohibition even after the tokenizer slice has returned.
        parser_document: Option<(Rc<V8BindingState>, NodeId)>,
    },
}
#[derive(Default)]
pub(super) struct Registry {
    global: Rc<DefinitionRegistry>,
    pending_restoration: RefCell<Option<PendingRestoration>>,
    elements: RefCell<BTreeMap<NodeId, Element>>,
    reactions: RefCell<BTreeMap<NodeId, VecDeque<Reaction>>>,
    queued_targets: RefCell<BTreeMap<NodeId, RcWeak<ReactionTarget>>>,
    pending_upgrades: RefCell<BTreeSet<NodeId>>,
    initial_reactions: RefCell<BTreeMap<NodeId, Vec<InitialReaction>>>,
    cursor: Cell<oryn_dom::MutationCursor>,
    parser_spans: RefCell<Vec<(u64, u64, NodeId)>>,
    converting_cursor: Cell<Option<u64>>,
    processing: Cell<bool>,
    constructing: Cell<usize>,
}

#[derive(Default)]
pub(super) struct DefinitionRegistry {
    id: Cell<u64>,
    scoped: Cell<bool>,
    owner: RefCell<RcWeak<V8BindingState>>,
    definitions: RefCell<BTreeMap<String, Rc<Definition>>>,
    promises: RefCell<BTreeMap<String, v8::Weak<v8::PromiseResolver>>>,
    object: RefCell<Option<v8::Weak<v8::Object>>>,
    defining: Cell<bool>,
}
impl std::ops::Deref for Registry {
    type Target = DefinitionRegistry;
    fn deref(&self) -> &Self::Target {
        &self.global
    }
}

#[derive(Default)]
struct RegistryPool {
    next: u64,
    scopes: BTreeMap<u64, Rc<DefinitionRegistry>>,
    native_roots: BTreeMap<u64, v8::Global<v8::Object>>,
}
fn definition_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    definition: &Definition,
) -> Result<v8::Local<'s, v8::Function>, V8BindingError> {
    definition
        .constructor
        .local(scope)
        .and_then(|value| value.try_cast::<v8::Function>().ok())
        .ok_or(V8BindingError::Allocation("custom constructor graph"))
}
fn graph_edge(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    values: &[v8::Local<v8::Value>],
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, name)
        .ok_or(V8BindingError::Allocation("custom registry graph key"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let values = v8::Array::new_with_elements(scope, values);
    if object.set_private(scope, key, values.into()) != Some(true) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(())
}
fn refresh_registry_graph(
    scope: &mut v8::PinScope,
    registry: &Rc<DefinitionRegistry>,
) -> Result<(), V8BindingError> {
    let object = registry_object(scope, registry)?;
    let mut values = Vec::new();
    for definition in registry.definitions.borrow().values() {
        if let Some(constructor) = definition.constructor.local(scope) {
            values.push(constructor);
        }
        for callback in definition.callbacks.iter().flatten() {
            if let Some(callback) = callback.local(scope) {
                values.push(callback);
            }
        }
    }
    for resolver in registry.promises.borrow().values() {
        if let Some(resolver) = resolver.to_local(scope) {
            values.push(resolver.get_promise(scope).into());
        }
    }
    graph_edge(scope, object, "oryn.custom.registry.definitions", &values)
}
pub(super) fn trace_node_registry(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    node: NodeId,
    object: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let mut ids = BTreeSet::new();
    ids.extend(node_registry_id(state, node)?);
    if let Some(element) = state.custom_elements.elements.borrow().get(&node) {
        ids.insert(element.definition.registry);
    }
    let mut values = Vec::new();
    for id in ids {
        if let Some(registry) = registry_by_id(scope, id) {
            values.push(registry_object(scope, &registry)?.into());
        }
    }
    graph_edge(scope, object, "oryn.custom.element.registries", &values)
}
/// A detached native subtree can contain scoped elements whose wrappers have
/// never been requested. Trace their registries from every wrapper that keeps
/// the component alive, without converting the collectable graph to Globals.
pub(super) fn trace_component_registries(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    nodes: &[NodeId],
    objects: &[(NodeId, v8::Local<v8::Object>)],
) -> Result<(), V8BindingError> {
    if nodes.is_empty() {
        for (_, object) in objects {
            graph_edge(scope, *object, "oryn.custom.component.registries", &[])?;
        }
        return Ok(());
    }
    let mut ids = BTreeSet::new();
    {
        let dom = state.dom.borrow();
        for node in nodes {
            ids.extend(dom.arena().get(*node)?.custom_element_registry.flatten());
        }
    }
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .filter(|other| Rc::ptr_eq(&other.dom, &state.dom))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for other in states {
        let elements = other.custom_elements.elements.borrow();
        for node in nodes {
            if let Some(element) = elements.get(node) {
                ids.insert(element.definition.registry);
            }
        }
    }
    let mut values = Vec::new();
    for id in ids {
        if let Some(registry) = registry_by_id(scope, id) {
            values.push(registry_object(scope, &registry)?.into());
        }
    }
    for (_, object) in objects {
        graph_edge(scope, *object, "oryn.custom.component.registries", &values)?;
    }
    Ok(())
}
fn native_retention(
    dom: &DomRuntime,
    roots: &traced_native_roots::Snapshot,
    node: NodeId,
    memo: &mut BTreeMap<NodeId, bool>,
) -> Result<bool, V8BindingError> {
    let mut path = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut current = Some(node);
    let retained = loop {
        let Some(node) = current else { break false };
        if let Some(retained) = memo.get(&node) {
            break *retained;
        }
        if !visiting.insert(node) {
            break false;
        }
        path.push(node);
        if roots.independent(dom, node)? {
            break true;
        }
        current = dom
            .retention_parent(node)?
            .or(dom.arena().get(node)?.template_host);
    };
    for node in path {
        memo.insert(node, retained);
    }
    Ok(retained)
}
// Registry rooting is existential within one validated refresh snapshot.
// Once one native-retained node proves an ID, other nodes carrying that ID
// cannot change the result. Failed proofs are not cached by registry ID.
fn retain_registry_from_node(
    dom: &DomRuntime,
    roots: &traced_native_roots::Snapshot,
    node: NodeId,
    registry: u64,
    retained: &mut BTreeSet<u64>,
    memo: &mut BTreeMap<NodeId, bool>,
) -> Result<(), V8BindingError> {
    if !retained.contains(&registry) && native_retention(dom, roots, node, memo)? {
        retained.insert(registry);
    }
    Ok(())
}

pub(super) fn refresh_native_registry_roots(
    scope: &mut v8::PinScope,
) -> Result<(), V8BindingError> {
    // Graph observation handles must end with this refresh, not the caller task.
    v8::scope!(let scope, scope);
    // Native migration may have committed while fallible wrapper publication
    // still owns the suffix. Preserve previous registry roots until it settles.
    if node_migration::has_publications(scope) {
        return Ok(());
    }
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|states| {
            states
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // Reentrant native mutations cannot publish a partial root snapshot.
    if states.iter().any(|state| state.dom.try_borrow().is_err()) {
        return Ok(());
    }
    traced_native_roots::refresh_range_endpoints(scope, &states)?;
    // A global registry associated with its live owner's Document has an
    // independent root regardless of any Range, iterator or detached subtree.
    // Prove that directly before constructing the full native-lease ledger.
    // Scoped registries and retired/rebound owners retain the general path.
    let registries = scope
        .get_slot::<RegistryPool>()
        .map(|pool| pool.scopes.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    if registries.iter().all(|registry| {
        !registry.scoped.get()
            && registry.owner.borrow().upgrade().is_some_and(|owner| {
                !owner.execution_retired()
                    && owner
                        .dom
                        .borrow()
                        .arena()
                        .get(owner.document_root())
                        .is_ok_and(|node| {
                            node.custom_element_registry.flatten() == Some(registry.id.get())
                        })
            })
    }) {
        let mut roots = BTreeMap::new();
        for registry in registries {
            let object = registry_object(scope, &registry)?;
            roots.insert(registry.id.get(), v8::Global::new(scope, object));
        }
        if let Some(pool) = scope.get_slot_mut::<RegistryPool>() {
            pool.native_roots = roots;
        }
        return Ok(());
    }
    let roots = traced_native_roots::Snapshot::capture(&states)?;
    let mut runtimes: BTreeMap<usize, (SharedDomRuntime, Vec<Rc<V8BindingState>>)> =
        BTreeMap::new();
    for state in states {
        runtimes
            .entry(Rc::as_ptr(&state.dom) as usize)
            .or_insert_with(|| (state.dom.clone(), Vec::new()))
            .1
            .push(state);
    }
    let mut ids = BTreeSet::new();
    for (_, (runtime, states)) in runtimes {
        let Ok(dom) = runtime.try_borrow() else {
            return Ok(());
        };
        // The cache lives for this borrow only: adoption, root-release and
        // document replacement cannot make an old reachability result valid.
        roots.validate(&dom)?;
        let mut memo = BTreeMap::new();
        for (node, record) in dom.arena().iter() {
            if let Some(id) = record.custom_element_registry.flatten() {
                retain_registry_from_node(&dom, &roots, node, id, &mut ids, &mut memo)?;
            }
        }
        for state in states {
            for (node, element) in state.custom_elements.elements.borrow().iter() {
                if dom.arena().get(*node).is_ok() {
                    retain_registry_from_node(
                        &dom,
                        &roots,
                        *node,
                        element.definition.registry,
                        &mut ids,
                        &mut memo,
                    )?;
                }
            }
        }
    }
    let mut roots = BTreeMap::new();
    for id in ids {
        if let Some(registry) = registry_by_id(scope, id) {
            let object = registry_object(scope, &registry)?;
            roots.insert(id, v8::Global::new(scope, object));
        }
    }
    if let Some(pool) = scope.get_slot_mut::<RegistryPool>() {
        pool.native_roots = roots;
    }
    let dead = scope
        .get_slot::<RegistryPool>()
        .map(|pool| {
            pool.scopes
                .iter()
                .filter_map(|(id, registry)| {
                    (!pool.native_roots.contains_key(id)
                        && registry
                            .object
                            .borrow()
                            .as_ref()
                            .is_none_or(|object| object.to_local(scope).is_none()))
                    .then_some(*id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(pool) = scope.get_slot_mut::<RegistryPool>() {
        for id in dead {
            pool.scopes.remove(&id);
        }
    }
    Ok(())
}
fn register_definition_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    registry: &Rc<DefinitionRegistry>,
    scoped: bool,
) -> Result<(), V8BindingError> {
    if registry.id.get() != 0 {
        return Ok(());
    }
    if scope.get_slot::<RegistryPool>().is_none() {
        scope.set_slot(RegistryPool::default());
    }
    let pool = scope.get_slot_mut::<RegistryPool>().unwrap();
    if pool.scopes.len() >= 65_536 {
        return Err(V8BindingError::Allocation("custom registry limit"));
    }
    pool.next = pool
        .next
        .checked_add(1)
        .ok_or(V8BindingError::Allocation("custom registry identity"))?;
    registry.id.set(pool.next);
    registry.scoped.set(scoped);
    *registry.owner.borrow_mut() = Rc::downgrade(state);
    pool.scopes.insert(pool.next, registry.clone());
    Ok(())
}
fn registry_by_id(scope: &mut v8::PinScope, id: u64) -> Option<Rc<DefinitionRegistry>> {
    scope
        .get_slot::<RegistryPool>()
        .and_then(|pool| pool.scopes.get(&id).cloned())
}
fn registry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    registry: &Rc<DefinitionRegistry>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Some(object) = registry
        .object
        .borrow()
        .as_ref()
        .and_then(|object| object.to_local(scope))
    {
        return Ok(object);
    }
    let owner = registry
        .owner
        .borrow()
        .upgrade()
        .ok_or(V8BindingError::Allocation("custom registry realm"))?;
    let id = register_platform_object(
        &owner,
        PlatformObjectRecord::CustomElementRegistry(registry.clone()),
    );
    let object = new_platform_object(scope, &owner, "CustomElementRegistry", id)?;
    *registry.object.borrow_mut() = Some(v8::Weak::new(scope, object));
    Ok(object)
}
fn registry_receiver(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Rc<DefinitionRegistry>, V8BindingError> {
    if object.is_proxy() {
        return Err(V8BindingError::Unsupported {
            interface: "CustomElementRegistry".into(),
            member: "TypeError".into(),
        });
    }
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("custom registry receiver"))?;
    let id = stateful_id(scope, object)?;
    let record = platform_object(&owner, id)?;
    match &*record {
        PlatformObjectRecord::CustomElementRegistry(registry) => Ok(registry.clone()),
        _ => Err(V8BindingError::Unsupported {
            interface: "CustomElementRegistry".into(),
            member: "TypeError".into(),
        }),
    }
}
pub(super) fn construct_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let registry = Rc::new(DefinitionRegistry::default());
    register_definition_registry(scope, state, &registry, true)?;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::CustomElementRegistry(registry.clone()),
    );
    bind_platform_object(scope, state, id, args.this());
    *registry.object.borrow_mut() = Some(v8::Weak::new(scope, args.this()));
    Ok(())
}
fn node_registry_id(state: &V8BindingState, node: NodeId) -> Result<Option<u64>, V8BindingError> {
    let dom = state.dom.borrow();
    let mut current = Some(node);
    let mut visited = BTreeSet::new();
    while let Some(node) = current {
        if !visited.insert(node) {
            return Err(V8BindingError::Allocation(
                "cyclic custom registry ancestry",
            ));
        }
        let record = dom.arena().get(node)?;
        if let Some(registry) = record.custom_element_registry {
            return Ok(registry);
        }
        if matches!(record.kind, NodeKind::Document) {
            return Ok(None);
        }
        current = record.parent.or(dom.arena().node_document(node)?);
    }
    Ok(None)
}
fn effective_global_registry(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    document: NodeId,
) -> Result<Option<u64>, V8BindingError> {
    Ok(node_registry_id(state, document)?
        .filter(|id| registry_by_id(scope, *id).is_some_and(|registry| !registry.scoped.get())))
}
fn node_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Option<Rc<DefinitionRegistry>>, V8BindingError> {
    Ok(node_registry_id(state, node)?.and_then(|id| registry_by_id(scope, id)))
}
pub(super) fn window_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    Ok(registry_object(scope, &state.custom_elements.global)?.into())
}
pub(super) fn registry_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match node_registry(scope, state, node)? {
        Some(registry) => Ok(registry_object(scope, &registry)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}
fn associate_registry(
    state: &V8BindingState,
    node: NodeId,
    registry: Option<u64>,
) -> Result<(), V8BindingError> {
    let mut dom = state.dom.borrow_mut();
    let previous = dom.arena().get(node)?.custom_element_registry;
    // Repeated definition lookup records the same explicit association. Keep
    // native document-order bookkeeping current without rebuilding unchanged
    // wrapper edges. Unassigned and explicitly null remain distinct here.
    dom.set_custom_element_registry(node, registry)?;
    if previous != Some(registry) {
        state.wrapper_graph_revision.set(None);
    }
    Ok(())
}
struct AssociationAdmission<'a> {
    state: &'a V8BindingState,
    checkpoint: Option<oryn_dom::CustomRegistryAdmission>,
}
impl Drop for AssociationAdmission<'_> {
    fn drop(&mut self) {
        if let Some(checkpoint) = self.checkpoint.take() {
            self.state
                .dom
                .borrow_mut()
                .rollback_custom_registry_admission(checkpoint);
            self.state.wrapper_graph_revision.set(None);
        }
    }
}

/// Explicit upgrade may supply a registry to inert/null-associated candidates.
/// It does not initialize their Document or replace another scoped registry.
fn explicit_upgrade(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    registry: &Rc<DefinitionRegistry>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let mut candidates = Vec::new();
    let mut upgrades = Vec::new();
    for node in subtree(state, root) {
        let association = node_registry_id(state, node)?;
        if association.is_some_and(|id| id != registry.id.get()) {
            continue;
        }
        if let Some(definition) = definition_in_registry(state, node, registry)? {
            if upgrade_already_admitted(scope, state, node) {
                continue;
            }
            if association.is_none() {
                candidates.push(node);
            }
            upgrades.push((node, definition));
        }
    }
    let checkpoint = state
        .dom
        .borrow()
        .checkpoint_custom_registry_admission(&candidates)?;
    let mut associations = AssociationAdmission {
        state,
        checkpoint: Some(checkpoint),
    };
    let reactions = MutationAdmission::new(scope, &state.custom_elements);
    for node in candidates {
        associate_registry(state, node, Some(registry.id.get()))?;
    }
    for (node, definition) in upgrades {
        enqueue_upgrade_definition(scope, state, node, definition)?;
    }
    wrapper_graphs::refresh(scope, state);
    refresh_native_registry_roots(scope)?;
    reactions.commit();
    associations.checkpoint.take();
    Ok(())
}

fn initialize_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    registry: &Rc<DefinitionRegistry>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let is_document = matches!(
        state.dom.borrow().arena().get(root)?.kind,
        NodeKind::Document
    );
    if is_document
        && (!registry.scoped.get()
            || node_registry_id(state, root)?.is_some_and(|id| id != registry.id.get()))
    {
        return Err(V8BindingError::Unsupported {
            interface: "CustomElementRegistry".into(),
            member: "NotSupportedError".into(),
        });
    }
    // Snapshot every original association before publishing the root's registry.
    // Resolving descendants afterwards would mistake inherited association for
    // a permanent per-element choice and lose it when the element is moved.
    let mut pending = vec![root];
    let mut candidates = Vec::new();
    let mut upgrades = Vec::new();
    while let Some(node) = pending.pop() {
        let (is_shadow, eligible, children) = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            (
                matches!(record.kind, NodeKind::ShadowRoot { .. }),
                matches!(
                    record.kind,
                    NodeKind::Document | NodeKind::Element { .. } | NodeKind::ShadowRoot { .. }
                ),
                record.children.clone(),
            )
        };
        if node != root && is_shadow {
            continue;
        }
        if eligible {
            match node_registry_id(state, node)? {
                None => {
                    candidates.push(node);
                    upgrades.push(node);
                }
                Some(id) if id == registry.id.get() => upgrades.push(node),
                _ => {}
            }
        }
        for child in children.into_iter().rev() {
            pending.push(child);
        }
    }
    let checkpoint = state
        .dom
        .borrow()
        .checkpoint_custom_registry_admission(&candidates)?;
    let mut associations = AssociationAdmission {
        state,
        checkpoint: Some(checkpoint),
    };
    let reactions = MutationAdmission::new(scope, &state.custom_elements);
    for node in &candidates {
        associate_registry(state, *node, Some(registry.id.get()))?;
    }
    for node in upgrades {
        #[cfg(test)]
        if FAIL_INITIALIZE_AFTER.with(|slot| match slot.get() {
            Some(0) => {
                slot.set(None);
                true
            }
            Some(n) => {
                slot.set(Some(n - 1));
                false
            }
            None => false,
        }) {
            return Err(V8BindingError::Allocation(
                "injected registry initialize admission",
            ));
        }
        enqueue_upgrade(scope, state, node, true)?;
    }
    wrapper_graphs::refresh(scope, state);
    refresh_native_registry_roots(scope)?;
    reactions.commit();
    associations.checkpoint.take();
    Ok(())
}

pub(super) struct CreationOptions {
    registry: Option<Option<u64>>,
    is_value: Option<String>,
}
pub(super) fn creation_options(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<CreationOptions, V8BindingError> {
    let mut result = CreationOptions {
        registry: None,
        is_value: None,
    };
    if value.is_null_or_undefined() {
        return Ok(result);
    }
    if !value.is_object() {
        result.is_value = Some({
            let string = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            rust_string_preserving_utf16(scope, string)?
        });
        return Ok(result);
    }
    let object = value
        .try_cast::<v8::Object>()
        .map_err(|_| V8BindingError::Unsupported {
            interface: "ElementCreationOptions".into(),
            member: "TypeError".into(),
        })?;
    result.registry = registry_option(scope, object)?;
    let key = v8_string(scope, "is")?;
    let value = object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !value.is_undefined() {
        result.is_value = Some({
            let string = value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            rust_string_preserving_utf16(scope, string)?
        });
    }
    Ok(result)
}
pub(super) fn registry_option(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<Option<Option<u64>>, V8BindingError> {
    let key = v8_string(scope, "customElementRegistry")?;
    let value = object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(None));
    }
    let object = value
        .try_cast::<v8::Object>()
        .map_err(|_| V8BindingError::Unsupported {
            interface: "CustomElementRegistry".into(),
            member: "TypeError".into(),
        })?;
    Ok(Some(Some(registry_receiver(scope, object)?.id.get())))
}
pub(super) fn initialize_created_element(
    state: &V8BindingState,
    document: NodeId,
    node: NodeId,
    options: CreationOptions,
) -> Result<(), V8BindingError> {
    let registry = match options.registry {
        Some(registry) => registry,
        None => node_registry_id(state, document)?,
    };
    // Both Document factories call this immediately after native allocation,
    // before wrapping the node or invoking a custom constructor. This fresh
    // node cannot change any existing wrapper's registry edge. Its insertion
    // journal and eventual wrapper publication establish the new edges; keep
    // associate_registry's full invalidation for changes to existing nodes.
    let mut dom = state.dom.borrow_mut();
    dom.set_custom_element_registry(node, registry)?;
    dom.arena_mut().get_mut(node)?.custom_element_is = options.is_value;
    Ok(())
}
pub(super) fn initialize_shadow_registry(
    state: &V8BindingState,
    host: NodeId,
    root: NodeId,
    registry: Option<Option<u64>>,
) -> Result<(), V8BindingError> {
    let document = state
        .dom
        .borrow()
        .arena()
        .node_document(host)?
        .unwrap_or(state.document_root());
    let registry = match registry {
        Some(registry) => registry,
        None => node_registry_id(state, document)?,
    };
    associate_registry(state, root, registry)
}

pub(super) fn import_options(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(bool, Option<u64>), V8BindingError> {
    if !value.is_object() {
        return Ok((value.boolean_value(scope), None));
    }
    let object = value.try_cast::<v8::Object>().unwrap();
    let registry = registry_option(scope, object)?;
    if registry == Some(None) {
        return Err(V8BindingError::Unsupported {
            interface: "ImportNodeOptions".into(),
            member: "TypeError".into(),
        });
    }
    let key = v8_string(scope, "selfOnly")?;
    let value = object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok((!value.boolean_value(scope), registry.flatten()))
}
pub(super) fn clone_registry_tree(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    document: NodeId,
    fallback: Option<u64>,
    use_document_fallback: bool,
) -> Result<(), V8BindingError> {
    if matches!(
        state.dom.borrow().arena().get(root)?.kind,
        NodeKind::Document
    ) {
        let registry = state
            .dom
            .borrow()
            .arena()
            .get(root)?
            .custom_element_registry
            .flatten()
            .filter(|id| registry_by_id(scope, *id).is_some_and(|registry| registry.scoped.get()));
        associate_registry(state, root, registry)?;
    }
    let document_registry = node_registry_id(state, document)?;
    let effective_global = effective_global_registry(scope, state, document)?;
    let mut pending = vec![(
        root,
        if use_document_fallback {
            fallback.or(document_registry)
        } else {
            fallback
        },
    )];
    while let Some((node, fallback)) = pending.pop() {
        let (association, shadow, children, template) = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            (
                record.custom_element_registry,
                matches!(record.kind, NodeKind::ShadowRoot { .. }),
                record.children.clone(),
                record.template_content,
            )
        };
        let candidate = association
            .flatten()
            .or(if shadow { None } else { fallback });
        let registry = match candidate {
            Some(id) if registry_by_id(scope, id).is_some_and(|registry| registry.scoped.get()) => {
                Some(id)
            }
            Some(_) => effective_global,
            None => None,
        };
        associate_registry(state, node, registry)?;
        if let Some(content) = template {
            pending.push((content, None));
        }
        // The clone algorithm passes its original fallback through light
        // descendants; an intervening scoped registry does not replace it.
        // Clonable shadow trees intentionally start with a null fallback.
        for child in children.into_iter().rev() {
            pending.push((child, if shadow { None } else { fallback }));
        }
    }
    Ok(())
}
pub(super) fn initialize_fragment_registry(
    state: &V8BindingState,
    fragment: NodeId,
    context: NodeId,
) -> Result<(), V8BindingError> {
    let registry_node = {
        let dom = state.dom.borrow();
        if matches!(dom.arena().get(fragment)?.kind, NodeKind::ShadowRoot { .. }) {
            dom.node_document(fragment)?
                .unwrap_or(state.document_root())
        } else {
            context
        }
    };
    if state
        .dom
        .borrow()
        .arena()
        .get(fragment)?
        .custom_element_registry
        .is_none()
    {
        associate_registry(state, fragment, node_registry_id(state, registry_node)?)?;
    }
    stamp_uninitialized_subtree(state, fragment)
}
pub(super) fn initialize_fragment_registry_from(
    destination: &V8BindingState,
    fragment: NodeId,
    source: &V8BindingState,
    context: NodeId,
) -> Result<(), V8BindingError> {
    let shadow = matches!(
        destination.dom.borrow().arena().get(fragment)?.kind,
        NodeKind::ShadowRoot { .. }
    );
    let registry = if shadow {
        let document = destination
            .dom
            .borrow()
            .node_document(fragment)?
            .unwrap_or(destination.document_root());
        node_registry_id(destination, document)?
    } else {
        node_registry_id(source, context)?
    };
    let uninitialized = destination
        .dom
        .borrow()
        .arena()
        .get(fragment)?
        .custom_element_registry
        .is_none();
    if uninitialized {
        associate_registry(destination, fragment, registry)?;
    }
    stamp_uninitialized_subtree(destination, fragment)
}
fn stamp_uninitialized_subtree(state: &V8BindingState, root: NodeId) -> Result<(), V8BindingError> {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        let (uninitialized, children) = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            (
                record.custom_element_registry.is_none(),
                record.children.clone(),
            )
        };
        if uninitialized {
            let registry_node = {
                let dom = state.dom.borrow();
                if matches!(dom.arena().get(node)?.kind, NodeKind::ShadowRoot { .. }) {
                    dom.node_document(node)?.unwrap_or(state.document_root())
                } else {
                    node
                }
            };
            let registry = node_registry_id(state, registry_node)?;
            associate_registry(state, node, registry)?;
        }
        for child in children.into_iter().rev() {
            pending.push(child);
        }
    }
    Ok(())
}

struct DefinitionGuard<'a>(&'a Cell<bool>);
impl Drop for DefinitionGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl Registry {
    /// A failed observation is busy; unlike has_pending this does not borrow
    /// infallibly and includes retained restoration/upgrades still awaiting work.
    pub(super) fn checkpoint_pending(&self, state: &V8BindingState) -> bool {
        self.initial_reactions
            .try_borrow()
            .map_or(true, |queue| !queue.is_empty())
            || self
                .reactions
                .try_borrow()
                .map_or(true, |queue| !queue.is_empty())
            || self
                .pending_upgrades
                .try_borrow()
                .map_or(true, |queue| !queue.is_empty())
            || self
                .pending_restoration
                .try_borrow()
                .map_or(true, |pending| pending.is_some())
            || self.processing.get()
            || self.constructing.get() != 0
            || state
                .dom
                .try_borrow()
                .map_or(true, |dom| self.cursor.get() != dom.mutation_cursor())
    }

    pub(super) fn has_pending(&self, state: &V8BindingState) -> bool {
        !state.execution_suspended()
            && !state.execution_retired()
            && (!self.initial_reactions.borrow().is_empty()
                || !self.reactions.borrow().is_empty()
                || self.cursor.get() != state.dom.borrow().mutation_cursor())
    }
}

/// Definitions live with the registry; per-element state lives only as long as
/// its native identity. Pending callback wrappers already retain live targets.
pub(super) fn collect_reclaimed_elements(state: &V8BindingState) {
    let dom = state.dom.borrow();
    let live = |node: &NodeId| dom.arena().get(*node).is_ok();
    state
        .custom_elements
        .elements
        .borrow_mut()
        .retain(|node, _| live(node));
    state
        .custom_elements
        .reactions
        .borrow_mut()
        .retain(|node, _| live(node));
    state
        .custom_elements
        .queued_targets
        .borrow_mut()
        .retain(|node, target| live(node) && target.strong_count() != 0);
    state
        .custom_elements
        .pending_upgrades
        .borrow_mut()
        .retain(live);
    state
        .custom_elements
        .initial_reactions
        .borrow_mut()
        .retain(|node, _| live(node));
}

pub(super) fn attach_internals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    options: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let original = wrap_node(scope, state, node)?;
    let prepared = submit_behavior::prepare(scope, options)?;
    let (owner_state, node) =
        node_identity::argument(scope, original.into())?.ok_or(DomError::StaleNode(node))?;
    let state = &owner_state;
    let denied = || V8BindingError::Unsupported {
        interface: "HTMLElement".into(),
        member: "NotSupportedError".into(),
    };
    let owner = agent_states(scope, state)
        .into_iter()
        .find(|owner| owner.custom_elements.elements.borrow().contains_key(&node))
        .ok_or_else(denied)?;
    {
        let elements = owner.custom_elements.elements.borrow();
        let element = elements.get(&node).ok_or_else(denied)?;
        if element.definition.disable_internals
            || element.internals_attached
            || (element.failed && !element.upgrading)
        {
            return Err(denied());
        }
    }
    let object = associated_wrapper(scope, state, node, "ElementInternals")?;
    prepared.commit(
        scope,
        state,
        node,
        object
            .try_cast::<v8::Object>()
            .map_err(|_| V8BindingError::Allocation("ElementInternals object"))?,
    )?;
    owner
        .custom_elements
        .elements
        .borrow_mut()
        .get_mut(&node)
        .unwrap()
        .internals_attached = true;
    mark_internals_changed(&owner, node);
    Ok(object)
}

pub(super) fn shadow_disabled(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    let owners = agent_states(scope, state);
    // An upgraded element retains its original definition after adoption.
    for owner in &owners {
        if let Some(element) = owner.custom_elements.elements.borrow().get(&node) {
            return Ok(element.definition.disable_shadow);
        }
    }
    {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        if record.custom_element_creation_failed {
            return Ok(record.custom_element_shadow_disabled.unwrap_or(false));
        }
    }
    Ok(
        definition_for_node(scope, state, node)?
            .is_some_and(|definition| definition.disable_shadow),
    )
}

#[derive(Clone, Default)]
struct ActiveConstructors(Rc<RefCell<Vec<Rc<Definition>>>>);
struct ActiveConstructorGuard {
    entries: Rc<RefCell<Vec<Rc<Definition>>>>,
    definition: Rc<Definition>,
}
impl Drop for ActiveConstructorGuard {
    fn drop(&mut self) {
        let popped = self.entries.borrow_mut().pop();
        debug_assert!(
            popped
                .as_ref()
                .is_some_and(|definition| Rc::ptr_eq(definition, &self.definition))
        );
    }
}
fn activate_constructor(
    scope: &mut v8::PinScope,
    definition: &Rc<Definition>,
) -> ActiveConstructorGuard {
    if scope.get_slot::<ActiveConstructors>().is_none() {
        scope.set_slot(ActiveConstructors::default());
    }
    let entries = scope.get_slot::<ActiveConstructors>().unwrap().0.clone();
    entries.borrow_mut().push(definition.clone());
    ActiveConstructorGuard {
        entries,
        definition: definition.clone(),
    }
}
fn constructor_definition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Value>,
) -> Result<Option<Rc<Definition>>, V8BindingError> {
    let active = scope
        .get_slot::<ActiveConstructors>()
        .map(|active| active.0.borrow().clone())
        .unwrap_or_default();
    if let Some(definition) = active
        .into_iter()
        .rev()
        .find(|definition| definition.constructor.equals(scope, target))
    {
        return Ok(Some(definition));
    }
    let Some(registry) = node_registry(scope, state, state.document_root())? else {
        return Ok(None);
    };
    let definition = registry
        .definitions
        .borrow()
        .values()
        .find(|definition| definition.constructor.equals(scope, target))
        .cloned();
    Ok(definition)
}

struct ConstructionEntry {
    definition: Rc<Definition>,
    target: Rc<ReactionTarget>,
    consumed: Cell<bool>,
}
/// Always release an entered construction stack, including native allocation
/// failure or execution termination after author code has started.
struct ConstructionGuard {
    state: Rc<V8BindingState>,
    stack: Rc<RefCell<Vec<Rc<ConstructionEntry>>>>,
    entry: Rc<ConstructionEntry>,
}
impl Drop for ConstructionGuard {
    fn drop(&mut self) {
        self.state.custom_elements.constructing.set(
            self.state
                .custom_elements
                .constructing
                .get()
                .saturating_sub(1),
        );
        let popped = self.stack.borrow_mut().pop();
        debug_assert!(
            popped
                .as_ref()
                .is_some_and(|entry| Rc::ptr_eq(entry, &self.entry))
        );
        if let Some(owner) = self.entry.target.owner.borrow().clone()
            && let Some(element) = owner
                .custom_elements
                .elements
                .borrow_mut()
                .get_mut(&self.entry.target.node.get())
        {
            element.upgrading = false;
        }
    }
}
#[derive(Clone, Default)]
struct ConstructionStack(Rc<RefCell<Vec<Rc<ConstructionEntry>>>>);
fn construction_stack(scope: &mut v8::PinScope) -> Rc<RefCell<Vec<Rc<ConstructionEntry>>>> {
    if scope.get_slot::<ConstructionStack>().is_none() {
        scope.set_slot(ConstructionStack::default());
    }
    scope.get_slot::<ConstructionStack>().unwrap().0.clone()
}
fn valid_base_interface(definition: &Definition, base: &str) -> bool {
    if definition.name == definition.local_name {
        return base == "HTMLElement";
    }
    let kind = NodeKind::Element {
        name: markup5ever::QualName::new(
            None,
            "http://www.w3.org/1999/xhtml".into(),
            definition.local_name.as_str().into(),
        ),
    };
    interface_for_node(&kind) == base
}
pub(super) fn constructor_realm(
    scope: &mut v8::PinScope,
    target: v8::Local<v8::Value>,
) -> Option<Rc<V8BindingState>> {
    let _ = scope;
    crate::v8_context_bridge::constructor_realm::<V8BindingState>(target)
}

fn report_callback_exception(
    scope: &mut v8::PinScope,
    callback: v8::Local<v8::Value>,
    exception: v8::Local<v8::Value>,
    message: Option<v8::Local<v8::Message>>,
) -> Result<bool, V8BindingError> {
    let owner = constructor_realm(scope, callback)
        .ok_or(V8BindingError::Allocation("custom callback realm"))?;
    let global = owner
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("custom callback global"))?;
    let context = global
        .get_creation_context(scope)
        .ok_or(V8BindingError::Allocation("custom callback context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    report_exception(scope, exception, message)
}
pub(super) fn html_constructor_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Value>,
    base: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let target_object = target
        .try_cast::<v8::Object>()
        .map_err(|_| V8BindingError::JavaScriptException)?;
    let key = v8_string(scope, "prototype")?;
    let prototype = target_object
        .get(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if prototype.is_object() {
        return Ok(prototype);
    }
    let realm = constructor_realm(scope, target).ok_or_else(|| V8BindingError::Unsupported {
        interface: base.into(),
        member: "TypeError: constructor realm is unavailable".into(),
    })?;
    let prototype = realm
        .html_prototypes
        .borrow()
        .get(base)
        .and_then(|prototype| prototype.to_local(scope))
        .ok_or(V8BindingError::Allocation(
            "HTML constructor fallback prototype",
        ))?;
    Ok(prototype.into())
}
pub(super) struct PendingConstruction {
    pub(super) owner: Rc<V8BindingState>,
    pub(super) node: NodeId,
    entry: Rc<ConstructionEntry>,
}
impl PendingConstruction {
    pub(super) fn refresh(mut self) -> Result<Self, V8BindingError> {
        if self.entry.consumed.get() {
            return Err(V8BindingError::Unsupported {
                interface: "HTMLElement".into(),
                member: "TypeError: construction stack already consumed".into(),
            });
        }
        self.owner = self
            .entry
            .target
            .owner
            .borrow()
            .clone()
            .ok_or(V8BindingError::Allocation("retired custom construction"))?;
        self.node = self.entry.target.node.get();
        Ok(self)
    }
    pub(super) fn accept(self) {
        self.entry.consumed.set(true);
    }
}
pub(super) fn pending_construction(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Value>,
    base: &str,
) -> Result<Option<PendingConstruction>, V8BindingError> {
    let Some(definition) = constructor_definition(scope, state, target)? else {
        return Ok(None);
    };
    let entries = construction_stack(scope).borrow().clone();
    let entry = entries
        .iter()
        .rev()
        .find(|entry| Rc::ptr_eq(&entry.definition, &definition))
        .cloned();
    let Some(entry) = entry else {
        return Ok(None);
    };
    if !valid_base_interface(&entry.definition, base) {
        return Err(V8BindingError::Unsupported {
            interface: base.into(),
            member: "TypeError: invalid custom construction stack".into(),
        });
    }
    let owner = entry
        .target
        .owner
        .borrow()
        .clone()
        .ok_or(V8BindingError::Allocation("retired custom construction"))?;
    let node = entry.target.node.get();
    Ok(Some(PendingConstruction { owner, node, entry }))
}
pub(super) struct DirectConstruction {
    definition: Rc<Definition>,
}

pub(super) fn construction_target(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: v8::Local<v8::Value>,
    base: &str,
) -> Result<Option<DirectConstruction>, V8BindingError> {
    let Some(definition) = constructor_definition(scope, state, target)? else {
        return Ok(None);
    };
    if !valid_base_interface(&definition, base) {
        return Err(V8BindingError::Unsupported {
            interface: base.into(),
            member: "TypeError: custom element base mismatch".into(),
        });
    }
    Ok(Some(DirectConstruction { definition }))
}
impl DirectConstruction {
    /// Commit native identity only after the observable prototype getter. The
    /// definition is frozen, but the current Window document may have changed.
    pub(super) fn materialize(
        self,
        scope: &mut v8::PinScope,
        state: &Rc<V8BindingState>,
    ) -> Result<(WrapperId, NodeId), V8BindingError> {
        let definition = self.definition;
        let name = definition.name.clone();
        let node = state
            .dom
            .borrow_mut()
            .create_element(&definition.local_name);
        associate_registry(state, node, Some(definition.registry))?;
        state
            .dom
            .borrow_mut()
            .arena_mut()
            .get_mut(node)?
            .custom_element_shadow_disabled = Some(definition.disable_shadow);
        if definition.local_name != name {
            state
                .dom
                .borrow_mut()
                .arena_mut()
                .get_mut(node)?
                .custom_element_is = Some(name.clone());
        }
        if definition.form_associated
            && let Err(error) = state
                .dom
                .borrow_mut()
                .register_form_associated_custom_element(node)
        {
            state.custom_elements.elements.borrow_mut().remove(&node);
            return Err(error.into());
        }
        state
            .dom
            .borrow_mut()
            .set_node_document(node, state.document_root())?;
        let object = wrap_node(scope, state, node)?;
        let wrapper = receiver_wrapper_id(scope, object)
            .ok_or(V8BindingError::Allocation("custom wrapper"))?;
        state.custom_elements.elements.borrow_mut().insert(
            node,
            Element {
                internals_revision: Rc::new(()),
                definition,
                connected: false,
                document: state.document_root(),
                failed: false,
                upgrading: false,
                internals_attached: false,
                form_owner: None,
                form_disabled: false,
                submission: Vec::new(),
                restoration: None,
            },
        );
        state
            .dom
            .borrow_mut()
            .set_custom_element_defined(node, true)?;
        Ok((wrapper, node))
    }
}

/// A same-origin initial Window commit retains its registry and definitions,
/// but the freshly imported Document has not passed through binding install.
/// Admit its associations before the host retires the initial Document.
pub(super) fn initialize_replacement_document(
    state: &V8BindingState,
    document: NodeId,
) -> Result<(), V8BindingError> {
    let nodes = {
        let dom = state.dom.borrow();
        dom.arena().document_id(document)?;
        let mut nodes = Vec::new();
        let mut pending = vec![document];
        while let Some(node) = pending.pop() {
            pending.extend(dom.arena().get(node)?.children.iter().rev().copied());
            nodes.push(node);
        }
        nodes
    };
    let checkpoint = state
        .dom
        .borrow()
        .checkpoint_custom_registry_admission(&nodes)?;
    let mut admission = AssociationAdmission {
        state,
        checkpoint: Some(checkpoint),
    };
    associate_registry(state, document, Some(state.custom_elements.id.get()))?;
    // Explicit null/scoped descendants keep their own associations. No author
    // callback runs and no definition/reaction cursor is replaced here.
    stamp_uninitialized_subtree(state, document)?;
    admission.checkpoint.take();
    Ok(())
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    register_definition_registry(scope, state, &state.custom_elements.global, false)?;
    state
        .dom
        .borrow_mut()
        .register_global_custom_element_registry(state.custom_elements.id.get());
    associate_registry(
        state,
        state.document_root(),
        Some(state.custom_elements.id.get()),
    )?;
    let object = registry_object(scope, &state.custom_elements.global)?;
    if let Some(window) = state.global_object.to_local(scope) {
        graph_edge(
            scope,
            window,
            "oryn.custom.global.registry",
            &[object.into()],
        )?;
    }
    stamp_uninitialized_subtree(state, state.document_root())?;
    state
        .custom_elements
        .cursor
        .set(state.dom.borrow().mutation_cursor());
    Ok(())
}

pub(super) fn registry_operation(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
    operation: &str,
    arity: u8,
) {
    if operation != "whenDefined" {
        registry_operation_impl(scope, args, &mut rv, operation, arity);
        return;
    }
    // Web IDL promise-returning operations reject on receiver, argument and
    // algorithm exceptions. Catch the original value, including authored
    // conversion failures, without converting execution termination.
    let exception = {
        v8::tc_scope!(let tc,scope);
        registry_operation_impl(tc, args, &mut rv, operation, arity);
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
            return;
        }
        let exception = tc.exception();
        if tc.has_caught() {
            tc.reset();
        }
        exception
    };
    if let Some(exception) = exception {
        let Some(resolver) = v8::PromiseResolver::new(scope) else {
            throw_type_error(scope, "whenDefined resolver allocation failed");
            return;
        };
        if resolver.reject(scope, exception).is_none() {
            return;
        }
        rv.set(resolver.get_promise(scope).into());
    }
}

fn registry_operation_impl(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    rv: &mut v8::ReturnValue,
    operation: &str,
    arity: u8,
) {
    if args.length() < i32::from(arity) {
        throw_type_error(
            scope,
            "CustomElementRegistry operation requires more arguments",
        );
        return;
    }
    let registry = match registry_receiver(scope, args.this()) {
        Ok(registry) => registry,
        Err(error) => {
            throw_binding_error(scope, &error);
            return;
        }
    };
    let Some(state) = registry.owner.borrow().upgrade() else {
        throw_type_error(scope, "custom registry realm unavailable");
        return;
    };
    let result = (|| -> Result<(), V8BindingError> {
        if operation == "getName" {
            let constructor = args.get(0).try_cast::<v8::Function>().map_err(|_| {
                V8BindingError::Unsupported {
                    interface: "CustomElementRegistry".into(),
                    member: "TypeError".into(),
                }
            })?;
            let name = registry
                .definitions
                .borrow()
                .iter()
                .find(|(_, definition)| definition.constructor.equals(scope, constructor.into()))
                .map(|(name, _)| name.clone());
            rv.set(match name {
                Some(name) => v8_string(scope, &name)?,
                None => v8::null(scope).into(),
            });
            return Ok(());
        }
        if operation == "upgrade" || operation == "initialize" {
            let (owner, node) = node_identity::argument(scope, args.get(0))?.ok_or(
                V8BindingError::Unsupported {
                    interface: "CustomElementRegistry".into(),
                    member: "TypeError".into(),
                },
            )?;
            if operation == "initialize" {
                initialize_registry(scope, &owner, &registry, node)?;
            } else {
                explicit_upgrade(scope, &owner, &registry, node)?;
            }
            return Ok(());
        }
        let Some(name) = args.get(0).to_string(scope) else {
            return Ok(());
        };
        let name = rust_string_preserving_utf16(scope, name)?;
        if operation == "get" {
            if let Some(definition) = registry.definitions.borrow().get(&name) {
                rv.set(definition_constructor(scope, definition)?.into());
            }
            return Ok(());
        }
        let definition_options = if operation == "define" {
            let Ok(constructor) = v8::Local::<v8::Function>::try_from(args.get(1)) else {
                throw_type_error(scope, "custom element constructor must be callable");
                return Ok(());
            };
            let extends = if args.get(2).is_null_or_undefined() {
                None
            } else {
                let options = args.get(2).try_cast::<v8::Object>().map_err(|_| {
                    V8BindingError::Unsupported {
                        interface: "CustomElementRegistry".into(),
                        member: "TypeError".into(),
                    }
                })?;
                let key = v8_string(scope, "extends")?;
                let value = options
                    .get(scope, key)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if value.is_undefined() {
                    None
                } else {
                    Some({
                        let string = value
                            .to_string(scope)
                            .ok_or(V8BindingError::JavaScriptException)?;
                        rust_string_preserving_utf16(scope, string)?
                    })
                }
            };
            Some((constructor, extends))
        } else {
            None
        };
        if definition_options.as_ref().is_some_and(|(constructor, _)| {
            !crate::v8_context_bridge::is_constructor((*constructor).into())
        }) {
            throw_type_error(scope, "custom element callback is not a constructor");
            return Ok(());
        }
        if !is_valid_custom_element_name(&name) {
            let error = dom_exception_value(scope, "SyntaxError")?;
            if operation == "whenDefined" {
                let resolver = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("whenDefined resolver"))?;
                resolver
                    .reject(scope, error)
                    .ok_or(V8BindingError::JavaScriptException)?;
                rv.set(resolver.get_promise(scope).into());
            } else {
                scope.throw_exception(error);
            }
            return Ok(());
        }
        if operation == "whenDefined" {
            let resolver = if let Some(resolver) = registry
                .promises
                .borrow()
                .get(&name)
                .and_then(|resolver| resolver.to_local(scope))
            {
                resolver
            } else {
                v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("whenDefined resolver"))?
            };
            // Promise resolution reads the constructor's authored `then`.
            // Retain the accepted definition, but release the registry borrow
            // before that getter can define another element in this registry.
            let definition = registry.definitions.borrow().get(&name).cloned();
            if let Some(definition) = definition {
                let constructor = definition_constructor(scope, &definition)?;
                resolver
                    .resolve(scope, constructor.into())
                    .ok_or(V8BindingError::JavaScriptException)?;
            } else {
                registry
                    .promises
                    .borrow_mut()
                    .insert(name, v8::Weak::new(scope, resolver));
            }
            refresh_registry_graph(scope, &registry)?;
            rv.set(resolver.get_promise(scope).into());
            return Ok(());
        }
        let (constructor, extends) =
            definition_options.ok_or(V8BindingError::Allocation("custom definition conversion"))?;
        if extends.as_ref().is_some_and(|local| {
            registry.scoped.get()
                || is_valid_custom_element_name(local)
                || !is_known_html_element(local)
        }) {
            return Err(V8BindingError::Unsupported {
                interface: "CustomElementRegistry".into(),
                member: "NotSupportedError".into(),
            });
        }
        let local_name = extends.unwrap_or_else(|| name.clone());
        if registry.definitions.borrow().contains_key(&name)
            || registry
                .definitions
                .borrow()
                .values()
                .any(|definition| definition.constructor.equals(scope, constructor.into()))
        {
            let error = dom_exception_value(scope, "NotSupportedError")?;
            scope.throw_exception(error);
            return Ok(());
        }
        if registry.defining.replace(true) {
            let error = dom_exception_value(scope, "NotSupportedError")?;
            scope.throw_exception(error);
            return Ok(());
        }
        let guard = DefinitionGuard(&registry.defining);
        let key = v8_string(scope, "prototype")?;
        let prototype = constructor
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?;
        let Ok(prototype) = prototype.try_cast::<v8::Object>() else {
            throw_type_error(
                scope,
                "custom element constructor requires an object prototype",
            );
            return Ok(());
        };
        let mut callbacks = [None, None, None, None, None, None, None, None, None];
        for (index, name) in [
            (0, "connectedCallback"),
            (1, "disconnectedCallback"),
            (8, "connectedMoveCallback"),
            (2, "adoptedCallback"),
            (3, "attributeChangedCallback"),
        ] {
            let key = v8_string(scope, name)?;
            let Some(value) = prototype.get(scope, key) else {
                return Ok(());
            };
            if !value.is_undefined() {
                let Ok(callback) = v8::Local::<v8::Function>::try_from(value) else {
                    throw_type_error(scope, "custom element lifecycle callback must be callable");
                    return Ok(());
                };
                callbacks[index] = Some(platform_listener_graph::Callback::new(
                    scope,
                    callback.into(),
                ));
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
        let key = v8_string(scope, "disabledFeatures")?;
        let features = constructor
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?;
        let features = if features.is_undefined() {
            Vec::new()
        } else {
            dom_string_sequence_from_iterable(scope, features)?
        };
        let disable_shadow = features.iter().any(|feature| feature == "shadow");
        let disable_internals = features.iter().any(|feature| feature == "internals");
        let key = v8_string(scope, "formAssociated")?;
        let form_associated = constructor
            .get(scope, key)
            .ok_or(V8BindingError::JavaScriptException)?
            .boolean_value(scope);
        if form_associated {
            for (index, name) in [
                (4, "formAssociatedCallback"),
                (6, "formResetCallback"),
                (5, "formDisabledCallback"),
                (7, "formStateRestoreCallback"),
            ] {
                let key = v8_string(scope, name)?;
                let value = prototype
                    .get(scope, key)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !value.is_undefined() {
                    let callback = v8::Local::<v8::Function>::try_from(value).map_err(|_| {
                        V8BindingError::Unsupported {
                            interface: "CustomElementRegistry".into(),
                            member: "TypeError: form callback must be callable".into(),
                        }
                    })?;
                    callbacks[index] = Some(platform_listener_graph::Callback::new(
                        scope,
                        callback.into(),
                    ));
                }
            }
        }
        drop(guard);
        let definition = Rc::new(Definition {
            name: name.clone(),
            local_name,
            registry: registry.id.get(),
            constructor: platform_listener_graph::Callback::new(scope, constructor.into()),
            callbacks,
            observed,
            disable_shadow,
            disable_internals,
            form_associated,
        });
        // The definition and all upgrade candidates publish atomically. No
        // author callback runs while this native staging can still roll back.
        registry
            .definitions
            .borrow_mut()
            .insert(name.clone(), definition.clone());
        struct DefinitionAdmission {
            registry: Rc<DefinitionRegistry>,
            name: String,
            committed: bool,
        }
        impl Drop for DefinitionAdmission {
            fn drop(&mut self) {
                if !self.committed {
                    self.registry.definitions.borrow_mut().remove(&self.name);
                }
            }
        }
        let mut definition_admission = DefinitionAdmission {
            registry: registry.clone(),
            name: name.clone(),
            committed: false,
        };
        let documents = ordered_registry_documents(scope, registry.id.get());
        let mut owners = Vec::<Rc<V8BindingState>>::new();
        for (_, owner, _) in &documents {
            if !owners.iter().any(|known| Rc::ptr_eq(known, owner)) {
                owners.push(owner.clone());
            }
        }
        let admissions = owners
            .iter()
            .map(|owner| MutationAdmission::new(scope, &owner.custom_elements))
            .collect::<Vec<_>>();
        for (_, owner, document) in documents {
            for node in subtree(&owner, document) {
                if definition_for_node(scope, &owner, node)?
                    .is_some_and(|candidate| Rc::ptr_eq(&candidate, &definition))
                {
                    #[cfg(test)]
                    if FAIL_DEFINITION_AFTER.with(|slot| match slot.get() {
                        Some(0) => {
                            slot.set(None);
                            true
                        }
                        Some(n) => {
                            slot.set(Some(n - 1));
                            false
                        }
                        None => false,
                    }) {
                        return Err(V8BindingError::Allocation(
                            "injected definition candidate admission",
                        ));
                    }
                    enqueue_upgrade(scope, &owner, node, true)?;
                }
            }
        }
        if disable_shadow && !registry.scoped.get() {
            state
                .dom
                .borrow()
                .arena()
                .document_id(state.document_root())?;
        }
        #[cfg(test)]
        if FAIL_DEFINITION_GRAPH.with(|slot| slot.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected definition graph publication",
            ));
        }
        refresh_registry_graph(scope, &registry)?;
        if disable_shadow && !registry.scoped.get() {
            state
                .dom
                .borrow_mut()
                .set_parser_shadow_disabled(state.document_root(), name.clone())
                .expect("preflighted definition document");
        }
        for admission in admissions {
            admission.commit();
        }
        definition_admission.committed = true;
        // Resolving with a constructor can execute its authored `then` getter.
        // At that boundary every candidate already has retained queue custody.
        let resolver = registry
            .promises
            .borrow_mut()
            .remove(&name)
            .and_then(|resolver| resolver.to_local(scope));
        if let Some(resolver) = resolver {
            resolver
                .resolve(scope, constructor.into())
                .ok_or(V8BindingError::JavaScriptException)?;
        }

        Ok(())
    })();
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

// Element queues belong to the agent, not the Document registry. A queue entry
// follows native adoption while each element keeps one shared reaction queue.
struct ReactionTarget {
    owner: RefCell<Option<Rc<V8BindingState>>>,
    node: Cell<NodeId>,
    lease: RefCell<Option<oryn_dom::TrackedNodeLease>>,
}
type ElementQueue = Rc<RefCell<VecDeque<Rc<ReactionTarget>>>>;
struct RetainedQueue {
    queue: ElementQueue,
    collect_pending: Cell<bool>,
}
#[derive(Default)]
struct ReactionAgent {
    stack: RefCell<Vec<ElementQueue>>,
    backup: ElementQueue,
    suspended: ElementQueue,
    retry: RefCell<VecDeque<Rc<RetainedQueue>>>,
    backup_queued: Cell<bool>,
    backup_owner: RefCell<Option<(RcWeak<V8BindingState>, u64, u64)>>,
    backup_serial: Cell<u64>,
    collecting: Cell<bool>,
    invocation_depth: Cell<usize>,
    remaining: Cell<usize>,
}
#[derive(Clone, Default)]
struct ReactionAgentSlot(Rc<ReactionAgent>);
fn reaction_agent(scope: &mut v8::PinScope) -> Rc<ReactionAgent> {
    if scope.get_slot::<ReactionAgentSlot>().is_none() {
        scope.set_slot(ReactionAgentSlot::default());
    }
    scope.get_slot::<ReactionAgentSlot>().unwrap().0.clone()
}
fn admission_queue(agent: &ReactionAgent) -> ElementQueue {
    agent
        .stack
        .borrow()
        .last()
        .cloned()
        .unwrap_or_else(|| agent.backup.clone())
}
fn reaction_target(
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Rc<ReactionTarget>, V8BindingError> {
    if let Some(target) = state
        .custom_elements
        .queued_targets
        .borrow()
        .get(&node)
        .and_then(RcWeak::upgrade)
    {
        return Ok(target);
    }
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    let target = Rc::new(ReactionTarget {
        owner: RefCell::new(Some(state.clone())),
        node: Cell::new(node),
        lease: RefCell::new(Some(lease)),
    });
    state
        .custom_elements
        .queued_targets
        .borrow_mut()
        .insert(node, Rc::downgrade(&target));
    Ok(target)
}
fn enqueue_element(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let queue = admission_queue(&reaction_agent(scope));
    queue
        .borrow_mut()
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("reaction element queue"))?;
    let target = reaction_target(state, node)?;
    queue.borrow_mut().push_back(target);
    Ok(())
}
// Definition lookup deliberately continues to use the same-arena agent_states.
// Reaction invocation spans all live Documents in this isolate/agent.
fn reaction_states(scope: &mut v8::PinScope) -> Vec<Rc<V8BindingState>> {
    scope
        .get_slot::<BindingStateRegistry>()
        .map(|registry| {
            registry
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .filter(|state| !state.execution_retired() && !state.execution_suspended())
                .collect()
        })
        .unwrap_or_default()
}
struct CollectionGuard {
    agent: Rc<ReactionAgent>,
    retained: Option<Rc<RetainedQueue>>,
}
impl Drop for CollectionGuard {
    fn drop(&mut self) {
        if self.retained.is_some() {
            self.agent.stack.borrow_mut().pop();
        }
        self.agent.collecting.set(false);
    }
}
fn collect_agent(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let agent = reaction_agent(scope);
    if agent.collecting.replace(true) {
        return Ok(());
    }
    let retained = agent
        .retry
        .borrow()
        .iter()
        .find(|entry| entry.collect_pending.get())
        .cloned();
    if let Some(retained) = &retained {
        agent.stack.borrow_mut().push(retained.queue.clone());
    }
    let _guard = CollectionGuard {
        agent,
        retained: retained.clone(),
    };
    for state in reaction_states(scope) {
        if state.custom_elements.processing.get() {
            continue;
        }
        let global = state
            .global_object
            .to_local(scope)
            .ok_or(V8BindingError::Allocation("reaction collection realm"))?;
        let context = global
            .get_creation_context(scope)
            .ok_or(V8BindingError::Allocation("reaction collection context"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        collect_for_migration(scope, &state)?;
        flush_pending_restoration(scope, &state)?;
    }
    if let Some(retained) = retained {
        retained.collect_pending.set(false);
    }
    refresh_native_registry_roots(scope)?;
    Ok(())
}
pub(super) struct ReactionBoundary {
    agent: Rc<ReactionAgent>,
    queue: ElementQueue,
    popped: bool,
}
impl ReactionBoundary {
    pub(super) fn begin(scope: &mut v8::PinScope) -> Result<Self, V8BindingError> {
        collect_agent(scope)?;
        Ok(Self::begin_parser_token(scope))
    }
    /// The native parser already preflights prior work before feeding input.
    /// At a token's construction point the specification only pushes a fresh
    /// reaction queue. Converting unrelated journals here could fail after the
    /// tokenizer accepted its token but before its constructor was entered.
    pub(super) fn begin_parser_token(scope: &mut v8::PinScope) -> Self {
        let agent = reaction_agent(scope);
        let queue = Rc::new(RefCell::new(VecDeque::new()));
        agent.stack.borrow_mut().push(queue.clone());
        Self {
            agent,
            queue,
            popped: false,
        }
    }
    fn pop(&mut self) {
        if !self.popped {
            let queue = self
                .agent
                .stack
                .borrow_mut()
                .pop()
                .expect("reaction stack boundary");
            debug_assert!(Rc::ptr_eq(&queue, &self.queue));
            self.popped = true;
        }
    }
    pub(super) fn complete(self, scope: &mut v8::PinScope) {
        if let Err(error) = self.complete_result(scope) {
            throw_binding_error(scope, &error);
        }
    }
    pub(super) fn complete_result(
        mut self,
        scope: &mut v8::PinScope,
    ) -> Result<(), V8BindingError> {
        let result = node_migration::resume_publications(scope).and_then(|_| collect_agent(scope));
        let collect_pending = result.is_err();
        self.pop();
        // Pop before invoking: a nested CEReactions operation creates a new
        // element queue, but can consume this same element's remaining reactions.
        let result = result.and_then(|_| invoke_queue(scope, &self.agent, &self.queue));
        if result.is_err() || !self.queue.borrow().is_empty() {
            self.agent
                .retry
                .borrow_mut()
                .push_back(Rc::new(RetainedQueue {
                    queue: self.queue.clone(),
                    collect_pending: Cell::new(collect_pending),
                }));
        }
        let deferred = if scope.is_execution_terminating() {
            Ok(())
        } else {
            collect_agent(scope).and_then(|_| schedule_backup(scope))
        };
        result.and(deferred)
    }
}
impl Drop for ReactionBoundary {
    fn drop(&mut self) {
        if !self.popped {
            self.pop();
            // Keep even an empty queue: retained native migration publication can
            // append adopted reactions to its original queue after this early exit.
            self.agent
                .retry
                .borrow_mut()
                .push_back(Rc::new(RetainedQueue {
                    queue: self.queue.clone(),
                    collect_pending: Cell::new(true),
                }));
        }
    }
}
fn release_resumed_elements(agent: &ReactionAgent) {
    let count = agent.suspended.borrow().len();
    for _ in 0..count {
        let target = agent.suspended.borrow_mut().pop_front().unwrap();
        let owner = target.owner.borrow().clone();
        if owner.is_some_and(|owner| owner.execution_suspended() && !owner.execution_retired()) {
            agent.suspended.borrow_mut().push_back(target);
        } else {
            agent.backup.borrow_mut().push_back(target);
        }
    }
}
fn schedule_backup(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let agent = reaction_agent(scope);
    release_resumed_elements(&agent);
    if agent.backup_queued.get() {
        let live =
            agent
                .backup_owner
                .borrow()
                .as_ref()
                .is_some_and(|(owner, generation, epoch)| {
                    owner.upgrade().is_some_and(|owner| {
                        !owner.execution_retired()
                            && !owner.execution_suspended()
                            && owner.document_generation() == *generation
                            && owner.document_stream_epoch() == *epoch
                    })
                });
        if live {
            return Ok(());
        }
        agent.backup_queued.set(false);
        agent.backup_owner.borrow_mut().take();
    }
    if !agent.stack.borrow().is_empty()
        || (agent.backup.borrow().is_empty() && agent.retry.borrow().is_empty())
    {
        return Ok(());
    }
    let Some(state) = reaction_states(scope).into_iter().next() else {
        return Ok(());
    };
    let global = state
        .global_object
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("reaction backup realm"))?;
    let context = global
        .get_creation_context(scope)
        .ok_or(V8BindingError::Allocation("reaction backup context"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    let serial = agent
        .backup_serial
        .get()
        .checked_add(1)
        .ok_or(V8BindingError::Allocation("reaction backup generation"))?;
    let data = v8::BigInt::new_from_u64(scope, serial);
    let callback = v8::Function::builder(backup_callback)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("reaction backup microtask"))?;
    agent.backup_serial.set(serial);
    *agent.backup_owner.borrow_mut() = Some((
        Rc::downgrade(&state),
        state.document_generation(),
        state.document_stream_epoch(),
    ));
    agent.backup_queued.set(true);
    scope.enqueue_microtask(callback);
    Ok(())
}
fn backup_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let agent = reaction_agent(scope);
    let Ok(serial) = args.data().try_cast::<v8::BigInt>() else {
        return;
    };
    if !agent.backup_queued.get() || serial.u64_value().0 != agent.backup_serial.get() {
        return;
    }
    agent.backup_queued.set(false);
    agent.backup_owner.borrow_mut().take();
    if let Err(error) = finish_agent(scope) {
        throw_binding_error(scope, &error);
    }
}
pub(super) fn defer_checkpoint(scope: &mut v8::PinScope) {
    if scope.is_execution_terminating() {
        return;
    }
    if let Err(error) = collect_agent(scope).and_then(|_| schedule_backup(scope)) {
        throw_binding_error(scope, &error);
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

fn ordered_registry_documents(
    scope: &mut v8::PinScope,
    registry: u64,
) -> Vec<(u64, Rc<V8BindingState>, NodeId)> {
    let states = scope
        .get_slot::<BindingStateRegistry>()
        .map(|states| {
            states
                .instances
                .borrow()
                .values()
                .filter_map(RcWeak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut documents = Vec::new();
    for state in &states {
        if state.execution_retired() {
            continue;
        }
        for (sequence, document) in state.dom.borrow().custom_registry_documents(registry) {
            if !seen.insert((Rc::as_ptr(&state.dom) as usize, document)) {
                continue;
            }
            let exact_owner = |candidate: &&Rc<V8BindingState>| {
                Rc::ptr_eq(&candidate.dom, &state.dom) && candidate.document_root() == document
            };
            let owner = states
                .iter()
                .filter(exact_owner)
                .find(|candidate| !candidate.execution_retired());
            let owner = match owner {
                Some(owner) => owner,
                None if states.iter().any(|candidate| {
                    Rc::ptr_eq(&candidate.dom, &state.dom) && candidate.document_root() == document
                }) =>
                {
                    continue;
                }
                None => state,
            };
            documents.push((sequence, owner.clone(), document));
        }
    }
    documents.sort_by_key(|(sequence, _, _)| *sequence);
    documents
}

fn subtree(state: &V8BindingState, root: NodeId) -> Vec<NodeId> {
    let dom = state.dom.borrow();
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Ok(record) = dom.arena().get(node) {
            result.push(node);
            // Shadow-including preorder visits the host's shadow tree before
            // light children, independently of when attachShadow was called.
            stack.extend(record.children.iter().rev().copied().filter(|child| {
                !matches!(
                    dom.arena().get(*child).map(|record| &record.kind),
                    Ok(NodeKind::Document | NodeKind::ShadowRoot { .. })
                )
            }));
            stack.extend(record.children.iter().rev().copied().filter(|child| {
                matches!(
                    dom.arena().get(*child).map(|record| &record.kind),
                    Ok(NodeKind::ShadowRoot { .. })
                )
            }));
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
        .filter(|element| !element.failed || element.upgrading)
        .and_then(|element| element.definition.callbacks[index].clone());
    if let Some(callback) = callback {
        #[cfg(test)]
        if FAIL_ENQUEUE_AFTER.with(|slot| match slot.get() {
            Some(0) => {
                slot.set(None);
                true
            }
            Some(n) => {
                slot.set(Some(n - 1));
                false
            }
            None => false,
        }) {
            return Err(V8BindingError::Allocation(
                "injected reaction enqueue failure",
            ));
        }
        let callback = callback
            .local(scope)
            .and_then(|value| value.try_cast::<v8::Function>().ok())
            .ok_or(V8BindingError::Allocation("custom reaction callback graph"))?;
        let target = wrap_node(scope, state, node)?;
        enqueue_element(scope, state, node)?;
        state
            .custom_elements
            .reactions
            .borrow_mut()
            .entry(node)
            .or_default()
            .push_back(Reaction::Callback {
                parser_document: {
                    let active =
                        state
                            .dom
                            .borrow()
                            .arena()
                            .node_document(node)?
                            .filter(|document| {
                                state
                                    .parser_custom_construction_depth
                                    .borrow()
                                    .get(document)
                                    .copied()
                                    .unwrap_or(0)
                                    > 0
                            });
                    let retained =
                        state
                            .custom_elements
                            .converting_cursor
                            .get()
                            .and_then(|cursor| {
                                state
                                    .custom_elements
                                    .parser_spans
                                    .borrow()
                                    .iter()
                                    .find(|(start, end, _)| *start < cursor && cursor <= *end)
                                    .map(|(_, _, document)| *document)
                            });
                    active
                        .or(retained)
                        .map(|document| (state.clone(), document))
                },
                target: v8::Global::new(scope, target),
                callback: v8::Global::new(scope, callback),
                arguments: arguments
                    .iter()
                    .map(|argument| v8::Global::new(scope, *argument))
                    .collect(),
            });
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
            let changed = element.connected != connected && (!element.failed || element.upgrading);
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
    for node in subtree(state, root) {
        enqueue_upgrade(scope, state, node, false)?;
    }
    Ok(())
}

fn definition_in_registry(
    state: &V8BindingState,
    node: NodeId,
    registry: &DefinitionRegistry,
) -> Result<Option<Rc<Definition>>, V8BindingError> {
    let (local, is_value) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        let NodeKind::Element { name } = &record.kind else {
            return Ok(None);
        };
        if record.custom_element_creation_failed
            || name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
        {
            return Ok(None);
        }
        (name.local.to_string(), record.custom_element_is.clone())
    };
    let name = if is_valid_custom_element_name(&local) {
        local.as_str()
    } else if let Some(value) = &is_value {
        value.as_str()
    } else {
        return Ok(None);
    };
    let result = registry
        .definitions
        .borrow()
        .get(name)
        .filter(|definition| definition.local_name == local)
        .cloned();
    Ok(result)
}
fn definition_for_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Option<Rc<Definition>>, V8BindingError> {
    {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        if record.custom_element_creation_failed
            || !matches!(&record.kind,NodeKind::Element{name}if name.ns.as_ref()=="http://www.w3.org/1999/xhtml")
        {
            return Ok(None);
        }
    }
    let Some(registry) = node_registry(scope, state, node)? else {
        return Ok(None);
    };
    associate_registry(state, node, Some(registry.id.get()))?;
    definition_in_registry(state, node, &registry)
}
fn upgrade_already_admitted(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> bool {
    agent_states(scope, state).iter().any(|owner| {
        owner.custom_elements.elements.borrow().contains_key(&node)
            || owner
                .custom_elements
                .pending_upgrades
                .borrow()
                .contains(&node)
    })
}
fn enqueue_upgrade(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    explicit: bool,
) -> Result<(), V8BindingError> {
    if !explicit && state.dom.borrow().arena().node_document(node)? != Some(state.document_root()) {
        return Ok(());
    }
    let Some(definition) = definition_for_node(scope, state, node)? else {
        return Ok(());
    };
    enqueue_upgrade_definition(scope, state, node, definition)
}
fn enqueue_upgrade_definition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    definition: Rc<Definition>,
) -> Result<(), V8BindingError> {
    if upgrade_already_admitted(scope, state, node) {
        return Ok(());
    }
    #[cfg(test)]
    if FAIL_UPGRADE_ENQUEUE_AFTER.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            true
        }
        Some(n) => {
            slot.set(Some(n - 1));
            false
        }
        None => false,
    }) {
        return Err(V8BindingError::Allocation(
            "injected upgrade enqueue failure",
        ));
    }
    // The upgrade reaction owns its accepted definition even if the target's
    // registry changes before delivery. Root the registry's private V8 graph
    // while its definition callbacks remain weak native handles.
    let registry = registry_by_id(scope, definition.registry)
        .ok_or(V8BindingError::Allocation("upgrade definition registry"))?;
    let registry_root = registry_object(scope, &registry)?;
    let registry_root = v8::Global::new(scope, registry_root);
    enqueue_element(scope, state, node)?;
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
        .push_back(Reaction::Upgrade {
            definition,
            registry_root,
        });
    Ok(())
}

pub(super) fn upgrade_for_document(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    for node in subtree(state, root) {
        enqueue_upgrade(scope, state, node, true)?;
    }
    Ok(())
}

/// The synchronous create-an-element algorithm is distinct from upgrading an
/// existing node: failure returns a fresh unknown element, preserving any
/// constructor-created object retained by author code.
pub(super) fn has_definition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<bool, V8BindingError> {
    Ok(definition_for_node(scope, state, node)?.is_some())
}
pub(super) fn create_synchronously(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    // Customized built-in construction is an upgrade of this exact object and
    // may adopt it. Retain the wrapper through construction and resolve its
    // actual owner instead of interpreting the old NodeId in the caller arena.
    let original = wrap_node(scope, state, node)?;
    let (created, error, _, _fallback) = create_synchronously_parser_outcome(scope, state, node);
    if let Some(error) = error {
        return Err(error);
    }
    if created == node {
        return node_identity::argument(scope, original.into())?
            .ok_or(V8BindingError::Allocation("created element identity"));
    }
    // Establish wrapper custody before the native fallback lease is released.
    let _ = wrap_node(scope, state, created)?;
    Ok((state.clone(), created))
}
/// A parser has already accepted its token. Preserve the fresh failed identity
/// independently of an infrastructure error after entering author construction.
pub(super) fn create_synchronously_parser_outcome(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> (
    NodeId,
    Option<V8BindingError>,
    bool,
    Option<oryn_dom::TrackedNodeLease>,
) {
    let mut attempt = SynchronousAttempt {
        fallback: None,
        entered: false,
        lease: None,
    };
    match create_synchronously_inner(scope, state, node, &mut attempt) {
        Ok(result) => (result, None, attempt.entered, attempt.lease),
        Err(error) => (
            if attempt.entered {
                attempt.fallback.unwrap_or(node)
            } else {
                node
            },
            Some(error),
            attempt.entered,
            attempt.lease,
        ),
    }
}
struct SynchronousAttempt {
    fallback: Option<NodeId>,
    entered: bool,
    lease: Option<oryn_dom::TrackedNodeLease>,
}
fn create_synchronously_inner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    attempt: &mut SynchronousAttempt,
) -> Result<NodeId, V8BindingError> {
    let definition = definition_for_node(scope, state, node)?;
    state
        .dom
        .borrow_mut()
        .arena_mut()
        .get_mut(node)?
        .custom_element_shadow_disabled = Some(
        definition
            .as_ref()
            .is_some_and(|definition| definition.disable_shadow),
    );
    let Some(definition) = definition else {
        return Ok(node);
    };
    // Customized built-ins use the upgrade algorithm and retain their identity.
    if definition.name != definition.local_name {
        upgrade_with_registry(scope, state, node)?;
        return Ok(node);
    }
    let (document, kind, registry) = {
        let dom = state.dom.borrow();
        let record = dom.arena().get(node)?;
        (
            dom.arena()
                .node_document(node)?
                .unwrap_or(state.document_root()),
            record.kind.clone(),
            record.custom_element_registry,
        )
    };
    // Reserve the failed identity before invoking author code. No error while
    // constructing/reporting afterward may expose the original undefined token
    // as a fresh upgrade candidate and invoke the constructor a second time.
    let fallback = state.dom.borrow_mut().create(kind.clone());
    attempt.lease = Some(oryn_dom::TrackedNodeLease::new(&state.dom, fallback)?);
    state
        .dom
        .borrow_mut()
        .set_node_document(fallback, document)?;
    {
        let mut dom = state.dom.borrow_mut();
        let record = dom.arena_mut().get_mut(fallback)?;
        record.custom_element_registry = registry;
        record.custom_element_creation_failed = true;
        record.custom_element_shadow_disabled = Some(definition.disable_shadow);
    }
    attempt.fallback = Some(fallback);
    // Autonomous synchronous creation has an active registry mapping, but no
    // upgrade construction-stack entry. HTMLConstructor creates the new object
    // in its own realm; nested direct construction must remain independent.
    let mut guard = Some(activate_constructor(scope, &definition));
    let mut result_node = None;
    {
        v8::tc_scope!(let tc,scope);
        let constructor = definition_constructor(tc, &definition)?;
        if !crate::v8_termination::checkpoint(tc) {
            tc.rethrow();
            return Err(V8BindingError::JavaScriptException);
        }
        attempt.entered = true;
        let object = constructor.new_instance(tc, &[]);
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        if let Some(object) =
            object.filter(|_| !tc.has_terminated() && !tc.is_execution_terminating())
        {
            // Non-node objects and proxies are constructor result validation
            // failures, not infrastructure errors escaping createElement.
            let identity = node_identity::argument(tc, object.into()).ok().flatten();
            let validation = if let Some((owner, result)) = identity {
                let dom = owner.dom.borrow();
                let record = dom.arena().get(result)?;
                match &record.kind {
                    NodeKind::Element { name }
                        if name.ns.as_ref() == "http://www.w3.org/1999/xhtml" =>
                    {
                        let children = record.children.iter().any(|child| {
                            dom.arena().get(*child).is_ok_and(|child| {
                                !matches!(child.kind, NodeKind::ShadowRoot { .. })
                            })
                        });
                        if !record.attributes.is_empty()
                            || children
                            || record.parent.is_some()
                            || !Rc::ptr_eq(&state.dom, &owner.dom)
                            || dom.arena().node_document(result)? != Some(document)
                            || name.local.as_ref() != definition.local_name
                        {
                            Some("NotSupportedError")
                        } else {
                            result_node = Some((owner.clone(), result));
                            None
                        }
                    }
                    _ => Some("TypeError"),
                }
            } else {
                Some("TypeError")
            };
            if let Some(error) = validation {
                #[cfg(test)]
                if FAIL_SYNCHRONOUS_REPORT.with(|slot| slot.replace(false)) {
                    return Err(V8BindingError::Allocation(
                        "injected constructor failure report admission",
                    ));
                }
                let exception = if error == "TypeError" {
                    let message = v8_string(tc, "custom constructor must return an HTMLElement")?;
                    v8::Exception::type_error(tc, message.try_cast::<v8::String>().unwrap())
                } else {
                    dom_exception_value(tc, error)?
                };
                drop(guard.take());
                let _ = report_callback_exception(tc, constructor.into(), exception, None);
            }
        } else if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        } else if let Some(exception) = tc.exception() {
            let message = tc.message();
            drop(guard.take());
            let _ = report_callback_exception(tc, constructor.into(), exception, message);
        }
        // Reporting invokes author error listeners, which can terminate even
        // when the constructor itself threw an ordinary exception.
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        }
    }
    drop(guard);
    // A parser already accepted this token before author code started. Even
    // when V8 terminates the constructor, finish its native fallback identity
    // now; the parser bridge retains the original termination and pauses after
    // inserting that handle. Retrying must not invoke the constructor again or
    // insert an author-retained partially constructed object as the fallback.
    if scope.is_execution_terminating() {
        result_node = None;
    }
    if let Some((owner, result)) = result_node {
        if let NodeKind::Element { name: requested } = &kind {
            let mut dom = owner.dom.borrow_mut();
            let record = dom.arena_mut().get_mut(result)?;
            if let NodeKind::Element { name } = &mut record.kind {
                name.prefix = requested.prefix.clone();
            }
            record.custom_element_is = None;
        }
        associate_registry(&owner, result, Some(definition.registry))?;
        owner
            .dom
            .borrow_mut()
            .set_custom_element_defined(result, true)?;
        return Ok(result);
    }
    // This preserves the native QualName, including literal colon local names.
    Ok(fallback)
}

fn upgrade_with_registry(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let Some(definition) = definition_for_node(scope, state, node)? else {
        return Ok(());
    };
    upgrade_with_definition(scope, state, node, definition)
}
fn upgrade_with_definition(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    definition: Rc<Definition>,
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
    let document = state
        .dom
        .borrow()
        .arena()
        .node_document(node)?
        .unwrap_or(state.document_root());
    // Resolve weak graph handles before publishing upgrading/failed metadata or
    // initial callbacks. Failure here has not entered author construction.
    #[cfg(test)]
    if FAIL_UPGRADE_CONSTRUCTOR.with(|slot| slot.replace(false)) {
        return Err(V8BindingError::Allocation(
            "injected upgrade constructor admission",
        ));
    }
    let constructor = definition_constructor(scope, &definition)?;
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
    let _wrapper =
        receiver_wrapper_id(scope, original).ok_or(V8BindingError::Allocation("custom wrapper"))?;
    state
        .dom
        .borrow_mut()
        .arena_mut()
        .get_mut(node)?
        .custom_element_shadow_disabled = Some(definition.disable_shadow);
    state.custom_elements.elements.borrow_mut().insert(
        node,
        Element {
            internals_revision: Rc::new(()),
            definition: definition.clone(),
            connected: false,
            document,
            failed: true,
            upgrading: true,
            internals_attached: false,
            form_owner: None,
            form_disabled: false,
            submission: Vec::new(),
            restoration: None,
        },
    );
    if definition.disable_shadow
        && state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .children
            .iter()
            .any(|child| {
                state
                    .dom
                    .borrow()
                    .arena()
                    .get(*child)
                    .is_ok_and(|record| matches!(record.kind, NodeKind::ShadowRoot { .. }))
            })
    {
        let exception = dom_exception_value(scope, "NotSupportedError")?;
        state
            .custom_elements
            .elements
            .borrow_mut()
            .get_mut(&node)
            .unwrap()
            .upgrading = false;
        let _ = report_exception(scope, exception, None);
        return Ok(());
    }
    if definition.form_associated {
        state
            .dom
            .borrow_mut()
            .register_form_associated_custom_element(node)?;
    }
    let target = match reaction_target(state, node) {
        Ok(target) => target,
        Err(error) => {
            state.custom_elements.elements.borrow_mut().remove(&node);
            state
                .dom
                .borrow_mut()
                .unregister_form_associated_custom_element(node);
            return Err(error);
        }
    };
    // Initial attribute and connected callbacks precede any reactions caused
    // by the constructor. They are staged transactionally before author code;
    // an allocation failure leaves this upgrade eligible for retry.
    let staged = (|| -> Result<(), V8BindingError> {
        let admission = MutationAdmission::new(scope, &state.custom_elements);
        for attribute in &initial_attributes {
            attribute_changed(
                scope,
                state,
                node,
                &attribute.local_name,
                None,
                Some(&attribute.value),
                attribute
                    .namespace
                    .as_ref()
                    .map(|namespace| namespace.as_ref()),
            )?;
        }
        let connected = state.dom.borrow().is_document_connected(node)?;
        if connected {
            enqueue(scope, state, node, 0, &[])?;
        }
        state
            .custom_elements
            .elements
            .borrow_mut()
            .get_mut(&node)
            .unwrap()
            .connected = connected;
        admission.commit();
        Ok(())
    })();
    if let Err(error) = staged {
        state.custom_elements.elements.borrow_mut().remove(&node);
        state
            .dom
            .borrow_mut()
            .unregister_form_associated_custom_element(node);
        return Err(error);
    }
    let stack = construction_stack(scope);
    let entry = Rc::new(ConstructionEntry {
        definition: definition.clone(),
        target: target.clone(),
        consumed: Cell::new(false),
    });
    stack.borrow_mut().push(entry.clone());
    state
        .custom_elements
        .constructing
        .set(state.custom_elements.constructing.get() + 1);
    let mut guard = Some(ConstructionGuard {
        state: state.clone(),
        stack: stack.clone(),
        entry: entry.clone(),
    });
    let mut active_constructor = Some(activate_constructor(scope, &definition));
    let success = {
        v8::tc_scope!(let tc, scope);
        let result = constructor.new_instance(tc, &[]);
        if !tc.has_caught() {
            let _ = crate::v8_termination::checkpoint(tc);
        }
        let result = result.filter(|_| !tc.has_terminated() && !tc.is_execution_terminating());
        let success = result.is_some_and(|object| {
            node_identity::argument(tc, object.into())
                .ok()
                .flatten()
                .is_some_and(|(owner, id)| {
                    target
                        .owner
                        .borrow()
                        .as_ref()
                        .is_some_and(|state| Rc::ptr_eq(state, &owner))
                        && id == target.node.get()
                })
                && entry.consumed.get()
        });
        drop(active_constructor.take());
        drop(guard.take());
        if !success && let Some(owner) = target.owner.borrow().clone() {
            owner
                .custom_elements
                .reactions
                .borrow_mut()
                .remove(&target.node.get());
            owner
                .custom_elements
                .initial_reactions
                .borrow_mut()
                .remove(&target.node.get());
        }
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        } else if let Some(exception) = tc.exception() {
            let message = tc.message();
            let _ = report_callback_exception(tc, constructor.into(), exception, message);
        } else if !success && !tc.is_execution_terminating() {
            let message = v8::String::new(
                tc,
                "custom element upgrade constructor returned a different object",
            )
            .ok_or(V8BindingError::Allocation("custom upgrade error"))?;
            let exception = v8::Exception::type_error(tc, message);
            let _ = report_callback_exception(tc, constructor.into(), exception, None);
        }
        if tc.has_terminated() || tc.is_execution_terminating() {
            tc.rethrow();
        }
        success
    };
    drop(guard);
    let Some(owner) = target.owner.borrow().clone() else {
        return Ok(());
    };
    let node = target.node.get();
    let state = &owner;
    state
        .custom_elements
        .elements
        .borrow_mut()
        .get_mut(&node)
        .ok_or(V8BindingError::Allocation("custom construction element"))?
        .upgrading = false;
    if success {
        state
            .custom_elements
            .elements
            .borrow_mut()
            .get_mut(&node)
            .unwrap()
            .failed = false;
        state
            .custom_elements
            .initial_reactions
            .borrow_mut()
            .insert(node, Vec::new());
        finish_initial_reactions(scope, state, node)?;
    } else {
        state.custom_elements.reactions.borrow_mut().remove(&node);
        state
            .custom_elements
            .initial_reactions
            .borrow_mut()
            .remove(&node);
        state
            .dom
            .borrow_mut()
            .set_custom_element_defined(node, false)?;
        state
            .dom
            .borrow_mut()
            .unregister_form_associated_custom_element(node);
    }
    Ok(())
}

// A successful constructor is never re-run if allocating its initial
// attribute/connection reactions fails. Keep that post-construction phase.
fn finish_initial_reactions(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    let Some(attributes) = state
        .custom_elements
        .initial_reactions
        .borrow()
        .get(&node)
        .cloned()
    else {
        return Ok(());
    };
    let admission = MutationAdmission::new(scope, &state.custom_elements);
    state
        .dom
        .borrow_mut()
        .set_custom_element_defined(node, true)?;
    #[cfg(test)]
    if attributes.is_empty() && FAIL_INITIAL_REACTION.with(|flag| flag.replace(false)) {
        return Err(V8BindingError::Allocation(
            "injected initial reaction failure",
        ));
    }
    for (name, value, namespace) in attributes {
        attribute_changed(
            scope,
            state,
            node,
            &name,
            None,
            Some(&value),
            namespace.as_deref(),
        )?;
        #[cfg(test)]
        if FAIL_INITIAL_REACTION.with(|flag| flag.replace(false)) {
            return Err(V8BindingError::Allocation(
                "injected initial reaction failure",
            ));
        }
    }
    let connected = state.dom.borrow().is_document_connected(node)?;
    connection(scope, state, node, connected)?;
    admission.commit();
    state
        .custom_elements
        .initial_reactions
        .borrow_mut()
        .remove(&node);
    Ok(())
}

// Mutation conversion enqueues native reactions but does not invoke author code.
// Roll back that staging on failure; replaying delivered callbacks is forbidden.
struct MutationAdmission<'a> {
    registry: &'a Registry,
    elements: Option<BTreeMap<NodeId, Element>>,
    reactions: BTreeMap<NodeId, VecDeque<Reaction>>,
    queue: ElementQueue,
    queue_len: usize,
    upgrades: BTreeSet<NodeId>,
}
impl<'a> MutationAdmission<'a> {
    fn new(scope: &mut v8::PinScope, registry: &'a Registry) -> Self {
        let queue = admission_queue(&reaction_agent(scope));
        let queue_len = queue.borrow().len();
        Self {
            registry,
            elements: Some(registry.elements.borrow().clone()),
            reactions: registry.reactions.borrow().clone(),
            queue,
            queue_len,
            upgrades: registry.pending_upgrades.borrow().clone(),
        }
    }
    fn commit(mut self) {
        self.elements.take();
    }
}
impl Drop for MutationAdmission<'_> {
    fn drop(&mut self) {
        if let Some(elements) = self.elements.take() {
            *self.registry.elements.borrow_mut() = elements;
            *self.registry.reactions.borrow_mut() = std::mem::take(&mut self.reactions);
            self.queue.borrow_mut().truncate(self.queue_len);
            *self.registry.pending_upgrades.borrow_mut() = std::mem::take(&mut self.upgrades);
        }
    }
}
/// Admit native lifecycle records without invoking author callbacks. Adoption
/// uses this while source identities are still addressable, before relocation.
pub(super) fn retain_parser_reaction_span(
    state: &V8BindingState,
    start: u64,
    end: u64,
    document: NodeId,
) {
    if start < end {
        state
            .custom_elements
            .parser_spans
            .borrow_mut()
            .push((start, end, document));
    }
}

pub(super) fn collect_for_migration(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let initial = state
        .custom_elements
        .initial_reactions
        .borrow()
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for node in initial {
        finish_initial_reactions(scope, state, node)?;
    }
    let batch = state
        .dom
        .borrow()
        .mutations_since(state.custom_elements.cursor.get());
    if batch.truncated {
        return Err(V8BindingError::EventCallback(
            "custom-element mutation journal overflow".into(),
        ));
    }
    let admission = MutationAdmission::new(scope, &state.custom_elements);
    struct ConversionCursor<'a>(&'a Cell<Option<u64>>);
    impl Drop for ConversionCursor<'_> {
        fn drop(&mut self) {
            self.0.set(None);
        }
    }
    let _conversion_cursor = ConversionCursor(&state.custom_elements.converting_cursor);
    let start = state.custom_elements.cursor.get().0;
    for (offset, mutation) in batch.mutations.into_iter().enumerate() {
        state
            .custom_elements
            .converting_cursor
            .set(Some(start + offset as u64 + 1));
        match mutation {
            Mutation::Removed { node, siblings, .. } => {
                if siblings.state_preserving_move {
                    continue;
                }
                for node in subtree(state, node) {
                    connection(scope, state, node, false)?;
                }
            }
            // Allocation records use parent=None; they are not DOM
            // insertions and must not observe later connectedness.
            Mutation::Inserted {
                node,
                parent: Some(_),
                siblings,
                ..
            } => {
                // A failed insertion can release its detached staging
                // tree before this historical journal entry is read.
                // NodeId includes its generation: a reused slot must
                // neither receive this work nor block later reactions.
                if matches!(
                    state.dom.borrow().arena().get(node),
                    Err(DomError::StaleNode(_))
                ) {
                    continue;
                }
                if siblings.state_preserving_move {
                    for node in subtree(state, node) {
                        if !state.dom.borrow().is_document_connected(node)? {
                            continue;
                        }
                        let callback = state
                            .custom_elements
                            .elements
                            .borrow()
                            .get(&node)
                            .is_some_and(|element| {
                                !element.failed && element.definition.callbacks[8].is_some()
                            });
                        if callback {
                            enqueue(scope, state, node, 8, &[])?;
                        } else {
                            enqueue(scope, state, node, 1, &[])?;
                            enqueue(scope, state, node, 0, &[])?;
                        }
                    }
                    continue;
                }
                stamp_uninitialized_subtree(state, node)?;
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
                    &name,
                    old_value.as_deref(),
                    new_value.as_deref(),
                    namespace.as_deref(),
                )?;
            }
            Mutation::DocumentOwnership {
                node,
                previous_document,
            } => {
                if matches!(
                    state.dom.borrow().arena().get(node),
                    Err(DomError::StaleNode(_))
                ) {
                    continue;
                }
                let document = state.dom.borrow().arena().node_document(node)?;
                if let Some(document) = document {
                    // Registry rebinding commits synchronously in native adoption.
                    let _ = previous_document;
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
        #[cfg(test)]
        mutation_conversion_fault()?;
    }
    state.custom_elements.converting_cursor.set(None);
    refresh_form_associations(scope, state)?;
    admission.commit();
    state.custom_elements.cursor.set(batch.next);
    state
        .custom_elements
        .parser_spans
        .borrow_mut()
        .retain(|(_, end, _)| *end > batch.next.0);
    Ok(())
}

pub(super) fn finish(
    scope: &mut v8::PinScope,
    _state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    finish_agent(scope)
}
fn finish_agent(scope: &mut v8::PinScope) -> Result<(), V8BindingError> {
    let agent = reaction_agent(scope);
    if agent.collecting.get() {
        return Ok(());
    }
    release_resumed_elements(&agent);
    node_migration::resume_publications(scope)?;
    collect_agent(scope)?;
    let agent = reaction_agent(scope);
    if !agent.stack.borrow().is_empty() || agent.invocation_depth.get() != 0 {
        return Ok(());
    }
    agent.remaining.set(1024);
    agent.invocation_depth.set(1);
    let _depth = InvocationDepthGuard {
        agent: agent.clone(),
        depth: 0,
    };
    // A failed invocation is resumed before later backup work. Do not remove
    // custody until the exact queue has completed successfully.
    loop {
        let retained = agent.retry.borrow().front().cloned();
        let Some(retained) = retained else {
            break;
        };
        if retained.collect_pending.get() {
            collect_agent(scope)?;
        }
        invoke_queue(scope, &agent, &retained.queue)?;
        if !retained.queue.borrow().is_empty() {
            return Ok(());
        }
        agent.retry.borrow_mut().pop_front();
    }
    invoke_queue(scope, &agent, &agent.backup)
}
struct InvocationDepthGuard {
    agent: Rc<ReactionAgent>,
    depth: usize,
}
impl Drop for InvocationDepthGuard {
    fn drop(&mut self) {
        self.agent.invocation_depth.set(self.depth);
    }
}
fn invoke_queue(
    scope: &mut v8::PinScope,
    agent: &Rc<ReactionAgent>,
    queue: &ElementQueue,
) -> Result<(), V8BindingError> {
    let depth = agent.invocation_depth.get();
    if depth >= 64 {
        return Err(V8BindingError::EventCallback(
            "custom-element reaction nesting limit exceeded".into(),
        ));
    }
    if depth == 0 {
        agent.remaining.set(1024);
    }
    agent.invocation_depth.set(depth + 1);
    let _depth = InvocationDepthGuard {
        agent: agent.clone(),
        depth,
    };
    loop {
        let target = queue.borrow().front().cloned();
        let Some(target) = target else {
            return Ok(());
        };
        loop {
            if scope.is_execution_terminating() {
                return Err(V8BindingError::JavaScriptException);
            }
            // Resolve again after every author callback: adoption may move the
            // element and its shared reaction queue into another Document arena.
            let Some(state) = target.owner.borrow().clone() else {
                break;
            };
            let node = target.node.get();
            if state.execution_retired() {
                state.custom_elements.reactions.borrow_mut().remove(&node);
                state
                    .custom_elements
                    .pending_upgrades
                    .borrow_mut()
                    .remove(&node);
                break;
            }
            if state.execution_suspended() {
                // A parked Document must not block active Documents sharing the
                // agent backup queue. Keep its relative element order separately.
                if queue
                    .borrow()
                    .front()
                    .is_some_and(|front| Rc::ptr_eq(front, &target))
                {
                    queue.borrow_mut().pop_front();
                    agent.suspended.borrow_mut().push_back(target.clone());
                }
                break;
            }
            if state.custom_elements.processing.get() {
                return Ok(());
            }
            let present = state
                .custom_elements
                .reactions
                .borrow()
                .get(&node)
                .is_some_and(|reactions| !reactions.is_empty());
            if !present {
                state.custom_elements.reactions.borrow_mut().remove(&node);
                break;
            }
            if agent.remaining.get() == 0 {
                return Err(V8BindingError::EventCallback(
                    "custom-element reaction limit exceeded".into(),
                ));
            }
            let global = state
                .global_object
                .to_local(scope)
                .ok_or(V8BindingError::Allocation("reaction invocation realm"))?;
            let context = global
                .get_creation_context(scope)
                .ok_or(V8BindingError::Allocation("reaction invocation context"))?;
            let scope = &mut v8::ContextScope::new(scope, context);
            // Service a pending native interrupt before consuming any queued
            // callback, including the first callback after native collection.
            {
                v8::tc_scope!(let caught,scope);
                if !crate::v8_termination::checkpoint(caught) {
                    caught.rethrow();
                    return Err(V8BindingError::JavaScriptException);
                }
            }
            let reaction = state
                .custom_elements
                .reactions
                .borrow_mut()
                .get_mut(&node)
                .and_then(VecDeque::pop_front)
                .unwrap();
            agent.remaining.set(agent.remaining.get() - 1);
            match reaction {
                Reaction::Upgrade {
                    definition,
                    registry_root,
                } => {
                    state
                        .custom_elements
                        .pending_upgrades
                        .borrow_mut()
                        .remove(&node);
                    if let Err(error) =
                        upgrade_with_definition(scope, &state, node, definition.clone())
                    {
                        if !state.custom_elements.elements.borrow().contains_key(&node) {
                            state
                                .custom_elements
                                .reactions
                                .borrow_mut()
                                .entry(node)
                                .or_default()
                                .push_front(Reaction::Upgrade {
                                    definition,
                                    registry_root,
                                });
                            state
                                .custom_elements
                                .pending_upgrades
                                .borrow_mut()
                                .insert(node);
                        }
                        return Err(error);
                    }
                }
                Reaction::Callback {
                    target,
                    callback,
                    arguments,
                    parser_document,
                } => {
                    struct ParserReaction(Option<(Rc<V8BindingState>, NodeId)>);
                    impl Drop for ParserReaction {
                        fn drop(&mut self) {
                            if let Some((state, document)) = &self.0 {
                                let mut depths =
                                    state.parser_custom_construction_depth.borrow_mut();
                                if let Some(depth) = depths.get_mut(document) {
                                    *depth = depth.saturating_sub(1);
                                    if *depth == 0 {
                                        depths.remove(document);
                                    }
                                }
                            }
                        }
                    }
                    if let Some((state, document)) = &parser_document {
                        *state
                            .parser_custom_construction_depth
                            .borrow_mut()
                            .entry(*document)
                            .or_default() += 1;
                    }
                    let _parser_reaction = ParserReaction(parser_document);
                    v8::tc_scope!(let tc,scope);
                    let callback = v8::Local::new(tc, &callback);
                    let target = v8::Local::new(tc, &target);
                    let args = arguments
                        .iter()
                        .map(|value| v8::Local::new(tc, value))
                        .collect::<Vec<_>>();
                    state.commit_parser_author_effect();
                    callback.call(tc, target.into(), &args);
                    if !tc.has_caught() {
                        let _ = crate::v8_termination::checkpoint(tc);
                    }
                    // Termination is not a reportable author exception. Keep it
                    // pending across TryCatch destruction and retain the queue
                    // suffix; the already-started callback stays consumed.
                    if tc.has_terminated() || tc.is_execution_terminating() {
                        tc.rethrow();
                        return Err(V8BindingError::JavaScriptException);
                    }
                    if let Some(exception) = tc.exception() {
                        let message = tc.message();
                        let _ = report_callback_exception(tc, callback.into(), exception, message);
                    }
                    if tc.has_terminated() || tc.is_execution_terminating() {
                        tc.rethrow();
                        return Err(V8BindingError::JavaScriptException);
                    }
                }
            }
            collect_agent(scope)?;
        }
        // A nested invocation may already have drained the backup front. Remove
        // only this exact entry, never a successor installed by author code.
        if queue
            .borrow()
            .front()
            .is_some_and(|front| Rc::ptr_eq(front, &target))
        {
            queue.borrow_mut().pop_front();
        }
    }
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

/// Reassociation follows the native tree on every reaction checkpoint, including
/// external form IDs, fieldset/legend mutations and same-agent adoption.
fn refresh_form_associations(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let nodes = state
        .custom_elements
        .elements
        .borrow()
        .iter()
        .filter(|(_, element)| element.definition.form_associated && !element.failed)
        .map(|(node, _)| *node)
        .collect::<Vec<_>>();
    for node in nodes {
        let (form, disabled) = {
            let dom = state.dom.borrow();
            if dom.arena().get(node).is_err() {
                continue;
            }
            (dom.form_owner(node)?, dom.control_disabled(node)?)
        };
        let (form_changed, disabled_changed) = {
            let elements = state.custom_elements.elements.borrow();
            let element = elements.get(&node).unwrap();
            (
                element.form_owner != form,
                element.form_disabled != disabled,
            )
        };
        if form_changed {
            let value = match form {
                Some(form) => wrap_node(scope, state, form)?.into(),
                None => v8::null(scope).into(),
            };
            enqueue(scope, state, node, 4, &[value])?;
            state
                .custom_elements
                .elements
                .borrow_mut()
                .get_mut(&node)
                .unwrap()
                .form_owner = form;
        }
        if disabled_changed {
            let value = v8::Boolean::new(scope, disabled).into();
            enqueue(scope, state, node, 5, &[value])?;
            state
                .custom_elements
                .elements
                .borrow_mut()
                .get_mut(&node)
                .unwrap()
                .form_disabled = disabled;
        }
    }
    Ok(())
}

pub(super) fn enqueue_form_reset(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<(), V8BindingError> {
    state.dom.borrow_mut().set_user_validity(node, false)?;
    for owner in agent_states(scope, state) {
        if owner.custom_elements.elements.borrow().contains_key(&node) {
            return enqueue(scope, &owner, node, 6, &[]);
        }
    }
    Ok(())
}

fn form_owner_state(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    agent_states(scope, state)
        .into_iter()
        .find(|owner| {
            owner
                .custom_elements
                .elements
                .borrow()
                .get(&node)
                .is_some_and(|element| {
                    element.definition.form_associated && (!element.failed || element.upgrading)
                })
        })
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "ElementInternals".into(),
            member: "NotSupportedError".into(),
        })
}

// Web IDL conversion may adopt the host into another arena. Keep native
// element custody across author code, then locate its moved custom-element
// record rather than reusing the source arena's now-obsolete local NodeId.
fn form_target_after_conversion(
    scope: &mut v8::PinScope,
    initial: &Rc<V8BindingState>,
    lease: &oryn_dom::TrackedNodeLease,
) -> Result<(Rc<V8BindingState>, NodeId), V8BindingError> {
    let (dom, node) = lease.current();
    let mut states = scope
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
    if !states.iter().any(|owner| Rc::ptr_eq(owner, initial)) {
        states.push(initial.clone());
    }
    let owner = states
        .into_iter()
        .find(|owner| {
            Rc::ptr_eq(&owner.dom, &dom)
                && owner
                    .custom_elements
                    .elements
                    .borrow()
                    .get(&node)
                    .is_some_and(|element| {
                        element.definition.form_associated && (!element.failed || element.upgrading)
                    })
        })
        .ok_or_else(|| V8BindingError::Unsupported {
            interface: "ElementInternals".into(),
            member: "NotSupportedError".into(),
        })?;
    Ok((owner, node))
}

pub(super) fn internals_get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if member == "behaviors" {
        return submit_behavior::behaviors(scope, state, node);
    }
    let _owner = form_owner_state(scope, state, node)?;
    match member {
        "form" => {
            let form = state.dom.borrow().form_owner(node)?;
            optional_node(scope, state, form)
        }
        "labels" => node_lists::labels(scope, state, node),
        "validity" => Ok(validation::wrap(scope, state, node)?.into()),
        "willValidate" => {
            Ok(v8::Boolean::new(scope, state.dom.borrow().control_will_validate(node)?).into())
        }
        "validationMessage" => {
            let dom = state.dom.borrow();
            v8_string(
                scope,
                if dom.control_will_validate(node)? {
                    dom.custom_validity_message(node)?
                } else {
                    ""
                },
            )
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "ElementInternals".into(),
            member: member.into(),
        }),
    }
}

/// Snapshot FormData entries at assignment. Neither later FormData mutation nor
/// forged object properties can change submission or restoration data.
fn form_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    value: v8::Local<'s, v8::Value>,
) -> Result<FormStateValue, V8BindingError> {
    if value.is_null_or_undefined() {
        let value: v8::Local<v8::Value> = v8::null(scope).into();
        return Ok((Vec::new(), v8::Global::new(scope, value)));
    }
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
        && let Some(owner) = binding_state_for_receiver(scope, object)
        && let Ok(id) = stateful_id(scope, object)
    {
        let record = platform_object(&owner, id)?;
        match &*record {
            PlatformObjectRecord::FormData(data) => {
                let entries = data.entries.clone();
                drop(record);
                let snapshot = entries
                    .iter()
                    .map(|(name, value)| (Some(name.clone()), value.clone()))
                    .collect();
                let id = register_platform_object(
                    state,
                    PlatformObjectRecord::FormData(FormDataRecord { entries }),
                );
                let object = new_platform_object(scope, state, "FormData", id)?;
                return Ok((
                    snapshot,
                    v8::Global::new(scope, v8::Local::<v8::Value>::from(object)),
                ));
            }
            PlatformObjectRecord::Blob(blob) if blob.file.is_some() => {
                return Ok((
                    vec![(None, v8::Global::new(scope, value))],
                    v8::Global::new(scope, value),
                ));
            }
            _ => {}
        }
    }
    let text = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    // USVString replaces unpaired surrogate code units.
    let text = text.to_rust_string_lossy(scope);
    let text = v8::String::new(scope, &text).ok_or(V8BindingError::Allocation("form value"))?;
    Ok((
        vec![(
            None,
            v8::Global::new(scope, v8::Local::<v8::Value>::from(text)),
        )],
        v8::Global::new(scope, v8::Local::<v8::Value>::from(text)),
    ))
}

pub(super) fn internals_call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    match member {
        "setFormValue" => {
            let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
            let (submission, default_state) = form_value(scope, state, args.get(0))?;
            // Optional Web IDL arguments without a default treat explicit
            // undefined as missing. Null remains an explicit null state.
            let restoration = if args.get(1).is_undefined() {
                default_state
            } else {
                form_value(scope, state, args.get(1))?.1
            };
            let (owner, node) = form_target_after_conversion(scope, state, &lease)?;
            let mut elements = owner.custom_elements.elements.borrow_mut();
            let element = elements
                .get_mut(&node)
                .ok_or(V8BindingError::Allocation("converted form host"))?;
            element.submission = submission;
            element.restoration = Some(restoration);
            element.internals_revision = Rc::new(());
        }
        "setValidity" => {
            let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
            let source = args.get(0);
            let object = if source.is_null_or_undefined() {
                None
            } else {
                Some(v8::Local::<v8::Object>::try_from(source).map_err(|_| {
                    V8BindingError::Unsupported {
                        interface: "ElementInternals".into(),
                        member: "TypeError: validity flags must be a dictionary".into(),
                    }
                })?)
            };
            let mut flags = BTreeSet::new();
            // Dictionary member conversion order is lexicographic Web IDL order.
            for name in [
                "badInput",
                "customError",
                "patternMismatch",
                "rangeOverflow",
                "rangeUnderflow",
                "stepMismatch",
                "tooLong",
                "tooShort",
                "typeMismatch",
                "valueMissing",
            ] {
                if let Some(object) = object {
                    let key = v8_string(scope, name)?;
                    if object
                        .get(scope, key)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .boolean_value(scope)
                    {
                        flags.insert(name.to_owned());
                    }
                }
            }
            let message = if args.get(1).is_undefined() {
                String::new()
            } else {
                let value = args
                    .get(1)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                rust_string_preserving_utf16(scope, value)?
            };
            // Finish Web IDL conversion before the form-associated semantic
            // check. A genuine foreign HTMLElement converts successfully.
            let anchor = if args.get(2).is_undefined() {
                None
            } else {
                let (owner, anchor) =
                    node_identity::argument(scope, args.get(2))?.ok_or_else(|| {
                        V8BindingError::Unsupported {
                            interface: "ElementInternals".into(),
                            member: "TypeError: anchor must be HTMLElement".into(),
                        }
                    })?;
                if !matches!(&owner.dom.borrow().arena().get(anchor)?.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml")
                {
                    return Err(V8BindingError::Unsupported {
                        interface: "ElementInternals".into(),
                        member: "TypeError: anchor must be HTMLElement".into(),
                    });
                }
                Some((owner, anchor))
            };
            let (owner, node) = form_target_after_conversion(scope, state, &lease)?;
            let state = &owner;
            if !flags.is_empty() && message.is_empty() {
                return Err(V8BindingError::Unsupported {
                    interface: "ElementInternals".into(),
                    member: "TypeError: invalid state requires message".into(),
                });
            }
            if let Some((anchor_owner, anchor)) = anchor {
                if !Rc::ptr_eq(&anchor_owner.dom, &state.dom) {
                    return Err(V8BindingError::Unsupported {
                        interface: "ElementInternals".into(),
                        member: "NotFoundError".into(),
                    });
                }
                let dom = state.dom.borrow();
                let mut cursor = Some(anchor);
                let mut descendant = false;
                while let Some(current) = cursor {
                    if current == node {
                        descendant = true;
                        break;
                    }
                    cursor = dom.parent(current)?;
                }
                if !descendant {
                    return Err(V8BindingError::Unsupported {
                        interface: "ElementInternals".into(),
                        member: "NotFoundError".into(),
                    });
                }
            }
            let message = if flags.is_empty() {
                String::new()
            } else {
                message
            };
            let host = wrap_node(scope, state, node)?;
            let key = v8::String::new(scope, "oryn.custom.validation.anchor")
                .ok_or(V8BindingError::Allocation("validation anchor"))?;
            let key = v8::Private::for_api(scope, Some(key));
            let anchor = if args.get(2).is_undefined() {
                host.into()
            } else {
                args.get(2)
            };
            host.set_private(scope, key, anchor)
                .ok_or(V8BindingError::JavaScriptException)?;
            state
                .dom
                .borrow_mut()
                .set_custom_element_validity(node, flags, message)?;
            mark_internals_changed(state, node);
        }
        "checkValidity" | "reportValidity" => {
            let _owner = form_owner_state(scope, state, node)?;
            let valid =
                validation::validate_control(scope, state, node, member == "reportValidity")?;
            return Ok(Some(v8::Boolean::new(scope, valid).into()));
        }
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: "ElementInternals".into(),
                member: member.into(),
            });
        }
    }
    Ok(None)
}

/// Undo keeps only weak identity evidence, never author values or callbacks.
pub(super) fn capture_internals_revision(
    state: &V8BindingState,
    node: NodeId,
) -> Option<RcWeak<()>> {
    state
        .custom_elements
        .elements
        .borrow()
        .get(&node)
        .map(|element| Rc::downgrade(&element.internals_revision))
}
pub(super) fn mark_internals_changed(state: &V8BindingState, node: NodeId) {
    if let Some(element) = state.custom_elements.elements.borrow_mut().get_mut(&node) {
        element.internals_revision = Rc::new(());
    }
}
pub(super) fn mark_internals_changed_in_dom(
    scope: &mut v8::PinScope,
    dom: &SharedDom,
    node: NodeId,
) {
    let states = scope
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
    for state in states {
        if Rc::ptr_eq(&state.dom, dom) {
            mark_internals_changed(&state, node);
        }
    }
}

pub(super) fn submission_entries(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<Vec<(String, v8::Global<v8::Value>)>, V8BindingError> {
    let owner = form_owner_state(scope, state, node)?;
    let name = state
        .dom
        .borrow()
        .attribute_in_null_namespace(node, "name")?
        .unwrap_or("")
        .to_owned();
    let elements = owner.custom_elements.elements.borrow();
    let element = elements.get(&node).unwrap();
    Ok(element
        .submission
        .iter()
        .filter_map(|(entry_name, value)| {
            let name = entry_name.as_deref().unwrap_or(&name);
            (!name.is_empty() || entry_name.is_some())
                .then(|| (oryn_dom::dom_string::usv(name), value.clone()))
        })
        .collect())
}

/// The history/autofill host supplies an actual saved state, not an author event.
/// Restoring does not overwrite the current submission value; the callback does.
pub(super) fn restore_form_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    value: v8::Local<'s, v8::Value>,
    mode: &str,
) -> Result<(), V8BindingError> {
    if !matches!(mode, "restore" | "autocomplete") {
        return Err(V8BindingError::Unsupported {
            interface: "ElementInternals".into(),
            member: "invalid restoration mode".into(),
        });
    }
    form_owner_state(scope, state, node)?;
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    let (_, value) = form_value(scope, state, value)?;
    let (owner, node) = form_target_after_conversion(scope, state, &lease)?;
    let value = v8::Local::new(scope, &value);
    let mode = v8_string(scope, mode)?;
    enqueue(scope, &owner, node, 7, &[value, mode])?;
    finish(scope, &owner)
}

#[cfg(test)]
mod form_associated_regressions {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn parser_constructor_termination_retains_fresh_fallback_without_replay() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let callback = v8::Function::new(scope, halt).unwrap();
        let name = v8::String::new(scope, "halt").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.constructed=0;customElements.define('x-halt-token',class extends HTMLElement{constructor(){super();constructed++;globalThis.retainedConstructorObject=this;halt()}})",
        );
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom.clone());
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let source = "<body><x-halt-token id=token><b id=child></b></x-halt-token></body>";
        let (progress, error) =
            document_parser_gate::with_parser_construction(scope, &state, || {
                gate.push_until_resource_or_script(source)
            });
        if error.is_none() && !scope.is_execution_terminating() {
            let observed=eval(scope,"JSON.stringify({constructed,retained:typeof retainedConstructorObject,defined:!!customElements.get('x-halt-token')})").to_rust_string_lossy(scope);
            panic!(
                "parser constructor did not stop: consumed={}, state={}, markup={}",
                progress.consumed_bytes,
                observed,
                dom.borrow().serialize(state.document_root()).unwrap()
            );
        }
        assert!(
            error.is_some(),
            "parser constructor termination must surface an error"
        );
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        assert!(dom.borrow().find_by_id("child").is_none());
        assert!(eval(scope,"constructed===1 && document.getElementById('token') instanceof HTMLUnknownElement && document.getElementById('token')!==retainedConstructorObject && retainedConstructorObject.parentNode===null").is_true());
        state.finish_custom_element_reactions(scope).unwrap();
        let (_, error) = document_parser_gate::with_parser_construction(scope, &state, || {
            gate.push_until_resource_or_script(&source[progress.consumed_bytes..])
        });
        assert!(error.is_none());
        assert!(eval(scope,"constructed===1 && document.getElementById('child').parentNode===document.getElementById('token')").is_true());
    }
    #[test]
    fn parser_constructor_error_listener_termination_retains_fallback_without_replay() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let callback = v8::Function::new(scope, halt).unwrap();
        let name = v8::String::new(scope, "halt").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.constructed=0;globalThis.reports=0;window.onerror=()=>{reports++;halt();return true};customElements.define('x-halt-token',class extends HTMLElement{constructor(){super();constructed++;globalThis.retainedConstructorObject=this;throw Error('constructor failure')}})",
        );
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom.clone());
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let source = "<body><x-halt-token id=token><b id=child></b></x-halt-token></body>";
        let (progress, error) =
            document_parser_gate::with_parser_construction(scope, &state, || {
                gate.push_until_resource_or_script(source)
            });
        if error.is_none() && !scope.is_execution_terminating() {
            let observed=eval(scope,"JSON.stringify({constructed,retained:typeof retainedConstructorObject,defined:!!customElements.get('x-halt-token')})").to_rust_string_lossy(scope);
            panic!(
                "parser constructor did not stop: consumed={}, state={}, markup={}",
                progress.consumed_bytes,
                observed,
                dom.borrow().serialize(state.document_root()).unwrap()
            );
        }
        assert!(
            error.is_some(),
            "parser constructor termination must surface an error"
        );
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        assert!(dom.borrow().find_by_id("child").is_none());
        assert!(eval(scope,"constructed===1 && reports===1 && document.getElementById('token') instanceof HTMLUnknownElement && document.getElementById('token')!==retainedConstructorObject && retainedConstructorObject.parentNode===null").is_true());
        state.finish_custom_element_reactions(scope).unwrap();
        let (_, error) = document_parser_gate::with_parser_construction(scope, &state, || {
            gate.push_until_resource_or_script(&source[progress.consumed_bytes..])
        });
        assert!(error.is_none());
        assert!(eval(scope,"constructed===1 && reports===1 && document.getElementById('child').parentNode===document.getElementById('token')").is_true());
    }
    #[test]
    fn parser_reaction_failure_retains_accepted_constructor_and_unconsumed_children() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.constructed=0;globalThis.events=[];customElements.define('x-token',class extends HTMLElement{static observedAttributes=['id'];constructor(){super();constructed++}attributeChangedCallback(){let error;try{document.write('forbidden')}catch(e){error=e}if(!error||error.name!=='InvalidStateError')throw Error('parser counter lost');events.push('attribute:'+this.childNodes.length)}connectedCallback(){events.push('connected:'+this.childNodes.length)}})",
        );
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom.clone());
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let source = "<body><x-token id=token><b id=child></b></x-token></body>";
        FAIL_ENQUEUE_AFTER.with(|failure| failure.set(Some(0)));
        let (progress, error) =
            document_parser_gate::with_parser_construction(scope, &state, || {
                gate.push_until_resource_or_script(source)
            });
        assert!(error.is_some());
        assert!(progress.consumed_bytes < source.len());
        assert!(dom.borrow().find_by_id("token").is_some());
        assert!(dom.borrow().find_by_id("child").is_none());
        assert!(eval(scope, "constructed===1 && events.length===0").is_true());
        state.finish_custom_element_reactions(scope).unwrap();
        assert!(eval(scope, "events.join()==='attribute:0,connected:0'").is_true());
        let (_, error) = document_parser_gate::with_parser_construction(scope, &state, || {
            gate.push_until_resource_or_script(&source[progress.consumed_bytes..])
        });
        assert!(error.is_none());
        assert!(dom.borrow().find_by_id("child").is_some());
        assert!(eval(scope, "constructed===1 && events.length===2").is_true());
    }
    #[test]
    fn upgrade_initial_reactions_precede_constructor_mutations_and_failure_clears_them() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><x-upgrade-order id=candidate data-observed=initial></x-upgrade-order><x-upgrade-fail id=bad data-observed=initial></x-upgrade-fail></body>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const calls=[];class Upgraded extends HTMLElement{static observedAttributes=['data-observed'];constructor(){super();calls.push('constructor');this.setAttribute('data-observed','changed');calls.push('constructor-end')}attributeChangedCallback(name,old,value){calls.push('attribute:'+old+':'+value)}connectedCallback(){calls.push('connected')}};
          customElements.define('x-upgrade-order',Upgraded);
          if(calls.join('|')!=='constructor|attribute:null:initial|connected|attribute:initial:changed|constructor-end')throw Error(calls.join('|'));
          calls.length=0;window.onerror=()=>true;
          customElements.define('x-upgrade-fail',class extends HTMLElement{static observedAttributes=['data-observed'];constructor(){super();throw Error('expected')}attributeChangedCallback(){calls.push('failed attribute')}connectedCallback(){calls.push('failed connected')}});
          return calls.length===0;
        })()"#).is_true());
    }
    #[test]
    fn form_internals_conversion_tracks_cross_arena_adoption_without_partial_updates() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "form-conversion-adoption").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><form id=f></form></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
        }
        let target = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><form id=f></form></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &target, target.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        assert!(eval(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};
          class Moving extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals()}};
          customElements.define('x-moving-conversion',Moving);
          const element=document.createElement('x-moving-conversion');element.setAttribute('name','field');
          const sourceForm=document.getElementById('f'),targetForm=destination.getElementById('f');sourceForm.append(element);
          element.i.setFormValue({toString(){targetForm.append(element);return 'submitted'}},
            {toString(){sourceForm.append(element);targetForm.append(element);return 'restoration'}});
          check(element.ownerDocument===destination,'converted owner');
          let delivered;targetForm.addEventListener('formdata',event=>{delivered=event;check(event.target===targetForm&&event.formData.get('field')==='submitted','foreign formdata event owner and entries')},{once:true});
          const entries=new FormData(targetForm);check(Object.getPrototypeOf(entries)===FormData.prototype&&entries.get('field')==='submitted','moved submission with constructor realm prototype');check(delivered&&delivered.formData===entries,'foreign event retains constructed FormData identity');
          const shadowChild=document.createElement('div');element.attachShadow({mode:'open'}).append(shadowChild);
          const nestedAnchor=document.createElement('input');shadowChild.attachShadow({mode:'closed'}).append(nestedAnchor);
          element.i.setValidity({get customError(){sourceForm.append(element);return true}},
            {toString(){targetForm.append(element);return 'invalid after conversion'}},nestedAnchor);
          check(element.i.validity.customError&&element.i.validationMessage==='invalid after conversion','moved validity');
          element.i.setValidity({get customError(){sourceForm.append(element);return false}},'');
          check(element.i.validity.valid&&element.i.validationMessage==='','returned validity');
          const sentinel={};let caught;
          try{element.i.setFormValue({toString(){targetForm.append(element);throw sentinel}})}catch(error){caught=error}
          check(caught===sentinel,'original conversion exception');
          check(new FormData(targetForm).get('field')==='submitted','failed conversion preserves prior submission');
          return true;
        })()"#).is_true());
    }
    #[test]
    fn form_state_restoration_conversion_follows_moved_host() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "restoration-adoption").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><form id=f></form></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body><form id=f></form></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        eval(
            scope,
            r#"
          globalThis.restored=[];
          class Restoring extends HTMLElement{
            static formAssociated=true;
            constructor(){super();this.i=this.attachInternals()}
            formStateRestoreCallback(value,mode){restored.push([this.ownerDocument===destination,value,mode])}
          }
          customElements.define('x-moving-restoration',Restoring);
          globalThis.moving=document.createElement('x-moving-restoration');moving.id='restore';moving.setAttribute('name','field');
          document.getElementById('f').append(moving);moving.i.setFormValue('original');
        "#,
        );
        let node = a.dom.borrow().find_by_id("restore").unwrap();
        let value = eval(
            scope,
            "({toString(){destination.getElementById('f').append(moving);return 'saved state'}})",
        );
        restore_form_state(scope, &a, node, value, "restore").unwrap();
        assert!(eval(scope,"JSON.stringify(restored)==='[[true,\"saved state\",\"restore\"]]'&&new FormData(destination.getElementById('f')).get('field')==='original'").is_true());
    }
    #[test]
    fn validation_anchor_adopted_to_another_arena_falls_back_to_host() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "validation-anchor-adoption").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        eval(
            scope,
            r#"
          customElements.define('x-anchor-owner',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals()}});
          globalThis.host=document.createElement('x-anchor-owner');host.id='host';document.body.append(host);
          globalThis.anchor=document.createElement('input');anchor.id='anchor';host.append(anchor);
          host.i.setValidity({customError:true},'message',anchor);
        "#,
        );
        let host = a.dom.borrow().find_by_id("host").unwrap();
        let original = a.dom.borrow().find_by_id("anchor").unwrap();
        assert_eq!(validation_anchor(scope, &a, host).unwrap(), original);
        eval(scope, "destination.body.append(anchor)");
        assert_eq!(validation_anchor(scope, &a, host).unwrap(), host);
        assert!(eval(scope,r#"(()=>{
          const rejected=(value,expected)=>{
            try{host.i.setValidity({badInput:true},'replacement',value);return false}
            catch(error){return error.name===expected&&host.i.validationMessage==='message'&&
              host.i.validity.customError&&!host.i.validity.badInput}
          };
          return rejected(anchor,'NotFoundError')&&rejected(destination.body,'NotFoundError')&&
            rejected(destination.createElementNS('http://www.w3.org/2000/svg','svg'),'TypeError')&&
            rejected(destination.createTextNode('text'),'TypeError')&&rejected(new Proxy(anchor,{}),'TypeError');
        })()"#).is_true());
        eval(scope, "host.append(anchor)");
        let returned = a.dom.borrow().find_by_id("anchor").unwrap();
        assert_eq!(validation_anchor(scope, &a, host).unwrap(), returned);
        assert!(
            eval(
                scope,
                "host.i.validationMessage==='message'&&host.i.validity.customError"
            )
            .is_true()
        );
        assert!(eval(scope,r#"(()=>{
          host.tabIndex=0;let calls=0;
          host.oninvalid=()=>{calls++;destination.body.append(anchor)};
          const result=host.i.reportValidity();
          return result===false&&calls===1&&anchor.ownerDocument===destination&&host.i.validationMessage==='message';
        })()"#).is_true());
        assert_eq!(validation_anchor(scope, &a, host).unwrap(), host);
    }
    #[test]
    fn borrowed_when_defined_uses_function_realm_and_preserves_pending_promise() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "when-defined-realm").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        for (index, context) in [first, second].into_iter().enumerate() {
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(DomRuntime::new()));
            install_generated_bindings(scope, context, dom, RealmId(index as u32), 1).unwrap();
        }
        for (name, expression) in [
            (
                "foreignWhenDefined",
                "CustomElementRegistry.prototype.whenDefined",
            ),
            ("ForeignPromise", "Promise"),
            ("ForeignTypeError", "TypeError"),
        ] {
            let value = {
                let scope = &mut v8::ContextScope::new(scope, second);
                eval(scope, expression)
            };
            let scope = &mut v8::ContextScope::new(scope, first);
            let key = v8::String::new(scope, name).unwrap();
            first.global(scope).set(scope, key.into(), value);
        }
        let scope = &mut v8::ContextScope::new(scope, first);
        assert!(eval(scope,r#"(()=>{
          globalThis.realmChecks=[];
          const rejected=foreignWhenDefined.call(customElements,Symbol());
          if(!(rejected instanceof ForeignPromise)||rejected instanceof Promise)return false;
          rejected.catch(error=>realmChecks.push(error instanceof ForeignTypeError&&!(error instanceof TypeError)));
          const sentinel={};
          foreignWhenDefined.call(customElements,{toString(){throw sentinel}})
            .catch(error=>realmChecks.push(error===sentinel));
          const pending=foreignWhenDefined.call(customElements,'x-cross-realm-pending');
          if(!(pending instanceof ForeignPromise)||customElements.whenDefined('x-cross-realm-pending')!==pending)return false;
          class Ready extends HTMLElement {}
          pending.then(value=>realmChecks.push(value===Ready));
          customElements.define('x-cross-realm-pending',Ready);
          const ready=foreignWhenDefined.call(customElements,'x-cross-realm-pending');
          if(!(ready instanceof ForeignPromise)||ready===pending)return false;
          ready.then(value=>realmChecks.push(value===Ready));
          return realmChecks.length===0;
        })()"#).is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "realmChecks.length===4&&realmChecks.every(Boolean)").is_true());
    }
    #[test]
    fn when_defined_rejects_binding_and_conversion_errors_without_throwing() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const method=CustomElementRegistry.prototype.whenDefined;
          const sentinel={conversionFailure:true};let conversions=0;
          globalThis.whenDefinedRejections=[];
          const cases=[
            [()=>customElements.whenDefined(),error=>error instanceof TypeError],
            [()=>method.call({}),error=>error instanceof TypeError],
            [()=>method.call({},'x-valid'),error=>error instanceof TypeError],
            [()=>method.call(new Proxy(customElements,{}),'x-valid'),error=>error instanceof TypeError],
            [()=>customElements.whenDefined(Symbol()),error=>error instanceof TypeError],
            [()=>customElements.whenDefined({toString(){conversions++;throw sentinel}}),error=>error===sentinel],
            [()=>customElements.whenDefined('invalid'),error=>error.name==='SyntaxError']
          ];
          for(const [invoke,check] of cases){
            let promise;
            try{promise=invoke()}catch(error){return false}
            if(!(promise instanceof Promise))return false;
            promise.then(()=>whenDefinedRejections.push(false),error=>whenDefinedRejections.push(check(error)));
          }
          // Non-promise registry operations keep synchronous binding errors.
          let synchronous=false;try{customElements.get()}catch(error){synchronous=error instanceof TypeError}
          return synchronous&&conversions===1&&whenDefinedRejections.length===0;
        })()"#).is_true());
        scope.perform_microtask_checkpoint();
        assert!(
            eval(
                scope,
                "whenDefinedRejections.length===7&&whenDefinedRejections.every(Boolean)"
            )
            .is_true()
        );
    }
    #[test]
    fn definition_callback_conversion_rejects_null_and_allows_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const callbacks=['connectedCallback','disconnectedCallback','connectedMoveCallback',
            'adoptedCallback','attributeChangedCallback','formAssociatedCallback',
            'formResetCallback','formDisabledCallback','formStateRestoreCallback'];
          let serial=0;
          for(const callback of callbacks){
            for(const invalid of [null,{},1,false,'callback']){
              const name='x-callback-conversion-'+serial++;
              class Candidate extends HTMLElement { static formAssociated=true; }
              const reads=[];
              for(const member of callbacks){
                Object.defineProperty(Candidate.prototype,member,{configurable:true,get(){
                  reads.push(member);return member===callback?invalid:undefined;
                }});
              }
              let rejected=false;
              try{customElements.define(name,Candidate)}catch(error){rejected=error instanceof TypeError}
              if(!rejected||customElements.get(name)!==undefined)return false;
              if(reads.join()!==callbacks.slice(0,callbacks.indexOf(callback)+1).join())return false;
              // Failed conversion must release the definition-running guard and
              // leave both this constructor and name available for a retry.
              Object.defineProperty(Candidate.prototype,callback,{value:undefined});
              customElements.define(name,Candidate);
              if(customElements.get(name)!==Candidate)return false;
            }
          }
          // Form callbacks are only converted for a form-associated definition.
          class Ordinary extends HTMLElement {}
          Ordinary.prototype.formAssociatedCallback=null;
          customElements.define('x-ordinary-unused-form-callback',Ordinary);
          return customElements.get('x-ordinary-unused-form-callback')===Ordinary;
        })()"#).is_true());
    }
    #[test]
    fn when_defined_constructor_then_can_reenter_registry_and_preserves_rejection() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(
            eval(
                scope,
                r#"(()=>{
          globalThis.getterCalls=0;globalThis.resolved=false;
          class Ready extends HTMLElement{};
          customElements.define('x-ready-reentrant',Ready);
          Object.defineProperty(Ready,'then',{get(){
            getterCalls++;customElements.define('x-defined-in-then',class extends HTMLElement{});
            return undefined;
          }});
          customElements.whenDefined('x-ready-reentrant').then(value=>resolved=value===Ready);
          globalThis.sentinel={};globalThis.rejected=false;
          class Throwing extends HTMLElement{};
          customElements.define('x-ready-throwing',Throwing);
          Object.defineProperty(Throwing,'then',{get(){throw sentinel}});
          customElements.whenDefined('x-ready-throwing').catch(error=>rejected=error===sentinel);
          return getterCalls===1&&customElements.get('x-defined-in-then')!==undefined;
        })()"#
            )
            .is_true()
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "resolved&&rejected&&getterCalls===1").is_true());
    }
    #[test]
    fn definition_candidate_failure_rolls_back_acceptance_and_retry_is_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<body><x-atomic id=a></x-atomic><x-atomic id=b></x-atomic></body>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.log=[];class Atomic extends HTMLElement{constructor(){super();log.push(this.id)}};customElements.whenDefined('x-atomic');Object.defineProperty(Atomic,'then',{get(){if(customElements.get('x-atomic')!==Atomic)throw Error('definition not committed');log.push('then');return undefined}})",
        );
        finish(scope, &state).unwrap();
        FAIL_DEFINITION_AFTER.with(|slot| slot.set(Some(1)));
        assert!(eval(scope,"try{customElements.define('x-atomic',Atomic);false}catch(error){customElements.get('x-atomic')===undefined&&log.length===0}").is_true());
        assert!(state.custom_elements.pending_upgrades.borrow().is_empty());
        finish(scope, &state).unwrap();
        FAIL_DEFINITION_GRAPH.with(|slot| slot.set(true));
        assert!(eval(scope,"try{customElements.define('x-atomic',Atomic);false}catch(error){customElements.get('x-atomic')===undefined&&log.length===0}").is_true());
        assert!(state.custom_elements.pending_upgrades.borrow().is_empty());
        assert!(
            state
                .custom_elements
                .reactions
                .borrow()
                .values()
                .all(VecDeque::is_empty)
        );
        finish(scope, &state).unwrap();
        assert!(
            eval(scope, "log.length===0").is_true(),
            "fully staged candidates cannot run after rejected definition publication"
        );
        assert!(
            eval(
                scope,
                "customElements.define('x-atomic',Atomic);log.join()==='then,a,b'"
            )
            .is_true()
        );
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.join()==='then,a,b'").is_true());
    }
    #[test]
    fn registry_document_owner_prefers_live_exact_binding_and_never_falls_back_from_retirement() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let document = dom.borrow().document();
        let inert = oryn_dom::html::parse("<body>other</body>");
        let other = dom
            .borrow_mut()
            .import_subtree(&inert.dom, inert.document)
            .unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let contexts = [
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
        ];
        let mut states = Vec::new();
        for (index, context) in contexts.iter().enumerate() {
            let scope = &mut v8::ContextScope::new(scope, *context);
            states.push(
                install_generated_bindings_with_root(
                    scope,
                    *context,
                    dom.clone(),
                    RealmId(index as u32),
                    1,
                    if index == 2 { other } else { document },
                )
                .unwrap(),
            );
        }
        dom.borrow_mut()
            .set_custom_element_registry(document, Some(987))
            .unwrap();
        states[0].retire_execution();
        let scope = &mut v8::ContextScope::new(scope, contexts[1]);
        let owners = ordered_registry_documents(scope, 987);
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].2, document);
        assert!(
            Rc::ptr_eq(&owners[0].1, &states[1]),
            "an earlier retired binding cannot hide the live exact owner"
        );
        states[1].retire_execution();
        assert!(!states[2].execution_retired());
        assert!(
            ordered_registry_documents(scope, 987).is_empty(),
            "a different live document in the same arena cannot own upgrades for a retired document"
        );
    }

    #[test]
    fn scoped_definition_uses_document_association_then_shadow_including_order() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let contexts = [
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
            v8::Context::new(scope, Default::default()),
        ];
        let token = v8::String::new(scope, "registry-document-order").unwrap();
        let mut states = Vec::new();
        for (index, context) in contexts.iter().enumerate() {
            context.set_security_token(token.into());
            let scope = &mut v8::ContextScope::new(scope, *context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            states.push(
                install_generated_bindings(scope, *context, dom, RealmId(index as u32), 1).unwrap(),
            );
        }
        let scope = &mut v8::ContextScope::new(scope, contexts[0]);
        for (index, state) in states.iter().enumerate() {
            let key = v8::String::new(scope, &format!("doc{index}")).unwrap();
            let document = wrap_node(scope, state, state.document_root()).unwrap();
            contexts[0]
                .global(scope)
                .set(scope, key.into(), document.into());
        }
        assert!(eval(scope,r#"(()=>{
          const registry=new CustomElementRegistry(),log=[];
          for(const [document,id] of [[doc1,'b'],[doc2,'c'],[doc0,'a']]){const host=document.createElement('div');document.body.append(host);const shadow=host.attachShadow({mode:'open',customElementRegistry:registry});shadow.innerHTML=`<x-ordered id=${id}></x-ordered>`}
          const host=doc0.createElement('div');doc0.body.append(host);const light=doc0.createElement('x-ordered',{customElementRegistry:registry});light.id='light';host.append(light);host.attachShadow({mode:'open',customElementRegistry:registry}).innerHTML='<x-ordered id=shadow></x-ordered>';
          registry.define('x-ordered',class extends HTMLElement{constructor(){super();log.push(this.id)}});
          globalThis.orderedRegistry=registry;globalThis.orderedLog=log;return log.join()==='b,c,a,shadow,light';
        })()"#).is_true());
        eval(
            scope,
            "doc2.body.firstElementChild.shadowRoot.innerHTML='<x-parked-definition></x-parked-definition>';orderedLog.length=0",
        );
        states[2].set_execution_suspended(true);
        assert!(eval(scope,"orderedRegistry.define('x-parked-definition',class extends HTMLElement{constructor(){super();orderedLog.push('restored')}});orderedLog.length===0").is_true());
        assert!(
            !states[2]
                .custom_elements
                .pending_upgrades
                .borrow()
                .is_empty()
        );
        states[2].set_execution_suspended(false);
        finish(scope, &states[0]).unwrap();
        assert!(eval(scope, "orderedLog.join()==='restored'").is_true());
        finish(scope, &states[0]).unwrap();
        assert!(eval(scope, "orderedLog.length===1").is_true());
    }
    #[test]
    fn definition_rethrows_original_getter_failure_and_reads_face_callbacks_in_spec_order() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(v,label)=>{if(!v)throw Error(label)};const failure={exact:'prototype failure'};
          const Bad=(function(){}).bind(null);Object.defineProperty(Bad,'prototype',{get(){throw failure}});
          try{customElements.define('x-failed-definition',Bad);throw Error('getter accepted')}catch(error){check(error===failure,'original thrown value')}
          check(customElements.get('x-failed-definition')===undefined,'failed definition is absent');
          class Good extends HTMLElement{};customElements.define('x-failed-definition',Good);
          const scoped=new CustomElementRegistry();try{scoped.define('x-scoped-button',class extends HTMLButtonElement{},{extends:'button'});throw Error('scoped customized built-in accepted')}catch(error){check(error.name==='NotSupportedError','scoped extends rejection')}
          let log=[];class Face extends HTMLElement{static formAssociated=true}
          for(const name of ['formAssociatedCallback','formResetCallback','formDisabledCallback','formStateRestoreCallback'])Object.defineProperty(Face.prototype,name,{get(){log.push(name);return undefined}});
          customElements.define('x-face-getters',Face);check(log.join()==='formAssociatedCallback,formResetCallback,formDisabledCallback,formStateRestoreCallback','FACE getter order');
          return true;
        })()"#).is_true());
    }
    #[test]
    fn html_constructor_observes_prototype_after_registration_and_before_consumed_marker() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(v,label)=>{if(!v)throw Error(label)};let gets=0;
          class Candidate extends HTMLElement{};
          const proxy=new Proxy(Candidate,{get(target,key,receiver){if(key==='prototype')++gets;return Reflect.get(target,key,receiver)}});
          try{Reflect.construct(HTMLElement,[],proxy);throw Error('unregistered admitted')}catch(error){check(error instanceof TypeError,'unregistered type')}
          check(gets===0,'no prototype access before registration');customElements.define('x-counted-native',proxy);gets=0;
          const direct=new proxy();check(gets===1&&direct.localName==='x-counted-native','direct exactly one');gets=0;
          const reflected=Reflect.construct(HTMLElement,[],proxy);check(gets===1&&reflected.localName==='x-counted-native','reflect exactly one');
          const existing=document.createElement('x-double-native');let secondError;
          class Double extends HTMLElement{constructor(){super();try{super()}catch(error){secondError=error}}}
          const doubleProxy=new Proxy(Double,{get(target,key,receiver){if(key==='prototype')++gets;return Reflect.get(target,key,receiver)}});
          customElements.define('x-double-native',doubleProxy);gets=0;customElements.upgrade(existing);
          check(gets===2&&secondError instanceof TypeError&&existing instanceof Double,'consumed marker is checked after prototype access');
          const reentrant=document.createElement('x-reentrant-native');let enter=false,inside=false,nested;window.onerror=()=>true;
          class Reentrant extends HTMLElement{}
          const reentrantProxy=new Proxy(Reentrant,{get(target,key,receiver){if(key==='prototype'&&enter&&!inside){inside=true;nested=Reflect.construct(HTMLElement,[],reentrantProxy);inside=false}return Reflect.get(target,key,receiver)}});
          customElements.define('x-reentrant-native',reentrantProxy);enter=true;customElements.upgrade(reentrant);
          check(nested===reentrant&&!reentrant.matches(':defined'),'getter consumption cannot reuse upgrade stack');
          return true;
        })()"#).is_true());
    }
    #[test]
    fn html_constructor_prototype_getter_revocation_only_requires_realm_for_fallback() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const primitive of [true,false]){
            let armed=false;let gets=0;function Target(){};
            Target.prototype=Object.create(HTMLElement.prototype);const prototype=Target.prototype;
            const revocable=Proxy.revocable(Target,{get(target,key,receiver){
              if(key==='prototype'&&armed){gets++;revocable.revoke();return primitive?1:prototype}
              return Reflect.get(target,key,receiver);
            }});
            const name=primitive?'x-revoked-fallback':'x-revoked-object';
            customElements.define(name,revocable.proxy);armed=true;
            let result;let failure;
            try{result=Reflect.construct(HTMLElement,[],revocable.proxy)}catch(error){failure=error}
            if(gets!==1)throw Error('prototype must be read exactly once');
            if(primitive){if(!(failure instanceof TypeError)||result!==undefined)throw Error('revoked fallback must throw TypeError')}
            else if(failure||Object.getPrototypeOf(result)!==prototype||result.localName!==name)throw Error('object prototype must not require another realm lookup');
          }
          return true;
        })()"#).is_true());
    }

    #[test]
    fn html_constructor_nonobject_prototype_uses_new_target_realm_intrinsic() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "constructor-prototype-realm").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let first_state = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let (foreign, prototype) = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let foreign = eval(scope, "(function Foreign(){})");
            let prototype = eval(scope, "HTMLElement.prototype");
            (
                v8::Global::new(scope, foreign),
                v8::Global::new(scope, prototype),
            )
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        for (name, value) in [("Foreign", &foreign), ("foreignPrototype", &prototype)] {
            let key = v8::String::new(scope, name).unwrap();
            let value = v8::Local::new(scope, value);
            first.global(scope).set(scope, key.into(), value);
        }
        assert!(eval(scope,"customElements.define('x-foreign-prototype',Foreign);Foreign.prototype=17;const result=Reflect.construct(HTMLElement,[],Foreign);Object.getPrototypeOf(result)===foreignPrototype&&result.ownerDocument===document&&result.localName==='x-foreign-prototype'").is_true());
        let _ = first_state;
    }
    #[test]
    fn constructor_and_reaction_errors_report_in_callback_realm() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "callback-report").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let (log, failure) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let log = eval(
                scope,
                "globalThis.reports=[];window.onerror=()=>{reports.push('caller');return true};reports",
            );
            let failure = eval(scope, "new Error('created in caller realm')");
            (v8::Global::new(scope, log), v8::Global::new(scope, failure))
        };
        let foreign = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            for (name, value) in [("reports", &log), ("failure", &failure)] {
                let key = v8::String::new(scope, name).unwrap();
                let value = v8::Local::new(scope, value);
                second.global(scope).set(scope, key.into(), value);
            }
            eval(
                scope,
                "window.onerror=()=>{reports.push('callback');return true}",
            );
            let function = eval(scope, "(function Foreign(){throw failure})");
            v8::Global::new(scope, function)
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let key = v8::String::new(scope, "Foreign").unwrap();
        let foreign = v8::Local::new(scope, &foreign);
        first.global(scope).set(scope, key.into(), foreign);
        assert!(eval(scope,"customElements.define('x-foreign-error',Foreign);document.createElement('x-foreign-error');class Reaction extends HTMLElement{};Reaction.prototype.connectedCallback=Foreign;customElements.define('x-reaction-error',Reaction);document.body.append(document.createElement('x-reaction-error'));reports.join()==='callback,callback'").is_true());
        let _ = a;
    }
    #[test]
    fn synchronous_construction_uses_active_registry_without_upgrade_stack() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};const registry=new CustomElementRegistry();let depth=0;let nested;const observations=[];
          class Prefix extends HTMLElement{constructor(){super();observations.push(this.prefix);if(depth++===0){nested=new Prefix();const other=document.createElementNS('http://www.w3.org/1999/xhtml','q:x-prefix',{customElementRegistry:registry});check(other.prefix==='q','nested prefix')}depth--}}
          registry.define('x-prefix',Prefix);const outer=document.createElementNS('http://www.w3.org/1999/xhtml','p:x-prefix',{customElementRegistry:registry});
          check(outer instanceof Prefix&&outer.prefix==='p'&&nested instanceof Prefix&&nested!==outer&&nested.prefix===null&&observations.every(value=>value===null),'independent synchronous construction and late prefix');
          let rejected=false;try{new Prefix()}catch(error){rejected=error instanceof TypeError}check(rejected,'active registry restored after return');
          customElements.define('x-illegal-interface',HTMLElement);rejected=false;try{new HTMLElement()}catch(error){rejected=error instanceof TypeError}check(rejected,'active HTML interface cannot construct itself');
          const events=[];class Reactions extends HTMLElement{static observedAttributes=['data-value'];constructor(){super();events.push('before');this.setAttribute('data-value','v');events.push('after')}attributeChangedCallback(){events.push('attribute')}}
          customElements.define('x-constructor-reactions',Reactions);window.onerror=()=>true;document.createElement('x-constructor-reactions');
          check(events.join()==='before,attribute,after','nested CEReactions run during constructor');return true;
        })()"#).is_true());
    }
    #[test]
    fn cross_arena_adoption_resolves_null_registry_from_original_parent_scope() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "registry-adoption").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<body><div id=declarative><template shadowrootmode=open shadowrootcustomelementregistry><x-null></x-null></template></div></body>").into_runtime().unwrap()));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        assert!(eval(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};const scoped=new CustomElementRegistry();
          for(const append of [false,true])for(const parentRegistry of [null,customElements,scoped])for(const childRegistry of [null,customElements,scoped]){
            const parent=document.createElement('div',{customElementRegistry:parentRegistry});const child=document.createElement('x-null',{customElementRegistry:childRegistry});parent.append(child);
            if(append)destination.body.append(parent);else destination.adoptNode(parent);
            check(parent.customElementRegistry===(parentRegistry===scoped?scoped:destination.customElementRegistry),'parent adoption');
            const expected=childRegistry===scoped?scoped:childRegistry===customElements?destination.customElementRegistry:parentRegistry===scoped?null:destination.customElementRegistry;
            check(child.customElementRegistry===expected,'child adoption');
          }
          const host=document.getElementById('declarative');const shadow=host.shadowRoot;const child=shadow.firstElementChild;destination.adoptNode(host);
          check(shadow.customElementRegistry===null&&child.customElementRegistry===null,'declarative null survives');
          const ordinary=document.createElement('div');const ordinaryShadow=ordinary.attachShadow({mode:'open',customElementRegistry:null});ordinaryShadow.append(document.createElement('x-null',{customElementRegistry:null}));destination.adoptNode(ordinary);
          check(ordinaryShadow.customElementRegistry===destination.customElementRegistry&&ordinaryShadow.firstChild.customElementRegistry===destination.customElementRegistry,'imperative null rebinds');
          return true;
        })()"#).is_true());
        let _ = a;
    }
    #[test]
    fn scoped_registry_definition_cycles_are_collectable() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let weak = {
            v8::scope!(let inner,scope);
            let registry=eval(inner,r#"(()=>{const registry=new CustomElementRegistry();class Cycle extends HTMLElement{static owner=registry;connectedCallback(){return registry}}registry.define('x-cycle',Cycle);return registry})()"#).try_cast::<v8::Object>().unwrap();
            v8::Weak::new(inner, registry)
        };
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(
            weak.to_local(scope).is_none(),
            "native registry records must not root constructor/registry cycles"
        );
    }
    #[test]
    fn detached_clone_traces_unwrapped_scoped_descendant_registry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        {
            v8::scope!(let inner,scope);
            assert!(eval(inner,r#"(()=>{
          const registry=new CustomElementRegistry();
          registry.define('x-unwrapped',class extends HTMLElement{ping(){return 42}});
          const source=document.createElement('section');source.append(document.createElement('x-unwrapped',{customElementRegistry:registry}));
          globalThis.retainedClone=source.cloneNode(true);return true;
        })()"#).is_true());
        }
        scope.low_memory_notification();
        scope.low_memory_notification();
        assert!(eval(scope,"retainedClone.firstChild.ping()===42 && retainedClone.firstChild.customElementRegistry.get('x-unwrapped')===retainedClone.firstChild.constructor").is_true());
    }
    #[test]
    fn scoped_registry_identity_initialization_and_creation_custody() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};
          const deferredRegistry=new CustomElementRegistry();const deferred=document.createElement('x-deferred-scope',{customElementRegistry:deferredRegistry});class Deferred extends HTMLElement{};deferredRegistry.define('x-deferred-scope',Deferred);check(!(deferred instanceof Deferred),'detached definition does not upgrade');deferredRegistry.initialize(deferred);check(deferred instanceof Deferred,'initialize upgrades already associated element');
          const registry=new CustomElementRegistry();class Scoped extends HTMLElement{constructor(){super();this.marker='scoped'}};
          registry.define('x-scope',Scoped);check(registry.get('x-scope')===Scoped&&registry.getName(Scoped)==='x-scope','definition identity');
          check(customElements.get('x-scope')===undefined&&customElements.getName(Scoped)===null,'global isolation');
          let threw=false;try{new Scoped()}catch(error){threw=error instanceof TypeError}check(threw,'scoped direct constructor');
          const nullElement=document.createElement('x-scope',{customElementRegistry:null});check(nullElement.customElementRegistry===null,'initial null ownership');
          document.body.append(nullElement);check(nullElement.customElementRegistry===null&&!nullElement.marker,'same document insertion preserves null');
          registry.initialize(nullElement);check(nullElement instanceof Scoped&&nullElement.customElementRegistry===registry,'initialize upgrades');
          const host=document.createElement('div');const root=host.attachShadow({mode:'open',customElementRegistry:null});
          const child=document.createElement('div',{customElementRegistry:null});root.append(child);registry.initialize(root);
          document.body.append(child);check(child.customElementRegistry===registry,'initialize stamps descendants before root changes');
          const made=document.createElement('x-scope',{customElementRegistry:registry});check(made instanceof Scoped&&made.marker==='scoped','scoped creation');
          check(made.cloneNode().customElementRegistry===registry,'clone retains scope');
          check(document.cloneNode().customElementRegistry===null,'global document clone starts with null registry');
          const fallbackRegistry=new CustomElementRegistry();class Fallback extends HTMLElement{};fallbackRegistry.define('x-fallback-clone',Fallback);
          const outer=document.createElement('div',{customElementRegistry:null});const middle=document.createElement('div',{customElementRegistry:registry});const inner=document.createElement('x-fallback-clone',{customElementRegistry:null});outer.append(middle);middle.append(inner);
          const cloned=outer.cloneNode(true);check(cloned.firstChild.firstChild.customElementRegistry===null,'clone null descendants remain null');
          const imported=document.importNode(outer,{customElementRegistry:fallbackRegistry});check(imported.customElementRegistry===fallbackRegistry&&imported.firstChild.customElementRegistry===registry&&imported.firstChild.firstChild instanceof Fallback,'import fallback crosses scoped ancestors');
          const blank=document.createElement('x-scope',{customElementRegistry:null});check(blank.cloneNode().customElementRegistry===null,'clone retains null');
          const inert=new Document();registry.initialize(inert);check(inert.customElementRegistry===registry,'inert document initialization');check(inert.cloneNode().customElementRegistry===registry,'scoped document clone retains registry');
          window.onerror=()=>true;const inertFallback=inert.createElementNS('http://www.w3.org/1999/xhtml','x-scope');check(inertFallback instanceof HTMLUnknownElement&&inertFallback.ownerDocument===inert,'constructor realm document mismatch uses inert fallback');
          return true;
        })()"#).is_true());
    }
    #[test]
    fn synchronous_custom_creation_checks_results_and_preserves_constructor_object() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const check=(value,label)=>{if(!value)throw Error(label)};const errors=[];window.onerror=(message,url,line,column,error)=>{errors.push(error.name);return true};
          let retained;class Invalid extends HTMLElement{constructor(){super();retained=this;this.id='retained'}};
          customElements.define('x-invalid-result',Invalid);const fallback=document.createElement('x-invalid-result');
          check(fallback instanceof HTMLUnknownElement&&fallback.localName==='x-invalid-result'&&!fallback.hasAttributes(),'fresh unknown fallback');
          check(retained instanceof Invalid&&retained.id==='retained'&&retained!==fallback,'author object preserved');
          check(errors.pop()==='NotSupportedError','attribute validation report');
          let moved;class Moved extends HTMLElement{constructor(){super();moved=this;document.body.append(this)}};
          customElements.define('x-moved-result',Moved);const movedFallback=document.createElement('x-moved-result');
          check(movedFallback instanceof HTMLUnknownElement&&movedFallback!==moved&&moved.parentNode===document.body&&movedFallback.parentNode===null&&errors.pop()==='NotSupportedError','constructor insertion preserves original object');
          const inert=document.implementation.createHTMLDocument();let adopted;class Adopted extends HTMLElement{constructor(){super();adopted=this;inert.adoptNode(this)}};
          customElements.define('x-adopted-result',Adopted);const adoptedFallback=document.createElement('x-adopted-result');
          check(adoptedFallback instanceof HTMLUnknownElement&&adoptedFallback.ownerDocument===document&&adopted.ownerDocument===inert&&adopted!==adoptedFallback&&errors.pop()==='NotSupportedError','constructor adoption preserves original ownership');
          class Plain extends HTMLElement{constructor(){return {}}};customElements.define('x-plain-result',Plain);
          check(document.createElement('x-plain-result') instanceof HTMLUnknownElement&&errors.pop()==='TypeError','result brand');
          class Repaired extends HTMLElement{constructor(){super();this.id='temporary';this.removeAttribute('id');const child=document.createTextNode('');this.append(child);child.remove()}};
          customElements.define('x-repaired-result',Repaired);check(document.createElement('x-repaired-result') instanceof Repaired,'final state validation');
          class Builtin extends HTMLButtonElement{constructor(){super();this.id='allowed'}};customElements.define('x-native-button',Builtin,{extends:'button'});
          const button=document.createElement('button',{is:'x-native-button'});check(button instanceof Builtin&&button.id==='allowed','customized builtin upgrade semantics');
          button.setAttribute('is','other-name');check(button instanceof Builtin,'creation is slot isolated');
          let conversions=[];let rejected=false;try{customElements.define('invalid',class extends HTMLElement{},{get extends(){conversions.push('extends');return undefined}})}catch(error){rejected=error.name==='SyntaxError'};
          check(rejected&&conversions.join()==='extends','dictionary conversion before definition validation');
          rejected=false;try{customElements.define('x-arrow',()=>{})}catch(error){rejected=error instanceof TypeError}check(rejected,'native IsConstructor');
          return errors.length===0;
        })()"#).is_true());
    }
    #[test]
    fn state_preserving_move_uses_captured_callback_and_element_order() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body><section id=destination></section></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const log=[];class Moving extends HTMLElement{connectedCallback(){log.push(this.id+' connected '+this.isConnected)}disconnectedCallback(){log.push(this.id+' disconnected '+this.isConnected)}connectedMoveCallback(){log.push(this.id+' moved '+this.isConnected)}};
          class Fallback extends HTMLElement{connectedCallback(){log.push(this.id+' connected '+this.isConnected)}disconnectedCallback(){log.push(this.id+' disconnected '+this.isConnected)}};
          customElements.define('x-moving',Moving);customElements.define('x-fallback',Fallback);
          const outer=document.createElement('x-moving');outer.id='outer';const inner=document.createElement('x-fallback');inner.id='inner';outer.attachShadow({mode:'open'}).append(inner);document.body.append(outer);log.length=0;
          Moving.prototype.connectedMoveCallback=()=>log.push('replaced');destination.moveBefore(outer,null);
          if(log.join('|')!=='outer moved true|inner disconnected true|inner connected true')throw Error(log.join('|'));
          log.length=0;destination.moveBefore(outer,outer);if(log.length)throw Error('self move');
          outer.remove();log.length=0;const detached=document.createElement('div');detached.moveBefore(outer,null);if(log.length)throw Error('detached move');
          const order=[];class Capture extends HTMLElement{};
          for(const name of ['connectedCallback','disconnectedCallback','connectedMoveCallback','adoptedCallback','attributeChangedCallback'])Object.defineProperty(Capture.prototype,name,{get(){order.push(name);return undefined}});
          customElements.define('x-capture',Capture);
          return order.join()==='connectedCallback,disconnectedCallback,connectedMoveCallback,adoptedCallback,attributeChangedCallback';
        })()"#).is_true());
    }
    #[test]
    fn borrowed_reaction_operation_flushes_foreign_document_before_return() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "same-agent-domain").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let foreign = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap();
            let element = eval(
                scope,
                "customElements.define('x-foreign',class extends HTMLElement{static observedAttributes=['data-value'];attributeChangedCallback(){this.observed=this.getAttribute('data-value')}});document.createElement('x-foreign')",
            );
            v8::Global::new(scope, element)
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap();
        let name = v8::String::new(scope, "foreignElement").unwrap();
        let foreign = v8::Local::new(scope, foreign);
        first.global(scope).set(scope, name.into(), foreign);
        assert!(eval(scope,"Element.prototype.setAttribute.call(foreignElement,'data-value','delivered');foreignElement.observed==='delivered'").is_true(),"CEReactions must finish across same-agent arenas before the borrowed operation returns");
    }
    #[test]
    fn face_reset_observes_output_defaults_and_click_uses_backup_microtask() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><x-reset id=x name=custom></x-reset><output id=o>default</output><input id=r type=reset></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          globalThis.log=[];
          customElements.define('x-reset',class extends HTMLElement{
            static formAssociated=true;
            constructor(){super();this.i=this.attachInternals();this.i.setFormValue('submitted');}
            formResetCallback(){log.push(o.value+':'+o.defaultValue);this.i.setFormValue('reset');}
          });
          o.value='changed';f.reset();if(log.join()!=='default:default')throw Error('built-in reset precedes synchronous FACE reaction');
          f.addEventListener('reset',e=>e.preventDefault(),{once:true});o.value='cancelled';f.reset();if(o.value!=='cancelled'||log.length!==1)throw Error('cancelled reset');
          o.defaultValue='next';r.click();if(log.length!==1||o.value!=='next')throw Error('click reaction ran synchronously');
          // Unrelated operations and a CE boundary for another node must not
          // consume the backup element queue before its native microtask.
          f.checkValidity();o.setAttribute('data-probe','yes');if(log.length!==1)throw Error('backup queue stolen');
          Promise.resolve().then(()=>log.push('promise'));return true;
        })()"#).is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope,"log.join('|')==='default:default|next:next|promise' && new FormData(f).get('custom')==='reset' && !('__oryn_run_custom_reactions' in globalThis)").is_true());
    }
    #[test]
    fn face_shadow_anchor_and_file_state_snapshot_preserve_native_values() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><x-files id=x name=proof></x-files></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          globalThis.restored=[];
          customElements.define('x-files',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();}formStateRestoreCallback(value,mode){restored.push(value,mode);}});
          const shadow=x.attachShadow({mode:'closed'}),inner=document.createElement('div');shadow.append(inner);const nested=inner.attachShadow({mode:'closed'}),anchor=document.createElement('input');nested.append(anchor);
          x.i.setValidity({customError:true},'anchor',anchor);x.i.setValidity({});
          const file=new File([new Uint8Array([0,255,13,10])],'proof.bin',{type:'application/octet-stream',lastModified:17});
          const data=new FormData();data.append('duplicate','first');data.append('proof',file);data.append('duplicate','second');
          x.i.setFormValue(data,data);data.delete('proof');data.set('duplicate','changed');
          const submitted=new FormData(f);if(submitted.get('proof')!==file||submitted.getAll('duplicate').join()!=='first,second')throw Error('FACE entry snapshot');return true;
        })()"#).is_true());
        let saved = capture_form_states(scope, &state).unwrap();
        let crate::form_state::FormState::Entries(entries) = &saved[0].state else {
            panic!("saved native entries")
        };
        let crate::form_state::FormEntryValue::File(file) = &entries[1].1 else {
            panic!("saved native file")
        };
        assert_eq!(file.bytes, [0, 255, 13, 10]);
        assert_eq!(file.last_modified, 17);
        restore_saved_form_state(
            scope,
            &state,
            &saved[0],
            crate::form_state::FormRestoreTarget::Exact,
            "restore",
        )
        .unwrap();
        assert!(eval(scope,"restored[1]==='restore'&&restored[0] instanceof FormData&&restored[0].get('proof').size===4&&restored[0].getAll('duplicate').join()==='first,second'").is_true());
    }
    #[test]
    fn custom_controls_validate_without_builtin_user_validity_selectors() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><x-validity id=x name=field></x-validity></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};let resets=0;
            customElements.define('x-validity',class extends HTMLElement{
                static formAssociated=true;
                constructor(){super();this.i=this.attachInternals();this.i.setFormValue('value');this.i.setValidity({valueMissing:true},'Required')}
                formResetCallback(){check(!this.matches(':user-invalid')&&!this.matches(':user-valid'),'reset state before callback');resets++}
            });
            const x=document.getElementById('x'),f=document.getElementById('f');f.onsubmit=e=>e.preventDefault();x.oninvalid=e=>e.preventDefault();
            check(!x.matches(':user-invalid'),'setValidity is not interaction');f.requestSubmit();check(x.matches(':invalid')&&!x.matches(':user-invalid'),'FACE is invalid but not a built-in user-validity control');
            check(x.matches('x-validity:invalid:not(:user-invalid)')&&x.closest('form:invalid')===f&&f.querySelector('x-validity:invalid')===x&&f.querySelectorAll('x-validity:invalid:not(:user-valid)').length===1,'full selector engine across all query operations');
            x.i.setValidity({});check(x.matches(':valid')&&!x.matches(':user-valid'),'live custom flags without user-validity');x.setAttribute('disabled','');check(!x.matches(':user-valid'),'disabled excludes');x.removeAttribute('disabled');check(x.matches(':valid')&&!x.matches(':user-valid'),'reenabled custom validity');
            f.addEventListener('reset',e=>e.preventDefault(),{once:true});f.reset();check(x.matches(':valid')&&!x.matches(':user-valid')&&resets===0,'cancelled reset');
            f.reset();check(resets===1&&!x.matches(':user-valid'),'accepted reset');
            x.i.setValidity({customError:true},'Invalid');f.noValidate=true;f.requestSubmit();check(x.matches(':invalid')&&!x.matches(':user-invalid'),'novalidate does not add FACE to user-validity selectors');
            const clone=x.cloneNode();check(!clone.matches(':user-invalid')&&!clone.matches(':user-valid'),'clone has fresh user validity');return true;
        })()"#).is_true());
    }
    #[test]
    fn custom_controls_submit_snapshots_and_participate_in_validation_and_reset() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=f><x-field id=x name=field></x-field></form><label for=x>Field</label>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};globalThis.events=[];
            customElements.define('x-field',class extends HTMLElement {
                static formAssociated=true;
                constructor(){super();this.i=this.attachInternals();this.i.setFormValue('initial')}
                formAssociatedCallback(form){events.push(form?.id??null)}
                formResetCallback(){events.push('reset');this.i.setFormValue('reset')}
            });
            const x=document.getElementById('x'),f=document.getElementById('f');
            check(x.i.form===f&&f.elements[0]===x&&x.i.labels.length===1,'native ownership');
            check(new FormData(f).get('field')==='initial','single value');
            const data=new FormData();data.append('a','first');data.append('a','second');x.i.setFormValue(data);data.set('a','mutated');
            check(new FormData(f).getAll('a').join('|')==='first|second','entry snapshot');
            x.i.setValidity({valueMissing:true},'Required');check(!f.checkValidity()&&!x.i.validity.valid&&x.i.validity.valueMissing&&!x.i.validity.customError,'live flags');
            let failed=false;try{x.i.setValidity({customError:true})}catch(e){failed=e.name==='TypeError'}check(failed&&x.i.validity.valueMissing,'atomic failed validity');
            x.i.setValidity({});check(f.checkValidity()&&x.i.validationMessage==='','clear validity');
            f.reset();check(events.includes('reset')&&new FormData(f).get('field')==='reset','reset callback');
            x.i.setFormValue(null);check([...new FormData(f)].length===0,'null excludes');return true;
        })()"#).is_true());
    }
    #[test]
    fn custom_control_entry_names_use_scalar_strings_without_changing_dom_names() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><x-named id=x></x-named><input id=builtin></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            customElements.define('x-named',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('custom')}});
            const x=document.getElementById('x'),builtin=document.getElementById('builtin'),f=document.getElementById('f');
            const raw='field\ud800';x.setAttribute('name',raw);builtin.name=raw;builtin.value='builtin';
            let data=new FormData(f);
            check(data.getAll('field\ufffd').join('|')==='custom|builtin','custom and built-in scalar entry names');
            check(x.getAttribute('name')===raw&&builtin.name===raw,'DOM names remain lossless');
            const entries=new FormData();entries.append('key\udc00','entry');entries.append('','empty key');
            x.i.setFormValue(entries);data=new FormData(f);
            check(data.get('key\ufffd')==='entry'&&data.get('')==='empty key','FormData keys and empty key preserved');
            const literal='\u{f0000}oryn-wtf16:[55296]';x.setAttribute('name',literal);x.i.setFormValue('literal');
            check(new FormData(f).get(literal)==='literal','reserved prefix is ordinary author text');
            return true;
        })()"#).is_true());
    }

    #[test]
    fn custom_control_reassociation_disabled_fieldset_and_negative_branding() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=a><fieldset id=fs disabled><legend><x-owned id=exempt></x-owned></legend><x-owned id=x name=x></x-owned></fieldset></form><form id=b></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};globalThis.changes=[];
            customElements.define('x-owned',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('value')}formAssociatedCallback(f){changes.push([this.id,'form',f?.id])}formDisabledCallback(d){changes.push([this.id,'disabled',d])}});
            const x=document.getElementById('x'),a=document.getElementById('a'),b=document.getElementById('b'),fs=document.getElementById('fs');
            check(!x.i.willValidate&&document.getElementById('exempt').i.willValidate,'legend exclusion');check(new FormData(a).get('x')===null,'disabled submission');
            fs.disabled=false;check(x.i.willValidate&&changes.some(e=>e[0]==='x'&&e[1]==='disabled'&&e[2]===false),'disabled reaction');
            x.setAttribute('form','b');check(x.i.form===b&&b.elements[0]===x,'external owner');
            b.id='renamed';check(x.i.form===null,'ID reassociation');
            customElements.define('x-plain',class extends HTMLElement{});const p=document.createElement('x-plain'),i=p.attachInternals();let denied=false;try{i.setFormValue('x')}catch(e){denied=e.name==='NotSupportedError'}check(denied,'non-FACE denied');
            denied=false;try{ElementInternals.prototype.setFormValue.call({},'x')}catch(e){denied=e.name==='TypeError'}check(denied,'brand');return true;
        })()"#).is_true());
    }
    #[test]
    fn input_history_uses_effective_text_state_for_unknown_and_empty_types() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form><input id=x name=x type=unknown><input id=y name=y type=''></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "document.getElementById('x').value='saved';document.getElementById('y').value='other'",
        );
        let x = state.dom.borrow().find_by_id("x").unwrap();
        state
            .text_control_selections
            .borrow_mut()
            .insert(x, (1, 4, "backward".into()));
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 2);
        assert!(
            saved
                .iter()
                .all(|entry| entry.definition == "#builtin/input/text")
        );
        eval(
            scope,
            "document.getElementById('x').setAttribute('type','text');document.getElementById('x').value='changed';document.getElementById('y').value='changed'",
        );
        queue_form_restoration(scope, &state, &saved).unwrap();
        assert!(eval(scope,"document.getElementById('x').value==='saved'&&document.getElementById('y').value==='other'&&document.getElementById('y').getAttribute('type')===''").is_true());
        assert_eq!(
            state.text_control_selections.borrow().get(&x),
            Some(&(1, 4, "backward".into()))
        );
    }

    #[test]
    fn builtin_history_restores_without_any_custom_element_definition() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><input id=text name=text><input id=check name=check type=checkbox><textarea id=area name=area></textarea></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "document.getElementById('text').value='saved text';document.getElementById('text').setSelectionRange(1,4,'backward');document.getElementById('check').checked=true;document.getElementById('area').value='saved area'",
        );
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 3);
        assert!(state.custom_elements.definitions.borrow().is_empty());
        eval(
            scope,
            "document.getElementById('text').value='changed';document.getElementById('check').checked=false;document.getElementById('area').value='changed'",
        );
        queue_form_restoration(scope, &state, &saved).unwrap();
        assert!(eval(scope,"document.getElementById('text').value==='saved text'&&document.getElementById('text').selectionStart===1&&document.getElementById('text').selectionEnd===4&&document.getElementById('text').selectionDirection==='backward'&&document.getElementById('check').checked&&document.getElementById('area').value==='saved area'").is_true());
        assert!(state.custom_elements.pending_restoration.borrow().is_none());
        eval(scope, "document.getElementById('text').value='later edit'");
        finish(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "document.getElementById('text').value==='later edit'"
            )
            .is_true()
        );
    }

    #[test]
    fn reflected_control_setters_follow_conversion_time_adoption() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "control-reflection-adoption").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let a = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(0), 1).unwrap()
        };
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        assert!(eval(scope,r#"(()=>{
          for(const [tag,names] of [['input',['accept','alt','placeholder','dirName','autocomplete','colorSpace','align','useMap']],['textarea',['placeholder','dirName','wrap','autocomplete']],['select',['autocomplete']],['form',['autocomplete']]]){
            for(const name of names){
              const control=document.createElement(tag);document.body.append(control);let calls=0;
              control[name]={toString(){calls++;destination.body.append(control);return 'value\ud800'}};
              const attribute=name==='dirName'?'dirname':name==='colorSpace'?'colorspace':name==='useMap'?'usemap':name;
              if(calls!==1||control.ownerDocument!==destination||control.getAttribute(attribute)!=='value\ud800')return false;
            }
          }
          for(const [tag,member] of [['div','classList'],['output','htmlFor'],['form','relList'],['link','relList'],['script','blocking']]){
            const element=document.createElement(tag);document.body.append(element);const list=element[member];
            list.value={toString(){destination.body.append(element);return 'one two'}};
            if(element.ownerDocument!==destination||list.value!=='one two'||element[member]!==list)return false;
            list.add({toString(){document.body.append(element);return 'three'}});
            if(element.ownerDocument!==document||list.value!=='one two three')return false;
            const order=[];
            if(!list.replace({toString(){order.push('old');destination.body.append(element);return 'two'}},{toString(){order.push('new');document.body.append(element);return 'four'}}))return false;
            if(order.join(',')!=='old,new'||list.value!=='one four three')return false;
            if(!list.contains({toString(){destination.body.append(element);return 'four'}}))return false;
            if(list.item({valueOf(){document.body.append(element);return 1}})!=='four')return false;
            if(list.toggle({toString(){destination.body.append(element);return 'four'}})!==false||list.value!=='one three')return false;
            list.remove({toString(){document.body.append(element);return 'one'}});
            if(list.value!=='three'||element.ownerDocument!==document)return false;
            const sentinel={};try{list.add({toString(){destination.body.append(element);throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
            if(list.value!=='three'||element.ownerDocument!==destination)return false;
            if(member==='classList'||member==='htmlFor'){
              let calls=0;try{list.supports({toString(){calls++;document.body.append(element);return 'three'}});return false}catch(error){if(!(error instanceof TypeError)||calls!==1)return false}
            }
          }
          for(const tag of ['label','script']){
            const element=document.createElement(tag);let calls=0;
            element.htmlFor={toString(){calls++;destination.adoptNode(element);return 'target\ud800'}};
            if(calls!==1||element.ownerDocument!==destination||element.htmlFor!=='target\ud800'||element.getAttribute('for')!=='target\ud800')return false;
            const sentinel={};try{element.htmlFor={toString(){document.adoptNode(element);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(element.htmlFor!=='target\ud800'||element.ownerDocument!==document)return false;
          }
          {
            const host=document.createElement('div');let calls=0;
            const root=host.attachShadow({mode:{toString(){destination.adoptNode(host);return 'open'}},referenceTarget:{toString(){calls++;return 'target\ud800'}}});
            if(calls!==1||host.ownerDocument!==destination||root.ownerDocument!==destination||host.shadowRoot!==root||root.referenceTarget!=='target\ud800')return false;
            root.referenceTarget={toString(){document.adoptNode(host);return 'moved\udc00'}};
            if(root.ownerDocument!==document||root.referenceTarget!=='moved\udc00'||host.shadowRoot!==root)return false;
            const sentinel={};try{root.referenceTarget={toString(){destination.adoptNode(host);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(root.ownerDocument!==destination||root.referenceTarget!=='moved\udc00')return false;
            root.referenceTarget=null;if(root.referenceTarget!==null)return false;
            root.referenceTarget='';if(root.referenceTarget!=='')return false;
            root.innerHTML='<span>moved</span>';
            const movedHTML=host.getHTML({shadowRoots:{*[Symbol.iterator](){yield root;document.adoptNode(host);}}});
            if(host.ownerDocument!==document||root.ownerDocument!==document||!movedHTML.includes('<span>moved</span>')||!movedHTML.startsWith('<template shadowrootmode="open"'))return false;
            const other=document.createElement('div');
            try{other.attachShadow({mode:'open',get referenceTarget(){destination.adoptNode(other);throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
            if(other.ownerDocument!==destination||other.shadowRoot!==null)return false;
          }
          {
            const element=document.createElement('div');let calls=0;
            element.popover={toString(){calls++;destination.adoptNode(element);return 'manual'}};
            if(calls!==1||element.ownerDocument!==destination||element.popover!=='manual')return false;
            element.popover={toString(){document.adoptNode(element);return 'invalid\ud800'}};
            if(element.ownerDocument!==document||element.getAttribute('popover')!=='invalid\ud800')return false;
            const sentinel={};try{element.popover={toString(){destination.adoptNode(element);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(element.getAttribute('popover')!=='invalid\ud800'||element.ownerDocument!==destination)return false;
            element.popover=null;if(element.hasAttribute('popover')||element.popover!==null)return false;
            element.popover='manual';element.popover=undefined;if(element.hasAttribute('popover'))return false;
          }
          for(const element of [document.createElement('div'),document.createElementNS('http://www.w3.org/2000/svg','svg')]){
            let calls=0;element.tabIndex={valueOf(){calls++;destination.adoptNode(element);return 4294967295}};
            if(calls!==1||element.ownerDocument!==destination||element.tabIndex!==-1||element.getAttribute('tabindex')!=='-1')return false;
            const sentinel={};try{element.tabIndex={valueOf(){document.adoptNode(element);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(element.tabIndex!==-1||element.ownerDocument!==document)return false;
          }
          for(const tag of ['img','input'])for(const member of ['width','height']){
            const element=document.createElement(tag);if(tag==='input')element.type='image';
            let calls=0;element[member]={valueOf(){calls++;destination.adoptNode(element);return 4294967299.75}};
            if(calls!==1||element.ownerDocument!==destination||element[member]!==3||element.getAttribute(member)!=='3')return false;
            element[member]=NaN;if(element[member]!==0||element.getAttribute(member)!=='0')return false;
            element[member]=12.9;if(element[member]!==12)return false;
            const sentinel={};try{element[member]={valueOf(){document.adoptNode(element);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(element.ownerDocument!==document||element[member]!==12)return false;
            if(tag==='input'){element.type='text';if(element[member]!==0||element.getAttribute(member)!=='12')return false;}
          }
          for(const [tag,names] of [['input',['name','value','defaultValue','pattern']],['textarea',['name','value','defaultValue']],['button',['name','value']],['select',['name']],['option',['value','label','text']],['optgroup',['label']],['form',['name']],['fieldset',['name']],['output',['name']]]){
            for(const name of names){
              const control=document.createElement(tag);document.body.append(control);let calls=0;
              control[name]={toString(){calls++;destination.body.append(control);return 'converted\ud800'}};
              if(calls!==1||control.ownerDocument!==destination||control[name]!=='converted\ud800')return false;
              const sentinel={};try{control[name]={toString(){document.body.append(control);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
              if(control.ownerDocument!==document||control[name]!=='converted\ud800')return false;
            }
          }
          for(const [tag,name] of [['input','size'],['textarea','cols'],['textarea','rows'],['select','size'],['input','minLength'],['input','maxLength'],['textarea','minLength'],['textarea','maxLength']]){
            const control=document.createElement(tag);document.body.append(control);let calls=0;
            control[name]={valueOf(){calls++;destination.body.append(control);return 7}};
            if(calls!==1||control.ownerDocument!==destination||control[name]!==7||control.getAttribute(name.toLowerCase())!=='7')return false;
            const sentinel={};try{control[name]={valueOf(){document.body.append(control);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(control.ownerDocument!==document||control[name]!==7)return false;
          }
          {
            const input=document.createElement('input');input.type='number';input.value='3';document.body.append(input);
            input.valueAsNumber={valueOf(){destination.body.append(input);return 7}};
            if(input.ownerDocument!==destination||input.value!=='7')return false;
            try{input.valueAsNumber={valueOf(){document.body.append(input);input.type='text';return 8}};return false}catch(error){if(error.name!=='InvalidStateError')return false}
            if(input.ownerDocument!==document||input.value!=='7')return false;
            try{input.valueAsNumber=Infinity;return false}catch(error){if(!(error instanceof TypeError))return false}
            input.valueAsNumber={valueOf(){destination.body.append(input);input.type='number';return 9}};
            if(input.ownerDocument!==destination||input.value!=='9')return false;
            const sentinel={};try{input.valueAsNumber={valueOf(){document.body.append(input);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(input.ownerDocument!==document||input.value!=='9')return false;
          }
          for(const tag of ['input','textarea']){
            const control=document.createElement(tag);control.value='abcdef';document.body.append(control);control.setSelectionRange(1,5,'forward');
            control.selectionStart={valueOf(){destination.body.append(control);return 2}};
            if(control.ownerDocument!==destination||control.selectionStart!==2||control.selectionEnd!==5)return false;
            control.selectionEnd={valueOf(){document.body.append(control);return 1}};
            if(control.ownerDocument!==document||control.selectionStart!==1||control.selectionEnd!==1)return false;
            control.selectionDirection={toString(){destination.body.append(control);return 'backward'}};
            if(control.ownerDocument!==destination||control.selectionDirection!=='backward')return false;
            const sentinel={};try{control.selectionStart={valueOf(){document.body.append(control);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(control.ownerDocument!==document||control.selectionStart!==1||control.selectionEnd!==1)return false;
            if(tag==='input'){
              try{control.selectionEnd={valueOf(){destination.body.append(control);control.type='number';return 3}};return false}catch(error){if(error.name!=='InvalidStateError')return false}
              if(control.ownerDocument!==destination||control.selectionEnd!==null)return false;
            }
          }
          for(const tag of ['input','textarea']){
            const control=document.createElement(tag);control.value='abcdef';document.body.append(control);const log=[];
            control.setSelectionRange({valueOf(){log.push('start');destination.body.append(control);return 2}},
              {valueOf(){log.push('end');document.body.append(control);return 5}},
              {toString(){log.push('direction');destination.body.append(control);return 'backward'}});
            if(log.join()!=='start,end,direction'||control.ownerDocument!==destination||control.selectionStart!==2||control.selectionEnd!==5||control.selectionDirection!=='backward')return false;
            control.setRangeText({toString(){document.body.append(control);return 'Q'}},
              {valueOf(){destination.body.append(control);return 1}},
              {valueOf(){document.body.append(control);return 3}},
              {toString(){destination.body.append(control);return 'end'}});
            if(control.ownerDocument!==destination||control.value!=='aQdef'||control.selectionStart!==2||control.selectionEnd!==2)return false;
            const range=control.createValueRange({valueOf(){document.body.append(control);return 1}},
              {valueOf(){destination.body.append(control);return 2}});
            if(control.ownerDocument!==destination||range.startOffset!==1||range.endOffset!==2)return false;
            const sentinel={};try{control.createValueRange({valueOf(){document.body.append(control);throw sentinel}},2);return false}catch(error){if(error!==sentinel)return false}
            if(control.ownerDocument!==document||control.value!=='aQdef')return false;
          }
          {
            const input=document.createElement('input');input.type='number';input.value='3';input.step='3';document.body.append(input);
            input.stepUp({valueOf(){destination.body.append(input);return 2}});
            if(input.ownerDocument!==destination||input.value!=='9')return false;
            input.stepDown({valueOf(){document.body.append(input);return 1}});
            if(input.ownerDocument!==document||input.value!=='6')return false;
          }
          for(const tag of ['input','textarea','select']){
            const control=document.createElement(tag);document.body.append(control);
            control.setCustomValidity({toString(){destination.body.append(control);return 'failure\ud800'}});
            if(control.ownerDocument!==destination||!control.validity.customError||control.validationMessage!=='failure\ud800')return false;
            const sentinel={};try{control.setCustomValidity({toString(){document.body.append(control);throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
            if(control.ownerDocument!==document||control.validationMessage!=='failure\ud800')return false;
            control.setCustomValidity({toString(){destination.body.append(control);return ''}});
            if(control.ownerDocument!==destination||control.validity.customError||control.validationMessage!=='')return false;
          }
          for(const member of ['value','nodeValue','textContent']){
            const element=document.createElement('div');element.setAttribute('title','old');document.body.append(element);
            const attribute=element.getAttributeNode('title');let calls=0;
            attribute[member]={toString(){calls++;destination.body.append(element);return 'attribute\ud800'}};
            if(calls!==1||attribute.ownerDocument!==destination||attribute.ownerElement!==element||element.getAttribute('title')!=='attribute\ud800')return false;
            const sentinel={};try{attribute[member]={toString(){document.body.append(element);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(attribute.ownerDocument!==document||attribute.value!=='attribute\ud800')return false;
            attribute[member]=null;
            if(attribute.value!==(member==='value'?'null':''))return false;
          }
          for(const name of ['selectedIndex','length']){
            const control=document.createElement('select');control.innerHTML='<option>a</option><option>b</option><option>c</option>';document.body.append(control);
            control[name]={valueOf(){destination.body.append(control);return 1}};
            if(control.ownerDocument!==destination||control[name]!==1)return false;
            const sentinel={};try{control[name]={valueOf(){document.body.append(control);throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(control.ownerDocument!==document||control[name]!==1)return false;
          }
          return true;
        })()"#).is_true());
        assert!(!a.execution_retired());
        assert!(!b.execution_retired());
    }
    #[test]
    fn get_html_shadow_flag_matrix_for_elements_and_shadow_roots() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const tag of ['article','aside','blockquote','body','div','footer','h1','h2','h3','h4','h5','h6','header','main','nav','p','section','span'])
          for(const mode of ['open','closed'])for(const delegatesFocus of [false,true])for(const serializable of [false,true])for(const clonable of [false,true])for(const empty of [false,true]){
            const host=document.createElement(tag);host.innerHTML='<span>light</span>';
            const root=host.attachShadow({mode,delegatesFocus,serializable,clonable});if(!empty)root.append(document.createElement('slot'));
            const shadow='<template shadowrootmode="'+mode+'"'+(delegatesFocus?' shadowrootdelegatesfocus=""':'')+(serializable?' shadowrootserializable=""':'')+(clonable?' shadowrootclonable=""':'')+'>'+(empty?'':'<slot></slot>')+'</template>';
            const light='<span>light</span>';
            if(host.getHTML()!==light||host.getHTML({shadowRoots:[root]})!==shadow+light||host.getHTML({serializableShadowRoots:true})!==(serializable?shadow:'')+light)return false;
            const wrapper=document.createElement('div').attachShadow({mode:'open'});wrapper.append(host);
            if(wrapper.getHTML({shadowRoots:[root]})!=='<'+tag+'>'+shadow+light+'</'+tag+'>')return false;
          }
          return true;
        })()"#).is_true());
    }
    #[test]
    fn get_html_selects_shadow_roots_and_closes_invalid_iterators() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const host=document.createElement('div');host.innerHTML='<span>light</span>';
          const root=host.attachShadow({mode:'closed',serializable:true});root.innerHTML='<template><b>shadow</b></template>';
          const expected='<template shadowrootmode="closed" shadowrootserializable=""><template><b>shadow</b></template></template><span>light</span>';
          if(host.getHTML()!==host.innerHTML||host.getHTML(null)!==host.innerHTML||host.getHTML({serializableShadowRoots:true})!==expected)return false;
          if(host.getHTML({shadowRoots:[root,root]})!==expected||root.getHTML()!=='<template><b>shadow</b></template>')return false;
          const order=[];host.getHTML(new Proxy({shadowRoots:[root]},{get(target,key){order.push(key);return target[key]}}));
          if(order.join(',')!=='serializableShadowRoots,shadowRoots')return false;
          let closed=false;
          const bad={*[Symbol.iterator](){try{yield root;yield host;}finally{closed=true}}};
          try{host.getHTML({shadowRoots:bad});return false}catch(error){if(!(error instanceof TypeError))return false}
          if(!closed)return false;
          const sentinel={};try{host.getHTML({get shadowRoots(){throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
          for(const value of [true,1,'x',Symbol()]){try{host.getHTML(value);return false}catch(error){if(!(error instanceof TypeError))return false}}
          try{host.getHTML({shadowRoots:null});return false}catch(error){if(!(error instanceof TypeError))return false}
          const outer=document.createElement('div'),outerRoot=outer.attachShadow({mode:'closed'});
          const inner=document.createElement('span');outerRoot.append(inner);
          const innerRoot=inner.attachShadow({mode:'open',serializable:true});innerRoot.textContent='nested';
          if(outer.getHTML({shadowRoots:[innerRoot]})!=='')return false;
          const nested='<template shadowrootmode="closed"><span><template shadowrootmode="open" shadowrootserializable="">nested</template></span></template>';
          if(outer.getHTML({shadowRoots:[outerRoot,innerRoot]})!==nested||outer.getHTML({serializableShadowRoots:true,shadowRoots:[outerRoot]})!==nested)return false;
          for(const registry of [customElements,new CustomElementRegistry(),null]){
            const scopedHost=document.createElement('div');
            const scoped=scopedHost.attachShadow({mode:'open',customElementRegistry:registry,referenceTarget:'target'});
            const suffix=registry===customElements?'':' shadowrootcustomelementregistry=""';
            if(scopedHost.getHTML({shadowRoots:[scoped]})!=='<template shadowrootmode="open" shadowrootreferencetarget="target"'+suffix+'></template>')return false;
          }
          return true;
        })()"#).is_true());
    }
    #[test]
    fn shadow_serializable_flag_survives_clone_and_dictionary_conversion() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const host=document.createElement('div'),order=[];
          const options=new Proxy({mode:'open',clonable:true,serializable:true},{get(target,key){order.push(key);return target[key]}});
          const root=host.attachShadow(options);
          if(order.join(',')!=='clonable,customElementRegistry,delegatesFocus,mode,referenceTarget,serializable,slotAssignment')return false;
          if(root.serializable!==true||Object.getOwnPropertyDescriptor(ShadowRoot.prototype,'serializable').set!==undefined)return false;
          root.innerHTML='<span>kept</span>';
          const copy=host.cloneNode(true);if(!copy.shadowRoot.serializable||copy.shadowRoot.innerHTML!=='<span>kept</span>')return false;
          const ordinary=document.createElement('div').attachShadow({mode:'open'});if(ordinary.serializable!==false)return false;
          const failing=document.createElement('div'),sentinel={};let later=false;
          try{failing.attachShadow({mode:'open',get serializable(){throw sentinel},get slotAssignment(){later=true;return 'named'}});return false}catch(error){if(error!==sentinel)return false}
          return !later&&failing.shadowRoot===null;
        })()"#).is_true());
    }
    #[test]
    fn token_list_replacement_validation_and_empty_updates() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const [tag,member,attribute] of [['div','classList','class'],['output','htmlFor','for'],['form','relList','rel'],['a','relList','rel'],['area','relList','rel'],['link','relList','rel'],['script','blocking','blocking'],['style','blocking','blocking'],['link','blocking','blocking']]){
            const element=document.createElement(tag),list=element[member];
            const observer=new MutationObserver(()=>{});observer.observe(element,{attributes:true});
            list.add();list.remove();list.remove('absent');
            if(element.hasAttribute(attribute)||observer.takeRecords().length)return false;
            list.value='';if(!element.hasAttribute(attribute)||observer.takeRecords().length!==1)return false;
            list.value='original';observer.takeRecords();
            for(const [old,next,name] of [[' ','','SyntaxError'],['',' ','SyntaxError'],['a','','SyntaxError'],[' a','b','InvalidCharacterError'],['a','b\t','InvalidCharacterError']]){
              try{list.replace(old,next);return false}catch(error){if(error.name!==name)return false}
              if(list.value!=='original'||observer.takeRecords().length)return false;
            }
            const sentinel={},order=[];
            try{list.replace({toString(){order.push('old');return ''}},{toString(){order.push('new');throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
            if(order.join(',')!=='old,new'||list.value!=='original'||observer.takeRecords().length)return false;
            for(const [before,old,next,expected] of [['c b a','c','a','a b'],['a b c','c','a','a b'],['a a b','a','a','a b']]){
              list.value=before;observer.takeRecords();
              if(!list.replace(old,next)||list.value!==expected||observer.takeRecords().length!==1)return false;
            }
            list.value=' a  b ';observer.takeRecords();
            if(list.replace('absent','c')||list.value!==' a  b '||observer.takeRecords().length)return false;
            observer.disconnect();
          }
          return true;
        })()"#).is_true());
    }
    #[test]
    fn associated_token_list_values_preserve_dom_strings_and_conversion_errors() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const [tag,member,attribute] of [['output','htmlFor','for'],['form','relList','rel'],['a','relList','rel'],['area','relList','rel'],['link','relList','rel'],['script','blocking','blocking'],['style','blocking','blocking'],['link','blocking','blocking']]){
            const element=document.createElement(tag),list=element[member];
            for(const value of ['\ud800','\udc00','first \ud800\udc00 \ud800','\u0000','one\vtwo']){
              list.value=value;if(list.value!==value||element.getAttribute(attribute)!==value||String(list)!==value)return false;
              element[member]=value;if(list.value!==value||element.getAttribute(attribute)!==value)return false;
            }
            let calls=0;list.value={toString(){calls++;return '\ud800 token'}};
            if(calls!==1||list.length!==2||list[0]!=='\ud800'||!list.contains('\ud800'))return false;
            list.add('\udc00');if(list.value!=='\ud800 token \udc00')return false;
            const invalidBefore=list.value;try{list.add('\ud800 token');return false}catch(error){if(error.name!=='InvalidCharacterError')return false}if(list.value!==invalidBefore)return false;
            list.remove('\ud800');if(list.value!=='token \udc00')return false;
            const before=list.value,sentinel={};
            try{list.value={toString(){throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(list.value!==before)return false;
            try{element[member]=Symbol();return false}catch(error){if(!(error instanceof TypeError))return false}
            if(list.value!==before)return false;
            list.value=null;if(list.value!=='null')return false;
          }
          return true;
        })()"#).is_true());
    }
    #[test]
    fn relation_lists_are_live_and_support_tokens_for_their_element() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          const navigation=['noopener','noreferrer','opener'];
          const resource=['modulepreload','preload','preconnect','dns-prefetch','stylesheet','icon','alternate','prefetch','prerender','next','manifest','apple-touch-icon','apple-touch-icon-precomposed','canonical','expect'];
          const metadata=['author','bookmark','external','help','import','license','nofollow','pingback','prev','search','tag'];
          for(const tag of ['form','a','area','link']){
            const element=document.createElement(tag),list=element.relList;
            if(!(list instanceof DOMTokenList)||list!==element.relList||Object.hasOwn(element,'relList'))return false;
            element.rel='unknown unknown next';if(list.length!==2||list[0]!=='unknown'||list[1]!=='next')return false;
            list.remove('unknown');list.add('extra');if(element.rel!=='next extra')return false;
            element.relList='one two';if(list.value!=='one two'||element.getAttribute('rel')!=='one two')return false;
            element.setAttribute('rel','changed');if(list.value!=='changed'||list!==element.relList)return false;
            const supported=tag==='link'?resource:navigation;
            const unsupported=metadata.concat(tag==='link'?navigation:resource);
            for(const token of supported){if(!list.supports(token)||!list.supports(token.toUpperCase()))return false;}
            for(const token of unsupported){if(list.supports(token)||list.supports(token.toUpperCase()))return false;}
            let conversions=0;if(!list.supports({toString(){conversions++;return supported[0]}})||conversions!==1)return false;
            const sentinel={};try{list.supports({toString(){throw sentinel}});return false}catch(error){if(error!==sentinel)return false}
            element.classList.add('separate');if(list.contains('separate'))return false;
            element.removeAttribute('rel');if(list.length!==0||list.value!=='')return false;
          }
          const textarea=document.createElement('textarea');textarea.setAttribute('type','password');
          const descriptor=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'type');
          return textarea.type==='textarea'&&descriptor.get!==undefined&&descriptor.set===undefined&&!Object.hasOwn(textarea,'type');
        })()"#).is_true());
    }
    #[test]
    fn control_dimension_reflection_and_textarea_text_length() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const [tag,member,fallback] of [['input','size',20],['textarea','cols',20],['textarea','rows',2]]){
            const element=document.createElement(tag);
            if(element[member]!==fallback)return false;
            for(const [raw,expected] of [['',fallback],['0',fallback],['-1',fallback],['2147483648',fallback],['\u000b7',fallback],['\u00a07',fallback],[' +7tail',7],['1.5',1],['2147483647',2147483647]]){
              element.setAttribute(member,raw);if(element[member]!==expected||element.getAttribute(member)!==raw)return false;
            }
            element[member]=3.9;if(element[member]!==3||element.getAttribute(member)!=='3')return false;
            if(tag==='input'){
              let rejected=false;try{element[member]=0}catch(error){rejected=error.name==='IndexSizeError'}
              if(!rejected||element[member]!==3)return false;
            }else{element[member]=0;if(element[member]!==fallback||element.getAttribute(member)!==String(fallback))return false;}
            element[member]=4294967297;if(element[member]!==1)return false;
            const sentinel={};try{element[member]={valueOf(){throw sentinel}};return false}catch(error){if(error!==sentinel)return false}
            if(element[member]!==1)return false;
          }
          const input=document.createElement('input');
          const textarea=document.createElement('textarea');textarea.value='a\ud83d\ude00';textarea.wrap='HARD';
          return !('textLength' in input)&&textarea.textLength===3&&!Object.hasOwn(textarea,'textLength')&&textarea.wrap==='HARD'&&textarea.getAttribute('wrap')==='HARD';
        })()"#).is_true());
    }
    #[test]
    fn control_string_attributes_use_native_reflection_and_reactions() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const [tag,names] of [['input',['accept','alt','placeholder','dirName']],['textarea',['placeholder','dirName']]]){
            const element=document.createElement(tag);
            for(const name of names){
              const attribute=name==='dirName'?'dirname':name;
              if(element[name]!==''||element.hasAttribute(attribute))return false;
              let conversions=0;element[name]={toString(){conversions++;return 'value\ud800'}};
              if(conversions!==1||element[name]!=='value\ud800'||element.getAttribute(attribute)!=='value\ud800'||Object.hasOwn(element,name))return false;
              element.setAttribute(attribute,'from attribute');if(element[name]!=='from attribute')return false;
              element[name]=null;if(element.getAttribute(attribute)!=='null')return false;
              element.removeAttribute(attribute);if(element[name]!=='')return false;
            }
          }
          const changes=[];
          customElements.define('x-reflected-input',class extends HTMLInputElement{
            static observedAttributes=['accept','alt','placeholder','dirname'];
            attributeChangedCallback(name,oldValue,newValue){changes.push([name,oldValue,newValue])}
          },{extends:'input'});
          const input=document.createElement('input',{is:'x-reflected-input'});
          input.accept='image/*';input.alt='image';input.placeholder='choose';input.dirName='direction';
          return changes.length===4&&changes.map(change=>change[0]).join()==='accept,alt,placeholder,dirname'&&changes.every(change=>change[1]===null);
        })()"#).is_true());
    }
    #[test]
    fn autocomplete_reflection_and_history_share_native_token_rules() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form autocomplete=off><input name=missing><input name=invalid autocomplete=unknown><input name=allowed autocomplete=email><input name=denied autocomplete=' OFF '></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          for(const tag of ['input','textarea','select']){
            const control=document.createElement(tag);
            if(control.autocomplete!=='')return false;
            const raw='  section-LOGIN  shipping work TEL webauthn ';
            control.autocomplete=raw;
            if(control.autocomplete!=='section-login shipping work tel webauthn'||control.getAttribute('autocomplete')!==raw)return false;
            control.autocomplete='home name';if(control.autocomplete!=='')return false;
          }
          const hidden=document.createElement('input');hidden.type='hidden';hidden.autocomplete='off';
          return hidden.autocomplete===''&&document.querySelector('form').autocomplete==='off';
        })()"#).is_true());
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "allowed");
        eval(
            scope,
            "document.querySelector('[name=allowed]').autocomplete='invalid'",
        );
        assert!(capture_form_states(scope, &state).unwrap().is_empty());
    }
    #[test]
    fn builtin_history_capture_accepts_exact_serialized_budget() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form id=f><input name=text value=hello><input name=check type=checkbox checked><textarea name=area>world</textarea><select name=choice multiple><option selected>a</option><option>b</option></select></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "document.querySelector('textarea').setSelectionRange(1,3,'backward');document.querySelector('select').options[1].selected=true",
        );
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 4);
        let bytes = crate::form_state::snapshot_bytes(&saved).unwrap();
        assert_eq!(
            capture_form_states_limited(scope, &state, bytes).unwrap(),
            saved
        );
        assert!(capture_form_states_limited(scope, &state, bytes - 1).is_err());
        assert_eq!(
            capture_form_states(scope, &state).unwrap(),
            saved,
            "failed capture does not change control state"
        );
    }
    #[test]
    fn builtin_select_history_preserves_clean_and_dirty_option_behavior() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form><select id=s name=choice multiple><option selected>a</option><option>b</option></select></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.select=document.getElementById('s');select.options[1].selected=true;select.options[1].selected=false",
        );
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 1);
        eval(
            scope,
            "select.options[0].selected=false;select.options[1].selected=true",
        );
        restore_saved_form_state(
            scope,
            &state,
            &saved[0],
            crate::form_state::FormRestoreTarget::Exact,
            "restore",
        )
        .unwrap();
        assert_eq!(capture_form_states(scope, &state).unwrap(), saved);
        assert!(eval(scope,r#"(()=>{
          const [clean,dirty]=select.options;
          if(!clean.selected||dirty.selected)return false;
          clean.removeAttribute('selected');dirty.setAttribute('selected','');
          return !clean.selected&&!dirty.selected&&clean.defaultSelected===false&&dirty.defaultSelected===true;
        })()"#).is_true());
    }
    #[test]
    fn invalid_saved_restore_mode_preserves_builtin_and_custom_state() {
        super::super::tests::initialize_v8();
        let dom=Rc::new(RefCell::new(oryn_dom::html::parse("<form><input id=builtin name=builtin value=before><x-mode-state name=custom id=custom></x-mode-state></form>").into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.restored=[];customElements.define('x-mode-state',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('submission','saved')}formStateRestoreCallback(value,mode){restored.push([value,mode])}})",
        );
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 2);
        eval(scope, "document.getElementById('builtin').value='current'");
        for entry in &saved {
            assert!(
                restore_saved_form_state(
                    scope,
                    &state,
                    entry,
                    crate::form_state::FormRestoreTarget::Exact,
                    "invalid"
                )
                .is_err()
            );
            let mut null = entry.clone();
            null.state = crate::form_state::FormState::Null;
            assert!(
                restore_saved_form_state(
                    scope,
                    &state,
                    &null,
                    crate::form_state::FormRestoreTarget::Exact,
                    "invalid"
                )
                .is_err()
            );
        }
        finish(scope, &state).unwrap();
        assert!(eval(scope,"restored.length===0&&document.getElementById('builtin').value==='current'&&new FormData(document.querySelector('form')).get('custom')==='submission'").is_true());
        for entry in &saved {
            restore_saved_form_state(
                scope,
                &state,
                entry,
                crate::form_state::FormRestoreTarget::Exact,
                "restore",
            )
            .unwrap();
        }
        assert!(eval(scope,"restored.length===1&&restored[0][0]==='saved'&&restored[0][1]==='restore'&&document.getElementById('builtin').value==='before'").is_true());
    }
    #[test]
    fn form_method_conversions_precede_form_association_checks() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
          customElements.define('x-not-form-associated',class extends HTMLElement{});
          const host=document.createElement('x-not-form-associated'),internals=host.attachInternals();
          const sentinel={},log=[];
          const fails=(invoke,check)=>{try{invoke();return false}catch(error){return check(error)}};
          const unsupported=error=>error.name==='NotSupportedError';
          if(!fails(()=>internals.setFormValue({toString(){throw sentinel}}),error=>error===sentinel))return false;
          if(!fails(()=>internals.setFormValue({toString(){log.push('value');return 'v'}},
            {toString(){log.push('state');return 's'}}),unsupported)||log.join()!=='value,state')return false;
          if(!fails(()=>internals.setValidity({get badInput(){throw sentinel}}),error=>error===sentinel))return false;
          log.length=0;
          if(!fails(()=>internals.setValidity({get badInput(){log.push('flags');return true}},
            {toString(){log.push('message');return 'm'}}),unsupported)||log.join()!=='flags,message')return false;
          if(!fails(()=>internals.setValidity({},'message',null),error=>error instanceof TypeError))return false;
          if(!fails(()=>internals.setValidity({badInput:true},''),unsupported))return false;
          if(!fails(()=>internals.setValidity({},'message',document.body),unsupported))return false;
          return fails(()=>internals.checkValidity(),unsupported)&&fails(()=>internals.reportValidity(),unsupported);
        })()"#).is_true());
    }
    #[test]
    fn undefined_optional_form_state_defaults_to_converted_submission() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form><x-optional-state name=field id=x></x-optional-state></form>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "customElements.define('x-optional-state',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals()}});globalThis.control=document.getElementById('x')",
        );
        for expression in [
            "'submitted'",
            "new File(['payload'],'name.txt',{type:'text/plain',lastModified:7})",
            "(()=>{const data=new FormData();data.append('one','first');data.append('one','second');return data})()",
        ] {
            eval(
                scope,
                &format!("globalThis.value={expression};control.i.setFormValue(value)"),
            );
            let omitted = capture_form_states(scope, &state).unwrap();
            assert_eq!(omitted.len(), 1);
            assert_ne!(omitted[0].state, crate::form_state::FormState::Null);
            eval(scope, "control.i.setFormValue(value,undefined)");
            assert_eq!(capture_form_states(scope, &state).unwrap(), omitted);
            eval(scope, "control.i.setFormValue(value,null)");
            assert_eq!(
                capture_form_states(scope, &state).unwrap()[0].state,
                crate::form_state::FormState::Null
            );
        }
        assert!(eval(scope,"globalThis.conversions=0;control.i.setFormValue({toString(){conversions++;return 'converted'}},undefined);conversions===1&&new FormData(document.querySelector('form')).get('field')==='converted'").is_true());
        assert_eq!(
            capture_form_states(scope, &state).unwrap()[0].state,
            crate::form_state::FormState::String("converted".into())
        );
        // The required first argument still converts undefined to nullable null.
        eval(scope, "control.i.setFormValue(undefined)");
        assert_eq!(
            capture_form_states(scope, &state).unwrap()[0].state,
            crate::form_state::FormState::Null
        );
        assert!(
            eval(
                scope,
                "!new FormData(document.querySelector('form')).has('field')"
            )
            .is_true()
        );
    }
    #[test]
    fn pending_history_value_replaces_live_snapshot_within_exact_byte_cap() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form><x-budget-state name=field id=x></x-budget-state></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "customElements.define('x-budget-state',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('live')}})",
        );
        let mut saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 1);
        saved[0].state = crate::form_state::FormState::String("x".repeat(1024));
        let bytes = crate::form_state::snapshot_bytes(&saved).unwrap();
        admit_form_restoration(&state, &saved).unwrap();
        let recaptured = capture_form_states_limited(scope, &state, bytes).unwrap();
        assert_eq!(recaptured, saved);
        assert!(capture_form_states_limited(scope, &state, bytes - 1).is_err());
        assert!(
            state
                .custom_elements
                .pending_restoration
                .borrow()
                .as_ref()
                .is_some_and(|(_, done)| done.is_empty())
        );
    }
    #[test]
    fn failed_form_restore_preparation_retains_snapshot_for_one_callback() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form><x-retry name=field id=x></x-retry></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.restorations=[];customElements.define('x-retry',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('value','saved')}formStateRestoreCallback(value,mode){restorations.push(value,mode)}})",
        );
        let saved = capture_form_states(scope, &state).unwrap();
        FAIL_NEXT_FORM_RESTORE.with(|fault| fault.set(true));
        assert!(queue_form_restoration(scope, &state, &saved).is_err());
        assert!(
            state
                .custom_elements
                .pending_restoration
                .borrow()
                .as_ref()
                .unwrap()
                .1
                .is_empty()
        );
        assert!(eval(scope, "restorations.length===0").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "restorations.join('|')==='saved|restore'").is_true());
        assert!(state.custom_elements.pending_restoration.borrow().is_none());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "restorations.length===2").is_true());
    }

    #[test]
    fn saved_state_is_distinct_from_submission_and_requires_exact_live_identity() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<form id=f><x-saved id=x name=x></x-saved></form>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.restored=[];customElements.define('x-saved',class extends HTMLElement{static formAssociated=true;constructor(){super();this.i=this.attachInternals();this.i.setFormValue('submitted','saved')}formStateRestoreCallback(value,mode){restored.push(value,mode)}})"#,
        );
        let saved = capture_form_states(scope, &state).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(
            saved[0].state,
            crate::form_state::FormState::String("saved".into())
        );
        assert_eq!(saved[0].occurrence, 0);
        assert_eq!(saved[0].form.as_ref().unwrap().id, "f");
        eval(
            scope,
            "document.getElementById('x').i.setFormValue('new submission','new state')",
        );
        restore_saved_form_state(
            scope,
            &state,
            &saved[0],
            crate::form_state::FormRestoreTarget::Exact,
            "restore",
        )
        .unwrap();
        assert!(eval(scope,"restored.join('|')==='saved|restore' && new FormData(document.getElementById('f')).get('x')==='new submission'").is_true());
        let mut stale = saved[0].clone();
        stale.node.generation = stale.node.generation.wrapping_add(1);
        assert!(
            restore_saved_form_state(
                scope,
                &state,
                &stale,
                crate::form_state::FormRestoreTarget::Exact,
                "restore"
            )
            .is_err()
        );
        state.retire_execution();
        assert!(
            restore_saved_form_state(
                scope,
                &state,
                &saved[0],
                crate::form_state::FormRestoreTarget::Exact,
                "restore"
            )
            .is_err()
        );
    }
}

fn saved_value(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    budget: &mut crate::form_state::FormSnapshotBudget,
) -> Result<crate::form_state::FormState, V8BindingError> {
    use crate::form_state::{FormEntryValue, FormFile, FormState};
    if value.is_null() {
        return Ok(FormState::Null);
    }
    if value.is_string() {
        let text = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| V8BindingError::Allocation("saved string"))?;
        let bytes = text
            .length()
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(32))
            .ok_or_else(snapshot_budget_error)?;
        budget
            .reserve_bytes(bytes)
            .map_err(|_| snapshot_budget_error())?;
        return Ok(FormState::String(rust_string_preserving_utf16(
            scope, text,
        )?));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| V8BindingError::Allocation("native saved form value"))?;
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("native saved form owner"))?;
    let id = stateful_id(scope, object)?;
    let record = platform_object(&owner, id)?;
    match &*record {
        PlatformObjectRecord::Blob(blob) => {
            let file = blob
                .file
                .as_ref()
                .ok_or(V8BindingError::Allocation("saved value requires File"))?;
            for bytes in [
                blob.bytes.len(),
                file.name.len(),
                blob.media_type.len(),
                std::mem::size_of::<FormFile>(),
            ] {
                budget
                    .reserve_bytes(bytes)
                    .map_err(|_| snapshot_budget_error())?;
            }
            Ok(FormState::File(FormFile {
                name: file.name.clone(),
                media_type: blob.media_type.clone(),
                last_modified: file.last_modified,
                bytes: blob.bytes.clone(),
            }))
        }
        PlatformObjectRecord::FormData(data) => {
            let bytes = data
                .entries
                .len()
                .checked_mul(std::mem::size_of::<(String, FormEntryValue)>())
                .ok_or_else(snapshot_budget_error)?;
            budget
                .reserve_bytes(bytes)
                .map_err(|_| snapshot_budget_error())?;
            for (name, _) in &data.entries {
                budget
                    .reserve_bytes(name.len())
                    .map_err(|_| snapshot_budget_error())?;
            }
            let entries = data.entries.clone();
            drop(record);
            let mut result = Vec::new();
            for (name, value) in entries {
                let value = v8::Local::new(scope, &value);
                let value = match saved_value(scope, value, budget)? {
                    FormState::String(value) => FormEntryValue::String(value),
                    FormState::File(file) => FormEntryValue::File(file),
                    _ => return Err(V8BindingError::Allocation("nested saved form value")),
                };
                result.push((name, value));
            }
            Ok(FormState::Entries(result))
        }
        _ => Err(V8BindingError::Allocation("unsupported saved form state")),
    }
}
fn restored_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    value: &crate::form_state::FormState,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    use crate::form_state::{FormEntryValue, FormState};
    match value {
        FormState::Null => Ok(v8::null(scope).into()),
        FormState::String(value) => v8_string(scope, value),
        FormState::File(file) => {
            let id = register_platform_object(
                state,
                PlatformObjectRecord::Blob(BlobRecord {
                    bytes: file.bytes.clone(),
                    media_type: file.media_type.clone(),
                    file: Some(FileRecord {
                        name: file.name.clone(),
                        last_modified: file.last_modified,
                    }),
                }),
            );
            Ok(new_platform_object(scope, state, "File", id)?.into())
        }
        FormState::Entries(entries) => {
            let mut output = Vec::new();
            for (name, value) in entries {
                let value = match value {
                    FormEntryValue::String(value) => FormState::String(value.clone()),
                    FormEntryValue::File(file) => FormState::File(file.clone()),
                };
                let value = restored_value(scope, state, &value)?;
                output.push((name.clone(), v8::Global::new(scope, value)));
            }
            let id = register_platform_object(
                state,
                PlatformObjectRecord::FormData(FormDataRecord { entries: output }),
            );
            Ok(new_platform_object(scope, state, "FormData", id)?.into())
        }
    }
}
pub(super) fn capture_form_states(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<Vec<crate::form_state::SavedFormControl>, V8BindingError> {
    capture_form_states_limited(scope, state, crate::form_state::MAX_FORM_SNAPSHOT_BYTES)
}
pub(super) fn capture_form_states_limited(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    max_bytes: usize,
) -> Result<Vec<crate::form_state::SavedFormControl>, V8BindingError> {
    let mut budget = crate::form_state::FormSnapshotBudget::default();
    budget
        .reserve_bytes(crate::form_state::MAX_FORM_SNAPSHOT_BYTES.saturating_sub(max_bytes))
        .map_err(|_| snapshot_budget_error())?;
    let mut captured = capture_form_descriptors_with_budget(scope, state, true, &mut budget)?;
    // Navigating away before a definition arrives must retain its outstanding
    // history value rather than overwrite it with an empty current snapshot.
    let pending = state.custom_elements.pending_restoration.borrow();
    if let Some((saved, consumed)) = pending.as_ref() {
        let live = capture_form_descriptors(scope, state, false)?;
        for (index, target) in crate::form_state::restoration_mapping(saved, &live) {
            if consumed.contains(&index) {
                continue;
            }
            let node = match target {
                crate::form_state::FormRestoreTarget::Exact => saved[index].node,
                crate::form_state::FormRestoreTarget::Mapped { node, .. } => node,
            };
            if let Some(descriptor) = live.iter().find(|entry| entry.node == node) {
                let retained = crate::form_state::snapshot_bytes(&captured)
                    .map_err(|_| snapshot_budget_error())?;
                let addition =
                    crate::form_state::snapshot_bytes(std::slice::from_ref(&saved[index]))
                        .map_err(|_| snapshot_budget_error())?;
                let replaced = captured
                    .iter()
                    .find(|entry| entry.node == node)
                    .map(|entry| crate::form_state::snapshot_bytes(std::slice::from_ref(entry)))
                    .transpose()
                    .map_err(|_| snapshot_budget_error())?
                    .unwrap_or(0);
                // An unconsumed history value replaces this live snapshot;
                // charging both would reject a final snapshot within its cap.
                if retained
                    .checked_sub(replaced)
                    .and_then(|bytes| bytes.checked_add(addition))
                    .is_none_or(|bytes| bytes > max_bytes)
                {
                    return Err(snapshot_budget_error());
                }
                let mut entry = descriptor.clone();
                entry.state = saved[index].state.clone();
                if let Some(current) = captured.iter_mut().find(|current| current.node == node) {
                    *current = entry;
                } else {
                    captured.push(entry);
                }
            }
        }
    }
    if crate::form_state::snapshot_bytes(&captured).map_err(|_| snapshot_budget_error())?
        > max_bytes
    {
        return Err(snapshot_budget_error());
    }
    Ok(captured)
}
pub(super) fn capture_form_descriptors(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    include_state: bool,
) -> Result<Vec<crate::form_state::SavedFormControl>, V8BindingError> {
    let mut budget = crate::form_state::FormSnapshotBudget::default();
    capture_form_descriptors_with_budget(scope, state, include_state, &mut budget)
}
fn capture_form_descriptors_with_budget(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    include_state: bool,
    budget: &mut crate::form_state::FormSnapshotBudget,
) -> Result<Vec<crate::form_state::SavedFormControl>, V8BindingError> {
    use crate::form_state::{FormOwnerIdentity, SavedFormControl};
    let mut saved = Vec::new();
    let mut occurrences = BTreeMap::new();
    let nodes = state
        .dom
        .borrow()
        .arena()
        .descendants_inclusive(state.document_root())?;
    for node in nodes {
        let registered = state.dom.borrow().is_form_associated_custom_element(node);
        let pending_candidate=!include_state&&state.custom_elements.pending_restoration.borrow().as_ref().is_some_and(|(saved,_)|state.dom.borrow().arena().get(node).ok().is_some_and(|record|matches!(&record.kind,NodeKind::Element{name} if name.ns.as_ref()=="http://www.w3.org/1999/xhtml"&&saved.iter().any(|entry|entry.definition==name.local.as_ref()))));
        if !registered && !pending_candidate {
            continue;
        }
        budget
            .reserve_control()
            .map_err(|_| snapshot_budget_error())?;
        {
            let dom = state.dom.borrow();
            if let NodeKind::Element { name } = &dom.arena().get(node)?.kind {
                budget
                    .reserve_bytes(name.local.len())
                    .map_err(|_| snapshot_budget_error())?;
            }
            budget
                .reserve_bytes(
                    dom.attribute_in_null_namespace(node, "name")?
                        .unwrap_or("")
                        .len(),
                )
                .map_err(|_| snapshot_budget_error())?;
            if let Some(form) = dom.form_owner(node)? {
                for name in ["id", "name", "action"] {
                    budget
                        .reserve_bytes(
                            dom.attribute_in_null_namespace(form, name)?
                                .unwrap_or("")
                                .len(),
                        )
                        .map_err(|_| snapshot_budget_error())?;
                }
            }
        }
        let restoration = if include_state {
            let owner = form_owner_state(scope, state, node)?;

            owner
                .custom_elements
                .elements
                .borrow()
                .get(&node)
                .and_then(|element| element.restoration.clone())
        } else {
            None
        };
        let (definition, name, form) = {
            let dom = state.dom.borrow();
            let NodeKind::Element { name: tag } = &dom.arena().get(node)?.kind else {
                continue;
            };
            let form = dom
                .form_owner(node)?
                .map(|form| {
                    Ok::<_, DomError>(FormOwnerIdentity {
                        node: form,
                        id: dom
                            .attribute_in_null_namespace(form, "id")?
                            .unwrap_or("")
                            .into(),
                        name: dom
                            .attribute_in_null_namespace(form, "name")?
                            .unwrap_or("")
                            .into(),
                        action: dom
                            .attribute_in_null_namespace(form, "action")?
                            .unwrap_or("")
                            .into(),
                    })
                })
                .transpose()?;
            (
                tag.local.to_string(),
                dom.attribute_in_null_namespace(node, "name")?
                    .unwrap_or("")
                    .to_owned(),
                form,
            )
        };
        let ordinal = occurrences
            .entry((
                definition.clone(),
                name.clone(),
                form.as_ref().map(|form| form.node),
            ))
            .or_insert(0usize);
        let occurrence = *ordinal;
        *ordinal += 1;
        let restoration = if let Some(value) = restoration {
            let value = v8::Local::new(scope, &value);
            saved_value(scope, value, budget)?
        } else {
            crate::form_state::FormState::Null
        };
        saved.push(SavedFormControl {
            node,
            document: state.document_root(),
            definition,
            name,
            form,
            occurrence,
            state: restoration,
        });
    }
    saved.extend(super::builtin_form_history::capture(
        state,
        include_state,
        budget,
    )?);
    Ok(saved)
}
pub(super) fn restore_saved_form_state(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    saved: &crate::form_state::SavedFormControl,
    target: crate::form_state::FormRestoreTarget,
    mode: &str,
) -> Result<(), V8BindingError> {
    use crate::form_state::FormRestoreTarget;
    // Validate the host command before built-in mutation or the null-state
    // early return, just as for direct custom-control restoration.
    if !matches!(mode, "restore" | "autocomplete") {
        return Err(V8BindingError::Unsupported {
            interface: "ElementInternals".into(),
            member: "invalid restoration mode".into(),
        });
    }
    if saved.definition.starts_with("#builtin/") {
        return super::builtin_form_history::restore(state, saved, target);
    }
    let (node, document) = match target {
        FormRestoreTarget::Exact => (saved.node, saved.document),
        FormRestoreTarget::Mapped { node, document } => (node, document),
    };
    let invalid = || V8BindingError::Unsupported {
        interface: "ElementInternals".into(),
        member: "InvalidStateError".into(),
    };
    if state.execution_retired() || document != state.document_root() {
        return Err(invalid());
    }
    {
        let dom = state.dom.borrow();
        if dom.arena().node_document(node)? != Some(document)
            || !dom.is_document_connected(node)?
            || !matches!(&dom.arena().get(node)?.kind,NodeKind::Element{name} if name.local.as_ref()==saved.definition)
            || dom.attribute_in_null_namespace(node, "name")?.unwrap_or("") != saved.name
        {
            return Err(invalid());
        }
    }
    if matches!(saved.state, crate::form_state::FormState::Null) {
        return Ok(());
    }
    let value = restored_value(scope, state, &saved.state)?;
    restore_form_state(scope, state, node, value, mode)
}

pub(super) fn validation_anchor(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<NodeId, V8BindingError> {
    if !state.dom.borrow().is_form_associated_custom_element(node) {
        return Ok(node);
    }
    let host = wrap_node(scope, state, node)?;
    let key = v8::String::new(scope, "oryn.custom.validation.anchor")
        .ok_or(V8BindingError::Allocation("validation anchor"))?;
    let key = v8::Private::for_api(scope, Some(key));
    let value = host
        .get_private(scope, key)
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_undefined() {
        return Ok(node);
    }
    let Some((anchor_owner, anchor)) = node_identity::argument(scope, value)? else {
        return Ok(node);
    };
    // The originally accepted anchor can subsequently be adopted elsewhere.
    // It is no longer a descendant, so report against the host instead of
    // interpreting its destination-local ID inside the host's arena.
    if !Rc::ptr_eq(&anchor_owner.dom, &state.dom) {
        return Ok(node);
    }
    let dom = state.dom.borrow();
    let mut cursor = Some(anchor);
    while let Some(current) = cursor {
        if current == node {
            return Ok(anchor);
        }
        cursor = dom.parent(current)?;
    }
    Ok(node)
}

fn snapshot_budget_error() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "ElementInternals".into(),
        member: "QuotaExceededError: form snapshot budget".into(),
    }
}

#[cfg(test)]
mod form_snapshot_fidelity_tests {
    use super::*;
    #[test]
    fn saved_native_string_preserves_utf16_and_budget_rejects_before_copy() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p></p>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let text =
            v8::String::new_from_two_byte(scope, &[0xd800, 97, 0xdc00], v8::NewStringType::Normal)
                .unwrap();
        let mut budget = crate::form_state::FormSnapshotBudget::default();
        let saved = saved_value(scope, text.into(), &mut budget).unwrap();
        let crate::form_state::FormState::String(value) = &saved else {
            panic!("string state")
        };
        assert_eq!(oryn_dom::dom_string::units(value), vec![0xd800, 97, 0xdc00]);
        let restored = restored_value(scope, &state, &saved).unwrap();
        let restored = v8::Local::<v8::String>::try_from(restored).unwrap();
        let mut units = vec![0; restored.length()];
        restored.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
        assert_eq!(units, vec![0xd800, 97, 0xdc00]);
        let mut budget = crate::form_state::FormSnapshotBudget::default();
        budget
            .reserve_bytes(crate::form_state::MAX_FORM_SNAPSHOT_BYTES)
            .unwrap();
        assert!(saved_value(scope, text.into(), &mut budget).is_err());
        let (_, normalized) = form_value(scope, &state, text.into()).unwrap();
        let normalized = v8::Local::new(scope, &normalized);
        assert_eq!(normalized.to_rust_string_lossy(scope), "\u{fffd}a\u{fffd}");
    }
}

/// Native history admission retains unmatched controls until their definition
/// upgrades. The bounded snapshot remains scoped to this document's lifetime.
pub(super) fn queue_form_restoration(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    saved: &[crate::form_state::SavedFormControl],
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired form restoration"));
    }
    crate::form_state::snapshot_bytes(saved).map_err(|_| snapshot_budget_error())?;
    admit_form_restoration(state, saved)?;
    finish(scope, state)
}
pub(super) fn admit_form_restoration(
    state: &Rc<V8BindingState>,
    saved: &[crate::form_state::SavedFormControl],
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired form restoration"));
    }
    crate::form_state::snapshot_bytes(saved).map_err(|_| snapshot_budget_error())?;
    *state.custom_elements.pending_restoration.borrow_mut() =
        Some((saved.to_vec().into(), BTreeSet::new()));
    Ok(())
}
#[cfg(test)]
std::thread_local! {static FAIL_NEXT_FORM_RESTORE:Cell<bool>=const{Cell::new(false)};}

fn flush_pending_restoration(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        state
            .custom_elements
            .pending_restoration
            .borrow_mut()
            .take();
        return Ok(());
    }
    let Some((saved, consumed)) = state.custom_elements.pending_restoration.borrow().clone() else {
        return Ok(());
    };
    let live = capture_form_descriptors(scope, state, false)?;
    for (index, target) in crate::form_state::restoration_mapping(&saved, &live) {
        if consumed.contains(&index) {
            continue;
        }
        let node = match target {
            crate::form_state::FormRestoreTarget::Exact => saved[index].node,
            crate::form_state::FormRestoreTarget::Mapped { node, .. } => node,
        };
        let ready = saved[index].definition.starts_with("#builtin/")
            || state
                .custom_elements
                .elements
                .borrow()
                .get(&node)
                .is_some_and(|element| {
                    !element.failed && !element.upgrading && element.definition.form_associated
                });
        if !ready {
            continue;
        }
        // Consume before invoking/queuing callbacks so reentrant checkpoints
        // cannot restore the same entry twice.
        let admitted = {
            let mut pending = state.custom_elements.pending_restoration.borrow_mut();
            pending.as_mut().is_some_and(|(current, done)| {
                std::sync::Arc::ptr_eq(current, &saved) && done.insert(index)
            })
        };
        if admitted {
            #[cfg(test)]
            let failed_preparation = FAIL_NEXT_FORM_RESTORE.with(|fault| fault.replace(false));
            #[cfg(not(test))]
            let failed_preparation = false;
            let result = if failed_preparation {
                Err(V8BindingError::Allocation(
                    "injected form restore preparation failure",
                ))
            } else {
                restore_saved_form_state(scope, state, &saved[index], target, "restore")
            };
            if let Err(error) = result {
                // Agent collection queues custom callbacks without invoking
                // them; failure precedes callback admission. Restore retry
                // custody only in the same snapshot, never a replacement.
                if let Some((current, done)) = state
                    .custom_elements
                    .pending_restoration
                    .borrow_mut()
                    .as_mut()
                    && std::sync::Arc::ptr_eq(current, &saved)
                {
                    done.remove(&index);
                }
                return Err(error);
            }
            let null = matches!(saved[index].state, crate::form_state::FormState::Null);
            if !null && !saved[index].definition.starts_with("#builtin/") {
                break;
            }
        }
    }
    let mut pending = state.custom_elements.pending_restoration.borrow_mut();
    if pending
        .as_ref()
        .is_some_and(|(saved, done)| saved.len() == done.len())
    {
        pending.take();
    }
    Ok(())
}
pub(super) fn clear_form_restoration(state: &V8BindingState) {
    state
        .custom_elements
        .pending_restoration
        .borrow_mut()
        .take();
}
/// Release native roots/environment custody even when the retired Document has
/// no active sibling left to run a later agent checkpoint.
pub(super) fn retire(state: &V8BindingState) {
    for target in state
        .custom_elements
        .queued_targets
        .borrow()
        .values()
        .filter_map(RcWeak::upgrade)
    {
        target.owner.borrow_mut().take();
        target.lease.borrow_mut().take();
    }
    state.custom_elements.queued_targets.borrow_mut().clear();
    state.custom_elements.reactions.borrow_mut().clear();
    state.custom_elements.parser_spans.borrow_mut().clear();
    state.custom_elements.pending_upgrades.borrow_mut().clear();
    state.custom_elements.initial_reactions.borrow_mut().clear();
    clear_form_restoration(state);
}

#[cfg(test)]
thread_local! { static FAIL_MUTATION_AFTER:Cell<Option<usize>>=const{Cell::new(None)}; static FAIL_INITIAL_REACTION:Cell<bool>=const{Cell::new(false)}; static FAIL_ENQUEUE_AFTER:Cell<Option<usize>>=const{Cell::new(None)}; static FAIL_DEFINITION_AFTER:Cell<Option<usize>>=const{Cell::new(None)}; static FAIL_DEFINITION_GRAPH:Cell<bool>=const{Cell::new(false)}; static FAIL_INITIALIZE_AFTER:Cell<Option<usize>>=const{Cell::new(None)}; static FAIL_UPGRADE_CONSTRUCTOR:Cell<bool>=const{Cell::new(false)}; static FAIL_SYNCHRONOUS_REPORT:Cell<bool>=const{Cell::new(false)}; }
#[cfg(test)]
fn mutation_conversion_fault() -> Result<(), V8BindingError> {
    let fail = FAIL_MUTATION_AFTER.with(|slot| match slot.get() {
        Some(0) => {
            slot.set(None);
            true
        }
        Some(n) => {
            slot.set(Some(n - 1));
            false
        }
        None => false,
    });
    if fail {
        Err(V8BindingError::Allocation(
            "injected reaction conversion failure",
        ))
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod reaction_custody_tests {
    use super::*;
    #[test]
    fn termination_preserves_unstarted_reaction_suffix_without_replaying_prefix() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-stop id=x></x-stop>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let callback = v8::Function::new(scope, halt).unwrap();
        let name = v8::String::new(scope, "halt").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.log=[];customElements.define('x-stop',class extends HTMLElement{static observedAttributes=['a','b'];attributeChangedCallback(name){log.push(name);if(name==='a')halt()}})",
        );
        let x = dom.borrow().find_by_id("x").unwrap();
        dom.borrow_mut().set_attribute(x, "a", "1").unwrap();
        dom.borrow_mut().set_attribute(x, "b", "2").unwrap();
        scope.terminate_execution();
        assert!(finish(scope, &state).is_err());
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        assert!(
            eval(scope, "log.length===0").is_true(),
            "pending interrupt cannot consume the first callback"
        );
        assert!(finish(scope, &state).is_err());
        assert!(scope.is_execution_terminating());
        assert_eq!(
            state
                .custom_elements
                .reactions
                .borrow()
                .get(&x)
                .map_or(0, VecDeque::len),
            1,
            "unstarted callback remains in native custody"
        );
        scope.cancel_terminate_execution();
        assert!(eval(scope, "log.join()==='a'").is_true());
        finish(scope, &state).unwrap();
        assert_eq!(
            eval(scope, "log.join()").to_rust_string_lossy(scope),
            "a,b",
            "resumed reaction order"
        );
        finish(scope, &state).unwrap();
        assert_eq!(
            eval(scope, "log.join()").to_rust_string_lossy(scope),
            "a,b",
            "resumed reaction order"
        );
    }

    #[test]
    fn error_listener_termination_preserves_reaction_suffix_without_replaying_report() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-report-stop id=x></x-report-stop>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        fn halt(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope.terminate_execution();
        }
        let callback = v8::Function::new(scope, halt).unwrap();
        let name = v8::String::new(scope, "halt").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), callback.into());
        eval(
            scope,
            "globalThis.log=[];globalThis.reports=0;window.onerror=()=>{reports++;halt();return true};customElements.define('x-report-stop',class extends HTMLElement{static observedAttributes=['a','b'];attributeChangedCallback(name){log.push(name);if(name==='a')throw Error('ordinary callback failure')}})",
        );
        let x = dom.borrow().find_by_id("x").unwrap();
        dom.borrow_mut().set_attribute(x, "a", "1").unwrap();
        dom.borrow_mut().set_attribute(x, "b", "2").unwrap();
        assert!(finish(scope, &state).is_err());
        assert!(scope.is_execution_terminating());
        assert_eq!(
            state
                .custom_elements
                .reactions
                .borrow()
                .get(&x)
                .map_or(0, VecDeque::len),
            1,
            "unstarted callback remains in native custody"
        );
        scope.cancel_terminate_execution();
        assert!(eval(scope, "log.join()==='a'&&reports===1").is_true());
        finish(scope, &state).unwrap();
        assert_eq!(
            eval(scope, "log.join()+'|'+reports").to_rust_string_lossy(scope),
            "a,b|1",
            "resumed reaction/report order"
        );
        finish(scope, &state).unwrap();
        assert_eq!(
            eval(scope, "log.join()+'|'+reports").to_rust_string_lossy(scope),
            "a,b|1",
            "resumed reaction/report order"
        );
    }

    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn form_association_failure_restores_both_metadata_and_reaction_prefix() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse(
                "<form id=a></form><form id=b></form><x-field id=x form=a></x-field>",
            )
            .into_runtime()
            .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.log=[];customElements.define('x-field',class extends HTMLElement{static formAssociated=true;constructor(){super();this.attachInternals()}formAssociatedCallback(form){log.push('form:'+form.id)}formDisabledCallback(value){log.push('disabled:'+value)}})",
        );
        finish(scope, &state).unwrap();
        eval(scope, "log.length=0");
        let cursor = state.custom_elements.cursor.get();
        state.custom_elements.processing.set(true);
        eval(
            scope,
            "document.getElementById('x').setAttribute('form','b');document.getElementById('x').setAttribute('disabled','')",
        );
        state.custom_elements.processing.set(false);
        FAIL_ENQUEUE_AFTER.with(|slot| slot.set(Some(1)));
        assert!(finish(scope, &state).is_err());
        assert_eq!(state.custom_elements.cursor.get(), cursor);
        assert!(eval(scope, "log.length===0").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.join(',')==='form:b,disabled:true'").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.length===2").is_true());
    }
    #[test]
    fn failed_mutation_conversion_retains_cursor_and_does_not_duplicate_callbacks() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-log id=x></x-log>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.log=[];customElements.define('x-log',class extends HTMLElement{static observedAttributes=['a','b'];attributeChangedCallback(n,o,v){log.push(n+':'+v)}})",
        );
        finish(scope, &state).unwrap();
        let cursor = state.custom_elements.cursor.get();
        state.custom_elements.processing.set(true);
        eval(
            scope,
            "document.getElementById('x').setAttribute('a','1');document.getElementById('x').setAttribute('b','2')",
        );
        state.custom_elements.processing.set(false);
        FAIL_MUTATION_AFTER.with(|slot| slot.set(Some(0)));
        assert!(finish(scope, &state).is_err());
        assert_eq!(state.custom_elements.cursor.get(), cursor);
        assert!(eval(scope, "log.length===0").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "JSON.stringify(log)==='[\"a:1\",\"b:2\"]'").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.length===2").is_true());
    }
    #[test]
    fn initial_reaction_failure_retries_without_running_constructor_twice() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-log id=x a=one></x-log>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.custom_elements.processing.set(true);
        eval(
            scope,
            "globalThis.constructed=0;globalThis.log=[];customElements.define('x-log',class extends HTMLElement{static observedAttributes=['a'];constructor(){super();constructed++}attributeChangedCallback(){log.push('attribute')}connectedCallback(){log.push('connected')}})",
        );
        state.custom_elements.processing.set(false);
        FAIL_INITIAL_REACTION.with(|flag| flag.set(true));
        assert!(finish(scope, &state).is_err());
        assert!(eval(scope, "constructed===1&&log.length===0").is_true());
        assert_eq!(state.custom_elements.initial_reactions.borrow().len(), 1);
        finish(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "constructed===1&&log.join(',')==='attribute,connected'"
            )
            .is_true()
        );
        assert!(state.custom_elements.initial_reactions.borrow().is_empty());
    }
    #[test]
    fn reaction_budget_failure_retains_undelivered_callback_without_replaying_prefix() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-log id=x></x-log>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=0;customElements.define('x-log',class extends HTMLElement{connectedCallback(){calls++}})",
        );
        finish(scope, &state).unwrap();
        eval(scope, "calls=0");
        let value = eval(scope, "document.getElementById('x')");
        let node = argument_node(scope, &state, value).unwrap();
        for _ in 0..1025 {
            enqueue(scope, &state, node, 0, &[]).unwrap();
        }
        assert!(finish(scope, &state).is_err());
        assert!(eval(scope, "calls===1024").is_true());
        assert!(state.custom_elements.has_pending(&state));
        finish(scope, &state).unwrap();
        assert!(eval(scope, "calls===1025").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "calls===1025").is_true());
    }
}

/// Move per-element definitions/reactions, preserving source constructors and
/// callback creation realms without registering those definitions by name in
/// the destination global registry.
pub(super) struct NodeMigration {
    source: Rc<V8BindingState>,
    entries: Vec<QueuedReaction>,
    queue: ElementQueue,
}
fn adopted_registry_snapshot(
    scope: &mut v8::PinScope,
    source: &V8BindingState,
    node: NodeId,
    nodes: &BTreeSet<NodeId>,
    global: Option<u64>,
    resolved: &mut BTreeMap<NodeId, Option<u64>>,
) -> Result<Option<u64>, V8BindingError> {
    let mut pending = vec![(node, false)];
    let mut visiting = BTreeSet::new();
    while let Some((current, expanded)) = pending.pop() {
        if resolved.contains_key(&current) {
            continue;
        }
        let (association, kind, parent, keep_null) = {
            let dom = source.dom.borrow();
            let record = dom.arena().get(current)?;
            (
                record.custom_element_registry.flatten(),
                record.kind.clone(),
                record.parent,
                record.custom_element_registry_keep_null,
            )
        };
        let scoped = association.is_some_and(|id| {
            registry_by_id(scope, id).is_some_and(|registry| registry.scoped.get())
        });
        let inherited_parent =
            if !scoped && association.is_none() && matches!(kind, NodeKind::Element { .. }) {
                parent.filter(|parent| {
                    nodes.contains(parent)
                        && source
                            .dom
                            .borrow()
                            .arena()
                            .get(*parent)
                            .is_ok_and(|record| !matches!(record.kind, NodeKind::DocumentFragment))
                })
            } else {
                None
            };
        if let Some(parent) = inherited_parent.filter(|parent| !resolved.contains_key(parent)) {
            if expanded || !visiting.insert(current) {
                return Err(V8BindingError::Allocation(
                    "cyclic adoption registry ancestry",
                ));
            }
            pending.push((current, true));
            pending.push((parent, false));
            continue;
        }
        let registry = if scoped {
            association
        } else {
            match kind {
                NodeKind::ShadowRoot { .. } if association.is_none() && keep_null => None,
                NodeKind::ShadowRoot { .. } => global,
                NodeKind::Element { .. } => {
                    if let Some(parent) = inherited_parent {
                        resolved[&parent].filter(|id| {
                            registry_by_id(scope, *id)
                                .is_some_and(|registry| !registry.scoped.get())
                        })
                    } else {
                        global
                    }
                }
                _ => association,
            }
        };
        visiting.remove(&current);
        resolved.insert(current, registry);
    }
    Ok(resolved[&node])
}

pub(super) fn capture_node_migration(
    scope: &mut v8::PinScope,
    source: &Rc<V8BindingState>,
    destination: &Rc<V8BindingState>,
    nodes: &BTreeSet<NodeId>,
    document: NodeId,
) -> Result<NodeMigration, V8BindingError> {
    let new_document = wrap_node(scope, destination, document)?;
    let mut entries = Vec::new();
    let destination_registry = effective_global_registry(scope, destination, document)?;
    let mut resolved_registries = BTreeMap::new();
    let queue = admission_queue(&reaction_agent(scope));
    // Admission reserves destination element-queue space before native identity
    // commits. Publication then updates queued references without author code.
    queue
        .borrow_mut()
        .try_reserve(nodes.len())
        .map_err(|_| V8BindingError::Allocation("adopted reaction element queue"))?;
    entries
        .try_reserve(nodes.len())
        .map_err(|_| V8BindingError::Allocation("adopted reaction custody"))?;
    for node in nodes {
        let element = source.custom_elements.elements.borrow().get(node).cloned();
        let reaction = if let Some(element) = element {
            if let Some(callback) = element.definition.callbacks[2]
                .clone()
                .filter(|_| !element.failed)
            {
                let old_document = wrap_node(scope, source, element.document)?;
                let target = wrap_node(scope, source, *node)?;
                let callback = callback
                    .local(scope)
                    .and_then(|value| value.try_cast::<v8::Function>().ok())
                    .ok_or(V8BindingError::Allocation("adopted callback graph"))?;
                Some(Reaction::Callback {
                    parser_document: None,
                    target: v8::Global::new(scope, target),
                    callback: v8::Global::new(scope, callback),
                    arguments: vec![
                        v8::Global::new(scope, v8::Local::<v8::Value>::from(old_document)),
                        v8::Global::new(scope, v8::Local::<v8::Value>::from(new_document)),
                    ],
                })
            } else {
                None
            }
        } else {
            None
        };
        let registry = adopted_registry_snapshot(
            scope,
            source,
            *node,
            nodes,
            destination_registry,
            &mut resolved_registries,
        )?;
        entries.push((*node, reaction, reaction_target(source, *node)?, registry));
    }
    Ok(NodeMigration {
        source: source.clone(),
        entries,
        queue,
    })
}
impl NodeMigration {
    pub(super) fn publish(
        &mut self,
        destination: &Rc<V8BindingState>,
        nodes: &BTreeMap<NodeId, NodeId>,
        document: NodeId,
    ) {
        for (old, adopted, target, registry) in self.entries.drain(..) {
            let new = nodes[&old];
            destination
                .dom
                .borrow_mut()
                .set_custom_element_registry(new, registry)
                .expect("live migration registry document");
            if let Some(mut element) = self
                .source
                .custom_elements
                .elements
                .borrow_mut()
                .remove(&old)
            {
                element.document = document;
                element.form_owner = element
                    .form_owner
                    .and_then(|owner| nodes.get(&owner).copied());
                destination
                    .custom_elements
                    .elements
                    .borrow_mut()
                    .insert(new, element);
            }
            if let Some(reactions) = self
                .source
                .custom_elements
                .reactions
                .borrow_mut()
                .remove(&old)
            {
                destination
                    .custom_elements
                    .reactions
                    .borrow_mut()
                    .entry(new)
                    .or_default()
                    .extend(reactions);
            }
            self.source
                .custom_elements
                .queued_targets
                .borrow_mut()
                .remove(&old);
            *target.owner.borrow_mut() = Some(destination.clone());
            target.node.set(new);
            destination
                .custom_elements
                .queued_targets
                .borrow_mut()
                .insert(new, Rc::downgrade(&target));
            if let Some(reaction) = adopted {
                destination
                    .custom_elements
                    .reactions
                    .borrow_mut()
                    .entry(new)
                    .or_default()
                    .push_back(reaction);
                self.queue.borrow_mut().push_back(target);
            }
            if self
                .source
                .custom_elements
                .pending_upgrades
                .borrow_mut()
                .remove(&old)
            {
                destination
                    .custom_elements
                    .pending_upgrades
                    .borrow_mut()
                    .insert(new);
            }
            if let Some(initial) = self
                .source
                .custom_elements
                .initial_reactions
                .borrow_mut()
                .remove(&old)
            {
                destination
                    .custom_elements
                    .initial_reactions
                    .borrow_mut()
                    .insert(new, initial);
            }
        }
    }
}

#[cfg(test)]
mod agent_reaction_order_regressions {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    fn halt(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
        scope.terminate_execution();
    }
    fn install(
        scope: &mut v8::PinScope<'_, '_, ()>,
        context: v8::Local<v8::Context>,
        realm: RealmId,
        log: &v8::Global<v8::Value>,
        label: &str,
    ) -> Rc<V8BindingState> {
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<x-order id=one></x-order><x-order id=two></x-order>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, realm, 1).unwrap();
        let key = v8::String::new(scope, "log").unwrap();
        let value = v8::Local::new(scope, log);
        context.global(scope).set(scope, key.into(), value);
        let key = v8::String::new(scope, "halt").unwrap();
        let value = v8::Function::new(scope, halt).unwrap();
        context.global(scope).set(scope, key.into(), value.into());
        eval(
            scope,
            &format!(
                "globalThis.label='{label}';globalThis.stopAfter=false;customElements.define('x-order',class extends HTMLElement{{static observedAttributes=['data-v'];attributeChangedCallback(){{log.push(label+this.id+'attr')}}connectedCallback(){{log.push(label+this.id);if(stopAfter){{stopAfter=false;halt()}}}}adoptedCallback(){{log.push(label+'adopted')}}}})"
            ),
        );
        scope.perform_microtask_checkpoint();
        state
    }
    fn node(state: &V8BindingState, id: &str) -> NodeId {
        state.dom.borrow().find_by_id(id).unwrap()
    }
    #[test]
    fn backup_queue_preserves_interleaved_document_element_order() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, first);
        eval(scope, "log.length=0");
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        enqueue(scope, &a, node(&a, "one"), 0, &[]).unwrap();
        enqueue(scope, &b, node(&b, "two"), 0, &[]).unwrap();
        defer_checkpoint(scope);
        assert!(eval(scope, "log.length===0").is_true());
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='Bone,Aone,Btwo'").is_true());
        finish(scope, &a).unwrap();
        assert!(eval(scope, "log.length===3").is_true());
    }
    #[test]
    fn retired_backup_owner_cannot_block_or_clear_replacement_microtask() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, second);
        eval(scope, "log.length=0");
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        defer_checkpoint(scope);
        let agent = reaction_agent(scope);
        let serial = agent.backup_serial.get();
        let owner = agent
            .backup_owner
            .borrow()
            .as_ref()
            .unwrap()
            .0
            .upgrade()
            .unwrap();
        assert!(Rc::ptr_eq(&owner, &a));
        a.retire_execution();
        defer_checkpoint(scope);
        assert!(agent.backup_serial.get() > serial);
        // Both callbacks remain in this test's queue; the superseded one must
        // neither drain nor clear the replacement callback's ownership.
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='Bone'").is_true());
        assert!(!agent.backup_queued.get());
    }
    #[test]
    fn parked_document_does_not_capture_active_backup_or_run_its_callbacks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, second);
        eval(scope, "log.length=0");
        a.set_execution_suspended(true);
        enqueue(scope, &a, node(&a, "one"), 0, &[]).unwrap();
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        defer_checkpoint(scope);
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='Bone'").is_true());
        let agent = reaction_agent(scope);
        assert_eq!(agent.suspended.borrow().len(), 1);
        assert!(!agent.backup_queued.get());
        defer_checkpoint(scope);
        assert!(
            !agent.backup_queued.get(),
            "parked-only custody must not spin microtasks"
        );
        a.set_execution_suspended(false);
        a.defer_custom_element_reactions(scope);
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "log.join()==='Bone,Aone'").is_true());
        assert!(agent.suspended.borrow().is_empty());
        a.set_execution_suspended(true);
        enqueue(scope, &a, node(&a, "two"), 0, &[]).unwrap();
        finish(scope, &b).unwrap();
        let queued = agent.suspended.borrow().front().unwrap().clone();
        assert!(queued.lease.borrow().is_some());
        a.retire_execution();
        assert!(queued.lease.borrow().is_none());
        assert!(queued.owner.borrow().is_none());
        finish(scope, &b).unwrap();
        assert!(eval(scope, "log.length===2").is_true());
    }
    #[test]
    fn conversion_failure_keeps_original_scoped_queue_ahead_of_backup() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, first);
        eval(scope, "log.length=0");
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        let boundary = ReactionBoundary::begin(scope).unwrap();
        let changed = node(&a, "one");
        a.dom
            .borrow_mut()
            .set_attribute(changed, "data-v", "value")
            .unwrap();
        FAIL_MUTATION_AFTER.with(|slot| slot.set(Some(0)));
        assert!(collect_agent(scope).is_err());
        drop(boundary);
        enqueue(scope, &b, node(&b, "two"), 0, &[]).unwrap();
        assert!(eval(scope, "log.length===0").is_true());
        finish(scope, &b).unwrap();
        assert!(eval(scope, "log.join()==='Aoneattr,Bone,Btwo'").is_true());
        finish(scope, &a).unwrap();
        assert!(eval(scope, "log.length===3").is_true());
    }
    #[test]
    fn termination_retains_cross_document_suffix_and_never_replays_started_callback() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, first);
        eval(scope, "log.length=0;stopAfter=true");
        enqueue(scope, &a, node(&a, "one"), 0, &[]).unwrap();
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        enqueue(scope, &a, node(&a, "two"), 0, &[]).unwrap();
        assert!(finish(scope, &a).is_err());
        assert!(scope.is_execution_terminating());
        scope.cancel_terminate_execution();
        assert!(eval(scope, "log.join()==='Aone'").is_true());
        finish(scope, &b).unwrap();
        assert_eq!(
            eval(scope, "log.join()").to_rust_string_lossy(scope),
            "Aone,Bone,Atwo",
            "resumed cross-document reaction order"
        );
        finish(scope, &a).unwrap();
        assert!(eval(scope, "log.length===3").is_true());
    }
    #[test]
    fn retirement_during_retained_suffix_releases_only_that_document_work() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = install(scope, second, RealmId(1), &log, "B");
        let scope = &mut v8::ContextScope::new(scope, first);
        eval(scope, "log.length=0;stopAfter=true");
        enqueue(scope, &a, node(&a, "one"), 0, &[]).unwrap();
        enqueue(scope, &b, node(&b, "one"), 0, &[]).unwrap();
        enqueue(scope, &a, node(&a, "two"), 0, &[]).unwrap();
        assert!(finish(scope, &a).is_err());
        scope.cancel_terminate_execution();
        b.retire_execution();
        finish(scope, &a).unwrap();
        assert!(eval(scope, "log.join()==='Aone,Atwo'").is_true());
        assert!(b.custom_elements.reactions.borrow().is_empty());
        assert!(reaction_agent(scope).backup.borrow().is_empty());
    }
    #[test]
    fn adoption_moves_queued_element_without_reordering_or_destination_definition() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "reaction-agent").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let log = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let value = eval(scope, "[]");
            v8::Global::new(scope, value)
        };
        let a = install(scope, first, RealmId(0), &log, "A");
        let b = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, second, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        eval(scope, "log.length=0");
        let target = wrap_node(scope, &a, node(&a, "one")).unwrap();
        let key = v8::String::new(scope, "moving").unwrap();
        first.global(scope).set(scope, key.into(), target.into());
        let destination = wrap_node(scope, &b, b.document_root()).unwrap();
        let key = v8::String::new(scope, "destination").unwrap();
        first
            .global(scope)
            .set(scope, key.into(), destination.into());
        enqueue(scope, &a, node(&a, "one"), 0, &[]).unwrap();
        enqueue(scope, &a, node(&a, "two"), 0, &[]).unwrap();
        // Adoption adds this same element to its scoped queue. Its shared pending
        // reaction is consumed there, followed by adoptedCallback, before return.
        assert!(eval(scope,"destination.adoptNode(moving);moving.ownerDocument===destination&&log.join()==='Aone,Aadopted'").is_true());
        finish(scope, &a).unwrap();
        assert!(eval(scope, "log.join()==='Aone,Aadopted,Atwo'").is_true());
        assert!(b.custom_elements.definitions.borrow().is_empty());
        assert_eq!(b.custom_elements.elements.borrow().len(), 1);
    }
}

#[cfg(test)]
mod relaxed_registry_name_tests {
    use super::*;
    #[test]
    fn registry_names_and_customized_is_preserve_distinct_utf16_values() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=ok=>{if(!ok)throw Error('registry name identity')};
          globalThis.namePromises=[];
          for(const registry of [customElements,new CustomElementRegistry()]) {
            for(const name of ['x-!','x-:','x-<','x-\uffff','x-\u{10ffff}','x-\ud800','x-\ufffd']) {
              const C=class extends HTMLElement {};
              const pending=registry.whenDefined(name);
              registry.define(name,C);
              check(registry.get(name)===C&&registry.getName(C)===name);
              namePromises.push(pending.then(value=>check(value===C)));
              const element=document.createElement(name,{customElementRegistry:registry});
              check(element instanceof C&&element.localName===name);
            }
            for(const name of ['x-A','x-/','x->','x-\ud800/','x-\ud800A','font-face']) {
              let caught=false;try{registry.define(name,class extends HTMLElement{})}catch(e){caught=e.name==='SyntaxError'}check(caught);
              namePromises.push(registry.whenDefined(name).then(()=>{throw Error('invalid name resolved')},error=>check(error.name==='SyntaxError')));
            }
          }
          const surrogate='x-button-\ud800',replacement='x-button-\ufffd';
          const A=class extends HTMLButtonElement{},B=class extends HTMLButtonElement{};
          customElements.define(surrogate,A,{extends:'button'});customElements.define(replacement,B,{extends:'button'});
          check(document.createElement('button',{is:surrogate}) instanceof A);
          check(document.createElement('button',{is:replacement}) instanceof B);
          check(customElements.get(surrogate)===A&&customElements.get(replacement)===B);
          globalThis.namesDone=false;globalThis.namesFailed=false;
          Promise.all(namePromises).then(()=>namesDone=true,()=>namesFailed=true);
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        scope.perform_microtask_checkpoint();
        let code = v8::String::new(scope, "namesDone&&!namesFailed").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod initialize_admission_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn initialize_failure_restores_associations_and_upgrade_prefix_before_retry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.log=[];globalThis.r=new CustomElementRegistry();r.define('x-stage',class extends HTMLElement{constructor(){super();log.push(this.id)}});
          globalThis.root=document.createElement('div',{customElementRegistry:null});
          for(const id of ['first','second']){const e=document.createElement('x-stage',{customElementRegistry:null});e.id=id;root.append(e)}
          globalThis.nested=root.firstChild.attachShadow({mode:'open',customElementRegistry:null});
          nested.append(document.createElement('x-stage',{customElementRegistry:null}));"#,
        );
        finish(scope, &state).unwrap();
        let root = eval(scope, "root");
        let (_, root) = node_identity::argument(scope, root).unwrap().unwrap();
        let registry = eval(scope, "r").try_cast::<v8::Object>().unwrap();
        let registry = registry_receiver(scope, registry).unwrap();
        let before = state
            .dom
            .borrow()
            .custom_registry_documents(registry.id.get());
        FAIL_INITIALIZE_AFTER.with(|slot| slot.set(Some(2)));
        assert!(initialize_registry(scope, &state, &registry, root).is_err());
        assert_eq!(
            state
                .dom
                .borrow()
                .custom_registry_documents(registry.id.get()),
            before
        );
        assert!(eval(scope,"root.customElementRegistry===null&&root.firstChild.customElementRegistry===null&&root.lastChild.customElementRegistry===null&&nested.customElementRegistry===null&&log.length===0").is_true());
        assert!(state.custom_elements.pending_upgrades.borrow().is_empty());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.length===0").is_true());
        initialize_registry(scope, &state, &registry, root).unwrap();
        finish(scope, &state).unwrap();
        assert!(eval(scope,"root.customElementRegistry===r&&root.firstChild.customElementRegistry===r&&root.lastChild.customElementRegistry===r&&nested.customElementRegistry===null&&nested.firstChild.customElementRegistry===null&&log.join()==='first,second'").is_true());
        initialize_registry(scope, &state, &registry, root).unwrap();
        finish(scope, &state).unwrap();
        assert!(eval(scope, "log.join()==='first,second'").is_true());
    }
}

#[cfg(test)]
mod queued_definition_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn queued_upgrade_uses_accepted_definition_after_native_registry_reassociation() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.calls=[];globalThis.original=new CustomElementRegistry();globalThis.destination=new CustomElementRegistry();
          globalThis.candidate=document.createElement('x-owned',{customElementRegistry:original});
          globalThis.First=class extends HTMLElement{constructor(){super();calls.push('first')}};
          globalThis.Second=class extends HTMLElement{constructor(){super();calls.push('second')}};
          original.define('x-owned',First);destination.define('x-owned',Second);"#,
        );
        finish(scope, &state).unwrap();
        let value = eval(scope, "candidate");
        let (_, node) = node_identity::argument(scope, value).unwrap().unwrap();
        enqueue_upgrade(scope, &state, node, true).unwrap();
        let registry = eval(scope, "destination").try_cast::<v8::Object>().unwrap();
        let registry = registry_receiver(scope, registry).unwrap();
        // This is the same native association update used by adoption. The
        // already-enqueued reaction must not perform lookup a second time.
        associate_registry(&state, node, Some(registry.id.get())).unwrap();
        let retained = state
            .custom_elements
            .reactions
            .borrow()
            .get(&node)
            .unwrap()
            .front()
            .unwrap()
            .clone();
        match retained {
            Reaction::Upgrade {
                definition,
                registry_root,
            } => {
                assert_eq!(definition.name, "x-owned");
                let object = v8::Local::new(scope, &registry_root);
                let accepted = registry_receiver(scope, object).unwrap();
                assert_ne!(accepted.id.get(), registry.id.get());
            }
            _ => panic!("upgrade reaction"),
        }
        finish(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "candidate instanceof First&&!(candidate instanceof Second)&&calls.join()==='first'"
            )
            .is_true()
        );
        enqueue_upgrade(scope, &state, node, true).unwrap();
        finish(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='first'").is_true());
    }
}

#[cfg(test)]
mod attribute_local_name_tests {
    use super::*;
    #[test]
    fn reactions_use_captured_local_name_without_resplitting_colons() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=ok=>{if(!ok)throw Error('attribute local name')};
          const calls=[];class C extends HTMLElement{static observedAttributes=['a:b','local:tail'];attributeChangedCallback(...args){calls.push(args)}};
          const candidate=document.createElement('x-colon-attribute');candidate.setAttribute('a:b','initial');
          candidate.setAttributeNS('urn:attribute','prefix:local:tail','namespaced');
          customElements.define('x-colon-attribute',C);customElements.upgrade(candidate);
          check(JSON.stringify(calls)===JSON.stringify([['a:b',null,'initial',null],['local:tail',null,'namespaced','urn:attribute']]));calls.length=0;
          candidate.setAttribute('a:b','updated');candidate.removeAttribute('a:b');
          candidate.setAttributeNS('urn:attribute','next:local:tail','updated');candidate.removeAttributeNS('urn:attribute','local:tail');
          check(JSON.stringify(calls)===JSON.stringify([['a:b','initial','updated',null],['a:b','updated',null,null],['local:tail','namespaced','updated','urn:attribute'],['local:tail','updated',null,'urn:attribute']]));calls.length=0;
          const attr=document.createAttribute('a:b');attr.value='attached';candidate.setAttributeNode(attr);attr.value='changed';candidate.removeAttributeNode(attr);
          check(JSON.stringify(calls)===JSON.stringify([['a:b',null,'attached',null],['a:b','attached','changed',null],['a:b','changed',null,null]]));
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod fallback_name_tests {
    use super::*;
    #[test]
    fn failed_constructor_fallback_preserves_literal_and_qualified_names() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=ok=>{if(!ok)throw Error('fallback native name')};
          const retained=[];class C extends HTMLElement{constructor(){super();retained.push(this);this.setAttribute('forbidden','value')}};
          customElements.define('x-failed:literal',C);customElements.define('x-failed-qualified',class extends HTMLElement{constructor(){super();this.setAttribute('forbidden','value')}});
          const literal=document.createElement('x-failed:literal');
          check(literal.localName==='x-failed:literal'&&literal.prefix===null&&literal.namespaceURI==='http://www.w3.org/1999/xhtml');
          check(literal instanceof HTMLUnknownElement&&literal!==retained[0]&&retained[0].getAttribute('forbidden')==='value');
          const qualified=document.createElementNS('http://www.w3.org/1999/xhtml','p:x-failed-qualified');
          check(qualified.localName==='x-failed-qualified'&&qualified.prefix==='p'&&qualified.namespaceURI==='http://www.w3.org/1999/xhtml');
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod parser_fallback_name_tests {
    use super::*;
    #[test]
    fn html_parser_failure_keeps_literal_colon_name_and_original_children() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,"globalThis.count=0;customElements.define('x-parser:failed',class extends HTMLElement{constructor(){super();count++;globalThis.saved=this;this.setAttribute('forbidden','yes')}})").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom.clone());
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let source = "<body><x-parser:failed id=token><b id=child></b></x-parser:failed></body>";
        let (_, error) = document_parser_gate::with_parser_construction(scope, &state, || {
            gate.push_until_resource_or_script(source)
        });
        assert!(error.is_none());
        let code=v8::String::new(scope,"(()=>{const token=document.getElementById('token');return count===1&&token instanceof HTMLUnknownElement&&token.localName==='x-parser:failed'&&token.prefix===null&&token!==saved&&saved.parentNode===null&&token.firstChild===document.getElementById('child')})()").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod upgrade_preflight_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn constructor_handle_failure_leaves_candidate_retryable_without_initial_callbacks() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=[];globalThis.candidate=document.createElement('x-preflight');candidate.setAttribute('data-first','value');customElements.define('x-preflight',class extends HTMLElement{static formAssociated=true;static observedAttributes=['data-first'];constructor(){super();calls.push('constructor')}attributeChangedCallback(){calls.push('attribute')}})",
        );
        finish(scope, &state).unwrap();
        let value = eval(scope, "candidate");
        let (_, node) = node_identity::argument(scope, value).unwrap().unwrap();
        let old_shadow = state
            .dom
            .borrow()
            .arena()
            .get(node)
            .unwrap()
            .custom_element_shadow_disabled;
        enqueue_upgrade(scope, &state, node, true).unwrap();
        FAIL_UPGRADE_CONSTRUCTOR.with(|slot| slot.set(true));
        assert!(finish(scope, &state).is_err());
        assert!(!state.custom_elements.elements.borrow().contains_key(&node));
        assert!(!state.dom.borrow().is_form_associated_custom_element(node));
        assert_eq!(
            state
                .dom
                .borrow()
                .arena()
                .get(node)
                .unwrap()
                .custom_element_shadow_disabled,
            old_shadow
        );
        assert!(
            state
                .custom_elements
                .pending_upgrades
                .borrow()
                .contains(&node)
        );
        assert!(eval(scope, "calls.length===0").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='constructor,attribute'").is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "calls.join()==='constructor,attribute'").is_true());
    }
}

#[cfg(test)]
mod parser_failure_outcome_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn post_constructor_report_failure_preserves_failed_token_and_does_not_replay() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.count=0;customElements.define('x-report:failed',class extends HTMLElement{constructor(){super();count++;globalThis.saved=this;this.setAttribute('forbidden','yes')}})",
        );
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom.clone());
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let source = "<body><x-report:failed id=token><b id=child></b></x-report:failed></body>";
        FAIL_SYNCHRONOUS_REPORT.with(|slot| slot.set(true));
        let (progress, error) =
            document_parser_gate::with_parser_construction(scope, &state, || {
                gate.push_until_resource_or_script(source)
            });
        assert!(matches!(
            error,
            Some(V8BindingError::Allocation(
                "injected constructor failure report admission"
            ))
        ));
        assert!(dom.borrow().find_by_id("child").is_none());
        assert!(eval(scope,"count===1&&document.getElementById('token') instanceof HTMLUnknownElement&&document.getElementById('token').localName==='x-report:failed'&&document.getElementById('token')!==saved&&saved.parentNode===null").is_true());
        state.finish_custom_element_reactions(scope).unwrap();
        let (_, error) = document_parser_gate::with_parser_construction(scope, &state, || {
            gate.push_until_resource_or_script(&source[progress.consumed_bytes..])
        });
        assert!(error.is_none());
        state.finish_custom_element_reactions(scope).unwrap();
        assert!(eval(scope,"count===1&&document.getElementById('token').firstChild===document.getElementById('child')").is_true());
    }
}

#[cfg(test)]
mod custom_name_case_tests {
    use super::*;
    #[test]
    fn html_factory_case_mapping_preserves_surrogate_names_and_registry_lookup() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=ok=>{if(!ok)throw Error('lossless name casing')};
          class C extends HTMLElement{};customElements.define('x-case-\ud800',C);
          const e=document.createElement('X-CASE-\ud800');
          check(e instanceof C&&e.localName==='x-case-\ud800'&&e.tagName==='X-CASE-\ud800'&&e.nodeName==='X-CASE-\ud800');
          const attr=document.createAttribute('DATA-\ud800');check(attr.name==='data-\ud800');
          e.setAttribute('DATA-\ud800','value');check(e.getAttribute('data-\ud800')==='value'&&e.getAttribute('DATA-\ud800')==='value');
          e.removeAttribute('DATA-\ud800');check(!e.hasAttribute('data-\ud800'));
          const xml=document.implementation.createDocument(null,'root');const exact=xml.createElement('X-CASE-\ud800');
          check(exact.localName==='X-CASE-\ud800'&&exact.tagName==='X-CASE-\ud800');
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
}

#[cfg(test)]
mod customized_creation_adoption_tests {
    use super::*;
    #[test]
    fn customized_factory_returns_original_wrapper_after_cross_arena_constructor_adoption() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source = v8::Context::new(scope, Default::default());
        let destination = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "customized-create-agent").unwrap();
        source.set_security_token(token.into());
        destination.set_security_token(token.into());
        let target = {
            let scope = &mut v8::ContextScope::new(scope, destination);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, destination, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, source);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, source, dom, RealmId(0), 1).unwrap();
        let target_document = wrap_node(scope, &target, target.document_root()).unwrap();
        let key = v8::String::new(scope, "targetDocument").unwrap();
        source
            .global(scope)
            .set(scope, key.into(), target_document.into())
            .unwrap();
        let code=v8::String::new(scope,r#"(()=>{
          const check=ok=>{if(!ok)throw Error('customized adoption identity')};
          let original,calls=0;class C extends HTMLButtonElement{constructor(){super();calls++;original=this;targetDocument.adoptNode(this)}};
          customElements.define('x-adopt-button',C,{extends:'button'});
          const first=document.createElement('button',{is:'x-adopt-button'});
          check(first===original&&first.ownerDocument===targetDocument&&first instanceof C&&first.localName==='button'&&calls===1);
          const second=document.createElementNS('http://www.w3.org/1999/xhtml','button',{is:'x-adopt-button'});
          check(second===original&&second!==first&&second.ownerDocument===targetDocument&&second instanceof C&&calls===2);
          return true;
        })()"#).unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        assert!(state.custom_elements.elements.borrow().is_empty());
        assert_eq!(target.custom_elements.elements.borrow().len(), 2);
    }
}

#[cfg(test)]
mod parser_cross_arena_handle_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn customized_token_and_open_parent_keep_actual_owners_across_constructor_adoption() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let source = v8::Context::new(scope, Default::default());
        let destination = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "parser-adoption-agent").unwrap();
        source.set_security_token(token.into());
        destination.set_security_token(token.into());
        let target = {
            let scope = &mut v8::ContextScope::new(scope, destination);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, destination, dom, RealmId(1), 1).unwrap()
        };
        let scope = &mut v8::ContextScope::new(scope, source);
        let dom = Rc::new(RefCell::new(DomRuntime::new()));
        let state = install_generated_bindings(scope, source, dom.clone(), RealmId(0), 1).unwrap();
        let target_document = wrap_node(scope, &target, target.document_root()).unwrap();
        let key = v8::String::new(scope, "targetDocument").unwrap();
        source
            .global(scope)
            .set(scope, key.into(), target_document.into())
            .unwrap();
        eval(
            scope,
            r#"globalThis.calls=0;globalThis.attributeOwners=[];
          customElements.define('x-parser-button',class extends HTMLButtonElement{
            static observedAttributes=['id'];constructor(){super();calls++;globalThis.original=this;targetDocument.adoptNode(this)}
            attributeChangedCallback(){attributeOwners.push(this.ownerDocument)}
          },{extends:'button'});
          customElements.define('x-move-parent',class extends HTMLElement{constructor(){super();calls++;globalThis.moved=document.getElementById('moving');targetDocument.body.appendChild(targetDocument.adoptNode(moved))}});"#,
        );
        let parser = oryn_dom::html::StreamingParser::with_runtime(dom);
        let gate = DocumentParserGate::new(
            parser,
            state.document_root(),
            state.document_stream_epoch(),
            false,
        );
        let input = "<body><button is=x-parser-button id=token>text</button><div id=moving><x-move-parent></x-move-parent><span id=after>after</span></div></body>";
        let mut offset = 0;
        while offset < input.len() {
            let (progress, error) =
                document_parser_gate::with_parser_construction(scope, &state, || {
                    gate.push_until_resource_or_script(&input[offset..])
                });
            assert!(error.is_none(), "{error:?}");
            assert!(progress.consumed_bytes > 0);
            offset += progress.consumed_bytes;
            gate.take_events();
        }
        let (ready, error) =
            document_parser_gate::with_parser_construction(scope, &state, || gate.prepare_finish());
        assert!(ready && error.is_none());
        let _ = gate.finish();
        finish(scope, &state).unwrap();
        assert!(eval(scope,r#"calls===2&&document.getElementById('token')===original&&original.ownerDocument===document&&original.textContent==='text'&&moved.ownerDocument===targetDocument&&targetDocument.getElementById('after').parentNode===moved&&targetDocument.getElementById('after').textContent==='after'&&document.getElementById('after')===null"#).is_true());
    }
}

#[cfg(test)]
mod synchronous_fallback_root_tests {
    use super::*;
    #[test]
    fn failed_constructor_outcome_owns_fallback_until_receiver_acquires_it() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let code=v8::String::new(scope,"customElements.define('x-root-fallback',class extends HTMLElement{constructor(){super();this.setAttribute('invalid','result')}})").unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let node = dom.borrow_mut().create_element("x-root-fallback");
        let document = state.document_root();
        dom.borrow_mut().set_node_document(node, document).unwrap();
        let input = oryn_dom::TrackedNodeLease::new(&dom, node).unwrap();
        let (fallback, error, entered, lease) =
            create_synchronously_parser_outcome(scope, &state, node);
        assert!(error.is_none() && entered && lease.is_some());
        assert_ne!(fallback, node);
        assert!(dom.borrow().arena().get(fallback).is_ok());
        let wrapper = wrap_node(scope, &state, fallback).unwrap();
        drop(lease);
        oryn_dom::drain_tracked_node_leases().unwrap();
        let (owner, actual) = node_identity::argument(scope, wrapper.into())
            .unwrap()
            .unwrap();
        assert!(Rc::ptr_eq(&owner, &state));
        assert_eq!(actual, fallback);
        assert!(
            dom.borrow()
                .arena()
                .get(fallback)
                .unwrap()
                .custom_element_creation_failed
        );
        drop(input);
    }
}

#[cfg(test)]
mod native_retention_memo_tests {
    use super::*;
    #[test]
    fn registry_witness_skips_duplicate_walks_but_rechecks_released_custody() {
        let mut dom = oryn_dom::html::parse("<body></body>")
            .into_runtime()
            .unwrap();
        let first = dom.create_element("script");
        let witness = dom.create_element("script");
        let duplicate = dom.create_element("script");
        let unrelated = dom.create_element("script");
        for node in [first, witness, duplicate, unrelated] {
            dom.retain_wrapper(node).unwrap();
        }
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.retain_native(witness).unwrap();
        let associations = [(first, 41), (witness, 41), (duplicate, 41), (unrelated, 42)];
        let roots = traced_native_roots::Snapshot::default();
        let mut ids = BTreeSet::new();
        let mut memo = BTreeMap::new();
        for (node, registry) in associations {
            retain_registry_from_node(&dom, &roots, node, registry, &mut ids, &mut memo).unwrap();
        }
        let expected = associations
            .iter()
            .filter_map(|(node, registry)| {
                dom.retained_by_native_root(*node)
                    .unwrap()
                    .then_some(*registry)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(ids, expected);
        assert_eq!(ids, BTreeSet::from([41]));
        assert!(
            memo.contains_key(&first),
            "a failed first witness is examined"
        );
        assert!(
            memo.contains_key(&witness),
            "a later witness can establish custody"
        );
        assert!(
            !memo.contains_key(&duplicate),
            "proven registry skips another ancestry walk"
        );
        assert!(
            memo.contains_key(&unrelated),
            "another registry requires its own proof"
        );

        dom.release_native(witness).unwrap();
        let mut ids = BTreeSet::new();
        let mut memo = BTreeMap::new();
        for (node, registry) in associations {
            retain_registry_from_node(&dom, &roots, node, registry, &mut ids, &mut memo).unwrap();
        }
        assert!(
            ids.is_empty(),
            "a new snapshot must release the old registry root"
        );
        assert!(
            memo.contains_key(&duplicate),
            "failed proofs never suppress later candidates"
        );
        for node in [first, witness, duplicate, unrelated] {
            dom.release_wrapper(node).unwrap();
        }
    }

    #[test]
    fn shared_ancestry_cache_matches_native_roots_and_excludes_wrapper_only_custody() {
        let mut dom = oryn_dom::html::parse("<section><div><i></i><b></b></div></section>")
            .into_runtime()
            .unwrap();
        let root = dom
            .query_selector(dom.document(), "section")
            .unwrap()
            .unwrap();
        let leaf = dom.query_selector(dom.document(), "i").unwrap().unwrap();
        dom.retain_wrapper(root).unwrap();
        dom.remove(root).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        let mut memo = BTreeMap::new();
        assert!(
            !native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                leaf,
                &mut memo
            )
            .unwrap(),
            "JS wrapper roots do not keep registry cycles alive"
        );
        dom.retain_native(root).unwrap();
        let mut memo = BTreeMap::new();
        for (node, _) in dom.arena().iter() {
            assert_eq!(
                native_retention(
                    &dom,
                    &traced_native_roots::Snapshot::default(),
                    node,
                    &mut memo
                )
                .unwrap(),
                dom.retained_by_native_root(node).unwrap()
            );
        }
        let computed = memo.len();
        assert!(
            native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                leaf,
                &mut memo
            )
            .unwrap()
        );
        assert_eq!(
            memo.len(),
            computed,
            "repeated ancestry reads reuse the same snapshot"
        );
        dom.release_native(root).unwrap();
        let mut memo = BTreeMap::new();
        assert!(
            !native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                leaf,
                &mut memo
            )
            .unwrap(),
            "fresh refresh observes custody release"
        );
        let body = dom.query_selector(dom.document(), "body").unwrap().unwrap();
        dom.append_child(body, root).unwrap();
        let mut memo = BTreeMap::new();
        for (node, _) in dom.arena().iter() {
            assert_eq!(
                native_retention(
                    &dom,
                    &traced_native_roots::Snapshot::default(),
                    node,
                    &mut memo
                )
                .unwrap(),
                dom.retained_by_native_root(node).unwrap()
            );
        }
        dom.release_wrapper(root).unwrap();
    }
    #[test]
    fn memo_follows_hosted_template_documents_and_reparenting() {
        let mut dom =
            oryn_dom::html::parse("<body><template><span></span></template><div></div></body>")
                .into_runtime()
                .unwrap();
        let template = dom
            .query_selector(dom.document(), "template")
            .unwrap()
            .unwrap();
        let content = dom.template_content(template).unwrap();
        let child = dom.query_selector(content, "span").unwrap().unwrap();
        let other = dom.query_selector(dom.document(), "div").unwrap().unwrap();
        dom.retain_wrapper(template).unwrap();
        dom.remove(template).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        dom.retain_native(template).unwrap();
        let mut memo = BTreeMap::new();
        assert!(
            native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                child,
                &mut memo
            )
            .unwrap()
        );
        dom.append_child(other, child).unwrap();
        let mutations = dom.drain_mutations();
        dom.release_mutation_roots(&mutations).unwrap();
        let mut memo = BTreeMap::new();
        assert_eq!(
            native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                child,
                &mut memo
            )
            .unwrap(),
            dom.retained_by_native_root(child).unwrap()
        );
        assert!(
            !native_retention(
                &dom,
                &traced_native_roots::Snapshot::default(),
                child,
                &mut memo
            )
            .unwrap(),
            "old hosted-document ancestor does not remain cached"
        );
        dom.release_native(template).unwrap();
        dom.release_wrapper(template).unwrap();
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_UPGRADE_ENQUEUE_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
}

#[cfg(test)]
mod explicit_inert_upgrade_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    #[test]
    fn explicit_upgrade_uses_receiver_registry_without_initializing_inert_document() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(940), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            let calls=0;class Explicit extends HTMLElement{constructor(){super();this.marker=++calls}}
            customElements.define('x-explicit',Explicit);
            const inert=document.implementation.createHTMLDocument('inert');inert.body.innerHTML='<x-explicit></x-explicit><x-unknown></x-unknown>';
            const candidate=inert.body.firstChild,unknown=inert.body.lastChild;
            if(candidate instanceof Explicit||candidate.customElementRegistry!==null)return false;
            customElements.upgrade(inert.body);customElements.upgrade(inert.body);
            if(!(candidate instanceof Explicit)||candidate.marker!==1||calls!==1||candidate.ownerDocument!==inert)return false;
            if(inert.customElementRegistry!==null||inert.body.customElementRegistry!==null||unknown.customElementRegistry!==null)return false;
            const scoped=new CustomElementRegistry(),foreign=new CustomElementRegistry();let foreignCalls=0;
            scoped.define('x-scoped-explicit',class extends HTMLElement{constructor(){super();this.marker=19}});
            const root=document.createElement('div',{customElementRegistry:null});
            root.innerHTML='<x-scoped-explicit></x-scoped-explicit>';
            const owned=document.createElement('x-scoped-explicit',{customElementRegistry:foreign});root.append(owned);
            scoped.upgrade(root);
            if(root.firstChild.marker!==19||owned.marker!==undefined||owned.customElementRegistry!==foreign||root.customElementRegistry!==null)return false;
            foreign.define('x-scoped-explicit',class extends HTMLElement{constructor(){super();foreignCalls++}});foreign.upgrade(root);
            return foreignCalls===1&&root.firstChild.marker===19&&owned.customElementRegistry===foreign;
        })()"#).is_true());
    }
    #[test]
    fn explicit_upgrade_failure_rolls_back_associations_and_reactions_before_retry() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(941), 1).unwrap();
        eval(
            scope,
            "globalThis.calls=0;customElements.define('x-inert-retry',class extends HTMLElement{constructor(){super();calls++}});globalThis.inert=document.implementation.createHTMLDocument();inert.body.innerHTML='<x-inert-retry></x-inert-retry><x-inert-retry></x-inert-retry>'",
        );
        let value = eval(scope, "inert.body");
        let (_, node) = node_identity::argument(scope, value).unwrap().unwrap();
        let registry = state.custom_elements.global.clone();
        let before = state
            .dom
            .borrow()
            .custom_registry_documents(registry.id.get());
        FAIL_UPGRADE_ENQUEUE_AFTER.with(|slot| slot.set(Some(1)));
        assert!(explicit_upgrade(scope, &state, &registry, node).is_err());
        assert_eq!(FAIL_UPGRADE_ENQUEUE_AFTER.with(Cell::get), None);
        assert_eq!(
            state
                .dom
                .borrow()
                .custom_registry_documents(registry.id.get()),
            before
        );
        assert!(state.custom_elements.pending_upgrades.borrow().is_empty());
        finish(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls===0&&[...inert.body.children].every(e=>e.customElementRegistry===null)"
            )
            .is_true()
        );
        explicit_upgrade(scope, &state, &registry, node).unwrap();
        finish(scope, &state).unwrap();
        assert!(eval(scope,"calls===2&&[...inert.body.children].every(e=>e.customElementRegistry===customElements)").is_true());
    }
}

#[cfg(test)]
mod initial_document_registry_tests {
    use super::*;

    #[test]
    fn initial_document_reuse_retains_registry_and_constructs_form_associated_candidates() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state =
            install_generated_bindings(scope, context, dom.clone(), RealmId(944), 1).unwrap();
        let eval = super::super::test_eval::eval;
        assert!(eval(scope, r#"(()=>{
            globalThis.savedRegistry=customElements;globalThis.savedDocument=document;globalThis.scoped=new CustomElementRegistry();
            globalThis.constructors=[];
            globalThis.BeforeCommit=class extends HTMLElement {
                static formAssociated=true;
                constructor(){super();constructors.push('before');this.i=this.attachInternals();this.i.setFormValue('before')}
            };
            customElements.define('x-before-commit',BeforeCommit);
            return document.customElementRegistry===savedRegistry;
        })()"#).is_true());
        let parsed = oryn_dom::html::parse(
            "<body><form><x-before-commit></x-before-commit><x-after-commit></x-after-commit></form><div id=null-root><x-unregistered></x-unregistered></div><div id=scoped-root><x-scoped></x-scoped></div></body>",
        );
        let document = dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let _document_custody = oryn_dom::TrackedNodeLease::new(&dom, document).unwrap();
        let null_root = dom
            .borrow()
            .get_element_by_id(document, "null-root")
            .unwrap()
            .unwrap();
        dom.borrow_mut()
            .set_custom_element_registry(null_root, None)
            .unwrap();
        let scoped_root = dom
            .borrow()
            .get_element_by_id(document, "scoped-root")
            .unwrap()
            .unwrap();
        let scoped_value = eval(scope, "scoped");
        let scoped = registry_receiver(
            scope,
            v8::Local::<v8::Object>::try_from(scoped_value).unwrap(),
        )
        .unwrap();
        dom.borrow_mut()
            .set_custom_element_registry(scoped_root, Some(scoped.id.get()))
            .unwrap();
        assert!(state.replace_initial_document(null_root, 2).is_err());
        assert!(!state.execution_retired());
        assert!(
            eval(
                scope,
                "document===savedDocument&&document.customElementRegistry===savedRegistry"
            )
            .is_true()
        );
        state.replace_initial_document(document, 2).unwrap();
        assert_eq!(state.document_root(), document);
        assert!(eval(scope, r#"(()=>{
            if(customElements!==savedRegistry||document===savedDocument)return false;
            if(document.customElementRegistry!==savedRegistry||savedDocument.customElementRegistry!==savedRegistry)return false;
            if(document.querySelector('x-after-commit').customElementRegistry!==savedRegistry)return false;
            if(document.getElementById('null-root').customElementRegistry!==null||document.querySelector('x-unregistered').customElementRegistry!==null)return false;
            if(document.getElementById('scoped-root').customElementRegistry!==scoped||document.querySelector('x-scoped').customElementRegistry!==scoped)return false;
            scoped.define('x-scoped',class extends HTMLElement{constructor(){super();this.marker='scoped'}});
            globalThis.AfterCommit=class extends HTMLElement {
                static formAssociated=true;
                constructor(){super();constructors.push('after');this.i=this.attachInternals();this.i.setFormValue('after')}
            };
            customElements.define('x-after-commit',AfterCommit);
            return true;
        })()"#).is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, r#"(()=>{
            const before=document.querySelector('x-before-commit'),after=document.querySelector('x-after-commit');
            if(!(before instanceof BeforeCommit)||!(after instanceof AfterCommit))return false;
            before.i.setFormValue('updated-before');after.i.setFormValue('updated-after');
            return before.i.form===document.querySelector('form')&&after.i.form===before.i.form&&
                before.matches(':defined')&&after.matches(':defined')&&document.querySelector('x-scoped').marker==='scoped'&&
                constructors.filter(x=>x==='before').length===1&&constructors.filter(x=>x==='after').length===1;
        })()"#).is_true());
        finish(scope, &state).unwrap();
        assert!(eval(scope, "constructors.length===2").is_true());
        // Invalid replacement cannot retire the active document or rewrite its
        // registry; the original initial-commit generation guard remains exact.
        assert!(state.replace_initial_document(document, 2).is_err());
        assert_eq!(state.document_root(), document);
        assert!(!state.execution_retired());
        assert!(
            eval(
                scope,
                "document.customElementRegistry===savedRegistry&&constructors.length===2"
            )
            .is_true()
        );
    }

    #[test]
    fn initial_document_reuse_keeps_foreign_constructor_registry_and_reactions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let token = v8::String::new(scope, "initial-foreign-constructor").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let parent_document = dom.borrow().document();
        let parsed = oryn_dom::html::parse("<body></body>");
        let initial_document = dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let _initial_custody = oryn_dom::TrackedNodeLease::new(&dom, initial_document).unwrap();
        let first_state = {
            let scope = &mut v8::ContextScope::new(scope, first);
            install_generated_bindings_with_root(
                scope,
                first,
                dom.clone(),
                RealmId(945),
                1,
                parent_document,
            )
            .unwrap()
        };
        let second_state = {
            let scope = &mut v8::ContextScope::new(scope, second);
            install_generated_bindings_with_root(
                scope,
                second,
                dom.clone(),
                RealmId(946),
                1,
                initial_document,
            )
            .unwrap()
        };
        let eval = super::super::test_eval::eval;
        {
            let scope = &mut v8::ContextScope::new(scope, first);
            let foreign_window = second.global(scope);
            let parent_window = first.global(scope);
            let key = v8::String::new(scope, "foreignWindow").unwrap();
            assert_eq!(
                parent_window.set(scope, key.into(), foreign_window.into()),
                Some(true)
            );
            assert!(
                eval(
                    scope,
                    r#"(()=>{
                globalThis.w=foreignWindow;globalThis.events=[];
                globalThis.savedForeignRegistry=w.customElements;
                globalThis.oldForeignDocument=w.document;
                globalThis.BeforeForeign=class extends w.HTMLElement{};
                w.customElements.define('before-foreign',BeforeForeign);
                return oldForeignDocument.customElementRegistry===savedForeignRegistry;
            })()"#
                )
                .is_true()
            );
        }
        let parsed = oryn_dom::html::parse("<body></body>");
        let replacement = dom
            .borrow_mut()
            .import_subtree(&parsed.dom, parsed.document)
            .unwrap();
        let _replacement_custody = oryn_dom::TrackedNodeLease::new(&dom, replacement).unwrap();
        assert_eq!(node_registry_id(&second_state, replacement).unwrap(), None);
        // This is the exact state transition used by a same-origin srcdoc
        // commit that retains the initial Window and its native constructors.
        second_state
            .replace_initial_document(replacement, 2)
            .unwrap();
        assert_eq!(second_state.document_root(), replacement);
        assert_eq!(
            node_registry_id(&second_state, replacement).unwrap(),
            Some(second_state.custom_elements.id.get())
        );
        let scope = &mut v8::ContextScope::new(scope, first);
        let result = eval(
            scope,
            r#"(()=>{
            const check=(value,label)=>{if(!value)throw Error(label)};
            check(w.customElements===savedForeignRegistry,'retained Window registry');
            check(w.document!==oldForeignDocument,'new active Document');
            check(w.document.customElementRegistry===savedForeignRegistry,'replacement registry');
            check(oldForeignDocument.customElementRegistry===savedForeignRegistry,'old Document registry');
            check(new BeforeForeign().ownerDocument===w.document,'retained foreign definition');
            class Foreign extends w.HTMLElement {
                connectedCallback(){events.push('connected')}
                disconnectedCallback(){events.push('disconnected')}
            }
            w.customElements.define('foreign-element',Foreign);
            check(w.customElements.get('foreign-element')===Foreign,'foreign registry definition');
            check(customElements.get('foreign-element')===undefined,'no caller-registry pollution');
            try{Reflect.construct(HTMLElement,[],Foreign);throw Error('wrong registry admitted')}
            catch(error){check(error.name==='TypeError','caller base must reject foreign-only definition')}
            const node=new Foreign();
            events.push(node.ownerDocument===w.document);
            check(node instanceof Foreign&&node instanceof w.HTMLElement,'foreign native prototype');
            w.document.body.appendChild(node);
            node.remove();
            class Local extends HTMLElement {
                adoptedCallback(){events.push('adopted')}
                connectedCallback(){events.push('local-connected')}
            }
            customElements.define('local-element',Local);
            w.customElements.define('local-element',class extends w.HTMLElement {
                constructor(){super();events.push('foreign-definition')}
            });
            const local=new Local();
            w.document.body.appendChild(local);
            check(local.ownerDocument===w.document&&local instanceof Local,'adoption preserves original definition');
            return JSON.stringify(events);
        })()"#,
        );
        assert_eq!(
            result.to_rust_string_lossy(scope),
            r#"[true,"connected","disconnected","adopted","local-connected"]"#
        );
        finish(scope, &first_state).unwrap();
        finish(scope, &second_state).unwrap();
        assert!(eval(scope, r#"JSON.stringify(events)==='[true,"connected","disconnected","adopted","local-connected"]'"#).is_true());
    }
}

#[cfg(test)]
mod checkpoint_pending_tests {
    use super::*;

    #[test]
    fn borrowed_reaction_queue_prevents_idle_without_panicking_or_consuming() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let held = state.custom_elements.reactions.borrow_mut();
        assert!(state.custom_elements.checkpoint_pending(&state));
        assert!(state.has_pending_executable_checkpoint_work(true));
        drop(held);
        assert!(state.custom_elements.reactions.borrow().is_empty());
    }
}

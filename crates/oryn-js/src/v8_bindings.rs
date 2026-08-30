//! Generated V8 templates backed by the Rust-owned live DOM.
//!
//! Browser objects carry a numeric wrapper identifier only. All interface,
//! realm, document-generation and node-generation checks happen before a
//! generated callback reaches a Rust behavior implementation.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::{Rc, Weak as RcWeak},
};

use oryn_dom::{DomError, DomRuntime, Mutation, NodeId, NodeKind, ShadowMode};

use crate::{
    bindings::{BindingError, InterfaceBrand, WrapperId, WrapperRegistry},
    generated::{GENERATED_INTERFACES, GeneratedMemberKind},
    realm::RealmId,
    trusted_types::{TrustedKind, TrustedTypesConfig, TrustedTypesRegistry, TrustedValue},
};

pub type SharedDom = Rc<RefCell<DomRuntime>>;

#[derive(Debug, thiserror::Error)]
pub enum V8BindingError {
    #[error("generated interface {0} is unavailable")]
    MissingInterface(&'static str),
    #[error("V8 failed to allocate {0}")]
    Allocation(&'static str),
    #[error("event callback failed: {0}")]
    EventCallback(String),
    #[error(transparent)]
    Binding(#[from] BindingError),
    #[error(transparent)]
    Dom(#[from] DomError),
    #[error("generated binding {interface}.{member} is not implemented")]
    Unsupported { interface: String, member: String },
    #[error("document.write exceeded the configured {resource} limit")]
    DocumentWriteLimit { resource: &'static str },
}

struct WrapperObject {
    handle: v8::Weak<v8::Object>,
    connected_root: Option<v8::Global<v8::Object>>,
}

#[derive(Debug, Clone)]
struct EventRecord {
    event_type: String,
    target: Option<NodeId>,
    current_target: Option<NodeId>,
    bubbles: bool,
    cancelable: bool,
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_stopped: bool,
}

struct EventListener {
    callback: v8::Global<v8::Function>,
}

#[derive(Debug, Clone, Copy, Default)]
struct MutationObserverOptions {
    child_list: bool,
    attributes: bool,
    character_data: bool,
    subtree: bool,
}

struct MutationObserverEntry {
    callback: v8::Global<v8::Function>,
    object: v8::Global<v8::Object>,
    target: Option<NodeId>,
    options: MutationObserverOptions,
    records: Vec<Mutation>,
}

struct TrustedPolicyCallbacks {
    name: String,
    create_html: Option<v8::Global<v8::Function>>,
    create_script: Option<v8::Global<v8::Function>>,
    create_script_url: Option<v8::Global<v8::Function>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BindingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone)]
pub enum BindingGeometry {
    Available(BindingRect),
    Hidden,
    Unsupported(String),
}

#[derive(Debug, Clone, Default)]
pub struct BindingLayout {
    pub styles: BTreeMap<u64, BTreeMap<String, String>>,
    pub geometry: BTreeMap<u64, BindingGeometry>,
}

/// Per-context binding state. The DOM may be shared with the page orchestrator,
/// but V8 handles and wrapper identities remain realm-local and thread-affine.
pub struct V8BindingState {
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
    wrappers: Rc<RefCell<WrapperRegistry>>,
    objects: RefCell<BTreeMap<WrapperId, WrapperObject>>,
    instance_templates: RefCell<BTreeMap<&'static str, v8::Global<v8::ObjectTemplate>>>,
    next_event_id: Cell<u64>,
    events: RefCell<BTreeMap<u64, EventRecord>>,
    listeners: RefCell<BTreeMap<(NodeId, String), Vec<EventListener>>>,
    next_mutation_observer_id: Cell<u64>,
    mutation_observers: RefCell<BTreeMap<u64, MutationObserverEntry>>,
    layout: RefCell<BindingLayout>,
    frame_roots: RefCell<BTreeMap<NodeId, NodeId>>,
    document_write_anchor: Cell<Option<NodeId>>,
    document_open: Cell<bool>,
    document_write_buffer: RefCell<String>,
    document_write_depth: Cell<u32>,
    pending_dynamic_scripts: RefCell<VecDeque<NodeId>>,
    prepared_scripts: RefCell<BTreeSet<NodeId>>,
    trusted_registry: RefCell<TrustedTypesRegistry>,
    trusted_policies: RefCell<BTreeMap<u64, TrustedPolicyCallbacks>>,
    trusted_values: RefCell<BTreeMap<u64, TrustedValue>>,
    next_trusted_id: Cell<u64>,
}

impl V8BindingState {
    pub fn dom(&self) -> SharedDom {
        self.dom.clone()
    }

    pub fn realm(&self) -> RealmId {
        self.realm
    }

    pub fn process_weak_releases(&self) -> Result<usize, BindingError> {
        self.objects
            .borrow_mut()
            .retain(|_, object| !object.handle.is_empty());
        self.wrappers
            .borrow_mut()
            .process_weak_releases(&mut self.dom.borrow_mut())
    }

    /// Keep wrappers for connected nodes alive and release that embedder root
    /// as soon as the node becomes detached. This mirrors browser wrapper
    /// tracing: connected custom elements retain their JavaScript identity and
    /// expando state, while unreachable detached subtrees remain collectable.
    /// Returns the number of roots released so the host can request GC only
    /// when it can reclaim detached wrapper graphs.
    pub fn refresh_connected_wrapper_roots(&self, scope: &mut v8::PinScope<'_, '_>) -> usize {
        let dom = self.dom.borrow();
        let wrappers = self.wrappers.borrow();
        let mut objects = self.objects.borrow_mut();
        let mut released = 0;
        for (wrapper, object) in objects.iter_mut() {
            let connected = wrappers
                .metadata(*wrapper)
                .is_some_and(|metadata| dom.is_connected(metadata.node).unwrap_or(false));
            if connected {
                if object.connected_root.is_none()
                    && let Some(local) = object.handle.to_local(scope)
                {
                    object.connected_root = Some(v8::Global::new(scope, local));
                }
            } else if object.connected_root.take().is_some() {
                released += 1;
            }
        }
        released
    }

    pub fn set_layout(&self, layout: BindingLayout) {
        *self.layout.borrow_mut() = layout;
    }

    pub fn set_frame_document(&self, frame: NodeId, document: NodeId) {
        self.frame_roots.borrow_mut().insert(frame, document);
    }

    pub fn set_document_write_anchor(&self, anchor: Option<NodeId>) {
        self.document_write_anchor.set(anchor);
    }

    pub fn configure_trusted_types(&self, config: TrustedTypesConfig) {
        *self.trusted_registry.borrow_mut() = TrustedTypesRegistry::new(config);
    }

    /// Drain scripts whose preparation was triggered by a JavaScript DOM
    /// insertion or by assigning executable content to an already-connected
    /// script element. The page orchestrator performs resource policy and
    /// execution; bindings remain the authoritative mutation boundary.
    pub fn take_pending_dynamic_scripts(&self) -> Vec<NodeId> {
        self.pending_dynamic_scripts
            .borrow_mut()
            .drain(..)
            .collect()
    }

    fn dynamic_script_candidates(&self, root: NodeId) -> Result<Vec<NodeId>, V8BindingError> {
        let candidates = {
            let dom = self.dom.borrow();
            if !dom.is_connected(root)? {
                return Ok(Vec::new());
            }
            let mut candidates = Vec::new();
            if dom
                .node_name(root)
                .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
            {
                candidates.push(root);
            }
            candidates.extend(dom.query_selector_all(root, "script")?);
            candidates
                .into_iter()
                .filter(|script| {
                    dom.attribute(*script, "src")
                        .ok()
                        .flatten()
                        .is_some_and(|src| !src.trim().is_empty())
                        || dom
                            .arena()
                            .text_content(*script)
                            .is_ok_and(|source| !source.trim().is_empty())
                })
                .collect::<Vec<_>>()
        };
        Ok(candidates)
    }

    fn mark_script_subtree_inert(&self, root: NodeId) -> Result<(), V8BindingError> {
        let scripts = {
            let dom = self.dom.borrow();
            let mut scripts = Vec::new();
            if dom
                .node_name(root)
                .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
            {
                scripts.push(root);
            }
            scripts.extend(dom.query_selector_all(root, "script")?);
            scripts
        };
        self.prepared_scripts.borrow_mut().extend(scripts);
        Ok(())
    }
}

impl Drop for V8BindingState {
    fn drop(&mut self) {
        let _ = self
            .wrappers
            .borrow_mut()
            .release_all(&mut self.dom.borrow_mut());
    }
}

pub fn install_generated_bindings(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let document = dom.borrow().document();
    install_generated_bindings_with_root(scope, context, dom, realm, document_generation, document)
}

pub fn install_generated_bindings_with_root(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    dom: SharedDom,
    realm: RealmId,
    document_generation: u64,
    document: NodeId,
) -> Result<Rc<V8BindingState>, V8BindingError> {
    let state = Rc::new(V8BindingState {
        dom,
        realm,
        document_generation,
        wrappers: Rc::new(RefCell::new(WrapperRegistry::default())),
        objects: RefCell::new(BTreeMap::new()),
        instance_templates: RefCell::new(BTreeMap::new()),
        next_event_id: Cell::new(1),
        events: RefCell::new(BTreeMap::new()),
        listeners: RefCell::new(BTreeMap::new()),
        next_mutation_observer_id: Cell::new(1),
        mutation_observers: RefCell::new(BTreeMap::new()),
        layout: RefCell::new(BindingLayout::default()),
        frame_roots: RefCell::new(BTreeMap::new()),
        document_write_anchor: Cell::new(None),
        document_open: Cell::new(false),
        document_write_buffer: RefCell::new(String::new()),
        document_write_depth: Cell::new(0),
        pending_dynamic_scripts: RefCell::new(VecDeque::new()),
        prepared_scripts: RefCell::new(BTreeSet::new()),
        trusted_registry: RefCell::new(TrustedTypesRegistry::new(TrustedTypesConfig {
            allow_any_policy: true,
            ..TrustedTypesConfig::default()
        })),
        trusted_policies: RefCell::new(BTreeMap::new()),
        trusted_values: RefCell::new(BTreeMap::new()),
        next_trusted_id: Cell::new(1),
    });
    context.set_slot(state.clone());

    let mut templates = BTreeMap::<&'static str, v8::Global<v8::FunctionTemplate>>::new();
    let mut remaining = GENERATED_INTERFACES.iter().enumerate().collect::<Vec<_>>();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut index = 0;
        while index < remaining.len() {
            let (interface_index, interface) = remaining[index];
            if interface
                .parent
                .is_some_and(|parent| !templates.contains_key(parent))
            {
                index += 1;
                continue;
            }
            let constructor_data = v8::Integer::new(scope, interface_index as i32);
            let template = v8::FunctionTemplate::builder(generated_constructor)
                .data(constructor_data.into())
                .build(scope);
            template.set_class_name(
                v8::String::new(scope, interface.name)
                    .ok_or(V8BindingError::Allocation("interface name"))?,
            );
            if let Some(parent) = interface.parent {
                let parent = v8::Local::new(
                    scope,
                    templates
                        .get(parent)
                        .ok_or(V8BindingError::MissingInterface(parent))?,
                );
                template.inherit(parent);
            }
            let instance = template.instance_template(scope);
            instance.set_internal_field_count(1);
            let prototype = template.prototype_template(scope);
            for (member_index, member) in interface.members.iter().enumerate() {
                let token = member_token(interface_index, member_index);
                let data = v8::Integer::new(scope, token);
                let name = v8::String::new(scope, member.name)
                    .ok_or(V8BindingError::Allocation("member name"))?;
                match member.kind {
                    GeneratedMemberKind::Attribute => {
                        let mut configuration =
                            v8::AccessorConfiguration::new(generated_getter).data(data.into());
                        if !member.readonly {
                            configuration = configuration.setter(generated_setter);
                        }
                        instance.set_accessor_with_configuration(name.into(), configuration);
                    }
                    GeneratedMemberKind::Operation => {
                        let function = v8::FunctionTemplate::builder(generated_operation)
                            .data(data.into())
                            .length(member.arity.into())
                            .constructor_behavior(v8::ConstructorBehavior::Throw)
                            .build(scope);
                        prototype.set(name.into(), function.into());
                    }
                    GeneratedMemberKind::Constant
                    | GeneratedMemberKind::Constructor
                    | GeneratedMemberKind::Iterable => {}
                }
            }
            let constructor = template
                .get_function(scope)
                .ok_or(V8BindingError::Allocation("interface constructor"))?;
            let global = context.global(scope);
            global.set(
                scope,
                v8::String::new(scope, interface.name).unwrap().into(),
                constructor.into(),
            );
            state
                .instance_templates
                .borrow_mut()
                .insert(interface.name, v8::Global::new(scope, instance));
            templates.insert(interface.name, v8::Global::new(scope, template));
            remaining.remove(index);
        }
        if before == remaining.len() {
            return Err(V8BindingError::MissingInterface(
                remaining[0].1.parent.unwrap_or(remaining[0].1.name),
            ));
        }
    }

    let wrapper = wrap_node(scope, &state, document)?;
    let name = v8::String::new(scope, "document").ok_or(V8BindingError::Allocation("document"))?;
    context
        .global(scope)
        .set(scope, name.into(), wrapper.into());
    let get_computed_style = v8::Function::new(scope, get_computed_style_callback)
        .ok_or(V8BindingError::Allocation("getComputedStyle function"))?;
    let name = v8::String::new(scope, "getComputedStyle")
        .ok_or(V8BindingError::Allocation("getComputedStyle name"))?;
    context
        .global(scope)
        .set(scope, name.into(), get_computed_style.into());
    install_trusted_types(scope, context)?;
    Ok(state)
}

fn install_trusted_types(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let factory = v8::Object::new(scope);
    set_generated_prototype(scope, context, factory, "TrustedTypePolicyFactory")?;
    macro_rules! install {
        ($name:literal, $callback:ident) => {{
            let function = v8::Function::new(scope, $callback)
                .ok_or(V8BindingError::Allocation("Trusted Types function"))?;
            let name = v8::String::new(scope, $name)
                .ok_or(V8BindingError::Allocation("Trusted Types member name"))?;
            factory.create_data_property(scope, name.into(), function.into());
        }};
    }
    install!("createPolicy", trusted_create_policy);
    install!("isHTML", trusted_is_html);
    install!("isScript", trusted_is_script);
    install!("isScriptURL", trusted_is_script_url);
    install!("getAttributeType", trusted_get_attribute_type);
    install!("getPropertyType", trusted_get_property_type);
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return Err(V8BindingError::Allocation("Trusted Types state"));
    };
    let empty_html = make_trusted_value(
        scope,
        &state,
        TrustedKind::Html,
        String::new(),
        String::new(),
    )?;
    let empty_script = make_trusted_value(
        scope,
        &state,
        TrustedKind::Script,
        String::new(),
        String::new(),
    )?;
    for (name, value) in [("emptyHTML", empty_html), ("emptyScript", empty_script)] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("Trusted Types constant name"))?;
        factory.create_data_property(scope, key.into(), value.into());
    }
    let name = v8::String::new(scope, "trustedTypes")
        .ok_or(V8BindingError::Allocation("trustedTypes global"))?;
    context
        .global(scope)
        .create_data_property(scope, name.into(), factory.into());
    Ok(())
}

fn trusted_create_policy(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "Trusted Types state is unavailable");
        return;
    };
    let name = args.get(0).to_rust_string_lossy(scope);
    if let Err(error) = state.trusted_registry.borrow_mut().create_policy(&name) {
        throw_type_error(scope, &error.to_string());
        return;
    }
    let options = args.get(1).to_object(scope);
    let callback = |scope: &mut v8::PinScope, name: &str| {
        let options = options?;
        let key = v8::String::new(scope, name)?;
        v8::Local::<v8::Function>::try_from(options.get(scope, key.into())?)
            .ok()
            .map(|function| v8::Global::new(scope, function))
    };
    let policy_id = state.next_trusted_id.get();
    state.next_trusted_id.set(policy_id.saturating_add(1));
    state.trusted_policies.borrow_mut().insert(
        policy_id,
        TrustedPolicyCallbacks {
            name: name.clone(),
            create_html: callback(scope, "createHTML"),
            create_script: callback(scope, "createScript"),
            create_script_url: callback(scope, "createScriptURL"),
        },
    );
    let policy = v8::Object::new(scope);
    if let Err(error) = set_generated_prototype(
        scope,
        scope.get_current_context(),
        policy,
        "TrustedTypePolicy",
    ) {
        throw_type_error(scope, &error.to_string());
        return;
    }
    if let (Some(key), Some(value)) = (
        v8::String::new(scope, "name"),
        v8::String::new(scope, &name),
    ) {
        policy.create_data_property(scope, key.into(), value.into());
    }
    for (member, kind) in [
        ("createHTML", TrustedKind::Html),
        ("createScript", TrustedKind::Script),
        ("createScriptURL", TrustedKind::ScriptUrl),
    ] {
        let token = policy_id.saturating_mul(4).saturating_add(kind_token(kind));
        let data = v8::BigInt::new_from_u64(scope, token);
        let function = v8::Function::builder(trusted_policy_create)
            .data(data.into())
            .build(scope)
            .expect("allocate Trusted Types policy function");
        let key = v8::String::new(scope, member).expect("allocate Trusted Types method name");
        policy.create_data_property(scope, key.into(), function.into());
    }
    return_value.set(policy.into());
}

fn trusted_policy_create(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(token) = exact_u64(args.data()) else {
        throw_type_error(scope, "invalid Trusted Types policy token");
        return;
    };
    let policy_id = token / 4;
    let kind = match token % 4 {
        1 => TrustedKind::Html,
        2 => TrustedKind::Script,
        3 => TrustedKind::ScriptUrl,
        _ => return,
    };
    let (policy_name, callback) = {
        let policies = state.trusted_policies.borrow();
        let Some(policy) = policies.get(&policy_id) else {
            throw_type_error(scope, "unknown Trusted Types policy");
            return;
        };
        let callback = match kind {
            TrustedKind::Html => policy.create_html.clone(),
            TrustedKind::Script => policy.create_script.clone(),
            TrustedKind::ScriptUrl => policy.create_script_url.clone(),
        };
        (policy.name.clone(), callback)
    };
    let input = args.get(0);
    let value = if let Some(callback) = callback {
        let callback = v8::Local::new(scope, &callback);
        let undefined = v8::undefined(scope).into();
        match callback.call(scope, undefined, &[input]) {
            Some(value) => value.to_rust_string_lossy(scope),
            None => return,
        }
    } else {
        input.to_rust_string_lossy(scope)
    };
    match make_trusted_value(scope, &state, kind, value, policy_name) {
        Ok(value) => return_value.set(value.into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn make_trusted_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: TrustedKind,
    value: String,
    policy_name: String,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let id = state.next_trusted_id.get();
    state.next_trusted_id.set(id.saturating_add(1));
    state
        .trusted_values
        .borrow_mut()
        .insert(id, TrustedValue::new(kind, value, policy_name));
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(2);
    let object = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("Trusted Type value"))?;
    set_generated_prototype(
        scope,
        scope.get_current_context(),
        object,
        kind.interface_name(),
    )?;
    object.set_internal_field(0, v8::Integer::new(scope, kind_token(kind) as i32).into());
    object.set_internal_field(1, v8::BigInt::new_from_u64(scope, id).into());
    let function = v8::Function::builder(trusted_value_to_string)
        .data(v8::BigInt::new_from_u64(scope, id).into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("Trusted Type toString"))?;
    let key =
        v8::String::new(scope, "toString").ok_or(V8BindingError::Allocation("toString name"))?;
    object.create_data_property(scope, key.into(), function.into());
    Ok(object)
}

fn set_generated_prototype(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, interface)
        .ok_or(V8BindingError::Allocation("generated interface name"))?;
    let constructor = context
        .global(scope)
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation(
            "generated interface constructor",
        ))?;
    let prototype_name =
        v8::String::new(scope, "prototype").ok_or(V8BindingError::Allocation("prototype name"))?;
    let prototype = constructor
        .get(scope, prototype_name.into())
        .ok_or(V8BindingError::Allocation("generated interface prototype"))?;
    object.set_prototype(scope, prototype);
    Ok(())
}

fn trusted_value_to_string(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(id) = exact_u64(args.data()) else {
        return;
    };
    if let Some(value) = state.trusted_values.borrow().get(&id)
        && let Some(value) = v8::String::new(scope, value.value())
    {
        return_value.set(value.into());
    }
}

fn trusted_is_html(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::Html);
}
fn trusted_is_script(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::Script);
}
fn trusted_is_script_url(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_is(scope, args, return_value, TrustedKind::ScriptUrl);
}

fn trusted_is(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
    kind: TrustedKind,
) {
    let matches = trusted_value_id(scope, args.get(0), kind).is_some();
    return_value.set(v8::Boolean::new(scope, matches).into());
}

fn trusted_get_attribute_type(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let tag = args.get(0).to_rust_string_lossy(scope).to_ascii_lowercase();
    let attribute = args.get(1).to_rust_string_lossy(scope).to_ascii_lowercase();
    let kind = if tag == "script" && attribute == "src" {
        Some(TrustedKind::ScriptUrl)
    } else if tag == "iframe" && attribute == "srcdoc" {
        Some(TrustedKind::Html)
    } else if attribute.starts_with("on") {
        Some(TrustedKind::Script)
    } else {
        None
    };
    if let Some(kind) = kind {
        return_value.set(
            v8::String::new(scope, kind.interface_name())
                .unwrap()
                .into(),
        );
    } else {
        return_value.set(v8::null(scope).into());
    }
}

fn trusted_get_property_type(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    return_value: v8::ReturnValue,
) {
    trusted_get_attribute_type(scope, args, return_value)
}

fn trusted_value_id(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    kind: TrustedKind,
) -> Option<u64> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if object.internal_field_count() < 2 {
        return None;
    }
    let tag = object.get_internal_field(scope, 0)?;
    let tag = v8::Local::<v8::Value>::try_from(tag)
        .ok()?
        .int32_value(scope)? as u64;
    (tag == kind_token(kind))
        .then(|| {
            object
                .get_internal_field(scope, 1)
                .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
                .and_then(exact_u64)
        })
        .flatten()
}

fn exact_u64(value: v8::Local<v8::Value>) -> Option<u64> {
    let bigint = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = bigint.u64_value();
    lossless.then_some(value)
}

const fn kind_token(kind: TrustedKind) -> u64 {
    match kind {
        TrustedKind::Html => 1,
        TrustedKind::Script => 2,
        TrustedKind::ScriptUrl => 3,
    }
}

fn get_computed_style_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let node = argument_node(scope, &state, args.get(0))?;
        let key = node_key(node);
        let styles = state
            .layout
            .borrow()
            .styles
            .get(&key)
            .cloned()
            .unwrap_or_default();
        let object = v8::Object::new(scope);
        for (name, value) in styles {
            let Some(name) = v8::String::new(scope, &name) else {
                continue;
            };
            let Some(value) = v8::String::new(scope, &value) else {
                continue;
            };
            object.create_data_property(scope, name.into(), value.into());
        }
        Ok(object)
    })();
    match result {
        Ok(object) => return_value.set(object.into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn node_key(node: NodeId) -> u64 {
    (u64::from(node.generation) << 32) | u64::from(node.slot)
}

fn member_token(interface: usize, member: usize) -> i32 {
    ((interface as i32) << 16) | member as i32
}

fn member_from_token(token: i32) -> Option<(&'static str, &'static str)> {
    let interface = GENERATED_INTERFACES.get(((token as u32) >> 16) as usize)?;
    let member = interface.members.get((token as u32 & 0xffff) as usize)?;
    Some((interface.name, member.name))
}

fn generated_constructor(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _return_value: v8::ReturnValue,
) {
    let interface = args
        .data()
        .int32_value(scope)
        .and_then(|index| GENERATED_INTERFACES.get(index as usize));
    let name = interface
        .map(|interface| interface.name)
        .unwrap_or("interface");
    if matches!(name, "Event" | "CustomEvent") && args.is_construct_call() {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let event_type = args.get(0).to_rust_string_lossy(scope);
        let init_value = args.get(1);
        let init = (!init_value.is_null_or_undefined())
            .then(|| init_value.to_object(scope))
            .flatten();
        let bubbles = init
            .and_then(|init| {
                let key = v8::String::new(scope, "bubbles")?;
                init.get(scope, key.into())
            })
            .is_some_and(|value| value.boolean_value(scope));
        let cancelable = init
            .and_then(|init| {
                let key = v8::String::new(scope, "cancelable")?;
                init.get(scope, key.into())
            })
            .is_some_and(|value| value.boolean_value(scope));
        let event_id = state.next_event_id.get();
        state.next_event_id.set(event_id.saturating_add(1));
        state.events.borrow_mut().insert(
            event_id,
            EventRecord {
                event_type,
                target: None,
                current_target: None,
                bubbles,
                cancelable,
                default_prevented: false,
                propagation_stopped: false,
                immediate_stopped: false,
            },
        );
        let id = v8::Number::new(scope, event_id as f64);
        args.this().set_internal_field(0, id.into());
        if name == "CustomEvent" {
            let detail = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "detail")?;
                    init.get(scope, key.into())
                })
                .unwrap_or_else(|| v8::null(scope).into());
            if let Some(key) = v8::String::new(scope, "detail") {
                args.this().create_data_property(scope, key.into(), detail);
            }
            let composed = init
                .and_then(|init| {
                    let key = v8::String::new(scope, "composed")?;
                    init.get(scope, key.into())
                })
                .is_some_and(|value| value.boolean_value(scope));
            if let Some(key) = v8::String::new(scope, "composed") {
                let value = v8::Boolean::new(scope, composed);
                args.this()
                    .create_data_property(scope, key.into(), value.into());
            }
        }
        return;
    }
    if name == "MutationObserver" && args.is_construct_call() {
        let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
            throw_type_error(scope, "generated binding state is unavailable");
            return;
        };
        let Ok(callback) = v8::Local::<v8::Function>::try_from(args.get(0)) else {
            throw_type_error(scope, "MutationObserver callback must be callable");
            return;
        };
        let observer_id = state.next_mutation_observer_id.get();
        state
            .next_mutation_observer_id
            .set(observer_id.saturating_add(1));
        state.mutation_observers.borrow_mut().insert(
            observer_id,
            MutationObserverEntry {
                callback: v8::Global::new(scope, callback),
                object: v8::Global::new(scope, args.this()),
                target: None,
                options: MutationObserverOptions::default(),
                records: Vec::new(),
            },
        );
        let id = v8::Number::new(scope, observer_id as f64);
        args.this().set_internal_field(0, id.into());
        return;
    }
    // Custom-element upgrade uses Reflect.construct(definition, [],
    // definition) to execute the user constructor and then transfers its own
    // initialized fields onto the existing Rust-backed element wrapper. Its
    // `super()` reaches this generated constructor; the temporary shell never
    // escapes and deliberately receives no Rust node identity.
    if name == "HTMLElement" && args.is_construct_call() {
        return;
    }
    throw_type_error(scope, &format!("Illegal constructor: {name}"));
}

fn generated_getter(
    scope: &mut v8::PinScope,
    _name: v8::Local<v8::Name>,
    info: v8::PropertyCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some((interface, member)) = info.data().int32_value(scope).and_then(member_from_token)
    else {
        throw_type_error(scope, "invalid generated attribute token");
        return;
    };
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    if interface == "Event" {
        match get_event_attribute(scope, &state, info.holder(), member) {
            Ok(value) => return_value.set(value),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, info.holder(), interface)
        .and_then(|node| get_attribute_value(scope, &state, node, interface, member));
    match result {
        Ok(value) => return_value.set(value),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn generated_setter(
    scope: &mut v8::PinScope,
    _name: v8::Local<v8::Name>,
    value: v8::Local<v8::Value>,
    info: v8::PropertyCallbackArguments,
    _return_value: v8::ReturnValue<()>,
) {
    let Some((interface, member)) = info.data().int32_value(scope).and_then(member_from_token)
    else {
        throw_type_error(scope, "invalid generated attribute token");
        return;
    };
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    let result = resolve_receiver(scope, &state, info.holder(), interface)
        .and_then(|node| set_attribute_value(scope, &state, node, interface, member, value));
    if let Err(error) = result {
        throw_type_error(scope, &error.to_string());
    }
}

fn generated_operation(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut return_value: v8::ReturnValue,
) {
    let Some((interface, member)) = args.data().int32_value(scope).and_then(member_from_token)
    else {
        throw_type_error(scope, "invalid generated operation token");
        return;
    };
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        throw_type_error(scope, "generated binding state is unavailable");
        return;
    };
    if interface == "Event" {
        match call_event_operation(scope, &state, args.this(), member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    if interface == "MutationObserver" {
        match call_mutation_observer_operation(scope, &state, args.this(), member, &args) {
            Ok(Some(value)) => return_value.set(value),
            Ok(None) => return_value.set(v8::undefined(scope).into()),
            Err(error) => throw_type_error(scope, &error.to_string()),
        }
        return;
    }
    let result = resolve_receiver(scope, &state, args.this(), interface)
        .and_then(|node| call_operation(scope, &state, node, interface, member, &args));
    match result {
        Ok(Some(value)) => return_value.set(value),
        Ok(None) => return_value.set(v8::undefined(scope).into()),
        Err(error) => throw_type_error(scope, &error.to_string()),
    }
}

fn resolve_receiver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
    interface: &str,
) -> Result<NodeId, V8BindingError> {
    let value = object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .ok_or(BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    let wrapper =
        exact_wrapper_id(value).ok_or(BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    let brand = brand_for_interface(interface);
    let node = state.wrappers.borrow().resolve(
        &state.dom.borrow(),
        wrapper,
        state.realm,
        state.document_generation,
        brand,
    )?;
    validate_interface_node(&state.dom.borrow(), node, interface)?;
    Ok(node)
}

fn brand_for_interface(interface: &str) -> InterfaceBrand {
    match interface {
        "Document" => InterfaceBrand::Document,
        "Element" => InterfaceBrand::Element,
        interface if interface == "HTMLElement" || interface.starts_with("HTML") => {
            InterfaceBrand::HtmlElement
        }
        _ => InterfaceBrand::Node,
    }
}

fn validate_interface_node(
    dom: &DomRuntime,
    node: NodeId,
    interface: &str,
) -> Result<(), V8BindingError> {
    let expected_tag = match interface {
        "HTMLInputElement" => Some("input"),
        "HTMLTextAreaElement" => Some("textarea"),
        "HTMLSelectElement" => Some("select"),
        "HTMLOptionElement" => Some("option"),
        "HTMLButtonElement" => Some("button"),
        "HTMLFormElement" => Some("form"),
        "HTMLAnchorElement" => Some("a"),
        _ => None,
    };
    if let Some(expected_tag) = expected_tag {
        let NodeKind::Element { name } = &dom.arena().get(node)?.kind else {
            return Err(BindingError::WrongBrand {
                expected: InterfaceBrand::HtmlElement,
                actual: InterfaceBrand::Node,
            }
            .into());
        };
        if name.local.as_ref() != expected_tag {
            return Err(BindingError::WrongBrand {
                expected: InterfaceBrand::HtmlElement,
                actual: InterfaceBrand::HtmlElement,
            }
            .into());
        }
    }
    Ok(())
}

fn most_derived_brand(kind: &NodeKind) -> InterfaceBrand {
    match kind {
        NodeKind::Document => InterfaceBrand::Document,
        NodeKind::Element { .. } => InterfaceBrand::HtmlElement,
        _ => InterfaceBrand::Node,
    }
}

fn interface_for_node(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Document => "Document",
        NodeKind::Element { name } => match name.local.as_ref() {
            "input" => "HTMLInputElement",
            "textarea" => "HTMLTextAreaElement",
            "select" => "HTMLSelectElement",
            "option" => "HTMLOptionElement",
            "button" => "HTMLButtonElement",
            "form" => "HTMLFormElement",
            "a" => "HTMLAnchorElement",
            "iframe" => "HTMLIFrameElement",
            _ => "HTMLElement",
        },
        NodeKind::Text { .. } => "Text",
        NodeKind::Comment { .. } => "Comment",
        NodeKind::DocumentFragment => "DocumentFragment",
        NodeKind::ShadowRoot { .. } => "ShadowRoot",
        NodeKind::DocumentType { .. } => "Node",
    }
}

/// Return the realm-identity-preserving V8 wrapper for a Rust DOM node.
///
/// Hosts use this only to seed task-local globals before invoking generated
/// interface behavior; page code never receives a Rust pointer.
pub fn wrap_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let kind = state.dom.borrow().arena().get(node)?.kind.clone();
    let brand = most_derived_brand(&kind);
    let wrapper = {
        let mut dom = state.dom.borrow_mut();
        state.wrappers.borrow_mut().get_or_create(
            &mut dom,
            state.realm,
            state.document_generation,
            node,
            brand,
        )?
    };
    if let Some(object) = state.objects.borrow().get(&wrapper)
        && let Some(local) = object.handle.to_local(scope)
    {
        return Ok(local);
    }
    let interface = interface_for_node(&kind);
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .cloned()
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let template = v8::Local::new(scope, &template);
    let object = template
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("wrapper object"))?;
    let id = v8::BigInt::new_from_u64(scope, wrapper.0);
    object.set_internal_field(0, id.into());
    // Inline event-handler content attributes are reflected as properties on
    // the Rust-backed wrapper. Event dispatch already invokes the `on*`
    // property, which gives parser-created and fragment-created elements the
    // same receiver, event argument, and cancellation semantics as handlers
    // assigned from JavaScript. The Rust DOM remains the attribute authority;
    // this is only the realm-local compiled callback.
    let inline_handlers = state
        .dom
        .borrow()
        .arena()
        .get(node)?
        .attributes
        .iter()
        .filter(|(name, _)| name.starts_with("on"))
        .map(|(name, source)| (name.clone(), source.clone()))
        .collect::<Vec<_>>();
    for (name, source) in inline_handlers {
        let function_source = format!("(function(event){{{source}}})");
        let function_source = v8::String::new(scope, &function_source)
            .ok_or(V8BindingError::Allocation("inline event handler"))?;
        let function = v8::Script::compile(scope, function_source, None)
            .and_then(|script| script.run(scope))
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| {
                V8BindingError::EventCallback("inline event handler compilation failed".into())
            })?;
        let key = v8::String::new(scope, &name)
            .ok_or(V8BindingError::Allocation("inline event property"))?;
        if !object
            .set(scope, key.into(), function.into())
            .unwrap_or(false)
        {
            return Err(V8BindingError::Allocation("inline event property"));
        }
    }
    let registry: RcWeak<RefCell<WrapperRegistry>> = Rc::downgrade(&state.wrappers);
    let weak = v8::Weak::with_finalizer(
        scope,
        object,
        Box::new(move |_isolate| {
            if let Some(registry) = registry.upgrade() {
                registry.borrow_mut().enqueue_weak_release(wrapper);
            }
        }),
    );
    let connected_root = state
        .dom
        .borrow()
        .is_connected(node)?
        .then(|| v8::Global::new(scope, object));
    state.objects.borrow_mut().insert(
        wrapper,
        WrapperObject {
            handle: weak,
            connected_root,
        },
    );
    Ok(object)
}

fn receiver_wrapper_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Option<WrapperId> {
    let field = object.get_internal_field(scope, 0)?;
    let value = v8::Local::<v8::Value>::try_from(field).ok()?;
    exact_wrapper_id(value)
}

fn exact_wrapper_id(value: v8::Local<v8::Value>) -> Option<WrapperId> {
    let bigint = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (value, lossless) = bigint.u64_value();
    lossless.then_some(WrapperId(value))
}

fn mutation_observer_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as u64)
        .ok_or(V8BindingError::Allocation("MutationObserver identity"))
}

fn call_mutation_observer_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let observer_id = mutation_observer_id(scope, object)?;
    match member {
        "observe" => {
            let target = argument_node(scope, state, args.get(0))?;
            let init = args
                .get(1)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("MutationObserverInit"))?;
            let boolean = |scope: &mut v8::PinScope<'s, '_>, name: &str| {
                v8::String::new(scope, name)
                    .and_then(|key| init.get(scope, key.into()))
                    .is_some_and(|value| value.boolean_value(scope))
            };
            let options = MutationObserverOptions {
                child_list: boolean(scope, "childList"),
                attributes: boolean(scope, "attributes"),
                character_data: boolean(scope, "characterData"),
                subtree: boolean(scope, "subtree"),
            };
            if !options.child_list && !options.attributes && !options.character_data {
                return Err(V8BindingError::Unsupported {
                    interface: "MutationObserver".into(),
                    member: "observe requires childList, attributes, or characterData".into(),
                });
            }
            let mut observers = state.mutation_observers.borrow_mut();
            let observer = observers
                .get_mut(&observer_id)
                .ok_or(V8BindingError::Allocation("MutationObserver record"))?;
            observer.target = Some(target);
            observer.options = options;
            Ok(None)
        }
        "disconnect" => {
            if let Some(observer) = state.mutation_observers.borrow_mut().get_mut(&observer_id) {
                observer.target = None;
                observer.records.clear();
            }
            Ok(None)
        }
        "takeRecords" => {
            let records = state
                .mutation_observers
                .borrow_mut()
                .get_mut(&observer_id)
                .map(|observer| std::mem::take(&mut observer.records))
                .unwrap_or_default();
            Ok(Some(mutation_record_array(scope, state, &records)?.into()))
        }
        _ => Err(V8BindingError::Unsupported {
            interface: "MutationObserver".into(),
            member: member.into(),
        }),
    }
}

fn mutation_target(mutation: &Mutation) -> Option<NodeId> {
    match mutation {
        Mutation::Inserted { parent, .. } => *parent,
        Mutation::Removed { former_parent, .. } => *former_parent,
        Mutation::Attribute { node, .. }
        | Mutation::CharacterData { node, .. }
        | Mutation::FormState { node }
        | Mutation::EventListener { node, .. } => Some(*node),
        Mutation::Focus { current, previous } => current.or(*previous),
        Mutation::ShadowAttached { host, .. } => Some(*host),
    }
}

fn observer_accepts(
    dom: &DomRuntime,
    target: NodeId,
    options: MutationObserverOptions,
    mutation: &Mutation,
) -> bool {
    let Some(mutation_target) = mutation_target(mutation) else {
        return false;
    };
    let in_scope = mutation_target == target
        || (options.subtree
            && dom
                .arena()
                .ancestors(mutation_target)
                .is_ok_and(|ancestors| ancestors.contains(&target)));
    in_scope
        && match mutation {
            Mutation::Inserted { .. }
            | Mutation::Removed { .. }
            | Mutation::ShadowAttached { .. } => options.child_list,
            Mutation::Attribute { .. } | Mutation::FormState { .. } => options.attributes,
            Mutation::CharacterData { .. } => options.character_data,
            Mutation::Focus { .. } | Mutation::EventListener { .. } => false,
        }
}

fn set_object_property(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("property name"))?;
    object.create_data_property(scope, key.into(), value);
    Ok(())
}

fn mutation_record_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    records: &[Mutation],
) -> Result<v8::Local<'s, v8::Array>, V8BindingError> {
    let array = v8::Array::new(scope, records.len() as i32);
    for (index, mutation) in records.iter().enumerate() {
        let object = v8::Object::new(scope);
        let (kind, target, attribute, old_value, added, removed) = match mutation {
            Mutation::Inserted { node, parent } => {
                ("childList", *parent, None, None, Some(*node), None)
            }
            Mutation::Removed {
                node,
                former_parent,
            } => ("childList", *former_parent, None, None, None, Some(*node)),
            Mutation::Attribute {
                node,
                name,
                old_value,
                ..
            } => (
                "attributes",
                Some(*node),
                Some(name.as_str()),
                old_value.as_deref(),
                None,
                None,
            ),
            Mutation::CharacterData {
                node, old_value, ..
            } => (
                "characterData",
                Some(*node),
                None,
                Some(old_value.as_str()),
                None,
                None,
            ),
            Mutation::FormState { node } => {
                ("attributes", Some(*node), Some("value"), None, None, None)
            }
            Mutation::ShadowAttached { host, root, .. } => {
                ("childList", Some(*host), None, None, Some(*root), None)
            }
            Mutation::Focus { .. } | Mutation::EventListener { .. } => continue,
        };
        let kind_value = v8_string(scope, kind)?;
        set_object_property(scope, object, "type", kind_value)?;
        let target_value = match target {
            Some(node) => wrap_node(scope, state, node)?.into(),
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "target", target_value)?;
        let attribute_value = match attribute {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "attributeName", attribute_value)?;
        let old_value = match old_value {
            Some(value) => v8_string(scope, value)?,
            None => v8::null(scope).into(),
        };
        set_object_property(scope, object, "oldValue", old_value)?;
        let added_nodes = added.map_or_else(Vec::new, |node| vec![node]);
        let removed_nodes = removed.map_or_else(Vec::new, |node| vec![node]);
        let added_value = node_array(scope, state, &added_nodes)?;
        set_object_property(scope, object, "addedNodes", added_value)?;
        let removed_value = node_array(scope, state, &removed_nodes)?;
        set_object_property(scope, object, "removedNodes", removed_value)?;
        array.set_index(scope, index as u32, object.into());
    }
    Ok(array)
}

/// Queue and deliver one stable MutationObserver checkpoint for this realm.
pub fn deliver_mutation_observers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    mutations: &[Mutation],
) -> Result<usize, V8BindingError> {
    {
        let dom = state.dom.borrow();
        for observer in state.mutation_observers.borrow_mut().values_mut() {
            let Some(target) = observer.target else {
                continue;
            };
            observer.records.extend(
                mutations
                    .iter()
                    .filter(|mutation| observer_accepts(&dom, target, observer.options, mutation))
                    .cloned(),
            );
        }
    }
    let ids = state
        .mutation_observers
        .borrow()
        .iter()
        .filter_map(|(id, observer)| (!observer.records.is_empty()).then_some(*id))
        .collect::<Vec<_>>();
    let mut delivered = 0;
    for id in ids {
        let (callback, object, records) = {
            let mut observers = state.mutation_observers.borrow_mut();
            let observer = observers.get_mut(&id).expect("queued observer exists");
            (
                observer.callback.clone(),
                observer.object.clone(),
                std::mem::take(&mut observer.records),
            )
        };
        let callback = v8::Local::new(scope, &callback);
        let object = v8::Local::new(scope, &object);
        let array = mutation_record_array(scope, state, &records)?;
        callback.call(
            scope,
            v8::undefined(scope).into(),
            &[array.into(), object.into()],
        );
        delivered += 1;
    }
    Ok(delivered)
}

fn css_property_name(member: &str) -> &str {
    match member {
        "cssText" => "css-text",
        _ => member,
    }
}

fn inline_style_entries(style: &str) -> BTreeMap<String, String> {
    style
        .split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

fn set_inline_style_property(
    dom: &mut DomRuntime,
    node: NodeId,
    property: &str,
    value: &str,
) -> Result<String, DomError> {
    let current = dom
        .arena()
        .get(node)?
        .attributes
        .get("style")
        .cloned()
        .unwrap_or_default();
    let mut entries = inline_style_entries(&current);
    let previous = if value.is_empty() {
        entries.remove(property).unwrap_or_default()
    } else {
        entries
            .insert(property.to_ascii_lowercase(), value.to_string())
            .unwrap_or_default()
    };
    let serialized = entries
        .into_iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("; ");
    dom.set_attribute(node, "style", serialized)?;
    Ok(previous)
}

fn get_attribute_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if interface == "Element" && member == "classList" {
        return associated_wrapper(scope, state, node, "DOMTokenList");
    }
    if interface == "HTMLElement" && member == "style" {
        return associated_wrapper(scope, state, node, "CSSStyleDeclaration");
    }
    if interface == "HTMLElement" && member == "dataset" {
        let entries = state
            .dom
            .borrow()
            .arena()
            .get(node)?
            .attributes
            .iter()
            .filter_map(|(name, value)| {
                let suffix = name.strip_prefix("data-")?;
                let mut property = String::new();
                let mut uppercase = false;
                for character in suffix.chars() {
                    if character == '-' {
                        uppercase = true;
                    } else if uppercase {
                        property.extend(character.to_uppercase());
                        uppercase = false;
                    } else {
                        property.push(character);
                    }
                }
                Some((property, value.clone()))
            })
            .collect::<Vec<_>>();
        let dataset = v8::Object::new(scope);
        for (name, value) in entries {
            let name = v8::String::new(scope, &name)
                .ok_or(V8BindingError::Allocation("dataset property"))?;
            let value = v8_string(scope, &value)?;
            if !dataset.set(scope, name.into(), value).unwrap_or(false) {
                return Err(V8BindingError::Allocation("dataset property"));
            }
        }
        return Ok(dataset.into());
    }
    if interface == "Document" && member == "styleSheets" {
        let styles = state.dom.borrow().query_selector_all(node, "style")?;
        return associated_node_array(scope, state, &styles, "CSSStyleSheet");
    }
    if interface == "Document" && member == "defaultView" {
        return Ok(scope.get_current_context().global(scope).into());
    }
    if interface == "HTMLIFrameElement" && member == "contentDocument" {
        let document = state.frame_roots.borrow().get(&node).copied();
        return optional_node(scope, state, document);
    }
    if matches!(
        member,
        "parentNode" | "firstChild" | "lastChild" | "previousSibling" | "nextSibling"
    ) {
        let related = {
            let dom = state.dom.borrow();
            let record = dom.arena().get(node)?;
            match member {
                "parentNode" => record.parent,
                "firstChild" => record.children.first().copied(),
                "lastChild" => record.children.last().copied(),
                "previousSibling" | "nextSibling" => record.parent.and_then(|parent| {
                    let siblings = &dom.arena().get(parent).ok()?.children;
                    let index = siblings.iter().position(|candidate| *candidate == node)?;
                    if member == "previousSibling" {
                        index
                            .checked_sub(1)
                            .and_then(|index| siblings.get(index))
                            .copied()
                    } else {
                        siblings.get(index + 1).copied()
                    }
                }),
                _ => unreachable!(),
            }
        };
        return optional_node(scope, state, related);
    }
    if member == "childNodes" {
        let children = state.dom.borrow().arena().get(node)?.children.clone();
        return node_array(scope, state, &children);
    }
    if interface == "Document" && member == "documentElement" {
        let document_element = state.dom.borrow().document_element();
        return optional_node(scope, state, document_element);
    }
    if interface == "Node" && member == "ownerDocument" {
        let owner = {
            let dom = state.dom.borrow();
            (!matches!(dom.arena().get(node)?.kind, NodeKind::Document)).then_some(dom.document())
        };
        return optional_node(scope, state, owner);
    }
    if interface == "Element" && member == "shadowRoot" {
        let root = {
            let dom = state.dom.borrow();
            dom.arena()
                .get(node)?
                .children
                .iter()
                .copied()
                .find(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::ShadowRoot {
                            mode: ShadowMode::Open
                        })
                    )
                })
        };
        return optional_node(scope, state, root);
    }
    if interface == "ShadowRoot" && member == "host" {
        let host = state.dom.borrow().arena().get(node)?.parent;
        return optional_node(scope, state, host);
    }
    let dom = state.dom.borrow();
    let record = dom.arena().get(node)?;
    let value = match (interface, member) {
        (_, "nodeName") => v8_string(scope, &dom.node_name(node)?)?,
        (_, "nodeType") => v8::Integer::new(scope, node_type(&record.kind)).into(),
        ("Node", "nodeValue") => match &record.kind {
            NodeKind::Text { data } | NodeKind::Comment { data } => v8_string(scope, data)?,
            _ => v8::null(scope).into(),
        },
        (_, "textContent") | ("HTMLElement", "innerText") => {
            v8_string(scope, &dom.arena().text_content(node)?)?
        }
        ("Element", "tagName") => v8_string(scope, &dom.node_name(node)?)?,
        ("Element", "namespaceURI") => match &record.kind {
            NodeKind::Element { name } => v8_string(scope, name.ns.as_ref())?,
            _ => v8::null(scope).into(),
        },
        ("Element", "id") => v8_string(
            scope,
            record.attributes.get("id").map_or("", String::as_str),
        )?,
        ("Element", "className") => v8_string(
            scope,
            record.attributes.get("class").map_or("", String::as_str),
        )?,
        ("DOMTokenList", "value") => v8_string(
            scope,
            record.attributes.get("class").map_or("", String::as_str),
        )?,
        ("CSSStyleDeclaration", "cssText") => v8_string(
            scope,
            record.attributes.get("style").map_or("", String::as_str),
        )?,
        ("CSSStyleDeclaration", member) => {
            let property = css_property_name(member);
            let styles =
                inline_style_entries(record.attributes.get("style").map_or("", String::as_str));
            v8_string(scope, styles.get(property).map_or("", String::as_str))?
        }
        ("Element" | "ShadowRoot", "innerHTML") => {
            let mut html = String::new();
            for child in &record.children {
                html.push_str(&dom.serialize(*child)?);
            }
            v8_string(scope, &html)?
        }
        ("Element", "outerHTML") => v8_string(scope, &dom.serialize(node)?)?,
        ("ShadowRoot", "mode") => match record.kind {
            NodeKind::ShadowRoot {
                mode: ShadowMode::Open,
            } => v8_string(scope, "open")?,
            NodeKind::ShadowRoot {
                mode: ShadowMode::Closed,
            } => v8_string(scope, "closed")?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        ("HTMLSelectElement", "value") => {
            let options = dom.query_selector_all(node, "option")?;
            let option = options
                .iter()
                .copied()
                .find(|option| {
                    dom.arena()
                        .get(*option)
                        .is_ok_and(|record| record.attributes.contains_key("selected"))
                })
                .or_else(|| options.first().copied());
            let value = option
                .and_then(|option| dom.arena().get(option).ok().map(|record| (option, record)))
                .map(|(option, record)| {
                    record
                        .attributes
                        .get("value")
                        .cloned()
                        .unwrap_or_else(|| dom.arena().text_content(option).unwrap_or_default())
                })
                .unwrap_or_default();
            v8_string(scope, &value)?
        }
        (interface, "value") if interface.starts_with("HTML") => v8_string(
            scope,
            dom.value(node)?
                .or_else(|| record.attributes.get("value").map(String::as_str))
                .unwrap_or(""),
        )?,
        (interface, "checked" | "selected" | "disabled") if interface.starts_with("HTML") => {
            v8::Boolean::new(scope, record.attributes.contains_key(member)).into()
        }
        (interface, "type" | "name" | "action" | "method" | "href" | "target")
            if interface.starts_with("HTML") =>
        {
            let fallback = match member {
                "method" => "get",
                "type" if interface == "HTMLButtonElement" => "submit",
                "type" if interface == "HTMLInputElement" => "text",
                _ => "",
            };
            v8_string(
                scope,
                record
                    .attributes
                    .get(member)
                    .map_or(fallback, String::as_str),
            )?
        }
        ("HTMLSelectElement", "selectedIndex") => {
            let options = dom.query_selector_all(node, "option")?;
            let index = options
                .iter()
                .position(|option| {
                    dom.arena()
                        .get(*option)
                        .is_ok_and(|record| record.attributes.contains_key("selected"))
                })
                .map(|index| index as i32)
                .unwrap_or_else(|| if options.is_empty() { -1 } else { 0 });
            v8::Integer::new(scope, index).into()
        }
        ("Text" | "Comment", "data") => match &record.kind {
            NodeKind::Text { data } | NodeKind::Comment { data } => v8_string(scope, data)?,
            _ => return Err(DomError::StaleNode(node).into()),
        },
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    };
    Ok(value)
}

fn set_attribute_value(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let value = if matches!(
        (interface, member),
        ("Element", "innerHTML" | "outerHTML") | ("ShadowRoot", "innerHTML")
    ) {
        trusted_string(
            scope,
            state,
            value,
            TrustedKind::Html,
            &format!("{interface}.{member}"),
        )?
    } else if matches!(member, "textContent" | "innerText")
        && state
            .dom
            .borrow()
            .node_name(node)
            .is_ok_and(|name| name.eq_ignore_ascii_case("script"))
    {
        trusted_string(
            scope,
            state,
            value,
            TrustedKind::Script,
            "HTMLScriptElement.text",
        )?
    } else {
        value.to_rust_string_lossy(scope)
    };
    let mut dom = state.dom.borrow_mut();
    match (interface, member) {
        (_, "textContent") | ("HTMLElement", "innerText") => {
            dom.set_text_content(node, value)?;
            drop(dom);
            prepare_dynamic_script_subtree(scope, state, node)?;
            return Ok(());
        }
        ("Node", "nodeValue") => {
            if matches!(
                dom.arena().get(node)?.kind,
                NodeKind::Text { .. } | NodeKind::Comment { .. }
            ) {
                dom.set_text(node, value)?;
            }
        }
        ("Element", "id") => dom.set_attribute(node, "id", value)?,
        ("Element", "className") => dom.set_attribute(node, "class", value)?,
        ("DOMTokenList", "value") => dom.set_attribute(node, "class", value)?,
        ("CSSStyleDeclaration", "cssText") => dom.set_attribute(node, "style", value)?,
        ("CSSStyleDeclaration", member) => {
            set_inline_style_property(&mut dom, node, css_property_name(member), &value)?;
        }
        ("Text" | "Comment", "data") => dom.set_text(node, value)?,
        ("HTMLSelectElement", "value") => {
            let options = dom.query_selector_all(node, "option")?;
            for option in options {
                let matches = dom
                    .attribute(option, "value")?
                    .is_some_and(|candidate| candidate == value)
                    || dom
                        .arena()
                        .text_content(option)
                        .is_ok_and(|text| text.trim() == value);
                if matches {
                    dom.set_attribute(option, "selected", "")?;
                } else {
                    dom.remove_attribute(option, "selected")?;
                }
            }
        }
        (interface, "value") if interface.starts_with("HTML") => {
            dom.set_value(node, value, false)?
        }
        (interface, "checked" | "selected" | "disabled") if interface.starts_with("HTML") => {
            if value == "true" {
                dom.set_attribute(node, member, "")?;
            } else {
                dom.remove_attribute(node, member)?;
            }
        }
        (interface, "type" | "name" | "action" | "method" | "href" | "target")
            if interface.starts_with("HTML") =>
        {
            dom.set_attribute(node, member, value)?
        }
        ("HTMLSelectElement", "selectedIndex") => {
            let index = value.parse::<isize>().unwrap_or(-1);
            let options = dom.query_selector_all(node, "option")?;
            for (option_index, option) in options.into_iter().enumerate() {
                if option_index as isize == index {
                    dom.set_attribute(option, "selected", "")?;
                } else {
                    dom.remove_attribute(option, "selected")?;
                }
            }
        }
        ("Element" | "ShadowRoot", "innerHTML") => {
            dom.set_inner_html(node, &value)?;
            drop(dom);
            // HTML fragment parsing marks script elements already-started;
            // moving the resulting subtree into a document must not execute
            // them later.
            state.mark_script_subtree_inert(node)?;
            upgrade_custom_element_subtree(scope, state, node)?;
            return Ok(());
        }
        ("Element", "outerHTML") => dom.set_outer_html(node, &value)?,
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    }
    Ok(())
}

fn upgrade_custom_element_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    let context = scope.get_current_context();
    let global = context.global(scope);
    let registry_key = v8::String::new(scope, "customElements")
        .ok_or(V8BindingError::Allocation("customElements key"))?;
    let Some(registry) = global
        .get(scope, registry_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Ok(());
    };
    let upgrade_key = v8::String::new(scope, "upgrade")
        .ok_or(V8BindingError::Allocation("customElements.upgrade key"))?;
    let Some(upgrade) = registry
        .get(scope, upgrade_key.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return Ok(());
    };
    let root = wrap_node(scope, state, root)?;
    upgrade
        .call(scope, registry.into(), &[root.into()])
        .ok_or_else(|| {
            V8BindingError::EventCallback("customElements.upgrade raised an exception".into())
        })?;
    Ok(())
}

fn call_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    match (interface, member) {
        ("EventTarget", "addEventListener") => {
            let event_type = required_string(scope, args, 0)?;
            if args.get(1).is_null_or_undefined() {
                return Ok(None);
            }
            let callback = v8::Local::<v8::Function>::try_from(args.get(1))
                .map_err(|_| V8BindingError::Allocation("event listener callback"))?;
            state
                .listeners
                .borrow_mut()
                .entry((node, event_type.clone()))
                .or_default()
                .push(EventListener {
                    callback: v8::Global::new(scope, callback),
                });
            state
                .dom
                .borrow_mut()
                .set_event_listener_presence(node, &event_type, true)?;
            Ok(None)
        }
        ("EventTarget", "removeEventListener") => {
            let event_type = required_string(scope, args, 0)?;
            let candidate = v8::Local::<v8::Function>::try_from(args.get(1)).ok();
            let is_click = event_type.eq_ignore_ascii_case("click");
            let mut listener_map = state.listeners.borrow_mut();
            if let Some(listeners) = listener_map.get_mut(&(node, event_type.clone())) {
                listeners.retain(|listener| {
                    candidate.is_none_or(|candidate| {
                        !v8::Local::new(scope, &listener.callback).strict_equals(candidate.into())
                    })
                });
            }
            let no_click_listeners = is_click
                && listener_map
                    .get(&(node, event_type))
                    .is_none_or(Vec::is_empty);
            drop(listener_map);
            if no_click_listeners {
                state
                    .dom
                    .borrow_mut()
                    .set_event_listener_presence(node, "click", false)?;
            }
            Ok(None)
        }
        ("EventTarget", "dispatchEvent") => {
            let event = v8::Local::<v8::Object>::try_from(args.get(0))
                .map_err(|_| V8BindingError::Allocation("event object"))?;
            let accepted = dispatch_event(scope, state, node, event)?;
            Ok(Some(v8::Boolean::new(scope, accepted).into()))
        }
        ("Document", "createElement") => {
            let name = required_string(scope, args, 0)?;
            let created = state
                .dom
                .borrow_mut()
                .create_element(&name.to_ascii_lowercase());
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createTextNode") => {
            let data = required_string(scope, args, 0)?;
            let created = state.dom.borrow_mut().create_text(data);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createComment") => {
            let data = required_string(scope, args, 0)?;
            let created = state.dom.borrow_mut().create_comment(data);
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createDocumentFragment") => {
            let created = state.dom.borrow_mut().create_fragment();
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document", "createEvent") => {
            let event = create_event_object(scope, state, "", false, false)?;
            Ok(Some(event.into()))
        }
        ("Element", "attachShadow") => {
            let options = args
                .get(0)
                .to_object(scope)
                .ok_or(V8BindingError::Allocation("ShadowRootInit"))?;
            let mode_key = v8::String::new(scope, "mode")
                .ok_or(V8BindingError::Allocation("ShadowRootInit.mode"))?;
            let mode = options
                .get(scope, mode_key.into())
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "open".into());
            let mode = if mode.eq_ignore_ascii_case("closed") {
                ShadowMode::Closed
            } else {
                ShadowMode::Open
            };
            let created = {
                let mut dom = state.dom.borrow_mut();
                if dom.arena().get(node)?.children.iter().any(|child| {
                    matches!(
                        dom.arena().get(*child).ok().map(|record| &record.kind),
                        Some(NodeKind::ShadowRoot { .. })
                    )
                }) {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: "attachShadow called more than once".into(),
                    });
                }
                let root = dom.create(NodeKind::ShadowRoot { mode });
                dom.append_child(node, root)?;
                root
            };
            Ok(Some(wrap_node(scope, state, created)?.into()))
        }
        ("Document" | "ShadowRoot", "getElementById") => {
            let id = required_string(scope, args, 0)?;
            let found = if interface == "Document" {
                state.dom.borrow().find_by_id(&id)
            } else {
                state.dom.borrow().query_selector(node, &format!("#{id}"))?
            };
            optional_node(scope, state, found).map(Some)
        }
        ("Document", "open") => {
            state.document_open.set(true);
            state.document_write_anchor.set(None);
            state.document_write_buffer.borrow_mut().clear();
            state.dom.borrow_mut().replace_document_contents("")?;
            Ok(Some(wrap_node(scope, state, node)?.into()))
        }
        ("Document", "write" | "writeln") => {
            let mut markup = trusted_string(
                scope,
                state,
                args.get(0),
                TrustedKind::Html,
                &format!("Document.{member}"),
            )?;
            if member == "writeln" {
                markup.push('\n');
            }
            if markup.len() > 1024 * 1024 {
                return Err(V8BindingError::DocumentWriteLimit {
                    resource: "input size",
                });
            }
            if state.document_open.get() {
                if state
                    .document_write_buffer
                    .borrow()
                    .len()
                    .saturating_add(markup.len())
                    > 1024 * 1024
                {
                    return Err(V8BindingError::DocumentWriteLimit {
                        resource: "buffer size",
                    });
                }
                state.document_write_buffer.borrow_mut().push_str(&markup);
            } else if let Some(anchor) = state.document_write_anchor.get() {
                let inserted = state.dom.borrow_mut().insert_html_after(anchor, &markup)?;
                if let Some(last) = inserted.last().copied() {
                    state.document_write_anchor.set(Some(last));
                }
                execute_parser_inserted_scripts(scope, state, &inserted)?;
            } else {
                let document = state.dom.borrow().document();
                let current = state.dom.borrow().serialize(document)?;
                state
                    .dom
                    .borrow_mut()
                    .replace_document_contents(&(current + &markup))?;
            }
            Ok(None)
        }
        ("Document", "close") => {
            if state.document_open.replace(false) {
                let markup = std::mem::take(&mut *state.document_write_buffer.borrow_mut());
                state.dom.borrow_mut().replace_document_contents(&markup)?;
            }
            Ok(None)
        }
        (_, "appendChild") => {
            let child = argument_node(scope, state, args.get(0))?;
            state.dom.borrow_mut().append_child(node, child)?;
            prepare_dynamic_script_subtree(scope, state, child)?;
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "insertBefore") => {
            let child = argument_node(scope, state, args.get(0))?;
            let reference = optional_argument_node(scope, state, args.get(1))?;
            state
                .dom
                .borrow_mut()
                .insert_before(node, child, reference)?;
            prepare_dynamic_script_subtree(scope, state, child)?;
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "removeChild") => {
            let child = argument_node(scope, state, args.get(0))?;
            state.dom.borrow_mut().remove_child(node, child)?;
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "replaceChild") => {
            let replacement = argument_node(scope, state, args.get(0))?;
            let child = argument_node(scope, state, args.get(1))?;
            state
                .dom
                .borrow_mut()
                .replace_child(node, replacement, child)?;
            prepare_dynamic_script_subtree(scope, state, replacement)?;
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "cloneNode") => {
            let deep = args.get(0).boolean_value(scope);
            let clone = state.dom.borrow_mut().clone_node(node, deep)?;
            Ok(Some(wrap_node(scope, state, clone)?.into()))
        }
        ("Node", "contains") => {
            let Some(other) = optional_argument_node(scope, state, args.get(0))? else {
                return Ok(Some(v8::Boolean::new(scope, false).into()));
            };
            let contains =
                node == other || state.dom.borrow().arena().ancestors(other)?.contains(&node);
            Ok(Some(v8::Boolean::new(scope, contains).into()))
        }
        ("Element", "getAttribute") => {
            let name = required_string(scope, args, 0)?;
            match state.dom.borrow().attribute(node, &name)? {
                Some(value) => Ok(Some(v8_string(scope, value)?)),
                None => Ok(Some(v8::null(scope).into())),
            }
        }
        ("Element", "setAttribute") => {
            let name = required_string(scope, args, 0)?;
            let tag = state.dom.borrow().node_name(node)?.to_ascii_lowercase();
            let required = if tag == "script" && name.eq_ignore_ascii_case("src") {
                Some(TrustedKind::ScriptUrl)
            } else if tag == "iframe" && name.eq_ignore_ascii_case("srcdoc") {
                Some(TrustedKind::Html)
            } else if name.to_ascii_lowercase().starts_with("on") {
                Some(TrustedKind::Script)
            } else {
                None
            };
            let value = match required {
                Some(kind) => trusted_string(
                    scope,
                    state,
                    args.get(1),
                    kind,
                    &format!("Element.setAttribute({name})"),
                )?,
                None => required_string(scope, args, 1)?,
            };
            state.dom.borrow_mut().set_attribute(node, name, value)?;
            prepare_dynamic_script_subtree(scope, state, node)?;
            Ok(None)
        }
        ("Element", "removeAttribute") => {
            let name = required_string(scope, args, 0)?;
            state.dom.borrow_mut().remove_attribute(node, &name)?;
            Ok(None)
        }
        ("Element", "hasAttribute") => {
            let name = required_string(scope, args, 0)?;
            let present = state.dom.borrow().attribute(node, &name)?.is_some();
            Ok(Some(v8::Boolean::new(scope, present).into()))
        }
        (_, "querySelector") => {
            let selector = required_string(scope, args, 0)?;
            let found = state.dom.borrow().query_selector(node, &selector)?;
            optional_node(scope, state, found).map(Some)
        }
        (_, "querySelectorAll") => {
            let selector = required_string(scope, args, 0)?;
            let found = state.dom.borrow().query_selector_all(node, &selector)?;
            node_array(scope, state, &found).map(Some)
        }
        ("Element", "remove") => {
            state.dom.borrow_mut().remove(node)?;
            Ok(None)
        }
        ("Element", "matches") => {
            let selector = required_string(scope, args, 0)?;
            let matched = state.dom.borrow().matches_selector(node, &selector)?;
            Ok(Some(v8::Boolean::new(scope, matched).into()))
        }
        ("Element", "closest") => {
            let selector = required_string(scope, args, 0)?;
            let mut current = Some(node);
            while let Some(candidate) = current {
                if state.dom.borrow().matches_selector(candidate, &selector)? {
                    return Ok(Some(wrap_node(scope, state, candidate)?.into()));
                }
                current = state.dom.borrow().parent(candidate)?;
            }
            Ok(Some(v8::null(scope).into()))
        }
        ("Element", "getBoundingClientRect") => {
            let layout = state.layout.borrow();
            let rect = match layout.geometry.get(&node_key(node)) {
                Some(BindingGeometry::Available(rect)) => *rect,
                Some(BindingGeometry::Hidden) => BindingRect::default(),
                Some(BindingGeometry::Unsupported(detail)) => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member: format!("getBoundingClientRect: {detail}"),
                    });
                }
                None => {
                    return Err(V8BindingError::Unsupported {
                        interface: "Element".into(),
                        member:
                            "getBoundingClientRect: geometry is not available at this checkpoint"
                                .into(),
                    });
                }
            };
            let object = v8::Object::new(scope);
            for (name, value) in [
                ("x", rect.x),
                ("y", rect.y),
                ("width", rect.width),
                ("height", rect.height),
                ("top", rect.y),
                ("right", rect.x + rect.width),
                ("bottom", rect.y + rect.height),
                ("left", rect.x),
            ] {
                let name = v8::String::new(scope, name)
                    .ok_or(V8BindingError::Allocation("DOMRect property"))?;
                object.create_data_property(
                    scope,
                    name.into(),
                    v8::Number::new(scope, value).into(),
                );
            }
            Ok(Some(object.into()))
        }
        ("DOMTokenList", "add") => {
            let token = required_string(scope, args, 0)?;
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(true))?;
            Ok(None)
        }
        ("DOMTokenList", "remove") => {
            let token = required_string(scope, args, 0)?;
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(false))?;
            Ok(None)
        }
        ("DOMTokenList", "contains") => {
            let token = required_string(scope, args, 0)?;
            let present = class_tokens(&state.dom.borrow(), node)?.any(|item| item == token);
            Ok(Some(v8::Boolean::new(scope, present).into()))
        }
        ("DOMTokenList", "toggle") => {
            let token = required_string(scope, args, 0)?;
            let current = class_tokens(&state.dom.borrow(), node)?.any(|item| item == token);
            let next = if args.length() > 1 {
                args.get(1).boolean_value(scope)
            } else {
                !current
            };
            update_class_token(&mut state.dom.borrow_mut(), node, &token, Some(next))?;
            Ok(Some(v8::Boolean::new(scope, next).into()))
        }
        ("CSSStyleSheet", "insertRule") => {
            let rule = required_string(scope, args, 0)?;
            let current = state.dom.borrow().arena().text_content(node)?;
            let index = current.matches('}').count();
            let combined = if current.trim().is_empty() {
                rule
            } else {
                format!("{current}\n{rule}")
            };
            state.dom.borrow_mut().set_text_content(node, combined)?;
            Ok(Some(
                v8::Integer::new_from_unsigned(scope, index as u32).into(),
            ))
        }
        ("CSSStyleDeclaration", "getPropertyValue") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let dom = state.dom.borrow();
            let style = dom
                .arena()
                .get(node)?
                .attributes
                .get("style")
                .cloned()
                .unwrap_or_default();
            let value = inline_style_entries(&style)
                .remove(&property)
                .unwrap_or_default();
            Ok(Some(v8_string(scope, &value)?))
        }
        ("CSSStyleDeclaration", "setProperty") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let value = required_string(scope, args, 1)?;
            set_inline_style_property(&mut state.dom.borrow_mut(), node, &property, &value)?;
            Ok(None)
        }
        ("CSSStyleDeclaration", "removeProperty") => {
            let property = required_string(scope, args, 0)?.to_ascii_lowercase();
            let previous =
                set_inline_style_property(&mut state.dom.borrow_mut(), node, &property, "")?;
            Ok(Some(v8_string(scope, &previous)?))
        }
        ("HTMLElement", "focus") => {
            state.dom.borrow_mut().focus(Some(node))?;
            Ok(None)
        }
        ("HTMLElement", "click") => {
            let event = create_event_object(scope, state, "click", true, true)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        ("HTMLFormElement", "checkValidity" | "reportValidity") => {
            let valid = form_is_valid(&state.dom.borrow(), node)?;
            Ok(Some(v8::Boolean::new(scope, valid).into()))
        }
        ("HTMLFormElement", "requestSubmit" | "submit") => {
            let event = create_event_object(scope, state, "submit", true, true)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        ("HTMLFormElement", "reset") => {
            let controls = {
                let dom = state.dom.borrow();
                ["input", "textarea"]
                    .into_iter()
                    .flat_map(|selector| dom.query_selector_all(node, selector).unwrap_or_default())
                    .map(|control| {
                        let record = dom.arena().get(control)?;
                        let value = if matches!(&record.kind, NodeKind::Element { name } if name.local.as_ref() == "textarea") {
                            dom.arena().text_content(control).unwrap_or_default()
                        } else {
                            record.attributes.get("value").cloned().unwrap_or_default()
                        };
                        Ok((control, value))
                    })
                    .collect::<Result<Vec<_>, DomError>>()?
            };
            let mut dom = state.dom.borrow_mut();
            for (control, value) in controls {
                dom.set_value(control, value, false)?;
            }
            drop(dom);
            let event = create_event_object(scope, state, "reset", true, false)?;
            dispatch_event(scope, state, node, event)?;
            Ok(None)
        }
        _ => Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: member.into(),
        }),
    }
}

fn trusted_string(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    required: TrustedKind,
    sink: &str,
) -> Result<String, V8BindingError> {
    if !state.trusted_registry.borrow().requires(required) {
        return Ok(value.to_rust_string_lossy(scope));
    }
    if let Some(id) = trusted_value_id(scope, value, required)
        && let Some(value) = state.trusted_values.borrow().get(&id)
    {
        return Ok(value.value().to_string());
    }
    let default_callback = state
        .trusted_policies
        .borrow()
        .values()
        .find(|policy| policy.name == "default")
        .and_then(|policy| match required {
            TrustedKind::Html => policy.create_html.clone(),
            TrustedKind::Script => policy.create_script.clone(),
            TrustedKind::ScriptUrl => policy.create_script_url.clone(),
        });
    if let Some(callback) = default_callback {
        let callback = v8::Local::new(scope, &callback);
        let undefined = v8::undefined(scope).into();
        if let Some(result) = callback.call(scope, undefined, &[value]) {
            return Ok(result.to_rust_string_lossy(scope));
        }
    }
    Err(V8BindingError::Unsupported {
        interface: required.interface_name().into(),
        member: format!("required by {sink}"),
    })
}

fn execute_parser_inserted_scripts(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    roots: &[NodeId],
) -> Result<(), V8BindingError> {
    if state.document_write_depth.get() >= 32 {
        return Err(V8BindingError::DocumentWriteLimit {
            resource: "nested script depth",
        });
    }
    let scripts = {
        let dom = state.dom.borrow();
        let mut scripts = Vec::new();
        for root in roots {
            if dom.node_name(*root).is_ok_and(|name| name == "SCRIPT") {
                scripts.push(*root);
            }
            scripts.extend(dom.query_selector_all(*root, "script")?);
        }
        scripts
            .into_iter()
            .map(|script| {
                dom.arena()
                    .text_content(script)
                    .map(|source| (script, source))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    for (script, source) in scripts {
        let previous_anchor = state.document_write_anchor.replace(Some(script));
        state
            .document_write_depth
            .set(state.document_write_depth.get().saturating_add(1));
        let result: Result<(), V8BindingError> = (|| {
            let source = v8::String::new(scope, &source)
                .ok_or(V8BindingError::Allocation("document.write script source"))?;
            let script = v8::Script::compile(scope, source, None)
                .ok_or(V8BindingError::Allocation("document.write script"))?;
            script
                .run(scope)
                .ok_or(V8BindingError::Allocation("document.write script result"))?;
            Ok(())
        })();
        state
            .document_write_depth
            .set(state.document_write_depth.get().saturating_sub(1));
        state.document_write_anchor.set(previous_anchor);
        result?;
    }
    Ok(())
}

/// Run connected inline classic scripts synchronously at the DOM insertion
/// boundary and enqueue external/module scripts for the page resource
/// scheduler. This preserves the observable `appendChild(script)` ordering
/// without granting the bindings layer any network authority.
fn prepare_dynamic_script_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), V8BindingError> {
    for script in state.dynamic_script_candidates(root)? {
        if state.prepared_scripts.borrow().contains(&script) {
            continue;
        }
        let (script_type, external, source) = {
            let dom = state.dom.borrow();
            (
                dom.attribute(script, "type")?
                    .unwrap_or_default()
                    .to_owned(),
                dom.attribute(script, "src")?.map(str::to_owned),
                dom.arena().text_content(script)?,
            )
        };
        let kind = script_type.trim().to_ascii_lowercase();
        let classic = matches!(
            kind.as_str(),
            "" | "text/javascript" | "application/javascript" | "text/babel"
        );
        let module = kind == "module";
        if !classic && !module {
            state.prepared_scripts.borrow_mut().insert(script);
            continue;
        }
        state.prepared_scripts.borrow_mut().insert(script);
        if external.is_some() || module {
            state.pending_dynamic_scripts.borrow_mut().push_back(script);
            continue;
        }
        if source.trim().is_empty() {
            continue;
        }
        let source = v8::String::new(scope, &source)
            .ok_or(V8BindingError::Allocation("dynamic script source"))?;
        let compiled = v8::Script::compile(scope, source, None)
            .ok_or(V8BindingError::Allocation("dynamic script"))?;
        compiled
            .run(scope)
            .ok_or(V8BindingError::Allocation("dynamic script result"))?;
        scope.perform_microtask_checkpoint();
    }
    Ok(())
}

fn associated_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: NodeId,
    interface: &'static str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let wrapper = state.wrappers.borrow_mut().get_or_create(
        &mut state.dom.borrow_mut(),
        state.realm,
        state.document_generation,
        node,
        InterfaceBrand::Node,
    )?;
    let template = state
        .instance_templates
        .borrow()
        .get(interface)
        .cloned()
        .ok_or(V8BindingError::MissingInterface(interface))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("associated wrapper object"))?;
    object.set_internal_field(0, v8::BigInt::new_from_u64(scope, wrapper.0).into());
    Ok(object.into())
}

fn associated_node_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    nodes: &[NodeId],
    interface: &'static str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let values = nodes
        .iter()
        .map(|node| associated_wrapper(scope, state, *node, interface))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v8::Array::new_with_elements(scope, &values).into())
}

fn class_tokens(dom: &DomRuntime, node: NodeId) -> Result<impl Iterator<Item = &str>, DomError> {
    Ok(dom
        .attribute(node, "class")?
        .unwrap_or_default()
        .split_ascii_whitespace())
}

fn update_class_token(
    dom: &mut DomRuntime,
    node: NodeId,
    token: &str,
    force: Option<bool>,
) -> Result<(), DomError> {
    let mut tokens = class_tokens(dom, node)?
        .map(str::to_string)
        .collect::<Vec<_>>();
    let present = tokens.iter().any(|item| item == token);
    match force {
        Some(true) if !present => tokens.push(token.into()),
        Some(false) if present => tokens.retain(|item| item != token),
        _ => {}
    }
    dom.set_attribute(node, "class", tokens.join(" "))
}

fn form_is_valid(dom: &DomRuntime, form: NodeId) -> Result<bool, DomError> {
    for control in ["input", "textarea", "select"]
        .into_iter()
        .flat_map(|selector| dom.query_selector_all(form, selector).unwrap_or_default())
    {
        let record = dom.arena().get(control)?;
        if record.attributes.contains_key("required")
            && dom
                .value(control)?
                .or_else(|| record.attributes.get("value").map(String::as_str))
                .unwrap_or("")
                .is_empty()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn event_id(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
) -> Result<u64, V8BindingError> {
    object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(|value| value.integer_value(scope))
        .map(|value| value as u64)
        .ok_or(V8BindingError::Allocation("event identity"))
}

fn create_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    event_type: &str,
    bubbles: bool,
    cancelable: bool,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let template = state
        .instance_templates
        .borrow()
        .get("Event")
        .cloned()
        .ok_or(V8BindingError::MissingInterface("Event"))?;
    let object = v8::Local::new(scope, &template)
        .new_instance(scope)
        .ok_or(V8BindingError::Allocation("event object"))?;
    let id = state.next_event_id.get();
    state.next_event_id.set(id.saturating_add(1));
    state.events.borrow_mut().insert(
        id,
        EventRecord {
            event_type: event_type.into(),
            target: None,
            current_target: None,
            bubbles,
            cancelable,
            default_prevented: false,
            propagation_stopped: false,
            immediate_stopped: false,
        },
    );
    object.set_internal_field(0, v8::Number::new(scope, id as f64).into());
    Ok(object)
}

fn get_event_attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = event_id(scope, object)?;
    let event = state
        .events
        .borrow()
        .get(&id)
        .cloned()
        .ok_or(V8BindingError::Allocation("event record"))?;
    match member {
        "type" => v8_string(scope, &event.event_type),
        "target" => optional_node(scope, state, event.target),
        "currentTarget" => optional_node(scope, state, event.current_target),
        "bubbles" => Ok(v8::Boolean::new(scope, event.bubbles).into()),
        "cancelable" => Ok(v8::Boolean::new(scope, event.cancelable).into()),
        "defaultPrevented" => Ok(v8::Boolean::new(scope, event.default_prevented).into()),
        _ => Ok(v8::undefined(scope).into()),
    }
}

fn call_event_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = event_id(scope, object)?;
    let mut events = state.events.borrow_mut();
    let event = events
        .get_mut(&id)
        .ok_or(V8BindingError::Allocation("event record"))?;
    match member {
        "initEvent" => {
            event.event_type = required_string(scope, args, 0)?;
            event.bubbles = args.get(1).boolean_value(scope);
            event.cancelable = args.get(2).boolean_value(scope);
            event.default_prevented = false;
        }
        "preventDefault" if event.cancelable => event.default_prevented = true,
        "stopPropagation" => event.propagation_stopped = true,
        "stopImmediatePropagation" => {
            event.propagation_stopped = true;
            event.immediate_stopped = true;
        }
        _ => {}
    }
    Ok(None)
}

fn dispatch_event(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: NodeId,
    object: v8::Local<v8::Object>,
) -> Result<bool, V8BindingError> {
    v8::tc_scope!(let scope, scope);
    let id = event_id(scope, object)?;
    {
        let mut events = state.events.borrow_mut();
        let event = events
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation("event record"))?;
        event.target = Some(target);
        event.propagation_stopped = false;
        event.immediate_stopped = false;
    }
    let mut path = vec![target];
    if state
        .events
        .borrow()
        .get(&id)
        .is_some_and(|event| event.bubbles)
    {
        path.extend(state.dom.borrow().arena().ancestors(target)?);
    }
    for current in path {
        {
            let mut events = state.events.borrow_mut();
            let event = events.get_mut(&id).unwrap();
            if event.propagation_stopped && event.current_target.is_some() {
                break;
            }
            event.current_target = Some(current);
            event.immediate_stopped = false;
        }
        let event_type = state.events.borrow().get(&id).unwrap().event_type.clone();
        let callbacks = state
            .listeners
            .borrow()
            .get(&(current, event_type))
            .map(|listeners| {
                listeners
                    .iter()
                    .map(|listener| listener.callback.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let receiver = wrap_node(scope, state, current)?;
        for callback in callbacks {
            let callback = v8::Local::new(scope, &callback);
            if callback
                .call(scope, receiver.into(), &[object.into()])
                .is_none()
            {
                let detail = scope
                    .stack_trace()
                    .or_else(|| scope.exception())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "JavaScript listener raised an exception".into());
                return Err(V8BindingError::EventCallback(detail));
            }
            if state
                .events
                .borrow()
                .get(&id)
                .is_some_and(|event| event.immediate_stopped)
            {
                break;
            }
        }
        let event_type = state.events.borrow().get(&id).unwrap().event_type.clone();
        let property = format!("on{event_type}");
        if !state
            .events
            .borrow()
            .get(&id)
            .is_some_and(|event| event.immediate_stopped)
            && let Some(property) = v8::String::new(scope, &property)
            && let Some(handler) = receiver.get(scope, property.into())
            && let Ok(handler) = v8::Local::<v8::Function>::try_from(handler)
        {
            let result = handler
                .call(scope, receiver.into(), &[object.into()])
                .ok_or_else(|| {
                    let detail = scope
                        .stack_trace()
                        .or_else(|| scope.exception())
                        .and_then(|value| value.to_string(scope))
                        .map(|value| value.to_rust_string_lossy(scope))
                        .unwrap_or_else(|| "event property handler raised an exception".into());
                    V8BindingError::EventCallback(detail)
                })?;
            if result.is_false() {
                let mut events = state.events.borrow_mut();
                let event = events.get_mut(&id).unwrap();
                if event.cancelable {
                    event.default_prevented = true;
                }
            }
        }
    }
    let mut events = state.events.borrow_mut();
    let event = events.get_mut(&id).unwrap();
    event.current_target = None;
    Ok(!event.default_prevented)
}

fn optional_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    node: Option<NodeId>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match node {
        Some(node) => Ok(wrap_node(scope, state, node)?.into()),
        None => Ok(v8::null(scope).into()),
    }
}

fn node_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    nodes: &[NodeId],
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let values = nodes
        .iter()
        .map(|node| wrap_node(scope, state, *node).map(Into::into))
        .collect::<Result<Vec<v8::Local<v8::Value>>, _>>()?;
    Ok(v8::Array::new_with_elements(scope, &values).into())
}

fn argument_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
) -> Result<NodeId, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    let wrapper = receiver_wrapper_id(scope, object)
        .ok_or(BindingError::UnknownWrapper(WrapperId(u64::MAX)))?;
    Ok(state.wrappers.borrow().resolve(
        &state.dom.borrow(),
        wrapper,
        state.realm,
        state.document_generation,
        InterfaceBrand::Node,
    )?)
}

fn optional_argument_node(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
) -> Result<Option<NodeId>, V8BindingError> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        argument_node(scope, state, value).map(Some)
    }
}

fn required_string(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    index: i32,
) -> Result<String, V8BindingError> {
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .ok_or(V8BindingError::Allocation("string conversion"))
}

fn v8_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    Ok(v8::String::new(scope, value)
        .ok_or(V8BindingError::Allocation("string"))?
        .into())
}

fn node_type(kind: &NodeKind) -> i32 {
    match kind {
        NodeKind::Element { .. } => 1,
        NodeKind::Text { .. } => 3,
        NodeKind::Comment { .. } => 8,
        NodeKind::Document => 9,
        NodeKind::DocumentType { .. } => 10,
        NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => 11,
    }
}

fn throw_type_error(scope: &mut v8::PinScope, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let exception = v8::Exception::type_error(scope, message);
        scope.throw_exception(exception);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use oryn_dom::html;

    use super::*;

    static INITIALIZE_V8: Once = Once::new();

    fn initialize_v8() {
        INITIALIZE_V8.call_once(|| {
            let platform = v8::new_default_platform(0, false).make_shared();
            v8::V8::initialize_platform(platform);
            v8::V8::initialize();
        });
    }

    #[test]
    fn generated_templates_mutate_the_rust_owned_dom() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const button=document.createElement('button');button.id='go';button.textContent='Run';document.querySelector('#root').appendChild(button);button===document.getElementById('go')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        let button = dom.borrow().find_by_id("go").unwrap();
        assert_eq!(dom.borrow().arena().text_content(button).unwrap(), "Run");
    }

    #[test]
    fn generated_event_target_dispatches_against_rust_parentage() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><button id='go'></button></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "let calls=[];const root=document.getElementById('root');const go=document.getElementById('go');root.addEventListener('click',e=>calls.push('root:'+e.target.id));go.addEventListener('click',e=>{calls.push('go');e.preventDefault()});const accepted=go.dispatchEvent(new Event('click',{bubbles:true,cancelable:true}));calls.join(',')+'|'+accepted",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "go,root:go|false"
        );
    }

    #[test]
    fn delegated_listener_state_never_changes_html_serialization() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');const listener=()=>{};root.addEventListener('click',listener);const clean=!root.outerHTML.includes('oryn-click');root.removeEventListener('click',listener);clean",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        let root = dom.borrow().find_by_id("root").unwrap();
        assert!(
            dom.borrow()
                .arena()
                .get(root)
                .unwrap()
                .event_listeners
                .is_empty()
        );
    }

    #[test]
    fn generated_form_controls_keep_state_in_the_rust_dom() {
        initialize_v8();
        let parsed = html::parse(
            "<form id='form'><input id='name' required><button id='save'></button></form>",
        );
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const form=document.getElementById('form'),name=document.getElementById('name');const before=form.checkValidity();name.value='Ada';const after=form.checkValidity();before+'|'+after+'|'+(name instanceof HTMLInputElement)",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert_eq!(
            script.run(scope).unwrap().to_rust_string_lossy(scope),
            "false|true|true"
        );
        let input = dom.borrow().find_by_id("name").unwrap();
        assert_eq!(dom.borrow().value(input).unwrap(), Some("Ada"));
    }

    #[test]
    fn generated_geometry_distinguishes_hidden_from_unsupported() {
        initialize_v8();
        let parsed =
            html::parse("<main><div id='hidden'></div><table id='unsupported'></table></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let hidden = dom.borrow().find_by_id("hidden").unwrap();
        let unsupported = dom.borrow().find_by_id("unsupported").unwrap();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_layout(BindingLayout {
            styles: BTreeMap::new(),
            geometry: BTreeMap::from([
                (node_key(hidden), BindingGeometry::Hidden),
                (
                    node_key(unsupported),
                    BindingGeometry::Unsupported("table geometry is unavailable".into()),
                ),
            ]),
        });
        let source = v8::String::new(
            scope,
            "const hidden=document.getElementById('hidden').getBoundingClientRect();let rejected=false;try{document.getElementById('unsupported').getBoundingClientRect()}catch(error){rejected=error instanceof TypeError&&String(error).includes('table geometry is unavailable')}hidden.width===0&&hidden.height===0&&rejected",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn detached_wrapper_survives_reinsertion_without_becoming_queryable_early() {
        initialize_v8();
        let parsed = html::parse("<main id='root'><button id='go'></button></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root'),go=document.getElementById('go');go.remove();const absent=document.getElementById('go')===null;root.appendChild(go);absent&&go===document.getElementById('go')",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
    }

    #[test]
    fn trusted_types_are_unforgeable_and_guard_html_sinks() {
        initialize_v8();
        let parsed = html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom.clone(), RealmId(0), 1).unwrap();
        state.configure_trusted_types(TrustedTypesConfig {
            require_script: true,
            allowed_policy_names: ["sanitizer".into()].into(),
            ..TrustedTypesConfig::default()
        });
        let source = v8::String::new(
            scope,
            "const root=document.getElementById('root');let raw=false;try{root.innerHTML='<b>raw</b>'}catch(e){raw=e instanceof TypeError}const policy=trustedTypes.createPolicy('sanitizer',{createHTML:s=>s.replace('unsafe','safe')});const value=policy.createHTML('<b>unsafe</b>');root.innerHTML=value;raw&&trustedTypes instanceof TrustedTypePolicyFactory&&policy instanceof TrustedTypePolicy&&value instanceof TrustedHTML&&trustedTypes.emptyHTML instanceof TrustedHTML&&trustedTypes.isHTML(value)&&!trustedTypes.isHTML({toString(){return '<i>fake</i>'}})&&root.textContent==='safe'",
        ).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).unwrap().boolean_value(scope));
        assert_eq!(
            dom.borrow()
                .arena()
                .text_content(dom.borrow().find_by_id("root").unwrap())
                .unwrap(),
            "safe"
        );
    }
}

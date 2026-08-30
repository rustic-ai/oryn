//! Generated V8 templates backed by the Rust-owned live DOM.
//!
//! Browser objects carry a numeric wrapper identifier only. All interface,
//! realm, document-generation and node-generation checks happen before a
//! generated callback reaches a Rust behavior implementation.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak as RcWeak},
};

use oryn_dom::{DomError, DomRuntime, NodeId, NodeKind};

use crate::{
    bindings::{BindingError, InterfaceBrand, WrapperId, WrapperRegistry},
    generated::{GENERATED_INTERFACES, GeneratedMemberKind},
    realm::RealmId,
};

pub type SharedDom = Rc<RefCell<DomRuntime>>;

#[derive(Debug, thiserror::Error)]
pub enum V8BindingError {
    #[error("generated interface {0} is unavailable")]
    MissingInterface(&'static str),
    #[error("V8 failed to allocate {0}")]
    Allocation(&'static str),
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
pub struct BindingRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Default)]
pub struct BindingLayout {
    pub styles: BTreeMap<u64, BTreeMap<String, String>>,
    pub rects: BTreeMap<u64, BindingRect>,
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
    layout: RefCell<BindingLayout>,
    frame_roots: RefCell<BTreeMap<NodeId, NodeId>>,
    document_write_anchor: Cell<Option<NodeId>>,
    document_open: Cell<bool>,
    document_write_buffer: RefCell<String>,
    document_write_depth: Cell<u32>,
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

    pub fn set_layout(&self, layout: BindingLayout) {
        *self.layout.borrow_mut() = layout;
    }

    pub fn set_frame_document(&self, frame: NodeId, document: NodeId) {
        self.frame_roots.borrow_mut().insert(frame, document);
    }

    pub fn set_document_write_anchor(&self, anchor: Option<NodeId>) {
        self.document_write_anchor.set(anchor);
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
        layout: RefCell::new(BindingLayout::default()),
        frame_roots: RefCell::new(BTreeMap::new()),
        document_write_anchor: Cell::new(None),
        document_open: Cell::new(false),
        document_write_buffer: RefCell::new(String::new()),
        document_write_depth: Cell::new(0),
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
    Ok(state)
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
    if name == "Event" && args.is_construct_call() {
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
        match call_event_operation(scope, &state, args.this(), member) {
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
        NodeKind::DocumentFragment | NodeKind::ShadowRoot { .. } => "DocumentFragment",
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
    state
        .objects
        .borrow_mut()
        .insert(wrapper, WrapperObject { handle: weak });
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
    if interface == "Document" && member == "styleSheets" {
        let styles = state.dom.borrow().query_selector_all(node, "style")?;
        return associated_node_array(scope, state, &styles, "CSSStyleSheet");
    }
    if interface == "HTMLIFrameElement" && member == "contentDocument" {
        let document = state.frame_roots.borrow().get(&node).copied();
        return optional_node(scope, state, document);
    }
    let dom = state.dom.borrow();
    let record = dom.arena().get(node)?;
    let value = match (interface, member) {
        (_, "nodeName") => v8_string(scope, &dom.node_name(node)?)?,
        (_, "nodeType") => v8::Integer::new(scope, node_type(&record.kind)).into(),
        (_, "textContent") | ("HTMLElement", "innerText") => {
            v8_string(scope, &dom.arena().text_content(node)?)?
        }
        (_, "parentNode") => return optional_node(scope, state, record.parent),
        (_, "firstChild") => return optional_node(scope, state, record.children.first().copied()),
        (_, "lastChild") => return optional_node(scope, state, record.children.last().copied()),
        (_, "childNodes") => return node_array(scope, state, &record.children),
        ("Document", "documentElement") => {
            return optional_node(scope, state, dom.document_element());
        }
        ("Element", "tagName") => v8_string(scope, &dom.node_name(node)?)?,
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
        ("Element", "innerHTML") => {
            let mut html = String::new();
            for child in &record.children {
                html.push_str(&dom.serialize(*child)?);
            }
            v8_string(scope, &html)?
        }
        ("Element", "outerHTML") => v8_string(scope, &dom.serialize(node)?)?,
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
    let value = value.to_rust_string_lossy(scope);
    let mut dom = state.dom.borrow_mut();
    match (interface, member) {
        (_, "textContent") | ("HTMLElement", "innerText") => dom.set_text_content(node, value)?,
        ("Element", "id") => dom.set_attribute(node, "id", value)?,
        ("Element", "className") => dom.set_attribute(node, "class", value)?,
        ("DOMTokenList", "value") => dom.set_attribute(node, "class", value)?,
        ("Text" | "Comment", "data") => dom.set_text(node, value)?,
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
        ("Element", "innerHTML") => dom.set_inner_html(node, &value)?,
        _ => {
            return Err(V8BindingError::Unsupported {
                interface: interface.into(),
                member: member.into(),
            });
        }
    }
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
                .entry((node, event_type))
                .or_default()
                .push(EventListener {
                    callback: v8::Global::new(scope, callback),
                });
            Ok(None)
        }
        ("EventTarget", "removeEventListener") => {
            let event_type = required_string(scope, args, 0)?;
            let candidate = v8::Local::<v8::Function>::try_from(args.get(1)).ok();
            if let Some(listeners) = state.listeners.borrow_mut().get_mut(&(node, event_type)) {
                listeners.retain(|listener| {
                    candidate.is_none_or(|candidate| {
                        !v8::Local::new(scope, &listener.callback).strict_equals(candidate.into())
                    })
                });
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
        ("Document", "getElementById") => {
            let id = required_string(scope, args, 0)?;
            let found = state.dom.borrow().find_by_id(&id);
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
            let mut markup = required_string(scope, args, 0)?;
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
            Ok(Some(wrap_node(scope, state, child)?.into()))
        }
        (_, "insertBefore") => {
            let child = argument_node(scope, state, args.get(0))?;
            let reference = optional_argument_node(scope, state, args.get(1))?;
            state
                .dom
                .borrow_mut()
                .insert_before(node, child, reference)?;
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
            let value = required_string(scope, args, 1)?;
            state.dom.borrow_mut().set_attribute(node, name, value)?;
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
            let rect = state
                .layout
                .borrow()
                .rects
                .get(&node_key(node))
                .copied()
                .unwrap_or_default();
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
        _ => Err(V8BindingError::Unsupported {
            interface: interface.into(),
            member: member.into(),
        }),
    }
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
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let id = event_id(scope, object)?;
    let mut events = state.events.borrow_mut();
    let event = events
        .get_mut(&id)
        .ok_or(V8BindingError::Allocation("event record"))?;
    match member {
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
            callback.call(scope, receiver.into(), &[object.into()]);
            if state
                .events
                .borrow()
                .get(&id)
                .is_some_and(|event| event.immediate_stopped)
            {
                break;
            }
        }
        if !state
            .events
            .borrow()
            .get(&id)
            .is_some_and(|event| event.immediate_stopped)
        {
            let property = format!("on{}", state.events.borrow().get(&id).unwrap().event_type);
            if let Some(property) = v8::String::new(scope, &property)
                && let Some(handler) = receiver.get(scope, property.into())
                && let Ok(handler) = v8::Local::<v8::Function>::try_from(handler)
            {
                handler.call(scope, receiver.into(), &[object.into()]);
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
}

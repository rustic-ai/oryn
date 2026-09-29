//! Native service-worker wrappers and promise polling. Registry and worker IDs
//! remain in private native tables; public properties never authorize a request.
type DecodedMessage = (v8::Global<v8::Value>, Vec<v8::Global<v8::Object>>);

use super::*;
use crate::{
    service_worker_wire::*,
    worker_wire::{AgentServiceRequest, AgentServiceResponse},
};
#[derive(Clone, Copy)]
enum Kind {
    Container,
    Preload(RegistrationId),
    Registration(RegistrationId),
    Worker(ServiceVersionId),
    Client(u64, bool),
}
struct Entity {
    kind: Kind,
    object: v8::Weak<v8::Object>,
}
struct Pending {
    settled: bool,
    operation: u64,
    resolver: v8::Global<v8::PromiseResolver>,
    result: Option<Result<ServiceResult, crate::worker_wire::WorkerError>>,
}
#[derive(Default)]
pub(super) struct Bindings {
    messages_enabled: bool,
    polling: bool,
    publications: std::collections::VecDeque<Publication>,
    relationships_dirty: bool,
    prototypes: BTreeMap<&'static str, v8::Weak<v8::Object>>,
    entities: BTreeMap<u64, Entity>,
    container: Option<u64>,
    registrations: BTreeMap<RegistrationId, RegistrationSnapshot>,
    clients: BTreeMap<u64, ClientSnapshot>,
    clients_object: Option<v8::Weak<v8::Object>>,
    workers: BTreeMap<ServiceVersionId, WorkerSnapshot>,
    pending: Vec<Pending>,
    ready: Option<v8::Weak<v8::Promise>>,
    controller: Option<ServiceVersionId>,
}
struct MessagePublication {
    activation: Option<[bool; 2]>,
    container: u64,
    source: WorkerSnapshot,
    origin: String,
    wire: Option<crate::message_wire::WireValue>,
    cleanup: Option<structured_clone::SharedReceiveGuard>,
    source_object: Option<v8::Global<v8::Value>>,
    decoded: Option<Option<DecodedMessage>>,
}
enum PublicationPayload {
    Event(u64, &'static str),
    Message(Box<MessagePublication>),
}
struct Publication {
    payload: PublicationPayload,
    _target: v8::Global<v8::Object>,
    dispatch: Option<native_platform::publication::Notification>,
}
struct PollBatch {
    state: Rc<V8BindingState>,
    pending: std::collections::VecDeque<Pending>,
    publication: Option<Publication>,
}
impl Drop for PollBatch {
    fn drop(&mut self) {
        let mut bindings = self.state.service_workers.borrow_mut();
        bindings.polling = false;
        if self.state.execution_retired() {
            if let Some(publication) = self.publication.take()
                && let Some(dispatch) = publication.dispatch
            {
                dispatch.cancel(&self.state);
            }
            return;
        }
        let mut pending = self.pending.drain(..).collect::<Vec<_>>();
        pending.append(&mut bindings.pending);
        bindings.pending = pending;
        if let Some(publication) = self.publication.take() {
            bindings.publications.push_front(publication);
        }
    }
}
fn failure(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "ServiceWorkerContainer".into(),
        member: name.into(),
    }
}
fn request(
    state: &V8BindingState,
    request: ServiceRequest,
) -> Result<ServiceResponse, V8BindingError> {
    workers::publish_secure_context(state)?;
    let transport = state
        .worker_transport
        .borrow()
        .clone()
        .ok_or_else(|| failure("InvalidStateError"))?;
    match transport.request(AgentServiceRequest::Service(request)) {
        Ok(AgentServiceResponse::Service(response)) => Ok(response),
        Err(error) => Err(V8BindingError::NativeService {
            name: error.name,
            message: error.message,
        }),
        _ => Err(failure("InvalidStateError")),
    }
}
fn entity(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    object: v8::Local<v8::Object>,
) -> Result<(u64, Kind), V8BindingError> {
    if state.execution_retired() {
        return Err(failure("InvalidStateError"));
    }
    let id = if object.internal_field_count() > 0 {
        stateful_id(scope, object)?
    } else {
        let key = client_key(scope);
        object
            .get_private(scope, key)
            .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
            .map(|value| value.u64_value().0)
            .ok_or_else(|| failure("TypeError"))?
    };
    let bindings = state.service_workers.borrow();
    let entity = bindings
        .entities
        .get(&id)
        .ok_or_else(|| failure("TypeError"))?;
    if !entity
        .object
        .to_local(scope)
        .is_some_and(|known| known.strict_equals(object.into()))
    {
        return Err(failure("TypeError"));
    }
    Ok((id, entity.kind))
}
fn method(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    index: i32,
) -> Result<(), V8BindingError> {
    let data = v8::Integer::new(scope, index);
    let function = v8::Function::builder(call)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("service method"))?;
    let name = v8::String::new(scope, name).unwrap();
    object.create_data_property(scope, name.into(), function.into());
    Ok(())
}
fn getter(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    index: i32,
) -> Result<(), V8BindingError> {
    let data = v8::Integer::new(scope, index);
    let function = v8::Function::builder(attribute)
        .data(data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("service attribute"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor =
        v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, name).unwrap();
    object.define_property(scope, name.into(), &descriptor);
    Ok(())
}
fn handler(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    name: &str,
    index: i32,
) -> Result<(), V8BindingError> {
    let data = v8::Integer::new(scope, index);
    let get = v8::Function::builder(handler_get)
        .data(data.into())
        .build(scope)
        .unwrap();
    let set = v8::Function::builder(handler_set)
        .data(data.into())
        .build(scope)
        .unwrap();
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, name).unwrap();
    object.define_property(scope, name.into(), &descriptor);
    Ok(())
}
fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    kind: Kind,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if let Some(object) = state
        .service_workers
        .borrow()
        .entities
        .values()
        .find_map(|entity| {
            let matches = match (kind, entity.kind) {
                (Kind::Container, Kind::Container) => true,
                (Kind::Registration(a), Kind::Registration(b)) => a == b,
                (Kind::Preload(a), Kind::Preload(b)) => a == b,
                (Kind::Worker(a), Kind::Worker(b)) => a == b,
                _ => false,
            };
            if matches {
                entity.object.to_local(scope)
            } else {
                None
            }
        })
    {
        return Ok(object);
    }
    let id = register_platform_object(state, PlatformObjectRecord::EventTarget);
    let object = new_platform_object(scope, state, "EventTarget", id)?;
    let name = kind_name(kind);
    let prototype = state
        .service_workers
        .borrow()
        .prototypes
        .get(name)
        .cloned()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    object.set_prototype(scope, prototype.into());
    state.service_workers.borrow_mut().entities.insert(
        id,
        Entity {
            kind,
            object: v8::Weak::new(scope, object),
        },
    );
    trace_relationships(scope, state, object, kind)?;
    Ok(object)
}
fn relationship_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.service.relationships").unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn trace_relationships(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    kind: Kind,
) -> Result<(), V8BindingError> {
    let mut edges = Vec::new();
    match kind {
        Kind::Registration(id) => {
            let snapshot = state
                .service_workers
                .borrow()
                .registrations
                .get(&id)
                .cloned();
            if let Some(snapshot) = snapshot {
                for worker in [snapshot.installing, snapshot.waiting, snapshot.active]
                    .into_iter()
                    .flatten()
                {
                    edges.push(wrap(scope, state, Kind::Worker(worker.version_id))?.into());
                }
            }
            let preload = wrap(scope, state, Kind::Preload(id))?;
            let key = relationship_key(scope);
            let backlink = v8::Array::new_with_elements(scope, &[object.into()]);
            preload
                .set_private(scope, key, backlink.into())
                .ok_or(V8BindingError::JavaScriptException)?;
            edges.push(preload.into());
        }
        Kind::Container => {
            let controller = state.service_workers.borrow().controller;
            if let Some(id) = controller {
                edges.push(wrap(scope, state, Kind::Worker(id))?.into());
            }
            if let Some(promise) = state
                .service_workers
                .borrow()
                .ready
                .as_ref()
                .and_then(|promise| promise.to_local(scope))
            {
                edges.push(promise.into());
            }
        }
        _ => return Ok(()),
    }
    let values = v8::Array::new_with_elements(scope, &edges);
    let key = relationship_key(scope);
    object
        .set_private(scope, key, values.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn refresh_relationships(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<(), V8BindingError> {
    let targets = {
        let bindings = state.service_workers.borrow();
        bindings
            .entities
            .values()
            .filter_map(|entity| {
                entity
                    .object
                    .to_local(scope)
                    .map(|object| (object, entity.kind))
            })
            .collect::<Vec<_>>()
    };
    for (object, kind) in targets {
        trace_relationships(scope, state, object, kind)?;
    }
    let mut bindings = state.service_workers.borrow_mut();
    bindings
        .entities
        .retain(|_, entity| entity.object.to_local(scope).is_some());
    let clients = bindings
        .entities
        .iter()
        .filter_map(|(id, entity)| matches!(entity.kind, Kind::Client(..)).then_some(*id))
        .collect::<BTreeSet<_>>();
    bindings.clients.retain(|id, _| clients.contains(id));
    Ok(())
}
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Container => "ServiceWorkerContainer",
        Kind::Preload(_) => "NavigationPreloadManager",
        Kind::Registration(_) => "ServiceWorkerRegistration",
        Kind::Worker(_) => "ServiceWorker",
        Kind::Client(_, false) => "Client",
        Kind::Client(_, true) => "WindowClient",
    }
}
fn define_prototype(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    kind: Kind,
) -> Result<(), V8BindingError> {
    match kind {
        Kind::Preload(_) => {
            for (name, index) in [
                ("enable", 9),
                ("disable", 10),
                ("setHeaderValue", 11),
                ("getState", 12),
            ] {
                method(scope, object, name, index)?;
            }
        }
        Kind::Container => {
            for (name, index) in [
                ("register", 0),
                ("getRegistration", 1),
                ("getRegistrations", 2),
                ("startMessages", 3),
            ] {
                method(scope, object, name, index)?;
            }
            getter(scope, object, "controller", 0)?;
            getter(scope, object, "ready", 1)?;
            handler(scope, object, "oncontrollerchange", 0)?;
            handler(scope, object, "onmessage", 1)?;
            handler(scope, object, "onmessageerror", 2)?;
        }
        Kind::Registration(_) => {
            getter(scope, object, "navigationPreload", 15)?;
            method(scope, object, "update", 4)?;
            method(scope, object, "unregister", 5)?;
            for (name, index) in [
                ("scope", 2),
                ("updateViaCache", 3),
                ("installing", 4),
                ("waiting", 5),
                ("active", 6),
            ] {
                getter(scope, object, name, index)?;
            }
            handler(scope, object, "onupdatefound", 3)?;
        }
        Kind::Worker(_) => {
            method(scope, object, "postMessage", 6)?;
            getter(scope, object, "scriptURL", 7)?;
            getter(scope, object, "state", 8)?;
            handler(scope, object, "onstatechange", 4)?;
        }
        Kind::Client(_, window) => {
            method(scope, object, "postMessage", 6)?;
            for (name, index) in [("id", 9), ("url", 10), ("type", 11), ("frameType", 12)] {
                getter(scope, object, name, index)?;
            }
            if window {
                getter(scope, object, "visibilityState", 13)?;
                getter(scope, object, "focused", 14)?;
                method(scope, object, "focus", 7)?;
                method(scope, object, "navigate", 8)?;
            }
        }
    }
    Ok(())
}
fn illegal(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor")
}
fn install_prototypes(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    window: bool,
) -> Result<(), V8BindingError> {
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "EventTarget").unwrap();
    let base = global
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("InvalidStateError"))?;
    let key = v8::String::new(scope, "prototype").unwrap();
    let parent = base
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    for kind in [
        Kind::Container,
        Kind::Preload(RegistrationId(0)),
        Kind::Registration(RegistrationId(0)),
        Kind::Worker(ServiceVersionId([0; 16])),
        Kind::Client(0, false),
        Kind::Client(0, true),
    ] {
        if (!window && matches!(kind, Kind::Container))
            || (window && matches!(kind, Kind::Client(..)))
        {
            continue;
        }
        let name = kind_name(kind);
        let constructor = v8::Function::new(scope, illegal)
            .ok_or(V8BindingError::Allocation("service constructor"))?;
        let key = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or(V8BindingError::Allocation("service prototype"))?;
        if matches!(kind, Kind::Client(_, true)) {
            let base = state
                .service_workers
                .borrow()
                .prototypes
                .get("Client")
                .cloned()
                .ok_or_else(|| failure("InvalidStateError"))?;
            let base = base
                .to_local(scope)
                .ok_or_else(|| failure("InvalidStateError"))?;
            prototype.set_prototype(scope, base.into());
        } else if !matches!(kind, Kind::Client(..) | Kind::Preload(_)) {
            prototype.set_prototype(scope, parent);
        }
        define_prototype(scope, prototype, kind)?;
        let tag = v8::Symbol::get_to_string_tag(scope);
        let text = v8::String::new(scope, name).unwrap();
        prototype.define_own_property(
            scope,
            tag.into(),
            text.into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        );
        let key = v8::String::new(scope, name).unwrap();
        global.create_data_property(scope, key.into(), constructor.into());
        let retained = context_intrinsics::retain_object_for(
            scope,
            state,
            &format!("service_workers-prototype-{name}"),
            prototype,
        )?;
        state
            .service_workers
            .borrow_mut()
            .prototypes
            .insert(name, retained);
    }
    Ok(())
}
fn hide_insecure_exposure(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let global = context.global(scope);
    let member = v8::String::new(scope, "serviceWorker")
        .ok_or(V8BindingError::Allocation("service exposure name"))?;
    let prototype = v8::String::new(scope, "prototype")
        .ok_or(V8BindingError::Allocation("service exposure prototype"))?;
    // The native container supersedes generated Navigator members. Preserve
    // secure-only absence on every native prototype and the navigator object.
    for owner in ["Navigator", "WorkerNavigator"] {
        let key = v8::String::new(scope, owner)
            .ok_or(V8BindingError::Allocation("service exposure owner"))?;
        let value = global
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Ok(constructor) = value.try_cast::<v8::Object>()
            && let Some(prototype) = constructor
                .get(scope, prototype.into())
                .and_then(|value| value.try_cast::<v8::Object>().ok())
            && prototype.delete(scope, member.into()) != Some(true)
        {
            return Err(V8BindingError::Allocation(
                "insecure service prototype exposure",
            ));
        }
    }
    let key = v8::String::new(scope, "navigator")
        .ok_or(V8BindingError::Allocation("service navigator name"))?;
    if let Some(navigator) = global
        .get(scope, key.into())
        .and_then(|value| value.try_cast::<v8::Object>().ok())
        && navigator.delete(scope, member.into()) != Some(true)
    {
        return Err(V8BindingError::Allocation(
            "insecure service navigator exposure",
        ));
    }
    for name in [
        "ServiceWorker",
        "ServiceWorkerRegistration",
        "ServiceWorkerContainer",
        "NavigationPreloadManager",
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("service exposure name"))?;
        if global.delete(scope, key.into()) != Some(true) {
            return Err(V8BindingError::Allocation(
                "insecure service global exposure",
            ));
        }
    }
    Ok(())
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let Some(descriptor) = context.get_slot::<crate::realm::RealmDescriptor>() else {
        return Ok(());
    };
    let secure = scope
        .get_current_context()
        .get_slot::<crate::realm::SecureContextState>()
        .map(|state| state.0)
        .unwrap_or_else(|| crate::realm::potentially_trustworthy_origin(&descriptor.origin));
    if !secure {
        return hide_insecure_exposure(scope, context);
    }
    install_prototypes(scope, &state, true)?;
    let key = v8::String::new(scope, "navigator").unwrap();
    let navigator = context
        .global(scope)
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| failure("InvalidStateError"))?;
    let container = wrap(scope, &state, Kind::Container)?;
    state.service_workers.borrow_mut().container = Some(stateful_id(scope, container)?);
    if state.binding_global() == BindingGlobal::Window {
        native_navigator::install_service_worker(scope, navigator, container)?;
    }
    let key = v8::String::new(scope, "serviceWorker").unwrap();
    navigator.define_own_property(
        scope,
        key.into(),
        container.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    Ok(())
}
fn begin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    command: ServiceRequest,
) -> Result<v8::Local<'s, v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("service promise"))?;
    match request(state, command) {
        Ok(ServiceResponse::Pending { operation }) => {
            state.service_workers.borrow_mut().pending.push(Pending {
                settled: false,
                operation,
                resolver: v8::Global::new(scope, resolver),
                result: None,
            })
        }
        Ok(ServiceResponse::Done) => {
            let value = v8::undefined(scope);
            resolver.resolve(scope, value.into());
        }
        Err(error) => {
            let name = match error {
                V8BindingError::Unsupported { member, .. } => member,
                V8BindingError::NativeService { name, .. } => name,
                _ => "InvalidStateError".into(),
            };
            let value = dom_exception_value(scope, &name)?;
            resolver.reject(scope, value);
        }
        _ => {
            let value = dom_exception_value(scope, "InvalidStateError")?;
            resolver.reject(scope, value);
        }
    }
    Ok(resolver.get_promise(scope))
}
fn call(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let (_, kind) = entity(scope, &state, args.this())?;
        let operation = args.data().int32_value(scope).unwrap_or(-1);
        let creator = workers::native_creator(&state)?;
        if operation == 6 {
            if args.length() == 0 {
                return Err(failure("TypeError"));
            }
            let options = structured_clone::post_transfer_options(scope, args.get(1))
                .ok_or(V8BindingError::JavaScriptException)?;
            let transfers = options.transfers;
            let value = structured_clone::serialize_wire_with_activation(
                scope,
                args.get(0),
                &transfers,
                |scope| {
                    structured_clone::activation_snapshot(scope, options.include_user_activation)
                },
            )
            .ok_or(V8BindingError::JavaScriptException)?;
            let ports = value.ports.clone();
            let _shared_send_guard = structured_clone::shared_send_guard(scope, &value);
            let command = match kind {
                Kind::Worker(worker) => ServiceRequest::PostMessage {
                    creator,
                    worker,
                    value,
                },
                Kind::Client(id, _) => {
                    let client = state
                        .service_workers
                        .borrow()
                        .clients
                        .get(&id)
                        .ok_or_else(|| failure("InvalidStateError"))?
                        .id
                        .clone();
                    ServiceRequest::ClientPostMessage { client, value }
                }
                _ => return Err(failure("TypeError")),
            };
            if let Err(error) = request(&state, command) {
                if let Some(transport) = state.worker_transport.borrow().clone() {
                    let _ = transport.request(AgentServiceRequest::Message(
                        crate::message_wire::MessageRequest::DiscardTransfers { ports },
                    ));
                }
                return Err(error);
            }
            return Ok(v8::undefined(scope).into());
        }
        let command = match (kind, operation) {
            (Kind::Preload(registration), 9..=12) => {
                let operation = match operation {
                    9 => NavigationPreloadOperation::Enable,
                    10 => NavigationPreloadOperation::Disable,
                    11 => {
                        if args.length() == 0 {
                            return Err(failure("TypeError"));
                        }
                        let string = args
                            .get(0)
                            .to_string(scope)
                            .ok_or(V8BindingError::JavaScriptException)?;
                        let mut units = vec![0; string.length()];
                        string.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
                        if units.iter().any(|unit| *unit > 255) {
                            return Err(failure("TypeError"));
                        }
                        NavigationPreloadOperation::SetHeaderValue(
                            units
                                .into_iter()
                                .map(|unit| char::from(unit as u8))
                                .collect(),
                        )
                    }
                    _ => NavigationPreloadOperation::GetState,
                };
                ServiceRequest::NavigationPreload {
                    creator,
                    registration,
                    operation,
                }
            }
            (Kind::Container, 0) => {
                if args.length() == 0 {
                    return Err(failure("TypeError"));
                }
                let url = required_string(scope, &args, 0)?;
                let options = args.get(1);
                let mut target = None;
                let mut script_type = crate::worker_wire::ScriptType::Classic;
                let mut update_via_cache = "imports".to_owned();
                if !options.is_null_or_undefined() {
                    let options = options
                        .to_object(scope)
                        .ok_or_else(|| failure("TypeError"))?;
                    for name in ["scope", "type", "updateViaCache"] {
                        let key = v8::String::new(scope, name).unwrap();
                        let value = options
                            .get(scope, key.into())
                            .ok_or(V8BindingError::JavaScriptException)?;
                        if value.is_undefined() {
                            continue;
                        }
                        let value = value
                            .to_string(scope)
                            .ok_or(V8BindingError::JavaScriptException)?
                            .to_rust_string_lossy(scope);
                        match name {
                            "scope" => target = Some(value),
                            "type" => {
                                script_type = match value.as_str() {
                                    "classic" => crate::worker_wire::ScriptType::Classic,
                                    "module" => crate::worker_wire::ScriptType::Module,
                                    _ => return Err(failure("TypeError")),
                                }
                            }
                            _ => update_via_cache = value,
                        }
                    }
                }
                ServiceRequest::Register {
                    creator,
                    url,
                    scope: target,
                    script_type,
                    update_via_cache,
                }
            }
            (Kind::Container, 1) => ServiceRequest::GetRegistration {
                creator,
                url: if args.get(0).is_undefined() {
                    document_base_url(scope, &state, true)?
                } else {
                    required_string(scope, &args, 0)?
                },
            },
            (Kind::Container, 2) => ServiceRequest::GetRegistrations { creator },
            (Kind::Container, 3) => {
                enable_messages(&state);
                return Ok(v8::undefined(scope).into());
            }
            (Kind::Client(id, true), 7) => ServiceRequest::ClientFocus {
                client: state
                    .service_workers
                    .borrow()
                    .clients
                    .get(&id)
                    .ok_or_else(|| failure("InvalidStateError"))?
                    .id
                    .clone(),
            },
            (Kind::Client(id, true), 8) => {
                if args.length() == 0 {
                    return Err(failure("TypeError"));
                }
                let url = required_string(scope, &args, 0)?;
                ServiceRequest::ClientNavigate {
                    client: state
                        .service_workers
                        .borrow()
                        .clients
                        .get(&id)
                        .ok_or_else(|| failure("InvalidStateError"))?
                        .id
                        .clone(),
                    url,
                }
            }
            (Kind::Registration(registration), 4) => ServiceRequest::Update {
                creator,
                registration,
            },
            (Kind::Registration(registration), 5) => ServiceRequest::Unregister {
                creator,
                registration,
            },
            _ => return Err(failure("TypeError")),
        };
        Ok(begin(scope, &state, command)?.into())
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn attribute(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let (_, kind) = entity(scope, &state, args.this())?;
        let index = args.data().int32_value(scope).unwrap_or(-1);
        match (kind, index) {
            (Kind::Registration(id), 15) => Ok(wrap(scope, &state, Kind::Preload(id))?.into()),
            (Kind::Container, 0) => {
                let creator = workers::native_creator(&state)?;
                let ServiceResponse::Controller(controller) =
                    request(&state, ServiceRequest::Controller { creator })?
                else {
                    return Err(failure("InvalidStateError"));
                };
                match controller {
                    Some(worker) => {
                        let id = worker.version_id;
                        {
                            let mut bindings = state.service_workers.borrow_mut();
                            bindings.controller = Some(id);
                            bindings.workers.insert(id, worker);
                        }
                        trace_relationships(scope, &state, args.this(), Kind::Container)?;
                        Ok(wrap(scope, &state, Kind::Worker(id))?.into())
                    }
                    None => {
                        state.service_workers.borrow_mut().controller = None;
                        Ok(v8::null(scope).into())
                    }
                }
            }
            (Kind::Container, 1) => {
                let existing = state
                    .service_workers
                    .borrow()
                    .ready
                    .as_ref()
                    .and_then(|promise| promise.to_local(scope));
                if let Some(promise) = existing {
                    return Ok(promise.into());
                }
                let creator = workers::native_creator(&state)?;
                let promise = begin(scope, &state, ServiceRequest::Ready { creator })?;
                state.service_workers.borrow_mut().ready = Some(v8::Weak::new(scope, promise));
                trace_relationships(scope, &state, args.this(), Kind::Container)?;
                Ok(promise.into())
            }
            (Kind::Registration(id), 2..=6) => {
                let snapshot = state
                    .service_workers
                    .borrow()
                    .registrations
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?;
                if index == 2 {
                    return v8_string(scope, &snapshot.scope);
                }
                if index == 3 {
                    return v8_string(scope, &snapshot.update_via_cache);
                }
                let worker = match index {
                    4 => snapshot.installing,
                    5 => snapshot.waiting,
                    _ => snapshot.active,
                };
                match worker {
                    Some(worker) => {
                        state
                            .service_workers
                            .borrow_mut()
                            .workers
                            .insert(worker.version_id, worker.clone());
                        Ok(wrap(scope, &state, Kind::Worker(worker.version_id))?.into())
                    }
                    None => Ok(v8::null(scope).into()),
                }
            }
            (Kind::Client(id, _), 9..=14) => {
                let client = state
                    .service_workers
                    .borrow()
                    .clients
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?;
                match index {
                    9 => v8_string(scope, &client.id),
                    10 => v8_string(scope, &client.url),
                    11 => v8_string(
                        scope,
                        match client.client_type {
                            ClientType::Window => "window",
                            ClientType::DedicatedWorker => "worker",
                            ClientType::SharedWorker => "sharedworker",
                        },
                    ),
                    12 => v8_string(scope, &client.frame_type),
                    13 => v8_string(scope, &client.visibility_state),
                    _ => Ok(v8::Boolean::new(scope, client.focused).into()),
                }
            }
            (Kind::Worker(id), 7..=8) => {
                let worker = state
                    .service_workers
                    .borrow()
                    .workers
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| failure("InvalidStateError"))?;
                if index == 7 {
                    v8_string(scope, &worker.script_url)
                } else {
                    v8_string(
                        scope,
                        match worker.state {
                            WorkerState::Installing => "installing",
                            WorkerState::Installed => "installed",
                            WorkerState::Activating => "activating",
                            WorkerState::Activated => "activated",
                            WorkerState::Redundant => "redundant",
                        },
                    )
                }
            }
            _ => Err(failure("TypeError")),
        }
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn handler_name(index: i32) -> Option<&'static str> {
    match index {
        0 => Some("controllerchange"),
        1 => Some("message"),
        2 => Some("messageerror"),
        3 => Some("updatefound"),
        4 => Some("statechange"),
        _ => None,
    }
}
fn handler_get(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok((id, _)) = entity(scope, &state, args.this()) else {
        return;
    };
    let Some(name) = handler_name(args.data().int32_value(scope).unwrap_or(-1)) else {
        return;
    };
    rv.set(platform_listener_graph::handler_get(
        scope, &state, id, name,
    ));
}
fn handler_set(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok((id, _)) = entity(scope, &state, args.this()) else {
        return;
    };
    let Some(name) = handler_name(args.data().int32_value(scope).unwrap_or(-1)) else {
        return;
    };
    if let Err(error) = platform_listener_graph::handler_set(scope, &state, id, name, args.get(0)) {
        throw_binding_error(scope, &error);
    } else if name == "message" && args.get(0).is_function() {
        enable_messages(&state);
    }
}
fn registration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    snapshot: RegistrationSnapshot,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let id = snapshot.id;
    {
        let mut bindings = state.service_workers.borrow_mut();
        for worker in [&snapshot.installing, &snapshot.waiting, &snapshot.active]
            .into_iter()
            .flatten()
        {
            bindings.workers.insert(worker.version_id, worker.clone());
        }
        bindings.registrations.insert(id, snapshot);
    }
    let object = wrap(scope, state, Kind::Registration(id))?;
    trace_relationships(scope, state, object, Kind::Registration(id))?;
    Ok(object.into())
}
fn result_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    result: ServiceResult,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match result {
        ServiceResult::NavigationPreload(value) => {
            let object = v8::Object::new(scope);
            let key = v8::String::new(scope, "enabled").unwrap();
            let enabled = v8::Boolean::new(scope, value.enabled);
            object.create_data_property(scope, key.into(), enabled.into());
            let key = v8::String::new(scope, "headerValue").unwrap();
            let value = v8_string(scope, &value.header_value)?;
            object.create_data_property(scope, key.into(), value);
            Ok(object.into())
        }
        ServiceResult::WindowClient(Some(client)) => message_source(
            scope,
            state,
            MessageSource::Client {
                client,
                origin: String::new(),
            },
        )
        .map(|(value, _)| value),
        ServiceResult::WindowClient(None) => Ok(v8::null(scope).into()),
        ServiceResult::Client(Some(client)) => message_source(
            scope,
            state,
            MessageSource::Client {
                client,
                origin: String::new(),
            },
        )
        .map(|(value, _)| value),
        ServiceResult::Client(None) => Ok(v8::undefined(scope).into()),
        ServiceResult::Clients(clients) => {
            let array = v8::Array::new(scope, clients.len() as i32);
            for (index, client) in clients.into_iter().enumerate() {
                let (value, _) = message_source(
                    scope,
                    state,
                    MessageSource::Client {
                        client,
                        origin: String::new(),
                    },
                )?;
                let key = v8::String::new(scope, &index.to_string())
                    .ok_or(V8BindingError::Allocation("service result index"))?;
                if array.create_data_property(scope, key.into(), value) != Some(true) {
                    return Err(V8BindingError::Allocation("service result property"));
                }
            }
            Ok(array.into())
        }
        ServiceResult::Registration(Some(snapshot)) => registration(scope, state, snapshot),
        ServiceResult::Registration(None) => Ok(v8::undefined(scope).into()),
        ServiceResult::Registrations(snapshots) => {
            let array = v8::Array::new(scope, snapshots.len() as i32);
            for (index, snapshot) in snapshots.into_iter().enumerate() {
                let value = registration(scope, state, snapshot)?;
                let key = v8::String::new(scope, &index.to_string())
                    .ok_or(V8BindingError::Allocation("service result index"))?;
                if array.create_data_property(scope, key.into(), value) != Some(true) {
                    return Err(V8BindingError::Allocation("service result property"));
                }
            }
            Ok(array.into())
        }
        ServiceResult::Bool(value) => Ok(v8::Boolean::new(scope, value).into()),
        ServiceResult::Done => Ok(v8::undefined(scope).into()),
    }
}
pub(super) fn enable_messages(state: &V8BindingState) {
    state.service_workers.borrow_mut().messages_enabled = true;
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state.execution_tasks_stopped() || state.worker_transport.borrow().is_none() {
        return Ok(0);
    }
    if !has_pending(state)
        && state.service_workers.borrow().registrations.is_empty()
        && state.service_workers.borrow().workers.is_empty()
    {
        return Ok(0);
    }
    {
        let mut bindings = state.service_workers.borrow_mut();
        if bindings.polling {
            return Ok(0);
        }
        bindings.polling = true;
    }
    let pending = std::mem::take(&mut state.service_workers.borrow_mut().pending);
    let mut batch = PollBatch {
        state: state.clone(),
        pending: pending.into(),
        publication: None,
    };
    let creator = workers::native_creator(state)?;
    let mut count = 0;
    while let Some(pending) = batch.pending.front_mut() {
        if state.execution_tasks_stopped() || scope.is_execution_terminating() {
            return Ok(count);
        }
        if pending.result.is_none() {
            match request(
                state,
                ServiceRequest::Poll {
                    creator: creator.clone(),
                    operation: pending.operation,
                },
            ) {
                Ok(ServiceResponse::Pending { operation }) if operation == pending.operation => {
                    let pending = batch.pending.pop_front().unwrap();
                    state.service_workers.borrow_mut().pending.push(pending);
                    continue;
                }
                Ok(ServiceResponse::Complete { operation, result })
                    if operation == pending.operation =>
                {
                    pending.result = Some(result)
                }
                Err(error) => return Err(error),
                _ => return Err(failure("InvalidStateError")),
            }
        }
        if !pending.settled {
            let resolver = v8::Local::new(scope, &pending.resolver);
            match pending.result.as_ref().unwrap() {
                Ok(value) => {
                    let value = result_value(scope, state, value.clone())?;
                    if resolver.resolve(scope, value) != Some(true) {
                        return Err(V8BindingError::JavaScriptException);
                    }
                }
                Err(error) => {
                    let value = dom_exception_value(scope, &error.name)?;
                    if resolver.reject(scope, value) != Some(true) {
                        return Err(V8BindingError::JavaScriptException);
                    }
                }
            }
            pending.settled = true;
        }
        if !matches!(
            request(
                state,
                ServiceRequest::Acknowledge {
                    creator: creator.clone(),
                    operation: pending.operation
                }
            )?,
            ServiceResponse::Done
        ) {
            return Err(failure("InvalidStateError"));
        }
        batch.pending.pop_front();
        count += 1;
        scope.perform_microtask_checkpoint();
    }
    if state.execution_tasks_stopped() || scope.is_execution_terminating() {
        return Ok(count);
    }
    // Complete publication of an already-received snapshot before consuming
    // another destructive native snapshot response.
    if !state.service_workers.borrow().publications.is_empty()
        || state.service_workers.borrow().relationships_dirty
    {
        return drain_publications(scope, state, &mut batch).map(|delivered| count + delivered);
    }
    let take_messages = state.binding_global == BindingGlobal::Worker
        || state.service_workers.borrow().messages_enabled;
    let ServiceResponse::Snapshot(snapshot) = request(
        state,
        ServiceRequest::Snapshot {
            creator,
            take_messages,
        },
    )?
    else {
        return Err(failure("InvalidStateError"));
    };
    let current_registrations = snapshot
        .registrations
        .iter()
        .map(|registration| registration.id)
        .collect::<BTreeSet<_>>();
    let messages = snapshot.messages;
    let mut events = Vec::new();
    {
        let mut bindings = state.service_workers.borrow_mut();
        let live = snapshot
            .registrations
            .iter()
            .flat_map(|registration| {
                [
                    &registration.installing,
                    &registration.waiting,
                    &registration.active,
                ]
                .into_iter()
                .flatten()
                .map(|worker| worker.version_id)
            })
            .chain(snapshot.controller.iter().map(|worker| worker.version_id))
            .collect::<BTreeSet<_>>();
        let retired = bindings
            .workers
            .iter()
            .filter(|(id, worker)| !live.contains(id) && worker.state != WorkerState::Redundant)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for worker in retired {
            bindings.workers.get_mut(&worker).unwrap().state = WorkerState::Redundant;
            if let Some((id, _)) = bindings
                .entities
                .iter()
                .find(|(_, entity)| matches!(entity.kind,Kind::Worker(id) if id==worker))
            {
                events.push((*id, "statechange"));
            }
        }
        let controller = snapshot.controller.as_ref().map(|worker| worker.version_id);
        if bindings.controller != controller {
            bindings.controller = controller;
            if let Some(id) = bindings.container {
                events.push((id, "controllerchange"));
            }
        }
        if let Some(controller) = snapshot.controller {
            bindings.workers.insert(controller.version_id, controller);
        }
        for registration in snapshot.registrations {
            let update = bindings
                .registrations
                .get(&registration.id)
                .is_some_and(|previous| {
                    previous.installing.as_ref().map(|worker| worker.version_id)
                        != registration
                            .installing
                            .as_ref()
                            .map(|worker| worker.version_id)
                })
                && registration.installing.is_some();
            if update
                && let Some((id,_))=bindings.entities.iter().find(|(_,entity)|matches!(entity.kind,Kind::Registration(id) if id==registration.id)){events.push((*id,"updatefound"));}
            for worker in [
                &registration.installing,
                &registration.waiting,
                &registration.active,
            ]
            .into_iter()
            .flatten()
            {
                if bindings
                    .workers
                    .get(&worker.version_id)
                    .is_some_and(|previous| previous.state != worker.state)
                    && let Some((id,_))=bindings.entities.iter().find(|(_,entity)|matches!(entity.kind,Kind::Worker(id) if id==worker.version_id)){events.push((*id,"statechange"));}
                bindings.workers.insert(worker.version_id, worker.clone());
            }
            bindings.registrations.insert(registration.id, registration);
        }
    }
    {
        let mut bindings = state.service_workers.borrow_mut();
        bindings.relationships_dirty = true;
        for (id, name) in events {
            if let Some(target) = bindings
                .entities
                .get(&id)
                .and_then(|entity| entity.object.to_local(scope))
            {
                bindings.publications.push_back(Publication {
                    payload: PublicationPayload::Event(id, name),
                    _target: v8::Global::new(scope, target),
                    dispatch: None,
                });
            }
        }
        for message in messages {
            let cleanup = structured_clone::shared_receive_guard(state, &message.value);
            if let Some(container) = bindings.container
                && let Some(target) = bindings
                    .entities
                    .get(&container)
                    .and_then(|entity| entity.object.to_local(scope))
            {
                bindings.publications.push_back(Publication {
                    payload: PublicationPayload::Message(Box::new(MessagePublication {
                        activation: message.value.user_activation(),
                        container,
                        source: message.source,
                        origin: message.origin,
                        wire: Some(message.value),
                        cleanup: Some(cleanup),
                        source_object: None,
                        decoded: None,
                    })),
                    _target: v8::Global::new(scope, target),
                    dispatch: None,
                });
            }
        }
    }
    {
        let mut bindings = state.service_workers.borrow_mut();
        let retained_workers = bindings
            .entities
            .values()
            .filter_map(|entity| {
                if let Kind::Worker(id) = entity.kind {
                    Some(id)
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        bindings.workers.retain(|id, worker| {
            worker.state != WorkerState::Redundant || retained_workers.contains(id)
        });
        let retained_registrations = bindings
            .entities
            .values()
            .filter_map(|entity| match entity.kind {
                Kind::Registration(id) | Kind::Preload(id) => Some(id),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        bindings.registrations.retain(|id, _| {
            current_registrations.contains(id) || retained_registrations.contains(id)
        });
    }
    count += drain_publications(scope, state, &mut batch)?;
    Ok(count)
}
fn drain_publications(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    batch: &mut PollBatch,
) -> Result<usize, V8BindingError> {
    if state.service_workers.borrow().relationships_dirty {
        refresh_relationships(scope, state)?;
        state.service_workers.borrow_mut().relationships_dirty = false;
    }
    let mut count = 0;
    loop {
        if state.execution_tasks_stopped() || scope.is_execution_terminating() {
            return Ok(count);
        }
        batch.publication = state.service_workers.borrow_mut().publications.pop_front();
        let Some(publication) = batch.publication.as_mut() else {
            break;
        };
        if publication.dispatch.is_none() {
            let (target, name, event) = match &mut publication.payload {
                PublicationPayload::Event(target, name) => (
                    *target,
                    *name,
                    create_event_object(scope, state, name, false, false)?,
                ),
                PublicationPayload::Message(message) => {
                    let MessagePublication {
                        activation,
                        container,
                        source,
                        origin,
                        wire,
                        cleanup,
                        source_object,
                        decoded,
                    } = message.as_mut();

                    if source_object.is_none() {
                        let (value, _) = message_source(
                            scope,
                            state,
                            MessageSource::ServiceWorker {
                                worker: source.clone(),
                                origin: origin.clone(),
                            },
                        )?;
                        *source_object = Some(v8::Global::new(scope, value));
                    }
                    if decoded.is_none() {
                        if let Some(cleanup) = cleanup.as_mut() {
                            cleanup.handoff_ports();
                        }
                        let value = wire
                            .take()
                            .ok_or(V8BindingError::Allocation("service message wire custody"))?;
                        *decoded = Some({
                            v8::tc_scope!(let tc,scope);
                            structured_clone::deserialize_wire(tc, value).map(|(value, ports)| {
                                (
                                    v8::Global::new(tc, value),
                                    ports
                                        .into_iter()
                                        .map(|port| v8::Global::new(tc, port))
                                        .collect::<Vec<_>>(),
                                )
                            })
                        });
                        cleanup.take();
                    }
                    let (event, name) = if let Some((value, ports)) = decoded.as_ref().unwrap() {
                        let value = v8::Local::new(scope, value);
                        let ports = ports
                            .iter()
                            .map(|port| v8::Local::new(scope, port))
                            .collect::<Vec<_>>();
                        (
                            message_events::delivery(scope, state, value, &ports)?,
                            "message",
                        )
                    } else {
                        let event = create_event_object_with_interface(
                            scope,
                            state,
                            "MessageEvent",
                            "messageerror",
                            false,
                            false,
                        )?;
                        message_events::initialize_empty(scope, state, event)
                            .ok_or(V8BindingError::Allocation("service messageerror"))?;
                        (event, "messageerror")
                    };
                    message_events::set_user_activation(scope, state, event, *activation)?;
                    let fields = event
                        .get_internal_field(scope, 2)
                        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
                        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
                        .ok_or(V8BindingError::Allocation("service message fields"))?;
                    let source = v8::Local::new(scope, source_object.as_ref().unwrap());
                    let origin = v8_string(scope, origin)?;
                    for (index, value) in [("3", source), ("1", origin)] {
                        let key = v8::String::new(scope, index)
                            .ok_or(V8BindingError::Allocation("service message index"))?;
                        if fields.create_data_property(scope, key.into(), value) != Some(true) {
                            return Err(V8BindingError::Allocation("service message field"));
                        }
                    }
                    (*container, name, event)
                }
            };
            publication.dispatch = Some(native_platform::publication::pending_platform_dispatch(
                scope, state, target, name, event,
            ));
        }
        let dispatch = publication.dispatch.as_mut().unwrap();
        if !dispatch.resume(scope, state)? {
            return Ok(count);
        }
        dispatch.finish(scope, state);
        batch.publication = None;
        count += 1;
        scope.perform_microtask_checkpoint();
    }
    Ok(count)
}
pub(super) fn retire(state: &V8BindingState) {
    let mut bindings = state.service_workers.borrow_mut();
    bindings.pending.clear();
    bindings.ready = None;
    bindings.relationships_dirty = false;
    for publication in bindings.publications.drain(..) {
        if let Some(dispatch) = publication.dispatch {
            dispatch.cancel(state);
        }
    }
}

/// Independently owned workers receive a native WorkerNavigator and the same
/// native container message queue. It never borrows a creator Window object.
pub fn install_worker_container(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let global = context.global(scope);
    let constructor = v8::Function::new(scope, illegal)
        .ok_or(V8BindingError::Allocation("WorkerNavigator constructor"))?;
    let key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("WorkerNavigator prototype"))?;
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "WorkerNavigator").unwrap();
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    let key = v8::String::new(scope, "WorkerNavigator").unwrap();
    global.create_data_property(scope, key.into(), constructor.into());
    let navigator = v8::Object::new(scope);
    navigator.set_prototype(scope, prototype.into());
    let key = v8::String::new(scope, "navigator").unwrap();
    global.define_own_property(
        scope,
        key.into(),
        navigator.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    install(scope, context)
}

pub fn install_agent(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let state = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| failure("InvalidStateError"))?;
    install_prototypes(scope, &state, false)?;
    let global = context.global(scope);
    let getter = v8::Function::new(scope, agent_registration)
        .ok_or(V8BindingError::Allocation("service registration"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "registration").unwrap();
    global.define_property(scope, key.into(), &descriptor);
    let skip = v8::Function::new(scope, agent_skip_waiting)
        .ok_or(V8BindingError::Allocation("skipWaiting"))?;
    let key = v8::String::new(scope, "skipWaiting").unwrap();
    global.create_data_property(scope, key.into(), skip.into());
    let constructor = v8::Function::new(scope, illegal)
        .ok_or(V8BindingError::Allocation("Clients constructor"))?;
    let key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Clients prototype"))?;
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "Clients").unwrap();
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    let key = v8::String::new(scope, "Clients").unwrap();
    global.create_data_property(scope, key.into(), constructor.into());
    let clients = v8::Object::new(scope);
    clients.set_prototype(scope, prototype.into());
    for (index, name) in [(0, "get"), (1, "matchAll"), (2, "claim"), (3, "openWindow")] {
        let data = v8::Integer::new(scope, index);
        let method = v8::Function::builder(agent_clients)
            .data(data.into())
            .build(scope)
            .ok_or(V8BindingError::Allocation("clients method"))?;
        let key = v8::String::new(scope, name).unwrap();
        prototype.create_data_property(scope, key.into(), method.into());
    }
    let retained =
        context_intrinsics::retain_object_for(scope, &state, "service-worker-clients", clients)?;
    state.service_workers.borrow_mut().clients_object = Some(retained);
    let key = v8::String::new(scope, "clients").unwrap();
    global.define_own_property(
        scope,
        key.into(),
        clients.into(),
        v8::PropertyAttribute::READ_ONLY,
    );
    Ok(())
}
fn agent_clients(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let object = state
            .service_workers
            .borrow()
            .clients_object
            .clone()
            .ok_or_else(|| failure("TypeError"))?;
        let object = object.to_local(scope).ok_or_else(|| failure("TypeError"))?;
        if !object.strict_equals(args.this().into()) {
            return Err(failure("TypeError"));
        }
        let command = match args.data().int32_value(scope).unwrap_or(-1) {
            0 => {
                if args.length() == 0 {
                    return Err(failure("TypeError"));
                }
                ServiceRequest::ClientsGet {
                    client: required_string(scope, &args, 0)?,
                }
            }
            1 => {
                let mut include_uncontrolled = false;
                let mut client_type = Some(ClientType::Window);
                let options = args.get(0);
                if !options.is_null_or_undefined() {
                    let options = options
                        .to_object(scope)
                        .ok_or_else(|| failure("TypeError"))?;
                    let key = v8::String::new(scope, "includeUncontrolled").unwrap();
                    include_uncontrolled = options
                        .get(scope, key.into())
                        .ok_or(V8BindingError::JavaScriptException)?
                        .boolean_value(scope);
                    let key = v8::String::new(scope, "type").unwrap();
                    let value = options
                        .get(scope, key.into())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    if !value.is_undefined() {
                        client_type = match value
                            .to_string(scope)
                            .ok_or(V8BindingError::JavaScriptException)?
                            .to_rust_string_lossy(scope)
                            .as_str()
                        {
                            "window" => Some(ClientType::Window),
                            "worker" => Some(ClientType::DedicatedWorker),
                            "sharedworker" => Some(ClientType::SharedWorker),
                            "all" => None,
                            _ => return Err(failure("TypeError")),
                        };
                    }
                }
                ServiceRequest::ClientsMatchAll {
                    include_uncontrolled,
                    client_type,
                }
            }
            2 => ServiceRequest::Claim,
            3 => {
                if args.length() == 0 {
                    return Err(failure("TypeError"));
                }
                ServiceRequest::ClientsOpenWindow {
                    url: required_string(scope, &args, 0)?,
                }
            }
            _ => return Err(failure("TypeError")),
        };
        begin(scope, &state, command).map(Into::into)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn agent_skip_waiting(
    scope: &mut v8::PinScope,
    _: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    agent_operation(scope, ServiceRequest::SkipWaiting, &mut rv)
}
fn agent_operation(scope: &mut v8::PinScope, request: ServiceRequest, rv: &mut v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    match begin(scope, &state, request) {
        Ok(promise) => rv.set(promise.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(super) fn has_pending(state: &V8BindingState) -> bool {
    if state.worker_transport.borrow().is_none() {
        return false;
    }
    let bindings = state.service_workers.borrow();
    !bindings.pending.is_empty()
        || !bindings.publications.is_empty()
        || bindings.relationships_dirty
        || bindings.entities.keys().any(|id| {
            state
                .platform_listeners
                .borrow()
                .keys()
                .any(|(target, _)| target == id)
                || state
                    .platform_handlers
                    .borrow()
                    .keys()
                    .any(|(target, _)| target == id)
        })
}

fn agent_registration(
    scope: &mut v8::PinScope,
    _: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<v8::Local<v8::Value>, V8BindingError> {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| failure("InvalidStateError"))?;
        let ServiceResponse::Registration(snapshot) =
            request(&state, ServiceRequest::WorkerRegistration)?
        else {
            return Err(failure("InvalidStateError"));
        };
        registration(scope, &state, snapshot)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

fn client_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.native.service.client").unwrap();
    v8::Private::for_api(scope, Some(name))
}
pub(super) fn message_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    source: MessageSource,
) -> Result<(v8::Local<'s, v8::Value>, String), V8BindingError> {
    match source {
        MessageSource::ServiceWorker { worker, origin } => {
            let id = worker.version_id;
            state
                .service_workers
                .borrow_mut()
                .workers
                .insert(id, worker);
            Ok((wrap(scope, state, Kind::Worker(id))?.into(), origin))
        }
        MessageSource::Client { client, origin } => {
            let existing = state
                .service_workers
                .borrow()
                .clients
                .iter()
                .find(|(_, existing)| existing.id == client.id)
                .map(|(id, _)| *id);
            let id = existing.unwrap_or_else(|| {
                register_platform_object(state, PlatformObjectRecord::EventTarget)
            });
            let window = client.client_type == ClientType::Window;
            state
                .service_workers
                .borrow_mut()
                .clients
                .insert(id, client);
            if let Some(object) = state
                .service_workers
                .borrow()
                .entities
                .get(&id)
                .and_then(|entity| entity.object.to_local(scope))
            {
                return Ok((object.into(), origin));
            }
            let object = v8::Object::new(scope);
            let key = client_key(scope);
            let value = v8::BigInt::new_from_u64(scope, id);
            object.set_private(scope, key, value.into());
            let prototype = state
                .service_workers
                .borrow()
                .prototypes
                .get(if window { "WindowClient" } else { "Client" })
                .cloned()
                .ok_or_else(|| failure("InvalidStateError"))?;
            let prototype = prototype
                .to_local(scope)
                .ok_or_else(|| failure("InvalidStateError"))?;
            object.set_prototype(scope, prototype.into());
            state.service_workers.borrow_mut().entities.insert(
                id,
                Entity {
                    kind: Kind::Client(id, window),
                    object: v8::Weak::new(scope, object),
                },
            );
            Ok((object.into(), origin))
        }
    }
}

pub(super) fn is_native_worker(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    if object.is_proxy() {
        return false;
    }
    let Some(context) = object.get_creation_context(scope) else {
        return false;
    };
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return false;
    };
    matches!(entity(scope, &state, object), Ok((_, Kind::Worker(_))))
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    #[test]
    fn native_secure_context_controls_window_and_worker_container_exposure() {
        super::super::tests::initialize_v8();
        for worker in [false, true] {
            for opaque in [false, true] {
                for secure in [false, true] {
                    let mut isolate = v8::Isolate::new(Default::default());
                    v8::scope!(let scope,&mut isolate);
                    let context = v8::Context::new(scope, Default::default());
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let origin = if opaque {
                        crate::origin::Origin::opaque()
                    } else {
                        crate::origin::Origin::from_url("https://service.test")
                    };
                    context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                        id: RealmId(0),
                        origin: origin.clone(),
                        document_generation: 1,
                        frame_id: None,
                    }));
                    context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
                    let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                    let document = dom.borrow().document();
                    let state = install_generated_bindings_with_root_for_global(
                        scope,
                        context,
                        dom,
                        RealmId(0),
                        1,
                        document,
                        if worker {
                            BindingGlobal::Worker
                        } else {
                            BindingGlobal::Window
                        },
                    )
                    .unwrap();
                    if worker {
                        install_worker_container(scope, context).unwrap();
                    }
                    assert_eq!(
                        context
                            .get_slot::<crate::realm::RealmDescriptor>()
                            .unwrap()
                            .origin,
                        origin
                    );
                    let source = if secure {
                        r#"(()=>{
                if(typeof navigator.serviceWorker!=='object'||!('serviceWorker' in navigator))throw Error('secure service container missing');
                const descriptor=Object.getOwnPropertyDescriptor(navigator,'serviceWorker');
                if(!descriptor||descriptor.value!==navigator.serviceWorker||navigator.serviceWorker!==navigator.serviceWorker)throw Error('service container identity');
                if(Object.getPrototypeOf(navigator.serviceWorker)!==ServiceWorkerContainer.prototype)throw Error('service native prototype');
                return true;
            })()"#
                    } else {
                        r#"(()=>{
                if(typeof navigator.serviceWorker!=='undefined'||'serviceWorker' in navigator)throw Error('insecure service container exposed');
                if(Object.getOwnPropertyDescriptor(navigator,'serviceWorker'))throw Error('insecure navigator descriptor');
                for(const name of ['Navigator','WorkerNavigator']){
                    const constructor=globalThis[name];
                    if(constructor&&(Object.getOwnPropertyDescriptor(constructor.prototype,'serviceWorker')||'serviceWorker' in constructor.prototype))throw Error('insecure service prototype');
                }
                for(const name of ['ServiceWorker','ServiceWorkerRegistration','ServiceWorkerContainer','NavigationPreloadManager'])if(name in globalThis)throw Error('insecure service constructor');
                return true;
            })()"#
                    };
                    let source = v8::String::new(scope, source).unwrap();
                    assert!(
                        v8::Script::compile(scope, source, None)
                            .unwrap()
                            .run(scope)
                            .unwrap()
                            .is_true()
                    );
                    assert_eq!(state.service_workers.borrow().container.is_some(), secure);
                }
            }
        }
    }
    use crate::worker_wire::{AgentId, Creator, WorkerError, WorkerTransport};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Transport {
        fail_ack: AtomicBool,
        results: Mutex<BTreeMap<u64, ServiceResult>>,
        snapshot: Mutex<Option<ContainerSnapshot>>,
        controller: Mutex<Option<WorkerSnapshot>>,
        polls: AtomicUsize,
        snapshots: AtomicUsize,
    }
    impl WorkerTransport for Transport {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            match request {
                AgentServiceRequest::Service(ServiceRequest::Poll { operation, .. }) => {
                    self.polls.fetch_add(1, Ordering::SeqCst);
                    let result =
                        self.results
                            .lock()
                            .unwrap()
                            .remove(&operation)
                            .ok_or_else(|| {
                                WorkerError::new("InvalidStateError", "result already consumed")
                            })?;
                    Ok(AgentServiceResponse::Service(ServiceResponse::Complete {
                        operation,
                        result: Ok(result),
                    }))
                }
                AgentServiceRequest::Service(ServiceRequest::Acknowledge { .. }) => {
                    if self.fail_ack.swap(false, Ordering::SeqCst) {
                        Err(WorkerError::new("OperationError", "lost acknowledgement"))
                    } else {
                        Ok(AgentServiceResponse::Service(ServiceResponse::Done))
                    }
                }
                AgentServiceRequest::Service(ServiceRequest::Controller { .. }) => {
                    let controller = self
                        .snapshot
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map(|snapshot| snapshot.controller.clone())
                        .unwrap_or_else(|| self.controller.lock().unwrap().clone());
                    Ok(AgentServiceResponse::Service(ServiceResponse::Controller(
                        controller,
                    )))
                }
                AgentServiceRequest::Service(ServiceRequest::Snapshot {
                    take_messages, ..
                }) => {
                    self.snapshots.fetch_add(1, Ordering::SeqCst);
                    let mut snapshot = self.snapshot.lock().unwrap();
                    let mut value = snapshot.clone().unwrap_or(ContainerSnapshot {
                        controller: self.controller.lock().unwrap().clone(),
                        registrations: Vec::new(),
                        messages: Vec::new(),
                    });
                    // Taking queued messages does not detach the client's controller.
                    *self.controller.lock().unwrap() = value.controller.clone();
                    if take_messages {
                        snapshot.take();
                    } else {
                        value.messages.clear();
                    }
                    Ok(AgentServiceResponse::Service(ServiceResponse::Snapshot(
                        value,
                    )))
                }
                _ => Ok(AgentServiceResponse::Done),
            }
        }
    }
    fn transport() -> Arc<Transport> {
        Arc::new(Transport {
            fail_ack: AtomicBool::new(false),
            results: Mutex::new(BTreeMap::new()),
            snapshot: Mutex::new(None),
            controller: Mutex::new(None),
            polls: AtomicUsize::new(0),
            snapshots: AtomicUsize::new(0),
        })
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    fn registration_snapshot() -> RegistrationSnapshot {
        RegistrationSnapshot {
            id: RegistrationId(4),
            scope: "https://example.test/".into(),
            update_via_cache: "imports".into(),
            navigation_preload: Default::default(),
            installing: None,
            waiting: None,
            active: None,
        }
    }
    #[test]
    fn service_transport_failure_preserves_native_name_and_detail() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.set_worker_transport(transport(), Some(Creator::Agent(AgentId(1))), None);
        let error = request(
            &state,
            ServiceRequest::Poll {
                creator: Creator::Agent(AgentId(1)),
                operation: 99,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "native service InvalidStateError: result already consumed"
        );
        v8::tc_scope!(let scope, scope);
        throw_binding_error(scope, &error);
        let exception = scope.exception().unwrap().try_cast::<v8::Object>().unwrap();
        scope.reset();
        let name = v8::String::new(scope, "name").unwrap();
        assert_eq!(
            exception
                .get(scope, name.into())
                .unwrap()
                .to_rust_string_lossy(scope),
            "InvalidStateError"
        );
    }
    #[test]
    fn completed_service_operations_survive_wrapper_preparation_failure_without_repoll() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_prototypes(scope, &state, true).unwrap();
        let transport = transport();
        state.set_worker_transport(transport.clone(), Some(Creator::Agent(AgentId(1))), None);
        transport.results.lock().unwrap().extend([
            (
                11,
                ServiceResult::Registration(Some(registration_snapshot())),
            ),
            (12, ServiceResult::Bool(true)),
        ]);
        let first = v8::PromiseResolver::new(scope).unwrap();
        let second = v8::PromiseResolver::new(scope).unwrap();
        for (operation, resolver) in [(11, first), (12, second)] {
            state.service_workers.borrow_mut().pending.push(Pending {
                settled: false,
                operation,
                resolver: v8::Global::new(scope, resolver),
                result: None,
            });
        }
        let prototype = state
            .service_workers
            .borrow_mut()
            .prototypes
            .remove("ServiceWorkerRegistration")
            .unwrap();
        assert!(poll(scope, &state).is_err());
        assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
        assert_eq!(state.service_workers.borrow().pending.len(), 2);
        assert_eq!(first.get_promise(scope).state(), v8::PromiseState::Pending);
        assert_eq!(second.get_promise(scope).state(), v8::PromiseState::Pending);
        state
            .service_workers
            .borrow_mut()
            .prototypes
            .insert("ServiceWorkerRegistration", prototype);
        transport.fail_ack.store(true, Ordering::SeqCst);
        assert!(poll(scope, &state).is_err());
        assert_eq!(
            first.get_promise(scope).state(),
            v8::PromiseState::Fulfilled
        );
        assert_eq!(second.get_promise(scope).state(), v8::PromiseState::Pending);
        assert!(state.service_workers.borrow().pending[0].settled);
        assert_eq!(transport.polls.load(Ordering::SeqCst), 1);
        assert_eq!(poll(scope, &state).unwrap(), 2);
        assert_eq!(transport.polls.load(Ordering::SeqCst), 2);
        assert_eq!(
            first.get_promise(scope).state(),
            v8::PromiseState::Fulfilled
        );
        assert!(second.get_promise(scope).result(scope).is_true());
    }
    #[test]
    fn service_message_queue_waits_for_start_messages_without_blocking_controller_metadata() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_prototypes(scope, &state, true).unwrap();
        let transport = transport();
        state.set_worker_transport(transport.clone(), Some(Creator::Agent(AgentId(1))), None);
        let container = wrap(scope, &state, Kind::Container).unwrap();
        state.service_workers.borrow_mut().container = Some(stateful_id(scope, container).unwrap());
        let key = v8::String::new(scope, "container").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), container.into());
        eval(
            scope,
            "globalThis.seen=[];container.addEventListener('controllerchange',()=>seen.push('controller'));container.addEventListener('message',()=>seen.push('message'))",
        );
        let worker = WorkerSnapshot {
            version_id: ServiceVersionId([2; 16]),
            id: AgentId(9),
            script_url: "https://example.test/service.js".into(),
            state: WorkerState::Activated,
        };
        let value = eval(scope, "'payload'");
        let wire = structured_clone::serialize_wire(scope, value, &[]).unwrap();
        *transport.snapshot.lock().unwrap() = Some(ContainerSnapshot {
            controller: Some(worker.clone()),
            registrations: Vec::new(),
            messages: vec![ClientMessage {
                source: worker,
                origin: "https://example.test".into(),
                value: wire,
            }],
        });
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(
            eval(
                scope,
                "seen.join(',')==='controller'&&container.controller!==null"
            )
            .is_true()
        );
        assert_eq!(
            transport
                .snapshot
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .messages
                .len(),
            1
        );
        eval(scope, "container.startMessages()");
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "seen.join(',')==='controller,message'").is_true());
        assert!(transport.snapshot.lock().unwrap().is_none());
    }
    #[test]
    fn service_snapshot_notifications_keep_phase_and_message_custody_across_retry() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_prototypes(scope, &state, true).unwrap();
        let transport = transport();
        state.set_worker_transport(transport.clone(), Some(Creator::Agent(AgentId(1))), None);
        let container = wrap(scope, &state, Kind::Container).unwrap();
        state.service_workers.borrow_mut().container = Some(stateful_id(scope, container).unwrap());
        let key = v8::String::new(scope, "container").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), container.into());
        eval(
            scope,
            "globalThis.seen=[];globalThis.saved=null;container.addEventListener('controllerchange',e=>{saved=e;seen.push('first')},{once:true});container.addEventListener('controllerchange',e=>{if(saved!==e)throw Error('event replaced');seen.push('second')});container.onmessage=e=>{globalThis.messageObserved=[e.type,e.data,e.origin,e.source===container.controller];if(e.data!=='payload'||e.origin!=='https://example.test'||e.source!==container.controller)throw Error('message identity');seen.push('message')};",
        );
        let worker = WorkerSnapshot {
            version_id: ServiceVersionId([1; 16]),
            id: AgentId(8),
            script_url: "https://example.test/service.js".into(),
            state: WorkerState::Activated,
        };
        let value = eval(scope, "'payload'");
        let wire = structured_clone::serialize_wire(scope, value, &[]).unwrap();
        *transport.snapshot.lock().unwrap() = Some(ContainerSnapshot {
            controller: Some(worker.clone()),
            registrations: Vec::new(),
            messages: vec![ClientMessage {
                source: worker,
                origin: "https://example.test".into(),
                value: wire,
            }],
        });
        native_platform::publication::fail_after_next_callback();
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='first'").is_true());
        let template = state
            .instance_templates
            .borrow_mut()
            .remove("MessageEvent")
            .unwrap();
        assert!(poll(scope, &state).is_err());
        assert!(eval(scope, "seen.join(',')==='first,second'").is_true());
        state
            .instance_templates
            .borrow_mut()
            .insert("MessageEvent", template);
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert_eq!(
            eval(scope, "seen.join(',')").to_rust_string_lossy(scope),
            "first,second,message",
            "message details: {}",
            eval(scope, "JSON.stringify(globalThis.messageObserved)").to_rust_string_lossy(scope)
        );
        assert_eq!(transport.snapshots.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn service_messages_enable_only_after_completed_current_document_content_loaded() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        install_prototypes(scope, &state, true).unwrap();
        let transport = transport();
        state.set_worker_transport(transport.clone(), Some(Creator::Agent(AgentId(1))), None);
        let container = wrap(scope, &state, Kind::Container).unwrap();
        state.service_workers.borrow_mut().container = Some(stateful_id(scope, container).unwrap());
        let key = v8::String::new(scope, "container").unwrap();
        context
            .global(scope)
            .create_data_property(scope, key.into(), container.into());
        eval(
            scope,
            "globalThis.seen=[];container.addEventListener('message',()=>seen.push('message'));document.addEventListener('DOMContentLoaded',()=>seen.push('first'),{once:true});document.addEventListener('DOMContentLoaded',()=>seen.push('last'))",
        );
        let worker = WorkerSnapshot {
            version_id: ServiceVersionId([3; 16]),
            id: AgentId(10),
            script_url: "https://example.test/service.js".into(),
            state: WorkerState::Activated,
        };
        let value = eval(scope, "'payload'");
        let wire = structured_clone::serialize_wire(scope, value, &[]).unwrap();
        *transport.snapshot.lock().unwrap() = Some(ContainerSnapshot {
            controller: None,
            registrations: Vec::new(),
            messages: vec![ClientMessage {
                source: worker,
                origin: "https://example.test".into(),
                value: wire,
            }],
        });
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(!state.service_workers.borrow().messages_enabled);
        native_platform::publication::fail_after_next_callback();
        let receipt =
            navigation_api::start_native_document_lifecycle_publication(scope, true).unwrap();
        assert!(eval(scope, "seen.join(',')==='first'").is_true());
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(!state.service_workers.borrow().messages_enabled);
        assert_eq!(
            transport
                .snapshot
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .messages
                .len(),
            1
        );
        // Exercise the real retained-publication retry admission delay.
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(navigation_api::resume_native_history_publication(scope, receipt).unwrap());
        assert!(state.service_workers.borrow().messages_enabled);
        assert_eq!(poll(scope, &state).unwrap(), 1);
        assert!(eval(scope, "seen.join(',')==='first,last,message'").is_true());
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[test]
    fn secure_window_and_service_agent_caches_collect_after_last_author_owner() {
        super::super::tests::initialize_v8();
        for agent in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            let (context, state, anchor) = {
                v8::scope!(let handles, &mut isolate);
                let context = v8::Context::new(handles, Default::default());
                let scope = &mut v8::ContextScope::new(handles, context);
                context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                    id: RealmId(995),
                    origin: crate::origin::Origin::from_url("https://service.test"),
                    document_generation: 1,
                    frame_id: None,
                }));
                context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body></body>")
                        .into_runtime()
                        .unwrap(),
                ));
                let document = dom.borrow().document();
                let state = install_generated_bindings_with_root_for_global(
                    scope,
                    context,
                    dom,
                    RealmId(995),
                    1,
                    document,
                    if agent {
                        BindingGlobal::Worker
                    } else {
                        BindingGlobal::Window
                    },
                )
                .unwrap();
                if agent {
                    install_agent(scope, context).unwrap();
                }
                let entries = state.service_workers.borrow().prototypes.clone();
                assert_eq!(entries.len(), if agent { 5 } else { 4 });
                for (name, prototype) in &entries {
                    let key = v8::String::new(scope, name).unwrap();
                    let replacement = v8::Object::new(scope);
                    assert_eq!(
                        context
                            .global(scope)
                            .set(scope, key.into(), replacement.into()),
                        Some(true)
                    );
                    assert!(prototype.to_local(scope).is_some());
                }
                let object = wrap(scope, &state, Kind::Registration(RegistrationId(31))).unwrap();
                assert!(
                    object.get_prototype(scope).unwrap().strict_equals(
                        entries["ServiceWorkerRegistration"]
                            .to_local(scope)
                            .unwrap()
                            .into()
                    )
                );
                if agent {
                    let clients = state
                        .service_workers
                        .borrow()
                        .clients_object
                        .clone()
                        .unwrap();
                    assert!(
                        super::super::test_eval::eval(scope, "clients")
                            .strict_equals(clients.to_local(scope).unwrap().into())
                    );
                }
                state.retire_execution();
                state.refresh_connected_wrapper_roots(scope);
                (
                    v8::Weak::new(scope, context),
                    state,
                    v8::Global::new(scope, object),
                )
            };
            isolate.low_memory_notification();
            isolate.low_memory_notification();
            {
                v8::scope!(let handles, &mut isolate);
                assert!(context.to_local(handles).is_some());
                assert!(
                    state
                        .service_workers
                        .borrow()
                        .prototypes
                        .values()
                        .all(|value| value.to_local(handles).is_some())
                );
            }
            drop(anchor);
            isolate.low_memory_notification();
            isolate.low_memory_notification();
            v8::scope!(let handles, &mut isolate);
            assert!(context.to_local(handles).is_none());
            assert_eq!(
                state.service_workers.borrow().prototypes.len(),
                if agent { 5 } else { 4 }
            );
            assert!(
                state
                    .service_workers
                    .borrow()
                    .prototypes
                    .values()
                    .all(|value| value.to_local(handles).is_none())
            );
            if agent {
                assert!(
                    state
                        .service_workers
                        .borrow()
                        .clients_object
                        .as_ref()
                        .unwrap()
                        .to_local(handles)
                        .is_none()
                );
            }
        }
    }
}

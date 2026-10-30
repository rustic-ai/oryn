mod remote_projection {
    use super::*;
    include!("remote_window_projection.rs");
}
// Native auxiliary browsing-context identity. Never inspect author properties or
// accept author marker fields as Window brands. Included in host::raw_v8 on Unix.
type RemoteWindowKey = (
    RealmId,
    PageId,
    oryn_js::message_wire::WindowTarget,
    u64,
    u64,
    Option<crate::browsing_groups::GroupToken>,
);

#[derive(Default)]
struct RemoteWindows {
    entries: BTreeMap<RemoteWindowKey, RemoteWindow>,
}
#[derive(Clone)]
struct RemoteWindow {
    group: Option<crate::browsing_groups::GroupToken>,
    page: PageId,
    target: oryn_js::message_wire::WindowTarget,
    owner: Rc<V8BindingState>,
    generation: u64,
    epoch: u64,
    object: v8::Global<v8::Object>,
    location: v8::Global<v8::Object>,
    functions: BTreeMap<i32, v8::Global<v8::Function>>,
}
fn remote_window_owner_live(entry: &RemoteWindow) -> bool {
    !entry.owner.execution_retired()
        && entry.owner.document_generation() == entry.generation
        && entry.owner.document_authority_epoch() == entry.epoch
}
fn remote_window_error(message: impl Into<String>) -> ScriptError {
    ScriptError {
        code: "no such window".into(),
        message: message.into(),
    }
}
fn remote_window_origin(
    scope: &mut v8::PinScope,
    entry: &RemoteWindow,
) -> Result<String, ScriptError> {
    current_environment(scope).map_err(remote_window_error)?;
    let source = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .ok_or_else(ScriptError::retired)?;
    let endpoint = context_endpoint_for_binding(scope, &source)
        .ok_or_else(|| remote_window_error("no parent context authority"))?;
    let state = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .ok_or_else(|| remote_window_error("Window source realm missing"))?;
    let creator = state
        .native_worker_creator()
        .map_err(|error| remote_window_error(error.to_string()))?;
    match endpoint
        .client
        .request(ContextServiceRequest::ReadNativeWindowOrigin {
            source_page_id: endpoint.page_id,
            creator,
            target_page_id: entry.page,
            target: entry.target.clone(),
            expected: entry.group,
        }) {
        Ok(ContextServiceResponse::SecurityOrigin { serialized, .. }) => Ok(serialized),
        _ => Err(remote_window_error(
            "SecurityError: Window origin is not accessible",
        )),
    }
}
fn remote_window_status(
    scope: &mut v8::PinScope,
    page: PageId,
    target: &oryn_js::message_wire::WindowTarget,
) -> Result<bool, ScriptError> {
    current_environment(scope).map_err(remote_window_error)?;
    let source = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .ok_or_else(ScriptError::retired)?;
    let endpoint = context_endpoint_for_binding(scope, &source)
        .ok_or_else(|| remote_window_error("no parent context authority"))?;
    let request = source
        .scoped_message_request(oryn_js::message_wire::MessageRequest::WindowStatus {
            page: page.0,
            target: target.clone(),
        })
        .map_err(|error| remote_window_error(error.to_string()))?;
    match endpoint.client.request(ContextServiceRequest::MessageWire {
        source_page_id: endpoint.page_id,
        request,
    }) {
        Ok(response) => decode_remote_window_status(response),
        Err(error) => Err(remote_window_error(error.to_string())),
    }
}
fn decode_remote_window_status(response: ContextServiceResponse) -> Result<bool, ScriptError> {
    match response {
        ContextServiceResponse::MessageWire {
            result: Ok(oryn_js::message_wire::MessageResponse::WindowLive(live)),
        } => Ok(live),
        ContextServiceResponse::MessageWire { result: Err(error) } => {
            Err(remote_window_error(error))
        }
        _ => Err(remote_window_error(
            "invalid native Window status acknowledgement",
        )),
    }
}
fn remote_window_live(
    scope: &mut v8::PinScope,
    page: PageId,
    target: &oryn_js::message_wire::WindowTarget,
) -> Result<(), ScriptError> {
    if remote_window_status(scope, page, target)? {
        Ok(())
    } else {
        Err(remote_window_error(
            "Window is closed or outside this browser context",
        ))
    }
}

fn remote_window_group(
    scope: &mut v8::PinScope,
    page: PageId,
    expected: Option<crate::browsing_groups::GroupToken>,
) -> Result<
    (
        Option<crate::browsing_groups::GroupToken>,
        Option<PageId>,
        bool,
        oryn_js::message_wire::WindowTarget,
    ),
    ScriptError,
> {
    let source = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .ok_or_else(ScriptError::retired)?;
    let endpoint = context_endpoint_for_binding(scope, &source)
        .ok_or_else(|| remote_window_error("no parent group authority"))?;
    match endpoint
        .client
        .request(ContextServiceRequest::NativeWindowGroup {
            source_page_id: endpoint.page_id,
            target_page_id: page,
            expected,
        }) {
        Ok(ContextServiceResponse::NativeWindowGroup {
            token,
            opener,
            opener_target,
            live,
        }) => Ok((token, opener, live, opener_target)),
        _ => Err(remote_window_error("native Window group unavailable")),
    }
}
fn remote_window_proxy_status(
    scope: &mut v8::PinScope,
    entry: &RemoteWindow,
) -> Result<bool, ScriptError> {
    if !remote_window_owner_live(entry) {
        return Ok(false);
    }
    if !remote_window_status(scope, entry.page, &entry.target)? {
        return Ok(false);
    }
    if let Some(group) = entry.group {
        return remote_window_group(scope, entry.page, Some(group)).map(|(_, _, live, _)| live);
    }
    Ok(true)
}
fn remote_window_proxy_live(scope: &mut v8::PinScope, entry: &RemoteWindow) -> bool {
    remote_window_proxy_status(scope, entry).unwrap_or(false)
}

fn remote_window_identity(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<(PageId, oryn_js::message_wire::WindowTarget)>, ScriptError> {
    if !value.is_object() || value.is_proxy() {
        return Ok(None);
    }
    let entries = scope
        .get_slot::<RemoteWindows>()
        .map(|registry| registry.entries.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for entry in entries {
        let object = v8::Local::new(scope, &entry.object);
        if object.strict_equals(value) {
            if !remote_window_owner_live(&entry) {
                return Err(remote_window_error(
                    "auxiliary Window owner document is retired",
                ));
            }
            if !remote_window_proxy_live(scope, &entry) {
                return Err(remote_window_error(
                    "Window proxy belongs to a retired browsing-context group",
                ));
            }
            return Ok(Some((entry.page, entry.target)));
        }
    }
    Ok(None)
}
fn local_native_root_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    page: PageId,
) -> Option<v8::Local<'s, v8::Object>> {
    let contexts = scope
        .get_slot::<NativeRealmIdentities>()?
        .contexts
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for weak in contexts {
        let Some(context) = weak.to_local(scope) else {
            continue;
        };
        if !context
            .get_slot::<RealmDescriptor>()
            .is_some_and(|descriptor| descriptor.frame_id.is_none())
        {
            continue;
        }
        let Some(state) = context.get_slot::<V8BindingState>() else {
            continue;
        };
        if state.execution_retired() || state.execution_suspended() {
            continue;
        }
        let Some(environment) = context.get_slot::<EnvironmentDescriptor>() else {
            continue;
        };
        if !scope
            .get_slot::<AgentEnvironments>()
            .is_some_and(|environments| environments.is_live(&environment))
        {
            continue;
        }
        if endpoints_for_context(scope, context)
            .and_then(|endpoints| endpoints.context)
            .is_some_and(|endpoint| endpoint.page_id == page)
        {
            return Some(context.global(scope));
        }
    }
    None
}

fn local_native_frame_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    page: PageId,
    token: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let frames = scope
        .get_slot::<FrameLifetimes>()
        .map(|registry| registry.live.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for frame in frames
        .into_iter()
        .filter(|frame| !frame.closed.get() && frame.message_token == token)
    {
        let realm = frame.environment.get().realm;
        let Some(weak) = scope
            .get_slot::<NativeRealmIdentities>()
            .and_then(|registry| registry.contexts.get(&realm))
            .cloned()
        else {
            continue;
        };
        let Some(context) = weak.to_local(scope) else {
            continue;
        };
        let Some(state) = context.get_slot::<V8BindingState>() else {
            continue;
        };
        if state.execution_retired() || state.execution_suspended() {
            continue;
        }
        let Some(environment) = context.get_slot::<EnvironmentDescriptor>() else {
            continue;
        };
        if !scope
            .get_slot::<AgentEnvironments>()
            .is_some_and(|environments| environments.is_live(&environment))
        {
            continue;
        }
        if endpoints_for_context(scope, context)
            .and_then(|endpoints| endpoints.context)
            .is_some_and(|endpoint| endpoint.page_id == page)
        {
            return Some(context.global(scope));
        }
    }
    None
}

fn remote_window_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    page: PageId,
    target: oryn_js::message_wire::WindowTarget,
    automation: bool,
    initial_opener: bool,
) -> Result<v8::Local<'s, v8::Object>, ScriptError> {
    if !initial_opener {
        remote_window_live(scope, page, &target)?;
    }
    let (source_group, _, same_group, _) = remote_window_group(scope, page, None)?;
    if same_group
        && matches!(&target, oryn_js::message_wire::WindowTarget::Root)
        && let Some(window) = local_native_root_window(scope, page)
    {
        return Ok(window);
    }
    if same_group
        && let oryn_js::message_wire::WindowTarget::Frame(token) = &target
        && let Some(window) = local_native_frame_window(scope, page, token)
    {
        return Ok(window);
    }
    let group = if same_group || initial_opener {
        source_group
    } else if automation {
        None
    } else {
        return Err(remote_window_error(
            "Window is in a different browsing-context group",
        ));
    };
    let context = scope.get_current_context();
    let owner = context
        .get_slot::<V8BindingState>()
        .ok_or_else(|| remote_window_error("Window realm is unavailable"))?;
    let key = (
        owner.realm(),
        page,
        target.clone(),
        owner.document_generation(),
        owner.document_authority_epoch(),
        group,
    );
    if let Some(entry) = scope
        .get_slot::<RemoteWindows>()
        .and_then(|registry| registry.entries.get(&key))
        .cloned()
        && remote_window_owner_live(&entry)
    {
        return Ok(v8::Local::new(scope, &entry.object));
    }
    if scope.get_slot::<RemoteWindows>().is_none() {
        scope.set_slot(RemoteWindows::default());
    }
    let registry = scope.get_slot_mut::<RemoteWindows>().unwrap();
    // Retain retired identities: saved old wrappers must fail as stale Windows,
    // never become unbranded author objects. The hard cap bounds retained handles.
    if registry.entries.len() >= 4096 {
        return Err(remote_window_error("auxiliary Window identity limit"));
    }
    let template = v8::ObjectTemplate::new(scope);
    template.set_internal_field_count(1);
    remote_projection::configure(scope, template, false);
    let mut functions = BTreeMap::new();
    let object = template
        .new_instance(scope)
        .ok_or_else(|| remote_window_error("cannot allocate native Window proxy"))?;
    for (name, operation, getter) in [
        ("closed", 0, true),
        ("origin", 1, true),
        ("focus", 2, false),
        ("close", 3, false),
        ("postMessage", 4, false),
        ("top", 5, true),
        ("parent", 6, true),
        ("opener", 7, true),
        ("length", 8, true),
        ("location", 9, true),
        ("blur", 12, false),
    ] {
        let data = v8::Integer::new(scope, operation);
        let function = v8::Function::builder(remote_window_operation)
            .data(data.into())
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(|| remote_window_error("cannot allocate auxiliary Window method"))?;
        functions.insert(operation, v8::Global::new(scope, function));
        let key = v8::String::new(scope, name).unwrap();
        let success = if getter {
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            object.define_property(scope, key.into(), &descriptor)
        } else {
            object.create_data_property(scope, key.into(), function.into())
        };
        if success != Some(true) {
            return Err(remote_window_error(
                "cannot install auxiliary Window method",
            ));
        }
    }
    for name in ["window", "self", "frames"] {
        let key = v8::String::new(scope, name).unwrap();
        object.create_data_property(scope, key.into(), object.into());
    }
    let self_data = v8::Integer::new(scope, 11);
    let self_get = v8::Function::builder(remote_window_operation)
        .data(self_data.into())
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or_else(|| remote_window_error("Window self accessor allocation"))?;
    functions.insert(11, v8::Global::new(scope, self_get));
    let location = remote_location_object(scope, &mut functions)?;
    let key_location = v8::String::new(scope, "location").unwrap();
    let getter_data = v8::Integer::new(scope, 9);
    let setter_data = v8::Integer::new(scope, 10);
    let getter = v8::Function::builder(remote_window_operation)
        .data(getter_data.into())
        .build(scope)
        .ok_or_else(|| remote_window_error("Location getter allocation failed"))?;
    let setter = v8::Function::builder(remote_window_operation)
        .data(setter_data.into())
        .build(scope)
        .ok_or_else(|| remote_window_error("Location setter allocation failed"))?;
    functions.insert(9, v8::Global::new(scope, getter));
    functions.insert(10, v8::Global::new(scope, setter));
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    if object.define_property(scope, key_location.into(), &descriptor) != Some(true) {
        return Err(remote_window_error("Location publication failed"));
    }
    let entry = RemoteWindow {
        functions,
        location: v8::Global::new(scope, location),
        group,
        page,
        target,
        generation: owner.document_generation(),
        epoch: owner.document_authority_epoch(),
        owner,
        object: v8::Global::new(scope, object),
    };
    scope
        .get_slot_mut::<RemoteWindows>()
        .unwrap()
        .entries
        .insert(key, entry);
    Ok(object)
}
fn remote_window_factory(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let input = args.get(0);
    if input.is_null_or_undefined() {
        rv.set(v8::null(scope).into());
        return;
    }
    // Existing context JSON uses numeric IDs. Refuse rounding into another PageId.
    let Some(id) = input.number_value(scope).filter(|value| {
        value.is_finite()
            && *value >= 0.0
            && *value <= 9_007_199_254_740_991.0
            && value.fract() == 0.0
    }) else {
        throw_type_error(scope, "invalid auxiliary page identity");
        return;
    };
    match remote_window_object(
        scope,
        PageId(id as u64),
        oryn_js::message_wire::WindowTarget::Root,
        false,
        false,
    ) {
        Ok(object) => rv.set(object.into()),
        Err(error) => throw_type_error(scope, &error.message),
    }
}
fn remote_window_operation(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    rv: v8::ReturnValue,
) {
    let operation = args.data().int32_value(scope).unwrap_or(-1);
    let receiver = args.this();
    remote_window_operation_for(scope, args, rv, operation, receiver);
}
fn remote_window_operation_for(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
    operation: i32,
    receiver: v8::Local<v8::Object>,
) {
    // Find the receiver without running its getters, including for closed windows.
    let entries = scope
        .get_slot::<RemoteWindows>()
        .map(|registry| registry.entries.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let entry = entries
        .into_iter()
        .find(|entry| v8::Local::new(scope, &entry.object).strict_equals(receiver.into()));
    let Some(entry) = entry else {
        if operation == 10 {
            native_window_navigate_callback(scope, args, rv);
        } else {
            throw_type_error(scope, "auxiliary Window receiver required");
        }
        return;
    };
    if operation == 11 {
        rv.set(v8::Local::new(scope, &entry.object).into());
        return;
    }
    let live = match remote_window_proxy_status(scope, &entry) {
        Ok(live) => live,
        Err(error) => {
            throw_type_error(scope, &error.message);
            return;
        }
    };
    if operation == 4 {
        remote_window_post_message(scope, &args, &entry, live);
        return;
    }
    if operation == 0 {
        let mut closed = !live;
        // A retained group-free reference may become related again when a
        // cached native document restores its group. Only a currently unrelated
        // reference falls back to endpoint liveness; related ones read closing.
        let related = if live
            && entry.group.is_none()
            && matches!(entry.target, oryn_js::message_wire::WindowTarget::Root)
        {
            match remote_window_group(scope, entry.page, None) {
                Ok((_, _, related, _)) => related,
                Err(error) => {
                    throw_type_error(scope, &error.message);
                    return;
                }
            }
        } else {
            entry.group.is_some()
        };
        if live
            && related
            && matches!(entry.target, oryn_js::message_wire::WindowTarget::Root)
            && let Some(source) =
                oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
                    .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
            && let (Ok(creator), Some(endpoint)) = (
                source.native_worker_creator(),
                context_endpoint_for_binding(scope, &source),
            )
        {
            match endpoint
                .client
                .request(ContextServiceRequest::ReadScriptCloseState {
                    source_page_id: endpoint.page_id,
                    creator,
                    target_page_id: entry.page,
                    expected: entry.group,
                }) {
                Ok(ContextServiceResponse::ScriptCloseState { closing }) => closed = closing,
                Ok(_) => {
                    throw_type_error(scope, "Window.closed state ACK mismatch");
                    return;
                }
                Err(error) => {
                    throw_type_error(scope, &error.to_string());
                    return;
                }
            }
        }
        rv.set_bool(closed);
        return;
    }
    if operation == 12 {
        return;
    }
    if !live && matches!(operation, 2 | 3) {
        return;
    }
    if !live {
        throw_type_error(scope, "auxiliary Window is closed or retired");
        return;
    }
    if operation == 1 {
        match remote_window_origin(scope, &entry) {
            Ok(origin) => {
                if let Some(value) = v8::String::new(scope, &origin) {
                    rv.set(value.into());
                } else {
                    throw_type_error(scope, "origin allocation failed");
                }
            }
            Err(_) => oryn_js::v8_bindings::window_messages::throw_external_message_error(
                scope,
                "SecurityError",
            ),
        }
        return;
    }
    if operation == 9 {
        rv.set(v8::Local::new(scope, &entry.location).into());
        return;
    }
    if operation == 10 {
        remote_location_navigate(scope, &args, &entry, false);
        return;
    }
    if (5..=8).contains(&operation) {
        let snapshot = match remote_window_snapshot(scope, &entry) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                oryn_js::v8_bindings::window_messages::throw_external_message_error(
                    scope,
                    "SecurityError",
                );
                return;
            }
        };
        if operation == 8 {
            rv.set(v8::Number::new(scope, snapshot.length as f64).into());
            return;
        }
        let destination = match operation {
            5 => Some((entry.page, oryn_js::message_wire::WindowTarget::Root)),
            6 => Some((entry.page, snapshot.parent)),
            7 => snapshot.opener,
            _ => None,
        };
        let Some((page, target)) = destination else {
            rv.set(v8::null(scope).into());
            return;
        };
        let endpoint = current_context_endpoint(scope).unwrap();
        if page==endpoint.page_id&&scope.get_current_context().get_slot::<V8BindingState>().is_some_and(|source|source.native_worker_creator().is_ok_and(|creator|matches!(creator,oryn_js::worker_wire::Creator::Window{target:current,..} if current==target))){
            let context=scope.get_current_context();let global=context.global(scope);rv.set(global.into());return
        }
        match remote_window_object(scope, page, target, false, true) {
            Ok(object) => rv.set(object.into()),
            Err(_) => oryn_js::v8_bindings::window_messages::throw_external_message_error(
                scope,
                "SecurityError",
            ),
        };
        return;
    }
    let Some(source) = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
    else {
        throw_type_error(scope, "Window caller retired");
        return;
    };
    let Some(endpoint) = context_endpoint_for_binding(scope, &source) else {
        throw_type_error(scope, "Window caller context unavailable");
        return;
    };
    let result: Result<(), String> = (|| {
        match operation {
            2 => {
                endpoint
                    .client
                    .request(ContextServiceRequest::ActivatePage {
                        source_page_id: endpoint.page_id,
                        target_page_id: entry.page,
                        creator: source.native_worker_creator().map_err(|e| e.to_string())?,
                        expected: entry.group,
                    })
                    .map_err(|e| e.to_string())?;
            }
            3 => {
                crate::native_timing::count("window.close.remote_callback", 1);
                if matches!(entry.target, oryn_js::message_wire::WindowTarget::Frame(_)) {
                    crate::native_timing::count("window.close.remote_frame_target", 1);
                    return Ok(());
                }
                flush_local_history_projections(scope).map_err(|error| error.to_string())?;
                match endpoint
                    .client
                    .request(ContextServiceRequest::ClosePage {
                        source_page_id: endpoint.page_id,
                        target_page_id: entry.page,
                        creator: source.native_worker_creator().map_err(|e| e.to_string())?,
                        expected: entry.group,
                    })
                    .map_err(|e| e.to_string())?
                {
                    ContextServiceResponse::PageClosed => {
                        crate::native_timing::count("window.close.remote_admitted", 1);
                    }
                    ContextServiceResponse::PageDenied { .. } => {
                        crate::native_timing::count("window.close.remote_denied", 1);
                    }
                    _ => return Err("Window.close admission ACK mismatch".into()),
                }
            }

            _ => return Err("unknown auxiliary Window operation".into()),
        }
        Ok(())
    })();
    if let Err(message) = result {
        throw_type_error(scope, &message)
    }
}

fn remote_window_snapshot(
    scope: &mut v8::PinScope,
    entry: &RemoteWindow,
) -> Result<crate::context_service::RelatedWindowSnapshot, ScriptError> {
    let source = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
        .ok_or_else(ScriptError::retired)?;
    let endpoint = context_endpoint_for_binding(scope, &source)
        .ok_or_else(|| remote_window_error("missing context"))?;
    let creator = source
        .native_worker_creator()
        .map_err(|error| remote_window_error(error.to_string()))?;
    match endpoint
        .client
        .request(ContextServiceRequest::ReadRelatedWindow {
            source_page_id: endpoint.page_id,
            creator,
            target_page_id: entry.page,
            target: entry.target.clone(),
            expected: entry.group,
            local_inventory: local_navigation_inventory(scope)
                .map_err(|error| remote_window_error(error.to_string()))?,
        }) {
        Ok(ContextServiceResponse::RelatedWindow { snapshot }) => Ok(snapshot),
        _ => Err(remote_window_error(
            "SecurityError: related Window authority retired",
        )),
    }
}
fn remote_location_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    functions: &mut BTreeMap<i32, v8::Global<v8::Function>>,
) -> Result<v8::Local<'s, v8::Object>, ScriptError> {
    let template = v8::ObjectTemplate::new(scope);
    remote_projection::configure(scope, template, true);
    let object = template
        .new_instance(scope)
        .ok_or_else(|| remote_window_error("Location allocation failed"))?;
    let get_data = v8::Integer::new(scope, 0);
    let set_data = v8::Integer::new(scope, 1);
    let get = v8::Function::builder(remote_location_operation)
        .data(get_data.into())
        .build(scope)
        .ok_or_else(|| remote_window_error("Location allocation failed"))?;
    let set = v8::Function::builder(remote_location_operation)
        .data(set_data.into())
        .build(scope)
        .ok_or_else(|| remote_window_error("Location allocation failed"))?;
    functions.insert(100, v8::Global::new(scope, get));
    functions.insert(101, v8::Global::new(scope, set));
    let key = v8::String::new(scope, "href").unwrap();
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(get.into(), set.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(false);
    if object.define_property(scope, key.into(), &descriptor) != Some(true) {
        return Err(remote_window_error("Location href publication failed"));
    }
    for (name, op) in [("replace", 2), ("assign", 3), ("toString", 0)] {
        let data = v8::Integer::new(scope, op);
        let function = v8::Function::builder(remote_location_operation)
            .data(data.into())
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(|| remote_window_error("Location method allocation failed"))?;
        functions.insert(100 + op, v8::Global::new(scope, function));
        let key = v8::String::new(scope, name).unwrap();
        if object.create_data_property(scope, key.into(), function.into()) != Some(true) {
            return Err(remote_window_error("Location method publication failed"));
        }
    }
    Ok(object)
}
fn remote_location_navigate(
    scope: &mut v8::PinScope,
    args: &v8::FunctionCallbackArguments,
    entry: &RemoteWindow,
    replace: bool,
) {
    let Some(source) = oryn_js::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>())
    else {
        return;
    };
    let identity = (
        source.document_root(),
        source.document_generation(),
        source.document_stream_epoch(),
        source.document_authority_epoch(),
    );
    let Some(endpoint) = context_endpoint_for_binding(scope, &source) else {
        return;
    };
    let Ok(creator) = source.native_worker_creator() else {
        return;
    };
    let Some(url) = args.get(0).to_string(scope) else {
        return;
    };
    let url = url.to_rust_string_lossy(scope);
    if source.execution_retired()
        || identity
            != (
                source.document_root(),
                source.document_generation(),
                source.document_stream_epoch(),
                source.document_authority_epoch(),
            )
    {
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope, "SecurityError");
        return;
    }
    let url = match source
        .native_document_base_url(true)
        .ok()
        .and_then(|base| url::Url::parse(&base).ok())
        .and_then(|base| base.join(&url).ok())
    {
        Some(url) => url.to_string(),
        None => {
            oryn_js::v8_bindings::window_messages::throw_external_message_error(
                scope,
                "SyntaxError",
            );
            return;
        }
    };
    // Conversion may execute script, close the source, or replace its document.
    if !remote_window_proxy_live(scope, entry) {
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope, "SecurityError");
        return;
    }
    if !matches!(
        endpoint
            .client
            .request(ContextServiceRequest::NavigateRelatedWindow {
                source_page_id: endpoint.page_id,
                creator,
                target_page_id: entry.page,
                target: entry.target.clone(),
                expected: entry.group,
                url,
                replace
            }),
        Ok(ContextServiceResponse::NavigationAuthorized)
    ) {
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope, "SecurityError");
    }
}
fn remote_location_operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let operation = args.data().int32_value(scope).unwrap_or(-1);
    let receiver = args.this();
    remote_location_operation_for(scope, args, rv, operation, receiver);
}
fn remote_location_operation_for<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
    operation: i32,
    receiver: v8::Local<'s, v8::Object>,
) {
    let entries = scope
        .get_slot::<RemoteWindows>()
        .map(|registry| registry.entries.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let Some(entry) = entries
        .into_iter()
        .find(|entry| v8::Local::new(scope, &entry.location).strict_equals(receiver.into()))
    else {
        if matches!(operation, 1 | 2) {
            oryn_js::v8_bindings::invoke_native_location_navigation(
                scope,
                args,
                rv,
                operation == 2,
            );
        } else {
            throw_type_error(scope, "native Location receiver required");
        }
        return;
    };
    if matches!(operation, 1..=3) && args.length() < 1 {
        throw_type_error(scope, "Location operation requires a URL");
        return;
    }
    if !remote_window_proxy_live(scope, &entry) {
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope, "SecurityError");
        return;
    }
    if operation == 1 || operation == 2 {
        remote_location_navigate(scope, &args, &entry, operation == 2);
        return;
    }
    let url = remote_window_snapshot(scope, &entry)
        .ok()
        .and_then(|snapshot| snapshot.url);
    let Some(url) = url else {
        oryn_js::v8_bindings::window_messages::throw_external_message_error(scope, "SecurityError");
        return;
    };
    if operation == 3 {
        remote_location_navigate(scope, &args, &entry, false);
        return;
    }
    if let Some(value) = v8::String::new(scope, &url) {
        rv.set(value.into())
    } else {
        throw_type_error(scope, "Location URL allocation failed")
    }
}

#[cfg(test)]
mod remote_window_tests {
    use super::*;
    #[tokio::test]
    async fn local_native_windows_exclude_suspended_and_retired_environments() {
        let browser = crate::Browser::new();
        let context = browser.new_context();
        let page = context.new_page();
        let mut host = host(&context, page.page_id());
        host.execute(
            "globalThis.frame=document.createElement('iframe');document.body.append(frame)",
        )
        .unwrap();
        let root = host.binding_states.get(&RealmId(0)).unwrap().clone();
        let root_context = host.contexts.get(&RealmId(0)).unwrap().clone();
        {
            v8::scope!(let handles,&mut host.isolate);
            let context = v8::Local::new(handles, &root_context);
            let scope = &mut v8::ContextScope::new(handles, context);
            let frame = scope
                .get_slot::<FrameLifetimes>()
                .unwrap()
                .live
                .values()
                .find(|frame| !frame.closed.get())
                .unwrap()
                .clone();
            let weak = scope
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .contexts
                .get(&frame.environment.get().realm)
                .unwrap()
                .clone();
            let child_context = weak.to_local(scope).unwrap();
            let child = child_context.get_slot::<V8BindingState>().unwrap();
            let root_window = context.global(scope);
            let child_window = child_context.global(scope);
            assert!(
                local_native_root_window(scope, page.page_id())
                    .unwrap()
                    .strict_equals(root_window.into())
            );
            assert!(
                local_native_frame_window(scope, page.page_id(), &frame.message_token)
                    .unwrap()
                    .strict_equals(child_window.into())
            );
            root.set_execution_suspended(true);
            child.set_execution_suspended(true);
            assert!(local_native_root_window(scope, page.page_id()).is_none());
            assert!(
                local_native_frame_window(scope, page.page_id(), &frame.message_token).is_none()
            );
            root.set_execution_suspended(false);
            child.set_execution_suspended(false);
            assert!(
                local_native_root_window(scope, page.page_id())
                    .unwrap()
                    .strict_equals(root_window.into())
            );
            assert!(
                local_native_frame_window(scope, page.page_id(), &frame.message_token)
                    .unwrap()
                    .strict_equals(child_window.into())
            );
            let child_environment = child_context.get_slot::<EnvironmentDescriptor>().unwrap();
            retire_environment(scope, child_environment.id);
            assert!(
                local_native_frame_window(scope, page.page_id(), &frame.message_token).is_none()
            );
            let root_environment = context.get_slot::<EnvironmentDescriptor>().unwrap();
            retire_environment(scope, root_environment.id);
            assert!(local_native_root_window(scope, page.page_id()).is_none());
        }
        drop(host);
        page.close().await.unwrap();
    }

    fn host(context: &crate::BrowserContext, page: PageId) -> RawV8Host {
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            None,
            SharedContextClient::direct(context.clone(), page),
            page,
            256 * 1024 * 1024,
        )
        .unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            1,
        )
        .unwrap();
        host
    }
    fn run(
        host: &mut RawV8Host,
        body: &str,
        args: Vec<ScriptValue>,
    ) -> Result<ScriptValue, ScriptError> {
        let id = host.begin_webdriver_script(ScriptRequest {
            body: body.encode_utf16().collect(),
            args,
            asynchronous: false,
            timeout_ms: Some(1000),
        })?;
        match host.poll_webdriver_script(id)? {
            ScriptPoll::Complete(value) => Ok(value),
            ScriptPoll::Pending => panic!("sync script pending"),
        }
    }
    // Finish a fixture's published response policy with a real parent-issued
    // document creator. Policy publication alone retires the old principal;
    // it does not authorize that principal to represent the new document.
    fn commit_remote_document(
        context: &crate::BrowserContext,
        page: PageId,
        url: &str,
    ) -> oryn_js::worker_wire::Creator {
        let (_, _, _, generation, _) = context.native_document_authority(page, url).unwrap();
        context
            .handle_message_wire(
                page,
                oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                    document_generation: generation,
                    stream_epoch: 0,
                },
            )
            .unwrap();
        oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: generation,
            stream_epoch: 0,
        }
    }
    #[test]
    fn remote_status_distinguishes_closed_from_transport_or_protocol_failure() {
        use oryn_js::message_wire::MessageResponse;
        assert!(
            !decode_remote_window_status(ContextServiceResponse::MessageWire {
                result: Ok(MessageResponse::WindowLive(false))
            })
            .unwrap()
        );
        assert!(
            decode_remote_window_status(ContextServiceResponse::MessageWire {
                result: Ok(MessageResponse::WindowLive(true))
            })
            .unwrap()
        );
        assert!(
            decode_remote_window_status(ContextServiceResponse::MessageWire {
                result: Err("native service unavailable".into())
            })
            .is_err()
        );
        assert!(
            decode_remote_window_status(ContextServiceResponse::NativeWindowInventoryPublished)
                .is_err()
        );
    }
    #[tokio::test]
    async fn related_window_child_snapshot_uses_native_dom_order_and_rejects_stale_local_inventory()
    {
        let context = crate::Browser::new().new_context();
        let page = context.new_page();
        context
            .install_document_security(page.page_id(), "https://a.test/".into(), &[])
            .unwrap();
        let child = context.new_popup(page.page_id(), None);
        let source = host(&context, page.page_id());
        let mut target = host(&context, child.page_id());
        target.execute("var a=document.createElement('iframe'),b=document.createElement('iframe');a.name='same';b.name='same';document.body.append(a,b)").unwrap();
        target.adopt_created_realms();
        let creator = source.binding_states[&source.active_realm()]
            .native_worker_creator()
            .unwrap();
        let inventory = |host: &mut RawV8Host| {
            let context = host.contexts[&host.active_realm()].clone();
            v8::scope!(let scope,&mut host.isolate);
            let context = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, context);
            local_navigation_inventory(scope).unwrap()
        };
        let first = inventory(&mut target);
        let initial = context
            .read_related_window_with_inventory(
                page.page_id(),
                &creator,
                child.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                None,
                &first,
            )
            .unwrap();
        assert_eq!(initial.children.len(), 2);
        assert_eq!(initial.children[0].0, "same");
        assert_ne!(initial.children[0].1, initial.children[1].1);
        let published = context
            .read_related_window_with_inventory(
                page.page_id(),
                &creator,
                child.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                None,
                &BTreeMap::new(),
            )
            .unwrap();
        assert_eq!(published.children, initial.children);
        target
            .execute("document.body.moveBefore(b,a);b.name='renamed'")
            .unwrap();
        let second = inventory(&mut target);
        let reordered = context
            .read_related_window_with_inventory(
                page.page_id(),
                &creator,
                child.page_id(),
                &oryn_js::message_wire::WindowTarget::Root,
                None,
                &second,
            )
            .unwrap();
        assert_eq!(
            reordered.children[0],
            ("renamed".into(), initial.children[1].1.clone())
        );
        assert_eq!(reordered.children[1].1, initial.children[0].1);
        target.execute("a.remove()").unwrap();
        assert!(
            context
                .read_related_window_with_inventory(
                    page.page_id(),
                    &creator,
                    child.page_id(),
                    &oryn_js::message_wire::WindowTarget::Root,
                    None,
                    &second
                )
                .is_err()
        );
        let third = inventory(&mut target);
        assert_eq!(
            context
                .read_related_window_with_inventory(
                    page.page_id(),
                    &creator,
                    child.page_id(),
                    &oryn_js::message_wire::WindowTarget::Root,
                    None,
                    &third
                )
                .unwrap()
                .children
                .len(),
            1
        );
        drop(target);
        drop(source);
        child.close().await.unwrap();
        page.close().await.unwrap();
    }

    #[tokio::test]
    async fn remote_window_cross_origin_matrix_hides_overrides_and_tracks_origin_transition() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/root".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut source = host(&context, parent.page_id());
        source.execute(&format!("globalThis.popup=__oryn_native_page_proxy({});globalThis.savedLocation=popup.location;popup.focus='override';popup.secret=17;var helper=document.createElement('iframe');document.body.append(helper);helper.contentWindow.popup=popup",child.page_id().0)).unwrap();
        context
            .install_document_security(child.page_id(), "https://b.test/foreign".into(), &[])
            .unwrap();
        commit_remote_document(&context, child.page_id(), "https://b.test/foreign");
        assert_eq!(source.evaluate_string(r#"(() => {
            const w=popup,l=w.location;
            const fail=(f,name='SecurityError')=>{try{f();return false}catch(e){return e.name===name}};
            if(w.window!==w||w.self!==w||w.frames!==w||w.closed||w.length!==0||l!==savedLocation)return false;
            if(typeof w.focus!=='function'||w.focus.name!=='focus'||w.focus.length!==0||typeof w.blur!=='function')return false;
            for(const o of [w,l]){
                if(Object.getPrototypeOf(o)!==null||!Object.isExtensible(o)||Reflect.preventExtensions(o)||!Reflect.setPrototypeOf(o,null)||Reflect.setPrototypeOf(o,{}))return false;
                for(const key of ['secret','0',Symbol.iterator])if(!fail(()=>o[key])||!fail(()=>key in o)||!fail(()=>Object.getOwnPropertyDescriptor(o,key)))return false;
                for(const key of Reflect.ownKeys(o))if(!fail(()=>Reflect.deleteProperty(o,key))||!fail(()=>Reflect.defineProperty(o,key,{value:1})))return false;
            }
            if(!fail(()=>w.origin)||!fail(()=>l.href)||!fail(()=>l.assign)||!fail(()=>Reflect.set(w,'focus',1)))return false;
            const close=Object.getOwnPropertyDescriptor(w,'close');if(close.value!==w.close||close.writable||close.enumerable||!close.configurable||w.close!==w.close)return false;
            const parent=Object.getOwnPropertyDescriptor(w,'parent');if(parent.get.name!=='get parent'||parent.get.length!==0||parent.set!==undefined||parent.enumerable||!parent.configurable)return false;
            const href=Object.getOwnPropertyDescriptor(l,'href');if(href.get!==undefined||href.set.name!=='set href'||href.set.length!==1||href.set!==Object.getOwnPropertyDescriptor(l,'href').set)return false;
            if(l.replace.name!=='replace'||l.replace.length!==1||Object.getPrototypeOf(l.replace)!==Function.prototype||!fail(()=>new l.replace(),'TypeError'))return false;
            if(!fail(()=>href.set.call({},'https://a.test/'),'TypeError')||!fail(()=>l.replace.call({},'https://a.test/'),'TypeError'))return false;
            if(!fail(()=>Reflect.set(w,'location','https://a.test/',{}),'TypeError')||!fail(()=>Reflect.set(l,'href','https://a.test/',{}),'TypeError'))return false;
            const other=helper.contentWindow.eval('popup.close');if(other===w.close||Object.getPrototypeOf(other)!==helper.contentWindow.Function.prototype)return false;
            if(!helper.contentWindow.eval(`(() => {const set=Object.getOwnPropertyDescriptor(popup.location,'href').set;try{set.call({},'/x');return false}catch(e){return e instanceof TypeError}})()`))return false;
            return Reflect.ownKeys(w).length===17&&Reflect.ownKeys(l).length===6;
        })()"#).unwrap(),"true");
        context
            .install_document_security(child.page_id(), "https://a.test/returned".into(), &[])
            .unwrap();
        commit_remote_document(&context, child.page_id(), "https://a.test/returned");
        assert_eq!(source.evaluate_string("popup.focus==='override'&&popup.secret===17&&popup.location.href==='https://a.test/returned'").unwrap(),"true");
        child.close().await.unwrap();
        assert_eq!(
            source
                .evaluate_string("popup.closed&&popup.self===popup&&popup.close()===undefined")
                .unwrap(),
            "true"
        );
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn borrowed_remote_navigation_rejects_incumbent_replacement_during_conversion() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/root".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut source = host(&context, parent.page_id());
        source.execute(&format!("globalThis.popup=__oryn_native_page_proxy({});var caller=document.createElement('iframe');document.body.append(caller);caller.contentWindow.remote=popup",child.page_id().0)).unwrap();
        assert_eq!(source.evaluate_string(r#"caller.contentWindow.eval(`(() => {
            const original=new Error('conversion');
            try{remote.location.href={toString(){throw original}};return 'lost exception'}catch(e){if(e!==original)return 'wrong exception'}
            try{remote.location.href={toString(){document.open();document.write('<p>replacement</p>');document.close();return 'https://a.test/should-not-navigate'}};return 'stale accepted'}catch(e){return e.name}
        })()` )"#).unwrap(),"SecurityError");
        assert_eq!(
            source
                .evaluate_string("caller.contentDocument.body.textContent")
                .unwrap(),
            "replacement"
        );
        assert_eq!(
            source.evaluate_string("popup.location.href").unwrap(),
            "about:blank"
        );
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn related_window_structure_and_location_keep_native_identity_and_origin_custody() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/root".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut source = host(&context, parent.page_id());
        source
            .execute(&format!(
                "globalThis.popup=__oryn_native_page_proxy({})",
                child.page_id().0
            ))
            .unwrap();
        assert_eq!(source.evaluate_string("String(popup.top===popup && popup.parent===popup && popup.opener===window && popup.length===0 && popup.location===popup.location)").unwrap(),"true");
        assert_eq!(
            source.evaluate_string("popup.location.href").unwrap(),
            "about:blank"
        );
        assert_eq!(source.evaluate_string("(()=>{let f=popup.location.replace;try{f.call({href:'https://a.test/'},'/x');return 'bad'}catch(e){return e.name}})()").unwrap(),"TypeError");
        context
            .install_document_security(child.page_id(), "https://b.test/target".into(), &[])
            .unwrap();
        commit_remote_document(&context, child.page_id(), "https://b.test/target");
        assert_eq!(
            source
                .evaluate_string(
                    "String(popup.top===popup && popup.parent===popup && popup.opener===window)"
                )
                .unwrap(),
            "true"
        );
        assert_eq!(
            source
                .evaluate_string("(()=>{try{return popup.location.href}catch(e){return e.name}})()")
                .unwrap(),
            "SecurityError"
        );
        assert_eq!(
            source
                .evaluate_string(
                    "(()=>{try{return popup.location.toString()}catch(e){return e.name}})()"
                )
                .unwrap(),
            "SecurityError"
        );
        child.close().await.unwrap();
        assert_eq!(
            source
                .evaluate_string("(()=>{try{return popup.location.href}catch(e){return e.name}})()")
                .unwrap(),
            "TypeError"
        );
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn popup_identity_is_native_and_roundtrips_without_author_properties() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let child = context.new_popup(parent.page_id(), None);
        let mut host = host(&context, parent.page_id());
        host.execute(&format!("globalThis.popup=__oryn_native_page_proxy({});Object.defineProperty(popup,'toJSON',{{get(){{throw 'poison'}}}});Object.defineProperty(popup,'__orynPageId',{{get(){{throw 'poison'}}}})",child.page_id().0)).unwrap();
        assert_eq!(
            run(&mut host, "return [popup,popup]", vec![]).unwrap(),
            ScriptValue::Array(vec![ScriptValue::WindowPage(child.page_id()); 2])
        );
        assert_eq!(
            run(
                &mut host,
                "return arguments[0]===popup",
                vec![ScriptValue::WindowPage(child.page_id())]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        let fake = run(&mut host, "return {__orynPageId:123,closed:false}", vec![]).unwrap();
        assert!(matches!(fake, ScriptValue::Object(_)));
        assert!(
            run(
                &mut host,
                "return new Proxy(popup,{get(){throw 'trap'}})",
                vec![]
            )
            .is_err()
        );
        child.close().await.unwrap();
        assert_eq!(
            run(&mut host, "return popup", vec![]).unwrap_err().code,
            "no such window"
        );
        assert_eq!(
            host.evaluate_string("String(popup.closed)").unwrap(),
            "true"
        );
        parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn coop_switch_retires_author_proxy_without_revoking_session_window_reference() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(
                parent.page_id(),
                "https://a.test/".into(),
                &[("Cross-Origin-Opener-Policy".into(), "same-origin".into())],
            )
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut root_host = host(&context, parent.page_id());
        root_host
            .execute(&format!(
                "globalThis.saved=__oryn_native_page_proxy({})",
                child.page_id().0
            ))
            .unwrap();
        assert_eq!(
            root_host.evaluate_string("String(saved.closed)").unwrap(),
            "false"
        );
        context
            .install_document_security(child.page_id(), "https://b.test/".into(), &[])
            .unwrap();
        assert_eq!(
            root_host.evaluate_string("String(saved.closed)").unwrap(),
            "true"
        );
        assert_eq!(
            run(&mut root_host, "return saved", vec![])
                .unwrap_err()
                .code,
            "no such window"
        );
        assert_eq!(
            run(
                &mut root_host,
                "return arguments[0]!==saved && !arguments[0].closed",
                vec![ScriptValue::WindowPage(child.page_id())]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        assert_eq!(
            run(
                &mut root_host,
                r#"
            globalThis.sessionWindow=arguments[0];
            const oldGet=Object.getOwnPropertyDescriptor(saved,'closed').get;
            const freshGet=Object.getOwnPropertyDescriptor(sessionWindow,'closed').get;
            return oldGet!==freshGet && oldGet()===true && freshGet()===false
                && freshGet===Object.getOwnPropertyDescriptor(sessionWindow,'closed').get;
        "#,
                vec![ScriptValue::WindowPage(child.page_id())]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        assert_eq!(
            run(
                &mut root_host,
                r#"
            let denied=0;
            try{void sessionWindow.document}catch(e){if(e.name==='SecurityError')denied++}
            try{void sessionWindow.location.href}catch(e){if(e.name==='SecurityError')denied++}
            return denied===2;
        "#,
                vec![]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        assert_eq!(
            run(&mut root_host, "return sessionWindow", vec![]).unwrap(),
            ScriptValue::WindowPage(child.page_id())
        );
        let mut child_host = host(&context, child.page_id());
        child_host
            .configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(
            child_host.evaluate_string("String(opener===null)").unwrap(),
            "true",
            "stale launcher opener cannot restore a severed relationship"
        );
        drop(child_host); // Restore the previous native isolate before using root_host.
        child.close().await.unwrap();
        assert_eq!(
            root_host
                .evaluate_string("String(saved.closed&&sessionWindow.closed)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            run(&mut root_host, "return sessionWindow", vec![])
                .unwrap_err()
                .code,
            "no such window"
        );
        parent.close().await.unwrap();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn retained_group_free_reference_reads_logical_close_after_native_history_restore() {
        use oryn_js::{
            message_wire::{MessageRequest, MessageResponse, WindowTarget},
            worker_wire::Creator,
        };
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let parent = context.new_page();
        context
            .install_document_security(
                parent.page_id(),
                "https://a.test/".into(),
                &[("Cross-Origin-Opener-Policy".into(), "same-origin".into())],
            )
            .unwrap();
        let mut root_host = host(&context, parent.page_id());
        let source = root_host.binding_states[&root_host.active_realm()]
            .native_worker_creator()
            .unwrap();
        let child = context
            .new_popup_from_creator(parent.page_id(), &source)
            .unwrap();
        let initial = context
            .read_related_window(
                parent.page_id(),
                &source,
                child.page_id(),
                &WindowTarget::Root,
                None,
            )
            .unwrap()
            .creator;
        let park = |creator: &Creator| {
            let Creator::Window {
                target: WindowTarget::Root,
                document_generation,
                stream_epoch,
            } = creator
            else {
                panic!("native root creator")
            };
            let MessageResponse::DocumentParked(token) = context
                .handle_message_wire(
                    child.page_id(),
                    MessageRequest::ParkDocument {
                        document_generation: *document_generation,
                        stream_epoch: *stream_epoch,
                    },
                )
                .unwrap()
            else {
                panic!("native parked document")
            };
            token
        };
        let original = park(&initial);
        context
            .install_document_security(child.page_id(), "https://b.test/".into(), &[])
            .unwrap();
        let foreign = commit_remote_document(&context, child.page_id(), "https://b.test/");
        assert!(
            !context
                .native_window_group(parent.page_id(), child.page_id(), None)
                .unwrap()
                .2
        );
        assert_eq!(
            run(
                &mut root_host,
                "globalThis.sessionWindow=arguments[0];return !sessionWindow.closed",
                vec![ScriptValue::WindowPage(child.page_id())]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        assert!(
            root_host
                .isolate
                .get_slot::<RemoteWindows>()
                .unwrap()
                .entries
                .values()
                .any(|entry| entry.page == child.page_id() && entry.group.is_none())
        );
        let foreign = park(&foreign);
        let restored = context
            .handle_message_wire(
                child.page_id(),
                MessageRequest::RestoreDocument { token: original },
            )
            .unwrap();
        let Creator::Window {
            document_generation,
            stream_epoch,
            ..
        } = &initial
        else {
            panic!("native initial creator")
        };
        assert!(
            matches!(restored,MessageResponse::DocumentRestored{document_generation:g,stream_epoch:e}if(g,e)==(*document_generation,*stream_epoch))
        );
        assert!(
            context
                .native_window_group(parent.page_id(), child.page_id(), None)
                .unwrap()
                .2
        );
        assert_eq!(
            root_host
                .evaluate_string("String(sessionWindow.closed)")
                .unwrap(),
            "false"
        );
        // Hold the exact native departure reply. The independent close runner
        // cannot physically retire the page while the assertions inspect it.
        let mut departure = context.hold_script_close_departure_for_test(child.page_id());
        assert!(
            context
                .request_script_close(parent.page_id(), &source, child.page_id(), None)
                .unwrap()
        );
        let (token, expected) = departure.receive().await;
        assert_ne!(token, 0);
        assert_eq!(expected, initial);
        assert!(
            context
                .script_close_state(parent.page_id(), &source, child.page_id(), None)
                .unwrap()
        );
        assert!(
            context
                .native_window_group(parent.page_id(), child.page_id(), None)
                .unwrap()
                .2,
            "logical close must be observed before native endpoint retirement"
        );
        assert_eq!(
            root_host
                .evaluate_string("String(sessionWindow.closed)")
                .unwrap(),
            "true"
        );
        assert_eq!(
            root_host
                .evaluate_string("String(sessionWindow.closed)")
                .unwrap(),
            "true"
        );
        departure.cancel().await;
        drop(departure);
        context
            .handle_message_wire(
                child.page_id(),
                MessageRequest::DiscardDocument { token: foreign },
            )
            .unwrap();
        drop(root_host);
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn origin_property_is_native_same_origin_guarded_without_severing_cross_origin_proxy() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut root_host = host(&context, parent.page_id());
        root_host
            .execute(&format!(
                "globalThis.popup=__oryn_native_page_proxy({})",
                child.page_id().0
            ))
            .unwrap();
        assert_eq!(
            root_host.evaluate_string("popup.origin").unwrap(),
            "https://a.test"
        );
        context
            .install_document_security(child.page_id(), "https://b.test/".into(), &[])
            .unwrap();
        assert_eq!(
            root_host.evaluate_string("String(popup.closed)").unwrap(),
            "false"
        );
        assert_eq!(
            root_host
                .evaluate_string("(()=>{try{return popup.origin}catch(e){return e.name}})()")
                .unwrap(),
            "SecurityError"
        );
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn null_opener_assignment_survives_native_reinitialization_and_nonnull_does_not_forge_parent()
     {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let child = context.new_popup(parent.page_id(), None);
        let mut child_host = host(&context, child.page_id());
        child_host
            .configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(child_host.evaluate_string(r#"(() => {
            const d = Object.getOwnPropertyDescriptor(window, 'opener');
            if (!d.enumerable || !d.configurable || d.get.name !== 'get opener' ||
                d.get.length !== 0 || d.set.name !== 'set opener' || d.set.length !== 1) return false;
            for (const f of [d.get, d.set]) {
                try { Reflect.construct(f, []); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
                try { f.call({}, null); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
            }
            return true;
        })()"#).unwrap(), "true");
        child_host.execute("opener={authorMarker:1}").unwrap();
        assert_eq!(
            child_host
                .evaluate_string("String(opener.authorMarker)")
                .unwrap(),
            "1"
        );
        child_host
            .configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(
            run(&mut child_host, "return opener", vec![]).unwrap(),
            ScriptValue::WindowPage(parent.page_id())
        );
        child_host.execute("opener=null").unwrap();
        assert!(
            context
                .native_window_group(child.page_id(), child.page_id(), None)
                .unwrap()
                .1
                .is_none()
        );
        child_host
            .configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(
            child_host.evaluate_string("String(opener===null)").unwrap(),
            "true"
        );
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }
    #[tokio::test]
    async fn opener_comes_from_parent_descriptor_and_retains_closed_identity() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let child = context.new_popup(parent.page_id(), None);
        let mut host = host(&context, child.page_id());
        host.configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(
            run(&mut host, "return opener", vec![]).unwrap(),
            ScriptValue::WindowPage(parent.page_id())
        );
        assert_eq!(
            run(
                &mut host,
                "return arguments[0]===opener",
                vec![ScriptValue::WindowPage(parent.page_id())]
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        parent.close().await.unwrap();
        host.configure_native_page_opener(Some(parent.page_id()))
            .unwrap();
        assert_eq!(
            host.evaluate_string("String(opener.closed)").unwrap(),
            "true"
        );
        assert_eq!(
            run(&mut host, "return opener", vec![]).unwrap_err().code,
            "no such window"
        );
        child.close().await.unwrap();
    }
    #[tokio::test]
    async fn only_driver_arguments_can_mint_unrelated_same_context_proxies() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let unrelated = context.new_page();
        let foreign_context = crate::Browser::new().new_context();
        let foreign = foreign_context.new_page();
        let mut host = host(&context, parent.page_id());
        assert!(
            host.execute(&format!(
                "__oryn_native_page_proxy({})",
                unrelated.page_id().0
            ))
            .is_err()
        );
        assert_eq!(
            run(
                &mut host,
                "return arguments[0]",
                vec![ScriptValue::WindowPage(unrelated.page_id())]
            )
            .unwrap(),
            ScriptValue::WindowPage(unrelated.page_id())
        );
        assert_eq!(
            run(
                &mut host,
                "return arguments[0]",
                vec![ScriptValue::WindowPage(foreign.page_id())]
            )
            .unwrap_err()
            .code,
            "no such window"
        );
        let forged = format!(
            "__oryn_context_call(JSON.stringify({{operation:'automation_window_status',source_page_id:{},target_page_id:{}}}))",
            parent.page_id().0,
            unrelated.page_id().0
        );
        assert!(host.execute(&forged).is_err());
        let client = SharedContextClient::direct(context.clone(), parent.page_id());
        assert!(
            client
                .request(ContextServiceRequest::AutomationWindowStatus {
                    source_page_id: unrelated.page_id(),
                    target_page_id: parent.page_id()
                })
                .is_err()
        );
        assert!(matches!(
            client
                .request(ContextServiceRequest::AutomationWindowStatus {
                    source_page_id: parent.page_id(),
                    target_page_id: foreign.page_id()
                })
                .unwrap(),
            ContextServiceResponse::AutomationWindowStatus { live: false }
        ));
        unrelated.close().await.unwrap();
        parent.close().await.unwrap();
        foreign.close().await.unwrap();
    }
    #[tokio::test]
    async fn saved_proxy_cannot_outlive_its_creating_document() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut host = host(&context, parent.page_id());
        host.execute(&format!(
            "globalThis.oldPopup=__oryn_native_page_proxy({})",
            child.page_id().0
        ))
        .unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>replacement</body>")
                    .into_runtime()
                    .unwrap(),
            )),
            2,
        )
        .unwrap();
        assert_eq!(
            run(&mut host, "return oldPopup", vec![]).unwrap_err().code,
            "no such window"
        );
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn closed_and_severed_window_then_fallback_preserves_native_clone_errors() {
        for sever_group in [false, true] {
            let context = crate::Browser::new().new_context();
            let parent = context.new_page();
            context
                .install_document_security(
                    parent.page_id(),
                    "https://a.test/".into(),
                    &[("Cross-Origin-Opener-Policy".into(), "same-origin".into())],
                )
                .unwrap();
            let child = context.new_popup(parent.page_id(), None);
            let mut source = host(&context, parent.page_id());
            source
                .execute(&format!(
                    "globalThis.saved=__oryn_native_page_proxy({})",
                    child.page_id().0
                ))
                .unwrap();
            if sever_group {
                context
                    .install_document_security(child.page_id(), "https://b.test/".into(), &[])
                    .unwrap();
            } else {
                child.close().await.unwrap();
            }
            assert_eq!(source.evaluate_string(r#"(() => {
                const denied = f => {try {f(); return false} catch(e) {return e.name === 'SecurityError'}};
                for (const key of ['then', Symbol.toStringTag, Symbol.hasInstance, Symbol.isConcatSpreadable]) {
                    const descriptor = Object.getOwnPropertyDescriptor(saved, key);
                    if (saved[key] !== undefined || !(key in saved) || descriptor.value !== undefined ||
                        descriptor.writable || descriptor.enumerable || !descriptor.configurable) return false;
                }
                return saved.closed && denied(() => saved.privateData) && denied(() => saved[0]);
            })()"#).unwrap(), "true");
            assert_eq!(
                run(&mut source, "return saved", vec![]).unwrap_err().code,
                "no such window"
            );
            let id = source
                .begin_webdriver_script(ScriptRequest {
                    body: "arguments[0](saved)".encode_utf16().collect(),
                    args: vec![],
                    asynchronous: true,
                    timeout_ms: Some(1000),
                })
                .unwrap();
            assert_eq!(
                source.poll_webdriver_script(id).unwrap_err().code,
                "no such window"
            );
            if sever_group {
                child.close().await.unwrap();
            }
            parent.close().await.unwrap();
        }
    }

    #[tokio::test]
    async fn automation_only_window_then_fallback_does_not_grant_child_authority() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        let unrelated = context.new_page();
        let mut source = host(&context, parent.page_id());
        assert_eq!(run(&mut source, r#"
            globalThis.saved = arguments[0];
            const descriptor = Object.getOwnPropertyDescriptor(saved, 'then');
            if (saved.then !== undefined || descriptor.value !== undefined || descriptor.writable ||
                descriptor.enumerable || !descriptor.configurable) throw new Error('invalid then fallback');
            for (const key of ['privateData', '0']) {
                let denied = false;
                try {saved[key]} catch(e) {denied = e.name === 'SecurityError'}
                if (!denied) throw new Error('unrelated child authority granted');
            }
            return saved;
        "#, vec![ScriptValue::WindowPage(unrelated.page_id())]).unwrap(), ScriptValue::WindowPage(unrelated.page_id()));
        let id = source
            .begin_webdriver_script(ScriptRequest {
                body: "arguments[1](arguments[0])".encode_utf16().collect(),
                args: vec![ScriptValue::WindowPage(unrelated.page_id())],
                asynchronous: true,
                timeout_ms: Some(1000),
            })
            .unwrap();
        assert_eq!(
            source.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(ScriptValue::WindowPage(unrelated.page_id()))
        );
        assert!(
            source
                .execute(&format!(
                    "__oryn_native_page_proxy({})",
                    unrelated.page_id().0
                ))
                .is_err()
        );
        unrelated.close().await.unwrap();
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn live_cross_origin_named_then_child_precedes_fallback() {
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        context
            .install_document_security(child.page_id(), "https://b.test/".into(), &[])
            .unwrap();
        // Construct and populate the target first. The source isolate remains
        // current during all reads; reverse drop order restores the target.
        let mut target = host(&context, child.page_id());
        target.execute("var frame=document.createElement('iframe');frame.name='then';document.body.append(frame)").unwrap();
        target.adopt_created_realms();
        let mut source = host(&context, parent.page_id());
        source
            .execute(&format!(
                "globalThis.popup=__oryn_native_page_proxy({})",
                child.page_id().0
            ))
            .unwrap();
        assert_eq!(
            source
                .evaluate_string(
                    r#"(() => {
            const descriptor = Object.getOwnPropertyDescriptor(popup, 'then');
            return popup.length === 1 && popup.then === popup[0] && descriptor.value === popup[0] &&
                !descriptor.writable && !descriptor.enumerable && descriptor.configurable;
        })()"#
                )
                .unwrap(),
            "true"
        );
        assert_eq!(
            run(&mut source, "return popup", vec![]).unwrap(),
            ScriptValue::WindowPage(child.page_id())
        );
        drop(source);
        drop(target);
        child.close().await.unwrap();
        parent.close().await.unwrap();
    }

    #[tokio::test]
    async fn remote_document_commit_rejects_stale_authority_and_preserves_proxy_identity() {
        use oryn_js::{message_wire::WindowTarget, worker_wire::Creator};
        let context = crate::Browser::new().new_context();
        let parent = context.new_page();
        context
            .install_document_security(parent.page_id(), "https://a.test/root".into(), &[])
            .unwrap();
        let child = context.new_popup(parent.page_id(), None);
        let mut source = host(&context, parent.page_id());
        source
            .execute(&format!(
                "globalThis.popup=__oryn_native_page_proxy({});globalThis.saved=popup",
                child.page_id().0
            ))
            .unwrap();
        let caller = source.binding_states[&source.active_realm()]
            .native_worker_creator()
            .unwrap();
        let before = context
            .read_related_window(
                parent.page_id(),
                &caller,
                child.page_id(),
                &WindowTarget::Root,
                None,
            )
            .unwrap();
        assert!(before.same_origin_domain);
        assert_eq!(before.url.as_deref(), Some("about:blank"));
        let Creator::Window {
            document_generation: old_generation,
            stream_epoch: old_epoch,
            ..
        } = before.creator.clone()
        else {
            panic!("native Window creator")
        };
        context
            .install_document_security(child.page_id(), "https://b.test/target".into(), &[])
            .unwrap();
        assert!(
            context
                .window_worker_creator(
                    child.page_id(),
                    &WindowTarget::Root,
                    old_generation,
                    old_epoch
                )
                .is_err()
        );
        assert!(
            context
                .read_related_window(
                    parent.page_id(),
                    &caller,
                    child.page_id(),
                    &WindowTarget::Root,
                    None
                )
                .is_err()
        );
        let replacement =
            commit_remote_document(&context, child.page_id(), "https://b.test/target");
        assert_ne!(replacement, before.creator);
        let cross = context
            .read_related_window(
                parent.page_id(),
                &caller,
                child.page_id(),
                &WindowTarget::Root,
                None,
            )
            .unwrap();
        assert_eq!(cross.creator, replacement);
        assert!(!cross.same_origin_domain);
        assert!(cross.url.is_none());
        assert_eq!(cross.opener, Some((parent.page_id(), WindowTarget::Root)));
        assert!(
            context
                .window_worker_creator(
                    child.page_id(),
                    &WindowTarget::Root,
                    old_generation,
                    old_epoch
                )
                .is_err()
        );
        assert_eq!(source.evaluate_string("popup===saved&&popup.top===saved&&popup.parent===saved&&popup.opener===window&&!popup.closed").unwrap(),"true");
        context
            .install_document_security(child.page_id(), "https://a.test/returned".into(), &[])
            .unwrap();
        let returned = commit_remote_document(&context, child.page_id(), "https://a.test/returned");
        assert_ne!(returned, replacement);
        assert_eq!(
            source
                .evaluate_string("popup===saved&&popup.location.href==='https://a.test/returned'")
                .unwrap(),
            "true"
        );
        child.close().await.unwrap();
        assert_eq!(
            source.evaluate_string("String(popup.closed)").unwrap(),
            "true"
        );
        assert_eq!(
            source
                .evaluate_string("(()=>{try{return popup.location.href}catch(e){return e.name}})()")
                .unwrap(),
            "TypeError"
        );
        parent.close().await.unwrap();
    }
}

impl RawV8Host {
    /// Called only with the opener from the parent-owned page descriptor, before
    /// document scripts. A closed opener retains identity and reports closed.
    pub(crate) fn configure_native_page_opener(
        &mut self,
        opener: Option<PageId>,
    ) -> Result<(), HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("missing opener realm".into()))?;
        v8::scope!(let handle_scope,&mut self.isolate);
        let context = v8::Local::new(handle_scope, &context);
        let scope = &mut v8::ContextScope::new(handle_scope, context);
        let prerender = context
            .get_slot::<V8BindingState>()
            .is_some_and(|state| state.document_prerendering());
        let (opener, target) = if prerender {
            (None, oryn_js::message_wire::WindowTarget::Root)
        } else if let Some(endpoint) = current_context_endpoint(scope) {
            let group = remote_window_group(scope, endpoint.page_id, None)
                .map_err(|error| HostError::Execution(error.message))?;
            (group.1, group.3)
        } else {
            (opener, oryn_js::message_wire::WindowTarget::Root)
        };
        let opener = opener
            .map(|page| remote_window_object(scope, page, target, true, true))
            .transpose()
            .map_err(|error| HostError::Execution(error.message))?;
        let window = context.global(scope);
        oryn_js::v8_bindings::set_native_window_opener(scope, window, opener)
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let key = v8::String::new(scope, "opener").unwrap();
        let getter = v8::Function::builder(native_window_opener_callback)
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(|| HostError::Execution("Window.opener getter allocation".into()))?;
        let setter = v8::Function::builder(native_window_set_opener_callback)
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(|| HostError::Execution("Window.opener setter allocation".into()))?;
        getter.set_name(v8::String::new(scope, "get opener")
            .ok_or_else(|| HostError::Execution("Window.opener getter name allocation".into()))?);
        setter.set_name(v8::String::new(scope, "set opener")
            .ok_or_else(|| HostError::Execution("Window.opener setter name allocation".into()))?);
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if window.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(HostError::Execution(
                "cannot initialize Window.opener".into(),
            ));
        }
        Ok(())
    }
}

fn publish_local_window_inventory(
    scope: &mut v8::PinScope,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    use oryn_js::v8_bindings::V8BindingError;
    let Some(state) = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .filter(|state| {
            state.binding_global() == BindingGlobal::Window && !state.execution_retired()
        })
    else {
        return Ok(());
    };
    // Private candidate trees have their own authenticated registry. Publishing
    // them as the active browsing-context tree would replace the incumbent.
    // Parked documents likewise retain cleanup authority, not public authority.
    if state.document_prerendering() || state.execution_suspended() {
        return Ok(());
    }
    let Some(endpoint) = context_endpoint_for_binding(scope, &state) else {
        return Ok(());
    };
    let creator = state.native_worker_creator()?;
    let inventory = local_navigation_inventory(scope)?;
    match endpoint
        .client
        .request(ContextServiceRequest::PublishNativeWindowInventory {
            page_id: endpoint.page_id,
            creator,
            inventory,
        }) {
        Ok(ContextServiceResponse::NativeWindowInventoryPublished) => Ok(()),
        Ok(_) => Err(V8BindingError::Allocation(
            "native Window tree publication acknowledgement",
        )),
        Err(error) => Err(V8BindingError::Message(error.to_string())),
    }
}
impl RawV8Host {
    fn publish_native_window_inventory(&mut self) -> Result<(), HostError> {
        let Some(context) = self.contexts.get(&self.active_realm).cloned() else {
            return Ok(());
        };
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        publish_local_window_inventory(scope)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
}

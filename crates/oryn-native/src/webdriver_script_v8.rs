// Included in host::raw_v8: V8 handles never leave the owning isolate/thread.
use super::webdriver_script::{
    ELEMENT_KEY, ElementCommand, FRAME_KEY, MAX_SCRIPT_BYTES, MAX_VALUE_BYTES, MAX_VALUE_NODES,
    SHADOW_KEY, ScriptError, ScriptId, ScriptPoll, ScriptRequest, ScriptValue, ValueBudget,
    WINDOW_KEY, WindowHandleBinding,
};

#[derive(Default)]
struct WebDriverScripts {
    next: u64,
    windows: BTreeMap<RealmId, WebDriverWindowReference>,
    pending: BTreeMap<ScriptId, WebDriverScript>,
    references: BTreeMap<String, WebDriverNodeReference>,
}
#[derive(Clone)]
struct WebDriverWindowReference {
    binding: WindowHandleBinding,
    authority_epoch: u64,
    owner: Rc<V8BindingState>,
    object: v8::Global<v8::Object>,
}
fn webdriver_window_live(reference: &WebDriverWindowReference) -> bool {
    !reference.owner.execution_retired()
        && reference.owner.document_generation() == reference.binding.document_generation
        && reference.owner.document_authority_epoch() == reference.authority_epoch
}
struct WebDriverScript {
    environment: EnvironmentDescriptor,
    authority_epoch: Option<u64>,
    deadline: Option<Instant>,
    initial_paused: Duration,
    resolver: v8::Global<v8::PromiseResolver>,
    promise: v8::Global<v8::Promise>,
    settled: bool,
}
#[derive(Clone)]
struct WebDriverNodeReference {
    owner: Rc<V8BindingState>,
    node: NodeId,
    generation: u64,
    authority_epoch: u64,
    shadow: bool,
    object: v8::Global<v8::Object>,
}

fn webdriver_live(
    scope: &mut v8::PinScope,
    environment: &EnvironmentDescriptor,
    epoch: Option<u64>,
) -> bool {
    scope
        .get_slot::<AgentEnvironments>()
        .is_some_and(|registry| registry.is_live(environment))
        && scope
            .get_current_context()
            .get_slot::<EnvironmentDescriptor>()
            .is_some_and(|current| *current == *environment)
        && scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .is_none_or(|state| {
                !state.execution_retired() && Some(state.document_authority_epoch()) == epoch
            })
}

fn webdriver_node_live(reference: &WebDriverNodeReference) -> bool {
    if reference.owner.execution_retired()
        || reference.owner.document_generation() != reference.generation
        || reference.owner.document_authority_epoch() != reference.authority_epoch
    {
        return false;
    }
    let dom = reference.owner.dom();
    let dom = dom.borrow();
    dom.is_connected(reference.node).unwrap_or(false)
        && dom.node_document(reference.node).ok().flatten() == Some(reference.owner.document_root())
}

fn webdriver_node_authority(
    scope: &mut v8::PinScope,
    owner: &V8BindingState,
) -> Result<(), ScriptError> {
    let environment = current_environment(scope).map_err(|_| ScriptError::retired())?;
    if owner.execution_retired()
        || !scope
            .get_slot::<NativeRealmIdentities>()
            .is_some_and(|registry| {
                registry
                    .realms
                    .same_origin(environment.realm, owner.realm())
            })
    {
        return Err(ScriptError::javascript(
            "node reference is outside the command's origin authority",
        ));
    }
    Ok(())
}

fn webdriver_node_json(
    scope: &mut v8::PinScope,
    owner: Rc<V8BindingState>,
    node: NodeId,
    object: v8::Local<v8::Object>,
) -> Result<ScriptValue, ScriptError> {
    webdriver_node_authority(scope, &owner)?;
    let shadow = {
        let dom = owner.dom();
        let dom = dom.borrow();
        match &dom
            .arena()
            .get(node)
            .map_err(|_| ScriptError::stale(false))?
            .kind
        {
            oryn_dom::NodeKind::Element { .. } => false,
            oryn_dom::NodeKind::ShadowRoot { .. } => true,
            _ => {
                return Err(ScriptError::javascript(
                    "only elements and shadow roots have WebDriver node references",
                ));
            }
        }
    };
    let reference = WebDriverNodeReference {
        generation: owner.document_generation(),
        authority_epoch: owner.document_authority_epoch(),
        owner,
        node,
        shadow,
        object: v8::Global::new(scope, object),
    };
    if !webdriver_node_live(&reference) {
        return Err(ScriptError::stale(shadow));
    }
    let existing = scope.get_slot::<WebDriverScripts>().and_then(|registry| {
        registry
            .references
            .iter()
            .find(|(_, item)| {
                item.node == node
                    && Rc::ptr_eq(&item.owner, &reference.owner)
                    && item.generation == reference.generation
                    && item.authority_epoch == reference.authority_epoch
            })
            .map(|(id, _)| id.clone())
    });
    let id = if let Some(id) = existing {
        id
    } else {
        let registry = scope
            .get_slot_mut::<WebDriverScripts>()
            .ok_or_else(|| ScriptError::javascript("script reference registry unavailable"))?;
        registry
            .references
            .retain(|_, item| webdriver_node_live(item));
        if registry.references.len() >= 4096 {
            return Err(ScriptError::javascript("script node reference limit"));
        }
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| ScriptError::javascript("cannot allocate node reference identity"))?;
        let id = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if registry.references.contains_key(&id) {
            return Err(ScriptError::javascript("node reference identity collision"));
        }
        registry.references.insert(id.clone(), reference);
        id
    };
    let result = vec![(
        if shadow { SHADOW_KEY } else { ELEMENT_KEY }
            .encode_utf16()
            .collect(),
        ScriptValue::String(id.encode_utf16().collect()),
    )];
    Ok(ScriptValue::Object(result))
}

fn webdriver_from_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &ScriptValue,
    budget: &mut ValueBudget,
    depth: usize,
) -> Result<v8::Local<'s, v8::Value>, ScriptError> {
    budget.consume(depth, 8)?;
    Ok(match value {
        ScriptValue::WindowRealm { page, frame } => {
            #[cfg(unix)]
            {
                native_window_target_object(
                    scope,
                    *page,
                    oryn_js::message_wire::WindowTarget::Frame(frame.clone()),
                    false,
                )?
                .into()
            }
            #[cfg(not(unix))]
            {
                return Err(ScriptError::invalid("remote frame transport unavailable"));
            }
        }
        ScriptValue::WindowPage(page) => {
            #[cfg(unix)]
            {
                let current = current_context_endpoint(scope).map(|endpoint| endpoint.page_id);
                if current == Some(*page) {
                    let root = scope
                        .get_slot::<WebDriverScripts>()
                        .and_then(|registry| {
                            registry
                                .windows
                                .values()
                                .find(|entry| entry.binding.top_level)
                        })
                        .cloned()
                        .ok_or_else(|| remote_window_error("top-level Window is unavailable"))?;
                    if !webdriver_window_live(&root) {
                        return Err(ScriptError::retired());
                    }
                    v8::Local::new(scope, &root.object).into()
                } else {
                    remote_window_object(
                        scope,
                        *page,
                        oryn_js::message_wire::WindowTarget::Root,
                        true,
                        false,
                    )?
                    .into()
                }
            }
            #[cfg(not(unix))]
            {
                return Err(ScriptError::invalid(
                    "auxiliary Window transport unavailable",
                ));
            }
        }
        ScriptValue::Null => v8::null(scope).into(),
        ScriptValue::Bool(value) => v8::Boolean::new(scope, *value).into(),
        ScriptValue::Number(value) => v8::Number::new(
            scope,
            value
                .as_f64()
                .ok_or_else(|| ScriptError::invalid("script numeric argument is not finite"))?,
        )
        .into(),
        ScriptValue::String(value) => {
            budget.consume(depth, value.len().saturating_mul(2))?;
            v8::String::new_from_two_byte(scope, value, v8::NewStringType::Normal)
                .ok_or_else(|| ScriptError::invalid("script string allocation failed"))?
                .into()
        }
        ScriptValue::Array(values) => {
            if values.len() > MAX_VALUE_NODES {
                return Err(ScriptError::invalid("script argument array exceeds limit"));
            }
            let array = v8::Array::new(scope, values.len() as i32);
            for (index, item) in values.iter().enumerate() {
                let item = webdriver_from_json(scope, item, budget, depth + 1)?;
                let key = v8::String::new(scope, &index.to_string())
                    .ok_or_else(|| ScriptError::invalid("script array index allocation failed"))?;
                if array.create_data_property(scope, key.into(), item) != Some(true) {
                    return Err(ScriptError::invalid(
                        "script argument array creation failed",
                    ));
                }
            }
            array.into()
        }
        ScriptValue::Object(values) => {
            if let Some((top_level, id)) = value
                .get(WINDOW_KEY)
                .map(|id| (true, id))
                .or_else(|| value.get(FRAME_KEY).map(|id| (false, id)))
            {
                let handle = id.as_string()?;
                let reference = scope
                    .get_slot::<WebDriverScripts>()
                    .and_then(|registry| {
                        registry.windows.values().find(|entry| {
                            entry.binding.handle == handle && entry.binding.top_level == top_level
                        })
                    })
                    .cloned()
                    .ok_or_else(|| ScriptError {
                        code: "no such window".into(),
                        message: "unknown browsing context handle".into(),
                    })?;
                if !webdriver_window_live(&reference) {
                    return Err(ScriptError {
                        code: "no such window".into(),
                        message: "browsing context was retired".into(),
                    });
                }
                v8::Local::new(scope, &reference.object).into()
            } else if let Some((shadow, id)) = value
                .get(ELEMENT_KEY)
                .map(|value| (false, value))
                .or_else(|| value.get(SHADOW_KEY).map(|value| (true, value)))
            {
                let id = id.as_string()?;
                let reference = scope
                    .get_slot::<WebDriverScripts>()
                    .and_then(|registry| registry.references.get(&id))
                    .cloned()
                    .ok_or_else(|| ScriptError {
                        code: if shadow {
                            "no such shadow root"
                        } else {
                            "no such element"
                        }
                        .into(),
                        message: "unknown script node reference".into(),
                    })?;
                if reference.shadow != shadow || !webdriver_node_live(&reference) {
                    return Err(ScriptError::stale(shadow));
                }
                webdriver_node_authority(scope, &reference.owner)?;
                v8::Local::new(scope, &reference.object).into()
            } else {
                let object = v8::Object::new(scope);
                for (key, item) in values {
                    budget.consume(depth, key.len().saturating_mul(2))?;
                    let key = v8::String::new_from_two_byte(scope, key, v8::NewStringType::Normal)
                        .ok_or_else(|| ScriptError::invalid("script key allocation failed"))?;
                    let item = webdriver_from_json(scope, item, budget, depth + 1)?;
                    // DefineOwnProperty semantics bypass author setters, including __proto__.
                    if object.create_data_property(scope, key.into(), item) != Some(true) {
                        return Err(ScriptError::invalid(
                            "script argument object creation failed",
                        ));
                    }
                }
                object.into()
            }
        }
    })
}

fn webdriver_to_json(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    budget: &mut ValueBudget,
    depth: usize,
    seen: &mut Vec<v8::Global<v8::Object>>,
) -> Result<ScriptValue, ScriptError> {
    let value = v8::Local::new(scope, value);
    budget.consume(depth, 8)?;
    if value.is_null_or_undefined() {
        return Ok(ScriptValue::Null);
    }
    if value.is_boolean() {
        return Ok(ScriptValue::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        let number = value
            .number_value(scope)
            .ok_or_else(|| ScriptError::javascript("numeric result conversion failed"))?;
        return Ok(serde_json::Number::from_f64(number)
            .map(ScriptValue::Number)
            .unwrap_or(ScriptValue::Null));
    }
    if value.is_string() {
        let value = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| ScriptError::javascript("invalid string result"))?;
        if value.length() > MAX_VALUE_BYTES / 3 {
            return Err(ScriptError::javascript(
                "script string exceeds result limit",
            ));
        }
        let mut units = vec![0; value.length()];
        value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
        budget.consume(depth, units.len().saturating_mul(2))?;
        let value = units;
        return Ok(ScriptValue::String(value));
    }
    if value.is_big_int() || value.is_symbol() {
        return Err(ScriptError::javascript(
            "script value is not JSON cloneable",
        ));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| ScriptError::javascript("script value is not an object"))?;
    #[cfg(unix)]
    if let Some((page, target)) = remote_window_identity(scope, value)? {
        return Ok(match target {
            oryn_js::message_wire::WindowTarget::Root => ScriptValue::WindowPage(page),
            oryn_js::message_wire::WindowTarget::Frame(frame) => {
                ScriptValue::WindowRealm { page, frame }
            }
        });
    }
    if let Some(realm) =
        oryn_js::v8_bindings::native_window_identity(scope, value).map_err(|error| ScriptError {
            code: "no such window".into(),
            message: error.to_string(),
        })?
    {
        #[cfg(unix)]
        if let Some(page) = current_context_endpoint(scope).map(|context| context.page_id) {
            let context = object
                .get_creation_context(scope)
                .ok_or_else(ScriptError::retired)?;
            if let Some(lifetime) = context.get_slot::<FrameLifetime>() {
                if lifetime.closed.get() {
                    return Err(ScriptError::retired());
                }
                return Ok(ScriptValue::WindowRealm {
                    page,
                    frame: lifetime.message_token.clone(),
                });
            }
            if context
                .get_slot::<RealmDescriptor>()
                .is_some_and(|descriptor| descriptor.frame_id.is_none())
            {
                return Ok(ScriptValue::WindowPage(page));
            }
        }
        let reference = scope
            .get_slot::<WebDriverScripts>()
            .and_then(|registry| registry.windows.get(&realm))
            .cloned()
            .ok_or_else(|| ScriptError {
                code: "no such window".into(),
                message: "session has no handle for this native browsing context".into(),
            })?;
        if !webdriver_window_live(&reference) {
            return Err(ScriptError::retired());
        }
        let key = if reference.binding.top_level {
            WINDOW_KEY
        } else {
            FRAME_KEY
        };
        return Ok(ScriptValue::Object(vec![(
            key.encode_utf16().collect(),
            ScriptValue::String(reference.binding.handle.encode_utf16().collect()),
        )]));
    }
    if let Some((owner, node)) = oryn_js::v8_bindings::native_node_identity(scope, value)
        .map_err(|error| ScriptError::javascript(error.to_string()))?
    {
        return webdriver_node_json(scope, owner, node, object);
    }
    if seen
        .iter()
        .any(|ancestor| object.strict_equals(v8::Local::new(scope, ancestor).into()))
    {
        return Err(ScriptError::javascript("cyclic script result"));
    }
    seen.push(v8::Global::new(scope, object));
    let result = (|| {
        if let Some(nodes) = oryn_js::v8_bindings::native_node_collection(scope, value)
            .map_err(|error| ScriptError::javascript(error.to_string()))?
        {
            if nodes.len() > MAX_VALUE_NODES {
                return Err(ScriptError::javascript(
                    "native collection exceeds result limit",
                ));
            }
            let mut result = Vec::with_capacity(nodes.len());
            for (owner, node) in nodes {
                webdriver_node_authority(scope, &owner)?;
                budget.consume(depth + 1, 8)?;
                let object = wrap_node(scope, &owner, node)
                    .map_err(|error| ScriptError::javascript(error.to_string()))?;
                result.push(webdriver_node_json(scope, owner.clone(), node, object)?);
            }
            return Ok(ScriptValue::Array(result));
        }
        if let Some((owner, values)) = oryn_js::v8_bindings::native_collection_values(scope, value)
            .map_err(|error| ScriptError::javascript(error.to_string()))?
        {
            webdriver_node_authority(scope, &owner)?;
            if values.len() > MAX_VALUE_NODES {
                return Err(ScriptError::javascript(
                    "native collection exceeds result limit",
                ));
            }
            let mut result = Vec::with_capacity(values.len());
            for item in values {
                result.push(webdriver_to_json(scope, item, budget, depth + 1, seen)?);
            }
            return Ok(ScriptValue::Array(result));
        }
        let to_json_name = v8::String::new(scope, "toJSON").unwrap();
        if object.has_own_property(scope, to_json_name.into()) == Some(true) {
            let to_json = object
                .get(scope, to_json_name.into())
                .ok_or_else(|| ScriptError::javascript("toJSON getter threw"))?;
            if let Ok(function) = v8::Local::<v8::Function>::try_from(to_json) {
                let value = function
                    .call(scope, object.into(), &[])
                    .ok_or_else(|| ScriptError::javascript("toJSON threw"))?;
                return webdriver_to_json(scope, value, budget, depth + 1, seen);
            }
        }
        if value.is_array() || value.is_arguments_object() {
            let length = if let Ok(array) = v8::Local::<v8::Array>::try_from(value) {
                array.length() as usize
            } else {
                let key = v8::String::new(scope, "length").unwrap();
                object
                    .get(scope, key.into())
                    .and_then(|value| value.uint32_value(scope))
                    .ok_or_else(|| ScriptError::javascript("invalid arguments length"))?
                    as usize
            };
            if length > MAX_VALUE_NODES {
                return Err(ScriptError::javascript("script array exceeds result limit"));
            }
            let mut result = Vec::with_capacity(length);
            for index in 0..length {
                let item = object
                    .get_index(scope, index as u32)
                    .ok_or_else(|| ScriptError::javascript("script array getter threw"))?;
                result.push(webdriver_to_json(scope, item, budget, depth + 1, seen)?);
            }
            return Ok(ScriptValue::Array(result));
        }
        let keys = object
            .get_own_property_names(
                scope,
                v8::GetPropertyNamesArgs {
                    key_conversion: v8::KeyConversionMode::ConvertToString,
                    ..Default::default()
                },
            )
            .ok_or_else(|| ScriptError::javascript("script property enumeration threw"))?;
        if keys.length() as usize > MAX_VALUE_NODES {
            return Err(ScriptError::javascript(
                "script object exceeds result limit",
            ));
        }
        let mut result = Vec::new();
        for index in 0..keys.length() {
            let key = keys
                .get_index(scope, index)
                .ok_or_else(|| ScriptError::javascript("script key retrieval failed"))?;
            let name = v8::Local::<v8::String>::try_from(key)
                .map_err(|_| ScriptError::javascript("script key was not a string"))?;
            if name.length() > MAX_VALUE_BYTES / 3 {
                return Err(ScriptError::javascript("script key exceeds result limit"));
            }
            let mut units = vec![0; name.length()];
            name.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
            budget.consume(depth, units.len().saturating_mul(2))?;
            let name = units;
            let item = object
                .get(scope, key)
                .ok_or_else(|| ScriptError::javascript("script property getter threw"))?;
            result.push((
                name,
                webdriver_to_json(scope, item, budget, depth + 1, seen)?,
            ));
        }
        Ok(ScriptValue::Object(result))
    })();
    seen.pop();
    result
}

fn webdriver_settle(
    scope: &mut v8::PinScope,
    id: ScriptId,
    value: v8::Local<v8::Value>,
    reject: bool,
) {
    let current = current_environment(scope).ok();
    let paused = scope
        .get_slot::<Arc<WatchdogState>>()
        .map(|watchdog| watchdog.paused_total())
        .unwrap_or_default();
    let epoch = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .map(|state| state.document_authority_epoch());
    let resolver = {
        let Some(registry) = scope.get_slot_mut::<WebDriverScripts>() else {
            return;
        };
        let Some(script) = registry.pending.get_mut(&id) else {
            return;
        };
        if script.settled
            || current != Some(script.environment)
            || epoch != script.authority_epoch
            || script
                .deadline
                .and_then(|deadline| {
                    deadline.checked_add(paused.saturating_sub(script.initial_paused))
                })
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return;
        }
        script.settled = true;
        script.resolver.clone()
    };
    // Promise resolution can inspect a thenable and run author code. No registry
    // or DOM borrow survives this boundary; late/duplicate callbacks are inert.
    let resolver = v8::Local::new(scope, &resolver);
    if reject {
        resolver.reject(scope, value);
    } else {
        resolver.resolve(scope, value);
    }
}

fn webdriver_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(id) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (id, exact) = id.u64_value();
    if exact {
        webdriver_settle(scope, ScriptId(id), args.get(0), false);
    }
}
fn webdriver_rejection_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Ok(id) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (id, exact) = id.u64_value();
    if exact {
        webdriver_settle(scope, ScriptId(id), args.get(0), true);
    }
}

// Forward an already-read then method without inspecting the original object a
// second time. The private data array cannot be reached through page properties.
fn webdriver_forward_then(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(method) = data.get_index(scope, 0) else {
        return;
    };
    let Some(receiver) = data.get_index(scope, 1) else {
        return;
    };
    let Ok(method) = v8::Local::<v8::Function>::try_from(method) else {
        return;
    };
    if let Some(value) = method.call(scope, receiver, &[args.get(0), args.get(1)]) {
        rv.set(value);
    }
}

fn webdriver_private_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<
    (
        v8::Local<'s, v8::PromiseResolver>,
        v8::Local<'s, v8::Promise>,
    ),
    ScriptError,
> {
    let resolver = v8::PromiseResolver::new(scope)
        .ok_or_else(|| ScriptError::javascript("script promise allocation failed"))?;
    let promise = resolver.get_promise(scope);
    let null = v8::null(scope);
    if promise.set_prototype(scope, null.into()) != Some(true) {
        return Err(ScriptError::javascript(
            "private script promise initialization failed",
        ));
    }
    promise.mark_as_handled();
    Ok((resolver, promise))
}

fn webdriver_await_async_return(
    scope: &mut v8::PinScope,
    id: ScriptId,
    value: v8::Local<v8::Value>,
) -> Result<(), ScriptError> {
    if !value.is_object() {
        return Ok(());
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| ScriptError::javascript("invalid async return"))?;
    let key = v8::String::new(scope, "then").unwrap();
    let method = object
        .get(scope, key.into())
        .ok_or_else(|| ScriptError::javascript("async then getter threw"))?;
    let Ok(method) = v8::Local::<v8::Function>::try_from(method) else {
        return Ok(());
    };
    let (resolver, promise) = webdriver_private_promise(scope)?;
    let data = v8::Array::new_with_elements(scope, &[method.into(), value]);
    let forward = v8::Function::builder(webdriver_forward_then)
        .data(data.into())
        .build(scope)
        .ok_or_else(|| ScriptError::javascript("thenable callback allocation failed"))?;
    let wrapper = v8::Object::new(scope);
    let null = v8::null(scope);
    wrapper.set_prototype(scope, null.into());
    if wrapper.create_data_property(scope, key.into(), forward.into()) != Some(true) {
        return Err(ScriptError::javascript(
            "thenable wrapper initialization failed",
        ));
    }
    resolver
        .resolve(scope, wrapper.into())
        .ok_or_else(|| ScriptError::javascript("thenable resolution failed"))?;
    let token = v8::BigInt::new_from_u64(scope, id.0);
    let fulfilled = v8::Function::builder(webdriver_callback)
        .data(token.into())
        .build(scope)
        .ok_or_else(|| ScriptError::javascript("async resolution handler allocation failed"))?;
    let rejected = v8::Function::builder(webdriver_rejection_callback)
        .data(token.into())
        .build(scope)
        .ok_or_else(|| ScriptError::javascript("async rejection handler allocation failed"))?;
    let continuation = promise
        .then2(scope, fulfilled, rejected)
        .ok_or_else(|| ScriptError::javascript("async resolution handlers failed"))?;
    continuation.mark_as_handled();
    Ok(())
}

impl RawV8Host {
    pub fn webdriver_element_command(
        &mut self,
        realm: RealmId,
        command: ElementCommand,
    ) -> Result<ScriptValue, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired() {
            return Err(ScriptError::retired());
        }
        let generation = state.document_generation();
        let epoch = state.document_stream_epoch();
        let authority = state.document_authority_epoch();
        let nodes = match &command {
            ElementCommand::Find { root, selectors } => {
                if selectors.is_empty()
                    || selectors.len() > 64
                    || selectors.iter().any(|value| value.len() > 65536)
                {
                    return Err(ScriptError::invalid("selector chain limit"));
                }
                let mut root = match root {
                    Some(reference) => {
                        let entry = self
                            .isolate
                            .get_slot::<WebDriverScripts>()
                            .and_then(|registry| registry.references.get(reference))
                            .ok_or_else(|| ScriptError {
                                code: "no such element".into(),
                                message: "unknown query root".into(),
                            })?;
                        if !webdriver_node_live(entry) {
                            return Err(ScriptError::stale(entry.shadow));
                        }
                        if entry.owner.realm() != realm {
                            return Err(ScriptError::invalid(
                                "query root belongs to another browsing context",
                            ));
                        }
                        entry.node
                    }
                    None => state.document_root(),
                };
                let mut nodes = Vec::new();
                for (index, selector) in selectors.iter().enumerate() {
                    nodes = oryn_js::v8_bindings::native_automation_query(&state, root, selector)
                        .map_err(|error| ScriptError {
                        code: "invalid selector".into(),
                        message: error.to_string(),
                    })?;
                    if nodes.len() > 4096 {
                        return Err(ScriptError::javascript("automation element result limit"));
                    }
                    if index + 1 < selectors.len() {
                        if nodes.len() != 1 {
                            return Err(ScriptError {
                                code: "no such element".into(),
                                message: "shadow host selector must match exactly one element"
                                    .into(),
                            });
                        }
                        root = state
                            .dom()
                            .borrow()
                            .attached_shadow_root(nodes[0], false)
                            .map_err(|error| ScriptError::javascript(error.to_string()))?
                            .ok_or_else(|| ScriptError {
                                code: "no such shadow root".into(),
                                message: "selector host has no shadow root".into(),
                            })?;
                    }
                }
                nodes
            }
            ElementCommand::Accessibility { reference, role } => {
                let entry = self
                    .isolate
                    .get_slot::<WebDriverScripts>()
                    .and_then(|registry| registry.references.get(reference))
                    .ok_or_else(|| ScriptError {
                        code: "no such element".into(),
                        message: "unknown accessibility element reference".into(),
                    })?;
                if entry.shadow || entry.owner.realm() != realm {
                    return Err(ScriptError {
                        code: "no such element".into(),
                        message: "accessibility target belongs to another context".into(),
                    });
                }
                if entry.owner.execution_retired()
                    || entry.owner.document_generation() != entry.generation
                    || entry.owner.document_authority_epoch() != entry.authority_epoch
                {
                    return Err(ScriptError::stale(false));
                }
                let node = entry.node;
                {
                    let shared = entry.owner.dom();
                    let dom = shared.borrow();
                    if dom.node_document(node).ok().flatten() != Some(entry.owner.document_root()) {
                        return Err(ScriptError::stale(false));
                    }
                    if !dom.is_connected(node).unwrap_or(false) {
                        return Ok(ScriptValue::String(Vec::new()));
                    }
                }

                let context = self
                    .contexts
                    .get(&realm)
                    .cloned()
                    .ok_or_else(ScriptError::retired)?;
                v8::scope!(let handles, &mut self.isolate);
                let context = v8::Local::new(handles, &context);
                let scope = &mut v8::ContextScope::new(handles, context);
                let value =
                    oryn_js::v8_bindings::native_automation_accessibility(scope, &state, node)
                        .map_err(|error| ScriptError::javascript(error.to_string()))?;
                return Ok(ScriptValue::String(
                    if *role { value.role } else { value.name }
                        .encode_utf16()
                        .collect(),
                ));
            }
            ElementCommand::AccessibilityProperties {
                reference,
                accessibility_node,
            } => {
                let reference = if *accessibility_node {
                    reference.strip_prefix("ax-").ok_or_else(|| {
                        ScriptError::invalid("invalid accessibility node identity")
                    })?
                } else {
                    reference.as_str()
                };
                vec![self.webdriver_action_element(realm, reference)?]
            }
            ElementCommand::FileInput { reference } => {
                let node = self.webdriver_action_element(realm, reference)?;
                return Ok(ScriptValue::Bool(state.dom().borrow().is_file_input(node)));
            }
            ElementCommand::Focus { reference }
            | ElementCommand::PrepareClick { reference }
            | ElementCommand::SetFiles { reference, .. } => {
                vec![self.webdriver_action_element(realm, reference)?]
            }
        };
        self.activate_realm(realm)?;
        if self.isolate.get_slot::<WebDriverScripts>().is_none() {
            self.isolate.set_slot(WebDriverScripts::default());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = (|| -> Result<ScriptValue, ScriptError> {
            match command {
                ElementCommand::Find { .. } => {
                    let mut values = Vec::new();
                    for node in nodes {
                        let wrapper = wrap_node(scope, &state, node)
                            .map_err(|error| ScriptError::javascript(error.to_string()))?;
                        values.push(webdriver_node_json(scope, state.clone(), node, wrapper)?);
                    }
                    Ok(ScriptValue::Array(values))
                }
                ElementCommand::Focus { .. } => {
                    let focused =
                        oryn_js::v8_bindings::native_automation_focus(scope, &state, nodes[0])
                            .map_err(|error| ScriptError::javascript(error.to_string()))?;
                    if !focused {
                        Err(ScriptError {
                            code: "element not interactable".into(),
                            message: "element could not receive keyboard focus".into(),
                        })
                    } else {
                        Ok(ScriptValue::Null)
                    }
                }
                ElementCommand::AccessibilityProperties { .. } => {
                    use oryn_common::v3::ProjectionProfile;
                    let layout = state
                        .native_accessibility_layout_in_scope(scope)
                        .map_err(ScriptError::javascript)?;
                    let shared = state.dom();
                    let dom = shared.borrow();
                    let graph =
                        crate::executable::layout_accessibility_graph(&dom, generation, &layout)
                            .map_err(ScriptError::javascript)?;
                    let semantic = graph
                        .nodes(ProjectionProfile::Full)
                        .into_iter()
                        .map(|node| (node.semantic_ref, node))
                        .collect::<BTreeMap<_, _>>();
                    let accessible = graph
                        .accessibility(ProjectionProfile::Full)
                        .into_iter()
                        .filter(|entry| {
                            let node = crate::semantic::unpack_node_ref(entry.semantic_ref.node);
                            !entry.excluded
                                && dom.node_document(node).ok().flatten()
                                    == Some(state.document_root())
                                && dom.is_connected(node).unwrap_or(false)
                        })
                        .collect::<Vec<_>>();
                    if accessible.len() > 4096 {
                        return Err(ScriptError::javascript("accessibility snapshot node limit"));
                    }
                    let target = accessible
                        .iter()
                        .find(|entry| {
                            crate::semantic::unpack_node_ref(entry.semantic_ref.node) == nodes[0]
                        })
                        .ok_or_else(|| ScriptError {
                            code: "no such element".into(),
                            message: "element has no exposed accessibility node".into(),
                        })?;
                    let included = accessible
                        .iter()
                        .map(|entry| entry.semantic_ref)
                        .collect::<std::collections::BTreeSet<_>>();
                    let parent = |entry: &oryn_common::v5::AccessibilityNode| {
                        let mut current = entry.owned_by.or_else(|| {
                            semantic
                                .get(&entry.semantic_ref)
                                .and_then(|node| node.parent)
                        });
                        let mut seen = std::collections::BTreeSet::new();
                        while let Some(reference) = current {
                            if !seen.insert(reference) {
                                return None;
                            }
                            if included.contains(&reference) {
                                return Some(reference);
                            }
                            current = semantic.get(&reference).and_then(|node| node.parent);
                        }
                        None
                    };
                    let mut identities = BTreeMap::new();
                    drop(dom);
                    for entry in &accessible {
                        let node = crate::semantic::unpack_node_ref(entry.semantic_ref.node);
                        let wrapper = wrap_node(scope, &state, node)
                            .map_err(|error| ScriptError::javascript(error.to_string()))?;
                        let reference = webdriver_node_json(scope, state.clone(), node, wrapper)?;
                        let id = reference
                            .get(ELEMENT_KEY)
                            .ok_or_else(|| {
                                ScriptError::javascript("missing native accessibility reference")
                            })?
                            .as_string()?;
                        identities.insert(entry.semantic_ref, format!("ax-{id}"));
                    }
                    let mut properties = serde_json::Map::new();
                    for (key, value) in &target.properties {
                        let value = if matches!(
                            key.as_str(),
                            "checked"
                                | "pressed"
                                | "selected"
                                | "expanded"
                                | "disabled"
                                | "required"
                                | "readonly"
                                | "busy"
                                | "modal"
                                | "multiline"
                        ) {
                            match value {
                                serde_json::Value::Bool(value) => {
                                    serde_json::Value::String(value.to_string())
                                }
                                other => other.clone(),
                            }
                        } else {
                            value.clone()
                        };
                        properties.insert(key.clone(), value);
                    }
                    properties.insert(
                        "accessibilityId".into(),
                        serde_json::json!(identities[&target.semantic_ref]),
                    );
                    properties.insert("role".into(), serde_json::json!(target.role));
                    properties.insert("label".into(), serde_json::json!(target.name));
                    properties.insert(
                        "parent".into(),
                        serde_json::json!(
                            parent(target).and_then(|reference| identities.get(&reference))
                        ),
                    );
                    properties.insert(
                        "children".into(),
                        serde_json::json!(
                            accessible
                                .iter()
                                .filter(|entry| parent(entry) == Some(target.semantic_ref))
                                .map(|entry| &identities[&entry.semantic_ref])
                                .collect::<Vec<_>>()
                        ),
                    );
                    Ok(ScriptValue::from(serde_json::Value::Object(properties)))
                }
                ElementCommand::SetFiles { files, .. } => {
                    let node = nodes[0];
                    if files.is_empty()
                        || files.len() > 128
                        || files
                            .iter()
                            .try_fold(0usize, |total, file| {
                                total.checked_add(file.file.bytes.len())
                            })
                            .is_none_or(|total| total > 64 * 1024 * 1024)
                    {
                        return Err(ScriptError::invalid("file selection payload limit"));
                    }
                    let selected = {
                        let shared = state.dom();
                        let dom = shared.borrow();
                        if !dom.is_file_input(node) {
                            return Err(ScriptError::invalid("element is not a file input"));
                        }
                        let multiple = dom
                            .attribute_in_null_namespace(node, "multiple")
                            .map_err(|error| ScriptError::javascript(error.to_string()))?
                            .is_some();
                        if !multiple && files.len() != 1 {
                            return Err(ScriptError::invalid(
                                "single file input requires exactly one file",
                            ));
                        }
                        let mut selected = if multiple {
                            dom.arena()
                                .get(node)
                                .map_err(|error| ScriptError::javascript(error.to_string()))?
                                .selected_files
                                .as_ref()
                                .clone()
                        } else {
                            Vec::new()
                        };
                        for upload in files {
                            let file = upload.file;
                            selected.push(oryn_dom::SelectedFile {
                                name: file.filename,
                                media_type: file.media_type,
                                bytes: file.bytes.into(),
                                last_modified: upload.last_modified,
                            });
                        }
                        if selected.len() > 128
                            || selected
                                .iter()
                                .try_fold(0usize, |total, file| total.checked_add(file.bytes.len()))
                                .is_none_or(|total| total > 64 * 1024 * 1024)
                        {
                            return Err(ScriptError::invalid("file input retained payload limit"));
                        }
                        selected
                    };
                    state
                        .dom()
                        .borrow_mut()
                        .set_selected_files(node, std::sync::Arc::new(selected))
                        .map_err(|error| ScriptError::javascript(error.to_string()))?;
                    oryn_js::v8_bindings::notify_native_control(
                        scope,
                        &state,
                        node,
                        oryn_js::v8_bindings::NativeControlNotification::InputAndChange,
                    )
                    .map_err(|error| ScriptError::javascript(error.to_string()))?;
                    Ok(ScriptValue::Null)
                }
                ElementCommand::PrepareClick { .. } => {
                    if state.dom().borrow().is_file_input(nodes[0]) {
                        return Err(ScriptError::invalid(
                            "file inputs require element send keys",
                        ));
                    }
                    let option_container =
                        oryn_js::v8_bindings::native_automation_option_container(&state, nodes[0])
                            .map_err(|error| ScriptError::javascript(error.to_string()))?;
                    let click_target = option_container.unwrap_or(nodes[0]);
                    if let Some(select) = option_container {
                        match oryn_js::v8_bindings::native_automation_click_ready(
                            scope, &state, select,
                        )
                        .map_err(|error| ScriptError::javascript(error.to_string()))?
                        {
                            Some(true) => {}
                            Some(false) => {
                                return Err(ScriptError {
                                    code: "element click intercepted".into(),
                                    message: "select container is obscured".into(),
                                });
                            }
                            None => {
                                return Err(ScriptError {
                                    code: "element not interactable".into(),
                                    message: "select container has no in-view center".into(),
                                });
                            }
                        }
                        oryn_js::v8_bindings::native_automation_option_click(
                            scope, &state, nodes[0], select,
                        )
                        .map_err(|error| ScriptError::javascript(error.to_string()))?;
                        return Ok(ScriptValue::Bool(true));
                    }
                    match oryn_js::v8_bindings::native_automation_click_ready(
                        scope,
                        &state,
                        click_target,
                    )
                    .map_err(|error| ScriptError::javascript(error.to_string()))?
                    {
                        Some(true) => Ok(ScriptValue::Null),
                        Some(false) => Err(ScriptError {
                            code: "element click intercepted".into(),
                            message: "another painted element covers the target center".into(),
                        }),
                        None => Err(ScriptError {
                            code: "element not interactable".into(),
                            message: "element has no in-view center".into(),
                        }),
                    }
                }
                ElementCommand::Accessibility { .. } | ElementCommand::FileInput { .. } => {
                    unreachable!()
                }
            }
        })();
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::timeout());
        }
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
            || state.document_authority_epoch() != authority
        {
            return Err(ScriptError::retired());
        }
        result
    }

    pub(crate) fn os_composition_target(&self, realm: RealmId) -> Option<NodeId> {
        self.binding_states
            .get(&realm)
            .and_then(|state| oryn_js::v8_bindings::native_composition_target(state))
    }
    pub(crate) fn os_keyboard_deferred_target(
        &self,
        realm: RealmId,
    ) -> Result<Option<NodeId>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        Ok(oryn_js::v8_bindings::native_keyboard_deferred_target(state))
    }
    pub(crate) fn os_keyboard_was_deferred(&self, realm: RealmId) -> Result<bool, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        Ok(oryn_js::v8_bindings::native_keyboard_was_deferred(state))
    }
    pub(crate) fn cancel_webdriver_keyboard_defaults(&self) {
        for realm in self.active_root_realm_ids() {
            if let Some(state) = self.binding_states.get(&realm) {
                oryn_js::v8_bindings::cancel_native_keyboard_defaults(state);
            }
        }
    }
    pub(crate) fn reset_webdriver_pointer_sources(&self) {
        for realm in self.active_root_realm_ids() {
            if let Some(state) = self.binding_states.get(&realm) {
                oryn_js::v8_bindings::reset_native_pointer_sources(state);
            }
        }
    }
    pub(crate) fn automation_keyboard_focus_document(
        &self,
        realm: RealmId,
    ) -> Result<Option<NodeId>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        oryn_js::v8_bindings::native_keyboard_focus_document(state)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn automation_pointer_owns_routing(&self, realm: RealmId, id: i32) -> bool {
        self.binding_states
            .get(&realm)
            .is_some_and(|state| oryn_js::v8_bindings::native_pointer_owns_routing(state, id))
    }
    pub(crate) fn automation_pointer_has_hover(&self, realm: RealmId, id: i32) -> bool {
        self.binding_states
            .get(&realm)
            .is_some_and(|state| oryn_js::v8_bindings::native_pointer_has_hover(state, id))
    }
    pub(crate) fn automation_pointer_drag_source(
        &self,
        realm: RealmId,
        id: i32,
        candidate: bool,
    ) -> bool {
        self.binding_states.get(&realm).is_some_and(|state| {
            oryn_js::v8_bindings::native_pointer_dragging(state, id)
                || candidate && oryn_js::v8_bindings::native_pointer_drag_candidate(state, id)
        })
    }
    pub(crate) fn abandon_pointer_drag(&self, realm: RealmId, id: i32) {
        if let Some(state) = self.binding_states.get(&realm) {
            oryn_js::v8_bindings::abandon_native_pointer_drag(state, id);
        }
    }
    pub(crate) fn pointer_drag_target_identity(
        &self,
        realm: RealmId,
    ) -> Option<(RealmId, NodeId, u64, u64)> {
        self.binding_states
            .get(&realm)
            .and_then(|state| oryn_js::v8_bindings::native_drag_target_identity(state))
    }
    pub(crate) fn automation_pointer_physical_target(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        x: f64,
        y: f64,
    ) -> Result<Option<NodeId>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let target = oryn_js::v8_bindings::native_pointer_physical_target(scope, &state, x, y)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        Ok(target)
    }
    pub(crate) fn automation_pointer_parent_coordinates(
        &mut self,
        realm: RealmId,
        frame: NodeId,
        x: f64,
        y: f64,
        movement_x: f64,
        movement_y: f64,
    ) -> Result<Option<oryn_js::v8_bindings::NativeFramePointerTarget>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        let state = state.clone();
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        oryn_js::v8_bindings::native_frame_pointer_parent_coordinates(
            scope, &state, frame, x, y, movement_x, movement_y,
        )
        .map_err(|error| ScriptError::javascript(error.to_string()))
    }
    pub(crate) fn automation_pointer_frame_coordinates(
        &mut self,
        realm: RealmId,
        frame: NodeId,
        x: f64,
        y: f64,
        movement_x: f64,
        movement_y: f64,
    ) -> Result<Option<oryn_js::v8_bindings::NativeFramePointerTarget>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        let state = state.clone();
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        oryn_js::v8_bindings::native_frame_pointer_coordinates(
            scope, &state, frame, x, y, movement_x, movement_y,
        )
        .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn automation_pointer_frame_target(
        &mut self,
        realm: RealmId,
        id: Option<i32>,
        x: f64,
        y: f64,
        movement_x: f64,
        movement_y: f64,
    ) -> Result<Option<oryn_js::v8_bindings::NativeFramePointerTarget>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        let state = state.clone();
        let lifetime = (
            state.document_root(),
            state.document_generation(),
            state.document_stream_epoch(),
            state.document_authority_epoch(),
        );
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let target = oryn_js::v8_bindings::native_frame_pointer_target(
            scope, &state, id, x, y, movement_x, movement_y,
        )
        .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if state.execution_retired()
            || lifetime
                != (
                    state.document_root(),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.document_authority_epoch(),
                )
        {
            return Err(ScriptError::retired());
        }
        Ok(target)
    }

    #[cfg(test)]
    pub(crate) fn dispatch_webdriver_pointer(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativePointerInput,
        action: oryn_js::v8_bindings::NativePointerAction,
    ) -> Result<(NodeId, bool), ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let root_realms = self.active_root_realm_ids();
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = oryn_js::v8_bindings::dispatch_native_pointer(scope, &state, input, action)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if input.buttons == 0
            && matches!(
                action,
                oryn_js::v8_bindings::NativePointerAction::Up
                    | oryn_js::v8_bindings::NativePointerAction::Cancel
            )
        {
            for other in &root_realms {
                if *other != realm
                    && let Some(owner) = self.binding_states.get(other)
                {
                    oryn_js::v8_bindings::synchronize_native_pointer_release(owner, input.id);
                }
            }
        }
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "pointer listener exceeded execution deadline",
            ));
        }
        if !live() {
            return Err(ScriptError::retired());
        }
        result
    }

    pub(crate) fn begin_pointer_boundary(
        &self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativePointerInput,
        target: Option<NodeId>,
    ) -> Result<oryn_js::v8_bindings::NativePointerBoundary, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        oryn_js::v8_bindings::NativePointerBoundary::new(state, input, target)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn poll_pointer_boundary(
        &mut self,
        receipt: &mut oryn_js::v8_bindings::NativePointerBoundary,
    ) -> Result<bool, ScriptError> {
        let state = receipt.owner();
        let realm = state.realm();
        if !self
            .binding_states
            .get(&realm)
            .is_some_and(|owner| Rc::ptr_eq(owner, &state))
        {
            receipt.cancel();
            return Err(ScriptError::retired());
        }
        let Some(context) = self.contexts.get(&realm).cloned() else {
            receipt.cancel();
            return Err(ScriptError::retired());
        };
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = receipt
            .poll(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            receipt.cancel();
            return Err(ScriptError::javascript(
                "pointer boundary exceeded execution deadline",
            ));
        }
        result
    }

    pub(crate) fn begin_pointer_dispatch(
        &self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativePointerInput,
        action: oryn_js::v8_bindings::NativePointerAction,
    ) -> Result<oryn_js::v8_bindings::NativePointerDispatch, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        oryn_js::v8_bindings::NativePointerDispatch::new(state, input, action)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn poll_pointer_dispatch(
        &mut self,
        receipt: &mut oryn_js::v8_bindings::NativePointerDispatch,
    ) -> Result<Option<(NodeId, bool)>, ScriptError> {
        let state = receipt.owner();
        let realm = state.realm();
        if !self
            .binding_states
            .get(&realm)
            .is_some_and(|owner| Rc::ptr_eq(owner, &state))
        {
            receipt.cancel();
            return Err(ScriptError::retired());
        }
        let Some(context) = self.contexts.get(&realm).cloned() else {
            receipt.cancel();
            return Err(ScriptError::retired());
        };
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = receipt
            .poll(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            receipt.cancel();
            return Err(ScriptError::javascript(
                "pointer listener exceeded execution deadline",
            ));
        }
        result
    }

    pub(crate) fn begin_pointer_drag_admission(
        &self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativePointerInput,
    ) -> Result<oryn_js::v8_bindings::NativePointerDragAdmission, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        oryn_js::v8_bindings::NativePointerDragAdmission::new(state, input)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn poll_pointer_drag_admission(
        &mut self,
        receipt: &mut oryn_js::v8_bindings::NativePointerDragAdmission,
    ) -> Result<Option<bool>, ScriptError> {
        let state = receipt.owner();
        let realm = state.realm();
        if !self
            .binding_states
            .get(&realm)
            .is_some_and(|owner| Rc::ptr_eq(owner, &state))
        {
            receipt.cancel();
            return Err(ScriptError::retired());
        }
        let Some(context) = self.contexts.get(&realm).cloned() else {
            receipt.cancel();
            return Err(ScriptError::retired());
        };
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = receipt
            .poll(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            receipt.cancel();
            return Err(ScriptError::javascript(
                "pointer drag admission exceeded execution deadline",
            ));
        }
        result
    }

    pub(crate) fn prepare_pointer_drag(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativePointerInput,
    ) -> Result<bool, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = oryn_js::v8_bindings::prepare_native_pointer_drag(scope, &state, input)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "pointer listener exceeded execution deadline",
            ));
        }
        if !live() {
            return Err(ScriptError::retired());
        }
        result
    }

    fn native_drag_ancestor_chain(
        &mut self,
        realm: RealmId,
    ) -> Result<Vec<oryn_js::v8_bindings::NativeDragAncestor>, ScriptError> {
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles,&mut self.isolate);
        let mut cursor = v8::Local::new(handles, &context);
        let mut seen = BTreeSet::new();
        let mut ancestors = Vec::new();
        while let Some(frame) = cursor.get_slot::<FrameLifetime>() {
            if frame.closed.get() || !seen.insert(frame.parent_environment.realm) || seen.len() > 32
            {
                return Err(ScriptError::retired());
            }
            let state = frame.parent.upgrade().ok_or_else(ScriptError::retired)?;
            if state.execution_retired()
                || state.dom().borrow().frame_context_generation(frame.frame)
                    != Some(frame.connection_generation)
            {
                return Err(ScriptError::retired());
            }
            ancestors.push(
                oryn_js::v8_bindings::NativeDragAncestor::new(state, frame.frame)
                    .map_err(|error| ScriptError::javascript(error.to_string()))?,
            );
            let parent = self
                .contexts
                .get(&frame.parent_environment.realm)
                .ok_or_else(ScriptError::retired)?;
            cursor = v8::Local::new(handles, parent);
        }
        Ok(ancestors)
    }

    pub(crate) fn begin_drag_between_realms(
        &mut self,
        source: (RealmId, u64, u64),
        target: Option<((RealmId, u64, u64), NodeId)>,
        action: oryn_js::v8_bindings::NativeCrossFrameDragAction,
        source_input: &oryn_js::v8_bindings::NativePointerInput,
        target_input: &oryn_js::v8_bindings::NativePointerInput,
        previous_input: Option<&oryn_js::v8_bindings::NativePointerInput>,
    ) -> Result<oryn_js::v8_bindings::NativeCrossFrameDrag, ScriptError> {
        use oryn_js::v8_bindings::NativeDragRealm;
        let mut chains = BTreeMap::from([(source.0, self.native_drag_ancestor_chain(source.0)?)]);
        if let Some((identity, _)) = target
            && identity.0 != source.0
        {
            chains.insert(identity.0, self.native_drag_ancestor_chain(identity.0)?);
        }
        let owner=|(realm,generation,epoch):(RealmId,u64,u64)|->Result<NativeDragRealm,ScriptError>{
            let state=self.binding_states.get(&realm).cloned().ok_or_else(ScriptError::retired)?;
            if state.execution_retired()||state.document_generation()!=generation||state.document_stream_epoch()!=epoch{return Err(ScriptError::retired());}
            let context=self.contexts.get(&realm).cloned().ok_or_else(ScriptError::retired)?;
            Ok(NativeDragRealm::new(state,context).with_ancestors(chains.get(&realm).cloned().unwrap_or_default()))
        };
        let source_owner = owner(source)?;
        let target_owner = target
            .map(|(identity, node)| owner(identity).map(|owner| (owner, node)))
            .transpose()?;
        let init =
            |input: &oryn_js::v8_bindings::NativePointerInput| oryn_js::data_transfer::EventInit {
                client_x: input.x as i32,
                client_y: input.y as i32,
                screen_x: input.screen_x as i32,
                screen_y: input.screen_y as i32,
                button: input.button as i16,
                buttons: input.buttons as u16,
                ctrl_key: input.control,
                alt_key: input.alt,
                shift_key: input.shift,
                meta_key: input.meta,
                ..Default::default()
            };
        let previous_init = previous_input.map(init);
        let mut receipt = oryn_js::v8_bindings::NativeCrossFrameDrag::new(
            &source_owner,
            target_owner.as_ref().map(|(owner, node)| (owner, *node)),
            action,
            &init(source_input),
            &init(target_input),
            previous_init.as_ref(),
        )
        .map_err(|error| ScriptError::javascript(error.to_string()))?;
        for owner in self.binding_states.values() {
            receipt.track_pointer_owner(owner);
        }
        Ok(receipt)
    }

    pub(crate) fn poll_drag_between_realms(
        &mut self,
        receipt: &mut oryn_js::v8_bindings::NativeCrossFrameDrag,
        input: &oryn_js::v8_bindings::NativePointerInput,
        terminal: bool,
    ) -> Result<bool, ScriptError> {
        let state = receipt.owner();
        let realm = state.realm();
        if !self
            .binding_states
            .get(&realm)
            .is_some_and(|owner| Rc::ptr_eq(owner, &state))
        {
            receipt.cancel();
            return Err(ScriptError::retired());
        }
        let Some(context) = self.contexts.get(&realm).cloned() else {
            receipt.cancel();
            return Err(ScriptError::retired());
        };
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = receipt
            .poll(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        let timed_out = deadline.expired() || scope.is_execution_terminating();
        if timed_out {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            receipt.cancel();
        }
        if matches!(&result, Ok(Some(_))) || timed_out {
            let terminal =
                terminal || timed_out || !oryn_js::v8_bindings::native_drag_session_active(&state);
            receipt.finish_pointer_transition(input, terminal);
        }
        if timed_out {
            return Err(ScriptError::javascript(
                "drag listener exceeded execution deadline",
            ));
        }
        result.map(|result| result.is_some())
    }

    pub(crate) fn dispatch_drag_between_realms(
        &mut self,
        source: (RealmId, u64, u64),
        target: Option<((RealmId, u64, u64), NodeId)>,
        action: oryn_js::v8_bindings::NativeCrossFrameDragAction,
        source_input: &oryn_js::v8_bindings::NativePointerInput,
        target_input: &oryn_js::v8_bindings::NativePointerInput,
        previous_input: Option<&oryn_js::v8_bindings::NativePointerInput>,
    ) -> Result<(), ScriptError> {
        let mut receipt = self.begin_drag_between_realms(
            source,
            target,
            action,
            source_input,
            target_input,
            previous_input,
        )?;
        let terminal = matches!(
            action,
            oryn_js::v8_bindings::NativeCrossFrameDragAction::Drop
                | oryn_js::v8_bindings::NativeCrossFrameDragAction::Cancel
        );
        if self.poll_drag_between_realms(&mut receipt, source_input, terminal)? {
            Ok(())
        } else {
            Err(ScriptError::javascript("cross-frame drag suspended"))
        }
    }

    pub(crate) fn begin_webdriver_keyboard_receipt(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativeKeyboardInput,
    ) -> Result<oryn_js::v8_bindings::NativeKeyboardReceipt, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.execution_suspended()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        self.update_root_pointer_modifiers(realm, &input.modifiers);
        oryn_js::v8_bindings::begin_native_keyboard_receipt(state, input)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }
    pub(crate) fn poll_webdriver_keyboard_receipt(
        &mut self,
        receipt: &mut oryn_js::v8_bindings::NativeKeyboardReceipt,
    ) -> Result<Option<bool>, ScriptError> {
        let realm = receipt.realm();
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        if !receipt.owns_state(&state)
            || state.execution_retired()
            || state.document_generation() != receipt.generation()
            || state.document_stream_epoch() != receipt.epoch()
        {
            return Err(ScriptError::retired());
        }
        if state.execution_suspended() {
            return Ok(None);
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = oryn_js::v8_bindings::poll_native_keyboard_receipt(scope, receipt)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "keyboard listener exceeded execution deadline",
            ));
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn dispatch_webdriver_keyboard(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativeKeyboardInput,
    ) -> Result<bool, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        self.update_root_pointer_modifiers(realm, &input.modifiers);
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = oryn_js::v8_bindings::dispatch_native_keyboard(scope, &state, input)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "keyboard listener exceeded execution deadline",
            ));
        }
        if !live() {
            return Err(ScriptError::retired());
        }
        result
    }

    pub(crate) fn dispatch_webdriver_wheel(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        input: &oryn_js::v8_bindings::NativeWheelInput,
    ) -> Result<oryn_js::v8_bindings::NativeWheelResult, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result =
            oryn_js::v8_bindings::dispatch_native_wheel_with_remainder(scope, &state, input)
                .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "wheel listener exceeded execution deadline",
            ));
        }
        if !live() {
            return Err(ScriptError::retired());
        }
        result
    }

    pub(crate) fn apply_webdriver_wheel_remainder(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        target: NodeId,
        delta: (f64, f64),
    ) -> Result<(f64, f64), ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        if !self.realm_execution_live(realm) {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        oryn_js::v8_bindings::apply_native_wheel_default(scope, &state, target, delta.0, delta.1)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }

    pub(crate) fn semantic_scroll_into_view(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        target: NodeId,
    ) -> Result<(), ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
            || state
                .dom()
                .borrow()
                .node_document(target)
                .map_err(|error| ScriptError::javascript(error.to_string()))?
                != Some(state.document_root())
            || !state
                .dom()
                .borrow()
                .is_connected(target)
                .map_err(|error| ScriptError::javascript(error.to_string()))?
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        state
            .scroll_node_into_view_in_scope(scope, target)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        Ok(())
    }

    pub(crate) fn semantic_pointer_point(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        target: NodeId,
    ) -> Result<(f64, f64), ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let point = oryn_js::v8_bindings::native_semantic_pointer_point(scope, &state, target)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        Ok(point)
    }

    pub(crate) fn webdriver_action_coordinates(
        &mut self,
        selected: &super::webdriver_script::AutomationRealmSnapshot,
        origin: &crate::webdriver_actions::Origin,
        x: i64,
        y: i64,
        pointer: (f64, f64),
    ) -> Result<(f64, f64), ScriptError> {
        let realm = selected.realm;
        let generation = selected.document_generation;
        let epoch = selected.stream_epoch;
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        let state = state.clone();
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        let target = match origin {
            crate::webdriver_actions::Origin::Element(reference) => {
                Some(self.webdriver_action_element(realm, reference)?)
            }
            _ => None,
        };
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let (viewport, rect) = oryn_js::v8_bindings::native_input_geometry(scope, &state, target)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if !live() {
            return Err(ScriptError::retired());
        }
        crate::webdriver_actions::resolve_coordinates(
            origin,
            x,
            y,
            pointer,
            viewport,
            rect.map(|rect| (rect.x, rect.y, rect.width, rect.height)),
        )
    }

    /// Resolve action origins in the selected browsing context, even when a
    /// different context happens to share its origin. Node IDs alone are not
    /// automation capabilities and never enter this lookup from request JSON.
    pub fn webdriver_action_element(
        &self,
        realm: RealmId,
        reference: &str,
    ) -> Result<NodeId, ScriptError> {
        let entry = self
            .isolate
            .get_slot::<WebDriverScripts>()
            .and_then(|registry| registry.references.get(reference))
            .ok_or_else(|| ScriptError {
                code: "no such element".into(),
                message: "unknown action element reference".into(),
            })?;
        if entry.shadow {
            return Err(ScriptError {
                code: "no such element".into(),
                message: "action origin must be an element".into(),
            });
        }
        if !webdriver_node_live(entry) {
            return Err(ScriptError::stale(false));
        }
        if entry.owner.realm() != realm {
            return Err(ScriptError {
                code: "no such element".into(),
                message: "action element belongs to another browsing context".into(),
            });
        }
        Ok(entry.node)
    }

    /// Historical selected-document regression adapter. Production keyboard
    /// Actions own the retained clipboard job through their native receipt.
    #[cfg(test)]
    pub(crate) fn dispatch_authorized_clipboard(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        target: NodeId,
        action: oryn_js::clipboard::ClipboardAction,
    ) -> Result<oryn_js::clipboard::ClipboardActionResult, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let live = || {
            !state.execution_retired()
                && state.document_generation() == generation
                && state.document_stream_epoch() == epoch
        };
        if !live() {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let mut grant = oryn_js::v8_bindings::grant_native_clipboard_action(&state, target, action)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let handles,&mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let result = oryn_js::v8_bindings::native_clipboard_action(scope, &state, &mut grant)
            .map_err(|error| ScriptError::javascript(error.to_string()));
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(ScriptError::javascript(
                "clipboard action listener exceeded execution deadline",
            ));
        }
        if !live() {
            return Err(ScriptError::retired());
        }
        result
    }

    pub fn webdriver_frame_element(
        &self,
        parent: RealmId,
        reference: &str,
    ) -> Result<NodeId, ScriptError> {
        let entry = self
            .isolate
            .get_slot::<WebDriverScripts>()
            .and_then(|registry| registry.references.get(reference))
            .ok_or_else(|| ScriptError {
                code: "no such element".into(),
                message: "unknown frame element reference".into(),
            })?;
        if entry.shadow || !webdriver_node_live(entry) {
            return Err(ScriptError::stale(entry.shadow));
        }
        if entry.owner.realm() != parent {
            return Err(ScriptError {
                code: "no such element".into(),
                message: "frame element belongs to another browsing context".into(),
            });
        }
        let dom = entry.owner.dom();
        let dom = dom.borrow();
        if dom.embedded_content_kind(entry.node).is_none() {
            return Err(ScriptError {
                code: "no such frame".into(),
                message: "element is not a frame".into(),
            });
        }
        Ok(entry.node)
    }

    /// Replace an actor-owned handle map atomically before beginning a command.
    /// A pending operation pins its mapping until completion or cancellation.
    pub fn set_webdriver_window_handles(
        &mut self,
        bindings: Vec<WindowHandleBinding>,
    ) -> Result<(), ScriptError> {
        if bindings.len() > 4096 {
            return Err(ScriptError::invalid("window handle limit"));
        }
        if self
            .isolate
            .get_slot::<WebDriverScripts>()
            .is_some_and(|registry| !registry.pending.is_empty())
        {
            return Err(ScriptError::invalid(
                "cannot replace window handles during a pending script",
            ));
        }
        self.adopt_created_realms();
        let mut references = BTreeMap::new();
        let mut handles = BTreeSet::new();
        for binding in bindings {
            if binding.handle.is_empty()
                || binding.handle.len() > 4096
                || !handles.insert(binding.handle.clone())
                || references.contains_key(&binding.realm)
            {
                return Err(ScriptError::invalid(
                    "invalid or duplicate browsing context handle",
                ));
            }
            let owner = self
                .binding_states
                .get(&binding.realm)
                .cloned()
                .ok_or_else(ScriptError::retired)?;
            if owner.execution_retired()
                || owner.document_generation() != binding.document_generation
                || owner.document_stream_epoch() != binding.stream_epoch
            {
                return Err(ScriptError::retired());
            }
            let context = self
                .contexts
                .get(&binding.realm)
                .cloned()
                .ok_or_else(ScriptError::retired)?;
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            let object = context.global(scope);
            let object = v8::Global::new(scope, object);
            let authority_epoch = owner.document_authority_epoch();
            references.insert(
                binding.realm,
                WebDriverWindowReference {
                    binding,
                    authority_epoch,
                    owner,
                    object,
                },
            );
        }
        if self.isolate.get_slot::<WebDriverScripts>().is_none() {
            self.isolate.set_slot(WebDriverScripts::default());
        }
        self.isolate
            .get_slot_mut::<WebDriverScripts>()
            .unwrap()
            .windows = references;
        Ok(())
    }

    pub fn begin_webdriver_script(
        &mut self,
        request: ScriptRequest,
    ) -> Result<ScriptId, ScriptError> {
        if request.body.len().saturating_mul(2) > MAX_SCRIPT_BYTES || request.args.len() > 4096 {
            return Err(ScriptError::invalid(
                "script source or argument count exceeds limit",
            ));
        }
        self.adopt_created_realms();
        if self.isolate.get_slot::<WebDriverScripts>().is_none() {
            self.isolate.set_slot(WebDriverScripts::default());
        }
        let id = {
            let registry = self.isolate.get_slot_mut::<WebDriverScripts>().unwrap();
            if registry.pending.len() >= 8 {
                return Err(ScriptError::javascript(
                    "native script operation capacity exhausted",
                ));
            }
            registry.next = registry
                .next
                .checked_add(1)
                .ok_or_else(|| ScriptError::javascript("script identity space exhausted"))?;
            ScriptId(registry.next)
        };
        let started = Instant::now();
        let deadline = request
            .timeout_ms
            .map(|milliseconds| {
                started
                    .checked_add(Duration::from_millis(milliseconds))
                    .ok_or_else(|| {
                        ScriptError::invalid("script timeout exceeds monotonic clock range")
                    })
            })
            .transpose()?;
        let wall_limit = request
            .timeout_ms
            .map(|milliseconds| Duration::from_millis(milliseconds).min(self.execution_wall_limit))
            .unwrap_or(self.execution_wall_limit);
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        let watchdog = ExecutionDeadline::arm(&self.isolate, wall_limit);
        let result = {
            v8::scope!(let handles, &mut self.isolate);
            let context = v8::Local::new(handles, &context);
            let scope = &mut v8::ContextScope::new(handles, context);
            v8::tc_scope!(let scope, scope);
            let result = (|| {
                let environment = current_environment(scope).map_err(|_| ScriptError::retired())?;
                let state = context.get_slot::<V8BindingState>();
                if state
                    .as_ref()
                    .is_some_and(|state| state.execution_retired())
                {
                    return Err(ScriptError::retired());
                }
                let authority_epoch = state.as_ref().map(|state| state.document_authority_epoch());
                let mut budget = ValueBudget::new();
                let mut arguments =
                    Vec::with_capacity(request.args.len() + usize::from(request.asynchronous));
                for value in &request.args {
                    arguments.push(webdriver_from_json(scope, value, &mut budget, 0)?);
                }
                let (resolver, promise) = webdriver_private_promise(scope)?;
                let initial_paused = scope
                    .get_slot::<Arc<WatchdogState>>()
                    .map(|watchdog| watchdog.paused_total())
                    .unwrap_or_default();
                let entry = WebDriverScript {
                    environment,
                    authority_epoch,
                    deadline,
                    initial_paused,
                    resolver: v8::Global::new(scope, resolver),
                    promise: v8::Global::new(scope, promise),
                    settled: false,
                };
                scope
                    .get_slot_mut::<WebDriverScripts>()
                    .unwrap()
                    .pending
                    .insert(id, entry);
                if request.asynchronous {
                    let token = v8::BigInt::new_from_u64(scope, id.0);
                    let callback = v8::Function::builder(webdriver_callback)
                        .data(token.into())
                        .build(scope)
                        .ok_or_else(|| {
                            ScriptError::javascript("async callback allocation failed")
                        })?;
                    arguments.push(callback.into());
                }
                let body =
                    v8::String::new_from_two_byte(scope, &request.body, v8::NewStringType::Normal)
                        .ok_or_else(|| ScriptError::javascript("script body allocation failed"))?;
                let mut source = v8::script_compiler::Source::new(body, None);
                let function = v8::script_compiler::compile_function(
                    scope,
                    &mut source,
                    &[],
                    &[],
                    v8::script_compiler::CompileOptions::NoCompileOptions,
                    v8::script_compiler::NoCacheReason::NoReason,
                )
                .ok_or_else(|| ScriptError::javascript("script body compilation failed"))?;
                let receiver = context.global(scope);
                if let Some(value) = function.call(scope, receiver.into(), &arguments) {
                    if request.asynchronous {
                        let settled = scope
                            .get_slot::<WebDriverScripts>()
                            .and_then(|registry| registry.pending.get(&id))
                            .is_some_and(|entry| entry.settled);
                        if !settled {
                            webdriver_await_async_return(scope, id, value)?;
                        }
                    } else {
                        webdriver_settle(scope, id, value, false);
                    }
                } else if let Some(exception) = scope.exception() {
                    // A callback that already settled wins over a subsequent throw.
                    scope.reset();
                    webdriver_settle(scope, id, exception, true);
                } else {
                    return Err(ScriptError::javascript("script execution terminated"));
                }
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if !webdriver_live(scope, &environment, authority_epoch) {
                    return Err(ScriptError::retired());
                }
                Ok(id)
            })();
            if watchdog.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                Err(ScriptError::timeout())
            } else if scope.has_caught() {
                let message = scope
                    .exception()
                    .map(|exception| v8::Exception::create_message(scope, exception).get(scope))
                    .filter(|message| message.length() <= 4096)
                    .map(|message| message.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "script threw an exception".into());
                Err(ScriptError::javascript(message))
            } else {
                result
            }
        };
        if result.is_err() {
            self.cancel_webdriver_script(id);
        }
        result
    }

    pub fn poll_webdriver_script(&mut self, id: ScriptId) -> Result<ScriptPoll, ScriptError> {
        let paused = self
            .isolate
            .get_slot::<Arc<WatchdogState>>()
            .map(|watchdog| watchdog.paused_total())
            .unwrap_or_default();
        let (environment, epoch, deadline, promise) = self
            .isolate
            .get_slot::<WebDriverScripts>()
            .and_then(|registry| registry.pending.get(&id))
            .map(|script| {
                (
                    script.environment,
                    script.authority_epoch,
                    script.deadline.and_then(|deadline| {
                        deadline.checked_add(paused.saturating_sub(script.initial_paused))
                    }),
                    script.promise.clone(),
                )
            })
            .ok_or_else(ScriptError::retired)?;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            self.cancel_webdriver_script(id);
            return Err(ScriptError::timeout());
        }
        let Some(context) = self.contexts.get(&environment.realm).cloned() else {
            self.cancel_webdriver_script(id);
            return Err(ScriptError::retired());
        };
        let wall_limit = deadline
            .map(|deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(self.execution_wall_limit)
            })
            .unwrap_or(self.execution_wall_limit);
        let watchdog = ExecutionDeadline::arm(&self.isolate, wall_limit);
        let result = {
            v8::scope!(let handles, &mut self.isolate);
            let context = v8::Local::new(handles, &context);
            let scope = &mut v8::ContextScope::new(handles, context);
            v8::tc_scope!(let scope, scope);
            let result = (|| {
                if !webdriver_live(scope, &environment, epoch) {
                    return Err(ScriptError::retired());
                }
                scope.perform_microtask_checkpoint();
                let promise = v8::Local::new(scope, &promise);
                let result = match promise.state() {
                    v8::PromiseState::Pending => Ok(ScriptPoll::Pending),
                    v8::PromiseState::Rejected => {
                        let exception = promise.result(scope);
                        let message = v8::Exception::create_message(scope, exception).get(scope);
                        Err(ScriptError::javascript(if message.length() <= 4096 {
                            message.to_rust_string_lossy(scope)
                        } else {
                            "script promise rejected".into()
                        }))
                    }
                    v8::PromiseState::Fulfilled => {
                        let value = promise.result(scope);
                        let result = webdriver_to_json(
                            scope,
                            value,
                            &mut ValueBudget::new(),
                            0,
                            &mut Vec::new(),
                        )?;
                        Ok(ScriptPoll::Complete(result))
                    }
                };
                if !webdriver_live(scope, &environment, epoch) {
                    return Err(ScriptError::retired());
                }
                result
            })();
            if watchdog.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                Err(ScriptError::timeout())
            } else if scope.has_caught() {
                Err(ScriptError::javascript(
                    "script result conversion or promise callback threw",
                ))
            } else {
                result
            }
        };
        if !matches!(result, Ok(ScriptPoll::Pending)) {
            self.cancel_webdriver_script(id);
        }
        result
    }

    pub fn cancel_webdriver_script(&mut self, id: ScriptId) {
        if let Some(registry) = self.isolate.get_slot_mut::<WebDriverScripts>() {
            registry.pending.remove(&id);
        }
    }
}

#[cfg(test)]
mod webdriver_script_tests {
    use super::*;
    use serde_json::json;

    fn request(body: &str, asynchronous: bool) -> ScriptRequest {
        ScriptRequest {
            body: body.encode_utf16().collect(),
            args: vec![],
            asynchronous,
            timeout_ms: Some(1000),
        }
    }
    fn run(host: &mut RawV8Host, body: &str) -> Result<ScriptValue, ScriptError> {
        let id = host.begin_webdriver_script(request(body, false))?;
        match host.poll_webdriver_script(id)? {
            ScriptPoll::Complete(value) => Ok(value),
            ScriptPoll::Pending => panic!("synchronous fixture unexpectedly pending"),
        }
    }
    #[test]
    fn accessibility_properties_use_native_checked_state_and_stable_scoped_tree_ids() {
        let mut page=crate::executable::RustExecutableDocument::load("<body><input id='check' type='checkbox' checked><div id='tabs' role='tablist'><div id='one' role='tab' aria-label='one'></div><div role='tab' aria-label='two'></div></div></body>").unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let check = run(&mut host, "return document.getElementById('check')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let first = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: check.clone(),
                    accessibility_node: false,
                },
            )
            .unwrap();
        assert_eq!(
            first.get("checked"),
            Some(&ScriptValue::from(json!("true")))
        );
        host.execute("document.getElementById('check').checked=false")
            .unwrap();
        let second = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: check,
                    accessibility_node: false,
                },
            )
            .unwrap();
        assert_eq!(
            second.get("checked"),
            Some(&ScriptValue::from(json!("false")))
        );
        assert_eq!(first.get("accessibilityId"), second.get("accessibilityId"));
        let tabs = run(&mut host, "return document.getElementById('tabs')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let tree = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: tabs,
                    accessibility_node: false,
                },
            )
            .unwrap();
        let Some(ScriptValue::Array(children)) = tree.get("children") else {
            panic!("AX child identities")
        };
        assert_eq!(children.len(), 2);
        let child = children[0].as_string().unwrap();
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: child.clone(),
                    accessibility_node: true,
                },
            )
            .unwrap();
        assert_eq!(
            properties.get("role"),
            Some(&ScriptValue::from(json!("tab")))
        );
        assert_eq!(
            properties.get("label"),
            Some(&ScriptValue::from(json!("one")))
        );
        assert_eq!(properties.get("parent"), tree.get("accessibilityId"));
        host.execute("document.getElementById('one').remove()")
            .unwrap();
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: child,
                    accessibility_node: true
                }
            )
            .unwrap_err()
            .code,
            "stale element reference"
        );
    }

    #[test]
    fn disconnected_accessibility_role_is_empty_without_relaxing_action_lifetimes() {
        let mut page = crate::executable::RustExecutableDocument::load("<body></body>").unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference=run(&mut host,"globalThis.detached=document.createElement('input');detached.type='checkbox';detached.switch=true;document.body.append(detached);return detached").unwrap().get(ELEMENT_KEY).unwrap().as_string().unwrap();
        assert!(host.webdriver_action_element(realm, &reference).is_ok());
        host.execute("detached.remove()").unwrap();
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference: reference.clone(),
                    role: true
                }
            )
            .unwrap(),
            ScriptValue::String(Vec::new())
        );
        assert!(host.webdriver_action_element(realm, &reference).is_err());
        host.execute("document.body.append(detached)").unwrap();
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference,
                    role: true
                }
            )
            .unwrap(),
            ScriptValue::from(json!("switch"))
        );
    }

    #[test]
    fn manual_label_pseudo_display_and_own_control_exclusion_reach_native_name() {
        let mut page=crate::executable::RustExecutableDocument::load(r#"<style>label::before{content:'This';display:block}label::after{content:'.'}</style><label for=target>is a test</label><input id=target>"#).unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference = run(&mut host, "return document.getElementById('target')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference,
                    role: false
                }
            )
            .unwrap()
            .as_string()
            .unwrap(),
            "This is a test."
        );
    }

    #[test]
    fn computed_name_uses_shared_layout_display_and_case_without_mutating_dom_text() {
        let mut page=crate::executable::RustExecutableDocument::load("<style id=s>#b{ display:block; text-transform:uppercase }#b span{display:block}</style><button id=b><span>one</span><span>two</span></button>").unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference = run(&mut host, "return document.getElementById('b')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let read = |host: &mut RawV8Host| {
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference: reference.clone(),
                    role: false,
                },
            )
            .unwrap()
            .as_string()
            .unwrap()
        };
        assert_eq!(read(&mut host), "ONE TWO");
        host.execute(
            "document.getElementById('s').textContent='#b{display:block}#b span{display:inline}'",
        )
        .unwrap();
        assert_eq!(read(&mut host), "onetwo");
        assert_eq!(
            run(&mut host, "return document.getElementById('b').textContent")
                .unwrap()
                .as_string()
                .unwrap(),
            "onetwo"
        );
    }

    #[test]
    fn computed_accessibility_name_keeps_flex_item_blockification() {
        let mut page=crate::executable::RustExecutableDocument::load("<style>#b{display:flex}#b span{display:inline}</style><button id=b><span>one</span><span>two</span></button>").unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference = run(&mut host, "return document.getElementById('b')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference,
                    role: false
                }
            )
            .unwrap()
            .as_string()
            .unwrap(),
            "one two"
        );
        assert_eq!(
            run(&mut host, "return document.getElementById('b').textContent")
                .unwrap()
                .as_string()
                .unwrap(),
            "onetwo"
        );
    }

    #[test]
    fn computed_accessibility_name_includes_native_marker_alternative_and_live_changes() {
        let mut page=crate::executable::RustExecutableDocument::load(r#"<style id=s>li::marker{content:'Icon ' / 'Bullet'}</style><ul><li id=item>Body</li></ul><button id=b aria-labelledby=item></button>"#).unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference = run(&mut host, "return document.getElementById('b')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let read = |host: &mut RawV8Host| {
            host.webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference: reference.clone(),
                    role: false,
                },
            )
            .unwrap()
            .as_string()
            .unwrap()
        };
        assert_eq!(read(&mut host), "Bullet Body");
        host.execute(
            "document.getElementById('s').textContent=\"li::marker{content:'Icon ' / ''}\"",
        )
        .unwrap();
        assert_eq!(read(&mut host), "Body");
        host.execute("document.getElementById('s').textContent=\"li::marker{content:'Two '}\"")
            .unwrap();
        assert_eq!(read(&mut host), "Two Body");
    }

    #[test]
    fn accessibility_properties_follow_native_switch_reflection_and_role_defaults() {
        let mut page = crate::executable::RustExecutableDocument::load(
            "<input id=control type=checkbox><button id=button></button>",
        )
        .unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let reference = run(&mut host, "return document.getElementById('control')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        host.execute("document.getElementById('control').switch=true")
            .unwrap();
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: reference.clone(),
                    accessibility_node: false,
                },
            )
            .unwrap();
        assert_eq!(
            properties.get("role"),
            Some(&ScriptValue::from(json!("switch")))
        );
        assert_eq!(
            properties.get("checked"),
            Some(&ScriptValue::from(json!("false")))
        );
        host.execute("document.getElementById('control').switch=false")
            .unwrap();
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference,
                    accessibility_node: false,
                },
            )
            .unwrap();
        assert_eq!(
            properties.get("role"),
            Some(&ScriptValue::from(json!("checkbox")))
        );
        let reference = run(&mut host, "return document.getElementById('button')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference,
                    accessibility_node: false,
                },
            )
            .unwrap();
        assert_eq!(
            properties.get("pressed"),
            Some(&ScriptValue::from(json!("undefined")))
        );
    }

    #[test]
    fn accessibility_properties_share_fresh_layout_visibility_and_generated_names() {
        let mut page=crate::executable::RustExecutableDocument::load("<style id=s>.gone{display:none}#b::before{content:'Prefix '}</style><div id=g role=group><button id=b>Core</button><button id=h class=gone>Hidden</button></div>").unwrap();
        let mut host = page.take_agent_host().unwrap();
        let realm = host.active_realm;
        let mut reference = |id: &str| {
            run(
                &mut host,
                &format!("return document.getElementById('{id}')"),
            )
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap()
        };
        let group = reference("g");
        let button = reference("b");
        let hidden = reference("h");
        let tree = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: group.clone(),
                    accessibility_node: false,
                },
            )
            .unwrap();
        let Some(ScriptValue::Array(children)) = tree.get("children") else {
            panic!("AX children")
        };
        assert_eq!(children.len(), 1);
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: hidden.clone(),
                    accessibility_node: false
                }
            )
            .unwrap_err()
            .code,
            "no such element"
        );
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: button.clone(),
                    accessibility_node: false,
                },
            )
            .unwrap();
        let label = properties.get("label").unwrap().as_string().unwrap();
        assert!(label.contains("Prefix") && label.contains("Core"));
        let computed = host
            .webdriver_element_command(
                realm,
                ElementCommand::Accessibility {
                    reference: button.clone(),
                    role: false,
                },
            )
            .unwrap()
            .as_string()
            .unwrap();
        assert_eq!(computed, label);
        host.execute("document.getElementById('s').textContent=\".gone{display:block}#b::before{content:'After '}\"").unwrap();
        let properties = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: button,
                    accessibility_node: false,
                },
            )
            .unwrap();
        let label = properties.get("label").unwrap().as_string().unwrap();
        assert!(label.contains("After") && !label.contains("Prefix"));
        let tree = host
            .webdriver_element_command(
                realm,
                ElementCommand::AccessibilityProperties {
                    reference: group,
                    accessibility_node: false,
                },
            )
            .unwrap();
        let Some(ScriptValue::Array(children)) = tree.get("children") else {
            panic!("updated AX children")
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn native_contenteditable_focus_collapses_at_end_but_preserves_existing_selection() {
        let mut host = RawV8Host::new();
        let dom=std::rc::Rc::new(std::cell::RefCell::new(oryn_dom::html::parse("<body><button id=other></button><div id=editor contenteditable><b>ab</b><i>cd</i></div></body>").into_runtime().unwrap()));
        host.install_dom(dom, 1).unwrap();
        let reference = run(&mut host, "return document.getElementById('editor')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        let realm = host.active_realm;
        host.execute("document.getElementById('other').focus()")
            .unwrap();
        host.webdriver_element_command(
            realm,
            ElementCommand::Focus {
                reference: reference.clone(),
            },
        )
        .unwrap();
        assert_eq!(run(&mut host,"const e=document.getElementById('editor'),s=getSelection();return [document.activeElement===e,s.anchorNode===e,s.anchorOffset,s.isCollapsed]").unwrap(),ScriptValue::from(json!([true,true,2,true])));
        host.execute("getSelection().setBaseAndExtent(document.getElementById('editor').firstChild.firstChild,0,document.getElementById('editor').firstChild.firstChild,1)").unwrap();
        host.webdriver_element_command(realm, ElementCommand::Focus { reference })
            .unwrap();
        assert_eq!(
            run(
                &mut host,
                "const s=getSelection();return [s.anchorOffset,s.focusOffset,s.toString()]"
            )
            .unwrap(),
            ScriptValue::from(json!([0, 1, "a"]))
        );
    }

    #[test]
    fn automatic_observer_environment_does_not_force_unobserved_layout() {
        struct Provider(std::rc::Rc<std::cell::Cell<usize>>);
        impl oryn_js::v8_bindings::BindingLayoutProvider for Provider {
            fn compute_layout(&self, _: &std::collections::BTreeMap<oryn_dom::NodeId, std::collections::BTreeSet<String>>) -> Result<oryn_js::v8_bindings::BindingLayout, String> {
                self.0.set(self.0.get() + 1);
                Ok(oryn_js::v8_bindings::BindingLayout { viewport: Some((100., 100.)), ..Default::default() })
            }
        }
        let mut host = RawV8Host::new();
        let dom = std::rc::Rc::new(std::cell::RefCell::new(
            oryn_dom::html::parse("<body><main></main></body>").into_runtime().unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        let realm = host.active_realm;
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        host.binding_states[&realm].set_layout_provider(std::rc::Rc::new(Provider(calls.clone())));
        assert!(!host.binding_states[&realm].has_native_geometry_observers());
        for _ in 0..10 {
            assert!(host.sample_native_observer_environment_in_scope(realm).is_some());
        }
        assert_eq!(calls.get(), 0, "automatic sampling must not lay out unobserved owners");
        assert!(host.sample_native_paint_revision_in_scope(realm).is_some());
        assert_eq!(calls.get(), 1, "explicit capture must still acquire geometry");
        for constructor in ["ResizeObserver", "IntersectionObserver"] {
            host.execute(&format!("globalThis.geometryObserver=new {constructor}(()=>{{}});geometryObserver.observe(document.querySelector('main'))")).unwrap();
            assert!(host.binding_states[&realm].has_native_geometry_observers());
            let before = calls.get();
            assert!(host.sample_native_observer_environment_in_scope(realm).is_some());
            assert!(calls.get() > before, "registered observers retain all-realm scoped sampling");
        }
    }

    #[test]
    fn native_focus_reopened_document_preserves_replacement_stream_selection() {
        let mut host = RawV8Host::new();
        let dom = std::rc::Rc::new(std::cell::RefCell::new(
            oryn_dom::html::parse("<body><div id=editor contenteditable>abcd</div></body>")
                .into_runtime()
                .unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        let reference = run(&mut host, "return document.getElementById('editor')")
            .unwrap()
            .get(ELEMENT_KEY)
            .unwrap()
            .as_string()
            .unwrap();
        host.execute("const editor=document.getElementById('editor');editor.addEventListener('focus',()=>{document.open();document.write('<body></body>');document.close();document.body.append(editor);editor.focus();getSelection().setBaseAndExtent(editor.firstChild,0,editor.firstChild,1);},{once:true});").unwrap();
        let realm = host.active_realm;
        assert!(
            host.webdriver_element_command(realm, ElementCommand::Focus { reference })
                .is_err()
        );
        assert_eq!(
            run(
                &mut host,
                "const s=getSelection();return [s.anchorOffset,s.focusOffset,s.toString()]"
            )
            .unwrap(),
            ScriptValue::from(json!([0, 1, "a"]))
        );
    }

    #[test]
    fn native_element_file_selection_retains_real_bytes_and_dispatches_trusted_events() {
        let mut host = RawV8Host::new();
        let dom = std::rc::Rc::new(std::cell::RefCell::new(
            oryn_dom::html::parse("<body><input id='target' type='file' multiple></body>")
                .into_runtime()
                .unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        host.execute("window.events=[];const input=document.getElementById('target');input.oninput=e=>events.push([e.type,e.isTrusted,input.files.length]);input.onchange=e=>events.push([e.type,e.isTrusted,input.files.length])").unwrap();
        let value = run(&mut host, "return document.getElementById('target')").unwrap();
        let reference = value.get(super::ELEMENT_KEY).unwrap().as_string().unwrap();
        let realm = host.active_realm;
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::FileInput {
                    reference: reference.clone()
                }
            )
            .unwrap(),
            ScriptValue::Bool(true)
        );
        for name in ["first.txt", "second.txt"] {
            host.webdriver_element_command(
                realm,
                ElementCommand::SetFiles {
                    reference: reference.clone(),
                    files: vec![crate::upload::WebDriverUpload {
                        file: crate::upload::UploadFile {
                            filename: name.into(),
                            media_type: "text/plain".into(),
                            bytes: b"payload".to_vec(),
                        },
                        last_modified: 1234,
                    }],
                },
            )
            .unwrap();
        }
        assert_eq!(run(&mut host,"return Array.from(document.getElementById('target').files,x=>[x.name,x.size,x.type])").unwrap(),ScriptValue::from(json!([["first.txt",7,"text/plain"],["second.txt",7,"text/plain"]])));
        assert_eq!(
            run(&mut host, "return events").unwrap(),
            ScriptValue::from(json!([
                ["input", true, 1],
                ["change", true, 1],
                ["input", true, 2],
                ["change", true, 2]
            ]))
        );
        host.execute("document.getElementById('target').removeAttribute('multiple')")
            .unwrap();
        let file = crate::upload::WebDriverUpload {
            file: crate::upload::UploadFile {
                filename: "bad.txt".into(),
                media_type: "text/plain".into(),
                bytes: vec![1],
            },
            last_modified: 1234,
        };
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::SetFiles {
                    reference,
                    files: vec![file.clone(), file]
                }
            )
            .unwrap_err()
            .code,
            "invalid argument"
        );
        assert_eq!(
            run(
                &mut host,
                "return document.getElementById('target').files.length"
            )
            .unwrap(),
            ScriptValue::from(json!(2))
        );
    }

    #[test]
    fn element_queries_use_native_selectors_and_revoke_detached_references() {
        let mut host = RawV8Host::new();
        let dom = std::rc::Rc::new(std::cell::RefCell::new(
            oryn_dom::html::parse("<body><button id='target'>go</button></body>")
                .into_runtime()
                .unwrap(),
        ));
        host.install_dom(dom, 1).unwrap();
        host.execute("document.querySelectorAll=()=>{throw Error('poisoned')};Element.prototype.querySelectorAll=()=>[]").unwrap();
        let realm = host.active_realm;
        let result = host
            .webdriver_element_command(
                realm,
                ElementCommand::Find {
                    root: None,
                    selectors: vec!["#target".into()],
                },
            )
            .unwrap();
        let ScriptValue::Array(values) = result else {
            panic!("element array expected")
        };
        assert_eq!(values.len(), 1);
        let Some(ScriptValue::String(reference)) =
            values[0].get("element-6066-11e4-a52e-4f735466cecf")
        else {
            panic!("native element reference expected")
        };
        let reference = String::from_utf16(reference).unwrap();
        let repeated = host
            .webdriver_element_command(
                realm,
                ElementCommand::Find {
                    root: None,
                    selectors: vec!["button".into()],
                },
            )
            .unwrap();
        assert_eq!(repeated, ScriptValue::Array(values));
        host.execute("document.getElementById('target').remove()")
            .unwrap();
        assert_eq!(
            host.webdriver_element_command(realm, ElementCommand::Focus { reference })
                .unwrap_err()
                .code,
            "stale element reference"
        );
        assert_eq!(
            host.webdriver_element_command(
                realm,
                ElementCommand::Find {
                    root: None,
                    selectors: vec!["[".into()]
                }
            )
            .unwrap_err()
            .code,
            "invalid selector"
        );
    }

    #[test]
    fn intrinsic_poisoning_does_not_replace_native_clone_or_callback() {
        let mut host = RawV8Host::new();
        host.execute("JSON.stringify=()=>{throw 1};JSON.parse=()=>{throw 2};Promise=()=>{throw 3};globalThis.__oryn_webdriver_result='forged'").unwrap();
        assert_eq!(
            run(
                &mut host,
                "return {a:[1,true,null],text:'ok'}; // trailing comment"
            )
            .unwrap(),
            ScriptValue::from(json!({"a":[1,true,null],"text":"ok"}))
        );
        let id = host
            .begin_webdriver_script(request("arguments[arguments.length-1]({real:1});", true))
            .unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(json!({"real":1}).into())
        );
    }
    #[test]
    fn native_arguments_preserve_proto_as_data_and_aliases_are_not_cycles() {
        let mut host = RawV8Host::new();
        let mut input = request(
            "return [arguments[0].__proto__,Object.getPrototypeOf(arguments[0])===Object.prototype]",
            false,
        );
        input.args = vec![json!({"__proto__":{"safe":true}}).into()];
        let id = host.begin_webdriver_script(input).unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(json!([{"safe":true},true]).into())
        );
        assert_eq!(
            run(&mut host, "let a={n:1};return [a,a]").unwrap(),
            ScriptValue::from(json!([{"n":1},{"n":1}]))
        );
        assert!(run(&mut host, "let a={};a.self=a;return a").is_err());
    }
    #[test]
    fn async_first_settlement_and_native_thenable_adoption() {
        let mut host = RawV8Host::new();
        let id = host
            .begin_webdriver_script(request(
                "let done=arguments[0];done(1);done(2);throw new Error('late');",
                true,
            ))
            .unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(json!(1).into())
        );
        let id = host
            .begin_webdriver_script(request("return {then(resolve){resolve(7)}}", true))
            .unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(json!(7).into())
        );
    }
    #[test]
    fn cancellation_revokes_late_callback_and_zero_deadline_expires() {
        let mut host = RawV8Host::new();
        let id = host
            .begin_webdriver_script(request("globalThis.savedDone=arguments[0]", true))
            .unwrap();
        assert_eq!(host.poll_webdriver_script(id).unwrap(), ScriptPoll::Pending);
        host.cancel_webdriver_script(id);
        host.execute("savedDone('late')").unwrap();
        assert!(host.poll_webdriver_script(id).is_err());
        let mut input = request("return 1", false);
        input.timeout_ms = Some(0);
        let result = host
            .begin_webdriver_script(input)
            .and_then(|id| host.poll_webdriver_script(id));
        assert_eq!(result.unwrap_err().code, "script timeout");
    }
    #[test]
    fn getters_to_json_and_exception_capture_follow_native_clone_rules() {
        let mut host = RawV8Host::new();
        assert_eq!(
            run(&mut host, "return {toJSON(){return {n:2}}}").unwrap(),
            ScriptValue::from(json!({"n":2}))
        );
        assert_eq!(
            run(&mut host, "return [undefined,NaN,Infinity]").unwrap(),
            ScriptValue::from(json!([null, null, null]))
        );
        for body in [
            "throw new Error('body')",
            "return {get a(){throw 1}}",
            "return {toJSON(){throw 1}}",
            "return 1n",
        ] {
            assert_eq!(run(&mut host, body).unwrap_err().code, "javascript error");
        }
    }
    #[test]
    fn utf16_arguments_keys_and_body_preserve_unpaired_surrogates() {
        let mut host = RawV8Host::new();
        let mut input = request("return arguments[0]", false);
        input.args = vec![ScriptValue::from_json_text(r#"{"\ud800":["\udfff","😀"]}"#).unwrap()];
        let expected = input.args[0].clone();
        let id = host.begin_webdriver_script(input).unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(expected)
        );
        let mut input = request("", false);
        input.body = "return '"
            .encode_utf16()
            .chain([0xd800])
            .chain("'".encode_utf16())
            .collect();
        let id = host.begin_webdriver_script(input).unwrap();
        assert_eq!(
            host.poll_webdriver_script(id).unwrap(),
            ScriptPoll::Complete(ScriptValue::String(vec![0xd800]))
        );
    }

    #[test]
    fn modal_wait_shifts_all_nested_execution_deadlines_once() {
        use oryn_js::prompts::PromptWaitObserver;
        let host = RawV8Host::new();
        let outer = ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30));
        let inner = ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30));
        let watchdog = host
            .isolate
            .get_slot::<Arc<WatchdogState>>()
            .unwrap()
            .clone();
        watchdog.begin_wait();
        watchdog.begin_wait();
        let before = {
            let mut schedule = watchdog.schedule.lock().unwrap();
            schedule.pause_started = Instant::now().checked_sub(Duration::from_secs(1));
            schedule
                .active
                .values()
                .map(|(deadline, _)| *deadline)
                .collect::<Vec<_>>()
        };
        watchdog.end_wait();
        assert_eq!(watchdog.schedule.lock().unwrap().pause_depth, 1);
        watchdog.end_wait();
        let schedule = watchdog.schedule.lock().unwrap();
        for ((deadline, _), before) in schedule.active.values().zip(before) {
            assert!(*deadline >= before + Duration::from_secs(1));
        }
        assert_eq!(schedule.pause_depth, 0);
        drop(schedule);
        assert!(!outer.expired());
        assert!(!inner.expired());
    }

    #[test]
    fn host_watchdog_owns_infinite_script_termination() {
        let mut host = RawV8Host::new_with_wall_limit(Duration::from_millis(20));
        assert_eq!(
            run(&mut host, "while(true){}").unwrap_err().code,
            "script timeout"
        );
        assert_eq!(
            run(&mut host, "return 3").unwrap(),
            ScriptValue::from(json!(3))
        );
    }
}

impl RawV8Host {
    pub(crate) fn capture_css_images(
        &self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        urls: &std::collections::BTreeSet<String>,
        owner: &crate::executable::NativeCssCapture,
    ) -> Result<Vec<oryn_common::v4::PaintImageResource>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        owner
            .retain(state, urls)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        Ok(state.captured_css_images())
    }
    pub(crate) fn capture_visual_viewport(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
    ) -> Result<Option<oryn_common::visual_viewport::Geometry>, ScriptError> {
        let Some(state) = self.binding_states.get(&realm) else {
            return Ok(None);
        };
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        if !self.realm_execution_live(realm) {
            return Err(ScriptError::retired());
        }
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let geometry = state
            .native_visual_viewport_geometry_in_scope(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        if !geometry.is_valid() {
            return Err(ScriptError::javascript("invalid native visual viewport"));
        }
        Ok(Some(geometry))
    }
    pub(crate) fn admit_paint_storage(
        &mut self, realm: RealmId, generation: u64, epoch: u64, bytes: usize,
    ) -> Result<oryn_js::drag_image::NativeCustody, ScriptError> {
        let state = self.binding_states.get(&realm).cloned().ok_or_else(ScriptError::retired)?;
        if state.execution_retired() || state.document_generation() != generation
            || state.document_stream_epoch() != epoch || !self.realm_execution_live(realm)
        { return Err(ScriptError::retired()); }
        let context = self.contexts.get(&realm).cloned().ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let allocation = state.capture_retained_allocation(scope,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        allocation.reserve(bytes).map(oryn_js::drag_image::NativeCustody)
            .map_err(ScriptError::javascript)
    }
    pub(crate) fn capture_paint_images(
        &mut self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
    ) -> Result<Vec<oryn_js::drag_image::Resource>, ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        if state.execution_retired()
            || state.document_generation() != generation
            || state.document_stream_epoch() != epoch
        {
            return Err(ScriptError::retired());
        }
        let state = state.clone();
        if !self.realm_execution_live(realm) { return Err(ScriptError::retired()); }
        let context = self.contexts.get(&realm).cloned().ok_or_else(ScriptError::retired)?;
        v8::scope!(let handles, &mut self.isolate);
        let context = v8::Local::new(handles, &context);
        let scope = &mut v8::ContextScope::new(handles, context);
        let _allocation_scope = state.enter_retained_allocation(scope)
            .map_err(|error| ScriptError::javascript(error.to_string()))?;
        oryn_js::v8_bindings::snapshot_native_paint_resources(&state, state.document_root())
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }
    pub(crate) fn take_drag_feedback(&self) -> Vec<oryn_js::drag_image::Feedback> {
        self.active_root_realm_ids()
            .iter()
            .filter_map(|realm| self.binding_states.get(realm))
            .flat_map(|state| oryn_js::v8_bindings::take_native_drag_feedback(state))
            .collect()
    }
}

impl RawV8Host {
    pub(crate) fn install_paint_capture_inventory(
        &self,
        realm: RealmId,
        generation: u64,
        epoch: u64,
        snapshot: &oryn_common::v4::PaintSnapshot,
    ) -> Result<(), ScriptError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(ScriptError::retired)?;
        state
            .install_native_paint_capture_inventory(generation, epoch, snapshot)
            .map_err(|error| ScriptError::javascript(error.to_string()))
    }
}

impl RawV8Host {
    /// Rendering/capture sampling is deliberately separate from cached poll
    /// comparisons. Every owner is entered in its exact live context; the
    /// scoped helper synchronizes its ancestors before its own admission.
    pub(crate) fn sample_native_paint_revision_in_scope(&mut self, realm: RealmId) -> Option<u64> {
        let owners = self
            .binding_states
            .iter()
            .filter(|(id, state)| !state.execution_retired() && self.realm_execution_live(**id))
            .map(|(id, state)| (*id, state.clone()))
            .collect::<Vec<_>>();
        for (id, state) in owners {
            if !self.realm_execution_live(id)
                || state.execution_retired()
                || !self
                    .binding_states
                    .get(&id)
                    .is_some_and(|current| Rc::ptr_eq(current, &state))
            {
                return None;
            }
            let lifetime = (
                state.document_root(),
                state.document_generation(),
                state.document_stream_epoch(),
                state.document_authority_epoch(),
            );
            let context = self.contexts.get(&id)?.clone();
            {
                v8::scope!(let handles, &mut self.isolate);
                let context = v8::Local::new(handles, &context);
                let scope = &mut v8::ContextScope::new(handles, context);
                state.native_paint_revision_in_scope(scope).ok()?;
            }
            if !self.realm_execution_live(id)
                || !self.contexts.contains_key(&id)
                || state.execution_retired()
                || lifetime
                    != (
                        state.document_root(),
                        state.document_generation(),
                        state.document_stream_epoch(),
                        state.document_authority_epoch(),
                    )
                || !self
                    .binding_states
                    .get(&id)
                    .is_some_and(|current| Rc::ptr_eq(current, &state))
            {
                return None;
            }
        }
        self.native_paint_revision(realm)
    }

    /// Automatic presentation already acquired the root and presented child
    /// layouts. Only geometry observers need the additional all-realm pass:
    /// otherwise it would lay out unpresented, still-parsing hidden documents
    /// on every rendering opportunity. Explicit capture keeps its scoped pass.
    pub(crate) fn sample_native_observer_environment_in_scope(&mut self, realm: RealmId) -> Option<u64> {
        if self.binding_states.iter().any(|(id, state)| {
            !state.execution_retired()
                && self.realm_execution_live(*id)
                && state.has_native_geometry_observers()
        }) {
            self.sample_native_paint_revision_in_scope(realm)
        } else {
            self.native_paint_revision(realm)
        }
    }

    pub(crate) fn native_paint_revision(&self, realm: RealmId) -> Option<u64> {
        let mut environments = Vec::new();
        for (id, state) in &self.binding_states {
            if !state.execution_retired() && self.realm_execution_live(*id) {
                environments.push((
                    u64::from(id.0),
                    state.document_generation(),
                    state.document_stream_epoch(),
                    state.native_paint_revision().ok()?,
                ));
            }
        }
        self.binding_states
            .get(&realm)?
            .native_page_paint_revision(&environments)
            .ok()
    }
}

/// The native binding portion of frame input admission is shared with callers
/// already inside the owning isolate. Host enumeration additionally verifies
/// these exact Rcs against its current bindings and live context membership.
struct NativePointerFrameBinding {
    parent: Rc<V8BindingState>,
    child: Rc<V8BindingState>,
    identity: oryn_js::v8_bindings::NativeFrameInputIdentity,
}

fn native_pointer_frame_binding(
    entry: &FrameLifetime,
    root: RealmId,
    environments: &AgentEnvironments,
) -> Option<NativePointerFrameBinding> {
    let environment = entry.environment.get();
    if entry.root_realm != root
        || entry.closed.get()
        || !environments.is_live(&entry.parent_environment)
        || !environments.is_live(&environment)
        || entry.connection_generation == u64::MAX
    {
        return None;
    }
    let parent = entry.parent.upgrade()?;
    let child = entry.child.borrow().upgrade()?;
    if parent.execution_retired()
        || child.execution_retired()
        || parent.document_generation() != entry.parent_environment.document_generation
        || child.document_generation() != environment.document_generation
    {
        return None;
    }
    {
        let dom = parent.dom();
        let dom = dom.borrow();
        if !dom.is_connected(entry.frame).unwrap_or(false)
            || !dom.embedded_content_allowed(entry.frame)
            || dom.node_document(entry.frame).ok().flatten() != Some(parent.document_root())
            || dom.frame_context_generation(entry.frame) != Some(entry.connection_generation)
        {
            return None;
        }
    }
    let identity = parent.native_input_frame_identity(entry.frame)?;
    if identity.realm != environment.realm
        || identity.document != child.document_root()
        || identity.generation != child.document_generation()
        || identity.stream_epoch != child.document_stream_epoch()
        || identity.authority_epoch != child.document_authority_epoch()
    {
        return None;
    }
    Some(NativePointerFrameBinding {
        parent,
        child,
        identity,
    })
}

impl RawV8Host {
    /// Native input membership includes insertion-created initial Documents,
    /// without consuming their independent driver/parser handoff.
    pub(crate) fn native_pointer_frame_snapshots(
        &self,
        root: RealmId,
    ) -> Vec<(super::webdriver_script::AutomationRealmSnapshot, bool)> {
        use super::webdriver_script::AutomationRealmSnapshot;
        let Some(registry) = self.isolate.get_slot::<FrameLifetimes>() else {
            return Vec::new();
        };
        let Some(environments) = self.isolate.get_slot::<AgentEnvironments>() else {
            return Vec::new();
        };
        registry
            .live
            .values()
            .filter_map(|entry| {
                let binding = native_pointer_frame_binding(entry, root, environments)?;
                let environment = entry.environment.get();
                let parent = self.binding_states.get(&entry.parent_environment.realm)?;
                let child = self.binding_states.get(&environment.realm)?;
                if !Rc::ptr_eq(&binding.parent, parent)
                    || !Rc::ptr_eq(&binding.child, child)
                    || !self.contexts.contains_key(&entry.parent_environment.realm)
                    || !self.contexts.contains_key(&environment.realm)
                {
                    return None;
                }
                let identity = binding.identity;
                Some((
                    AutomationRealmSnapshot {
                        realm: identity.realm,
                        parent_realm: Some(entry.parent_environment.realm),
                        frame: Some(entry.frame),
                        frame_token: Some(entry.message_token.clone()),
                        frame_generation: Some(entry.connection_generation),
                        document_root: identity.document,
                        document_generation: identity.generation,
                        stream_epoch: identity.stream_epoch,
                        document_authority_epoch: identity.authority_epoch,
                        url: self
                            .automation_document_url(identity.realm)
                            .unwrap_or_default(),
                        live: true,
                    },
                    entry.initial_input_document.get()
                        == Some((identity.document, identity.generation)),
                ))
            })
            .collect()
    }

    /// A bounded, numeric failure explanation; no author text or capability
    /// tokens are copied into a failed input response.
    pub(crate) fn native_pointer_frame_diagnostic(&self, parent: RealmId, frame: NodeId) -> String {
        let connection = self.automation_frame_generation(parent, frame);
        let Some(registry) = self.isolate.get_slot::<FrameLifetimes>() else {
            return format!("registry=absent connection={connection:?}");
        };
        let mut entries = registry
            .live
            .values()
            .filter(|entry| entry.parent_environment.realm == parent && entry.frame == frame);
        let Some(entry) = entries.next() else {
            return format!("lifetime=absent connection={connection:?}");
        };
        format!(
            "closed={} connection={connection:?} expected={} child={} child_live={} initial={} additional={}",
            entry.closed.get(),
            entry.connection_generation,
            entry.environment.get().realm.0,
            entry
                .child
                .borrow()
                .upgrade()
                .is_some_and(|child| !child.execution_retired()),
            entry.initial_input_document.get().is_some(),
            entries.next().is_some(),
        )
    }
}

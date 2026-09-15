//! Window posted-message tasks. Serialization and origin authority are native;
//! author replacements of MessageEvent or dispatchEvent cannot intercept delivery.
use super::*;
use crate::{origin::Origin, realm::RealmDescriptor};

pub(super) struct Queue {
    origin: Origin,
    pub(super) tasks: VecDeque<Message>,
    bytes: usize,
}

impl Queue {
    pub(super) fn new(context: v8::Local<v8::Context>) -> Self {
        Self {
            origin: context
                .get_slot::<RealmDescriptor>()
                .map(|realm| realm.origin.clone())
                .unwrap_or_else(Origin::opaque),
            tasks: VecDeque::new(),
            bytes: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.tasks.clear();
        self.bytes = 0;
    }
}

pub(super) struct Message {
    payload: structured_clone::SerializedValue,
    target_origin: Option<Origin>,
    source_origin: String,
    source: Option<v8::Global<v8::Object>>,
}

pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(post)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Window.postMessage"))?;
    let key = v8::String::new(scope, "postMessage")
        .ok_or(V8BindingError::Allocation("postMessage name"))?;
    function.set_name(key);
    context
        .global(scope)
        .create_data_property(scope, key.into(), function.into());
    Ok(())
}

fn post(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let receiver = args.this();
    let Some(context) = receiver.get_creation_context(scope) else {
        return;
    };
    let Some(target) = context.get_slot::<V8BindingState>().filter(|state| {
        state.binding_global == BindingGlobal::Window
            && global_events::is_global(scope, state, receiver)
    }) else {
        throw_type_error(scope, "postMessage requires a Window receiver");
        return;
    };
    if args.length() == 0 {
        throw_type_error(scope, "postMessage requires a message");
        return;
    }
    let source = crate::v8_context_bridge::incumbent_slot::<V8BindingState>(scope)
        .or_else(|| scope.get_current_context().get_slot::<V8BindingState>());
    let result = enqueue(
        scope,
        &target,
        source.as_ref(),
        args.get(0),
        args.get(1),
        args.get(2),
    );
    if let Err(error) = result {
        throw_binding_error(scope, &error);
    }
}

fn enqueue(
    scope: &mut v8::PinScope,
    target: &Rc<V8BindingState>,
    source: Option<&Rc<V8BindingState>>,
    value: v8::Local<v8::Value>,
    options: v8::Local<v8::Value>,
    transfer: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    // Web IDL selects the dictionary overload for object/null/undefined.
    let dictionary = options.is_object() || options.is_null_or_undefined();
    let origin_value = if dictionary {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(options) {
            let key = v8::String::new(scope, "targetOrigin").unwrap();
            let Some(value) = object.get(scope, key.into()) else {
                return Ok(());
            };
            value
        } else {
            v8::undefined(scope).into()
        }
    } else {
        options
    };
    let origin_text = if origin_value.is_undefined() && dictionary {
        "/".into()
    } else {
        let Some(text) = origin_value.to_string(scope) else {
            return Ok(());
        };
        text.to_rust_string_lossy(scope)
    };
    let transfer_options = if dictionary {
        options
    } else {
        let object = v8::Object::new(scope);
        let key = v8::String::new(scope, "transfer").unwrap();
        object.create_data_property(scope, key.into(), transfer);
        object.into()
    };
    let Some(transfers) = structured_clone::transfer_list(scope, transfer_options) else {
        return Ok(());
    };
    let source_origin = source
        .map(|state| state.window_messages.borrow().origin.clone())
        .unwrap_or_else(Origin::opaque);
    let target_origin = match origin_text.as_str() {
        "*" => None,
        "/" => Some(source_origin.clone()),
        text => {
            if Url::parse(text).is_err() {
                let error = dom_exception_value(scope, "SyntaxError")?;
                scope.throw_exception(error);
                return Ok(());
            }
            Some(Origin::from_url(text))
        }
    };
    let Some(payload) = structured_clone::serialize(scope, value, &transfers) else {
        return Ok(());
    };
    // Serialization and transfer side effects occur even for an inactive target.
    if target.execution_retired() {
        return Ok(());
    }
    let source = source
        .filter(|state| state.binding_global == BindingGlobal::Window)
        .and_then(|state| state.global_object.to_local(scope))
        .map(|window| v8::Global::new(scope, window));
    let bytes = payload.retained_bytes();
    let mut queue = target.window_messages.borrow_mut();
    if queue.tasks.len() >= 1024 || bytes > (32 * 1024 * 1024usize).saturating_sub(queue.bytes) {
        drop(queue);
        let error = dom_exception_value(scope, "QuotaExceededError")?;
        scope.throw_exception(error);
        return Ok(());
    }
    queue.bytes += bytes;
    queue.tasks.push_back(Message {
        payload,
        target_origin,
        source_origin: source_origin.serialize(),
        source,
    });
    Ok(())
}

/// Run one posted-message task in its destination realm. The host performs a
/// microtask checkpoint afterwards, just as for other native event tasks.
pub fn deliver(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<bool, V8BindingError> {
    let message = {
        let mut queue = state.window_messages.borrow_mut();
        let Some(message) = queue.tasks.pop_front() else {
            return Ok(false);
        };
        queue.bytes -= message.payload.retained_bytes();
        if state.execution_retired()
            || message
                .target_origin
                .as_ref()
                .is_some_and(|origin| origin != &queue.origin)
        {
            return Ok(true);
        }
        message
    };
    let Some((data, ports)) = structured_clone::deserialize_with_ports(scope, message.payload)
    else {
        return Err(V8BindingError::Allocation("Window message deserialization"));
    };
    let event = message_events::delivery(scope, state, data, &ports)?;
    let fields = event
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Window message fields"))?;
    let origin = v8::String::new(scope, &message.source_origin)
        .ok_or(V8BindingError::Allocation("Window message origin"))?;
    fields.set_index(scope, 1, origin.into());
    let source = message
        .source
        .as_ref()
        .map(|source| v8::Local::new(scope, source).into())
        .unwrap_or_else(|| v8::null(scope).into());
    fields.set_index(scope, 3, source);
    global_events::dispatch(scope, state, event)?;
    Ok(true)
}

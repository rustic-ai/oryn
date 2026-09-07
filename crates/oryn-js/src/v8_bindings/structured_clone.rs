//! Realm-independent structured serialization for native message delivery.
use super::*;
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

#[derive(Clone)]
enum HostData {
    Blob(BlobRecord),
    DomException {
        name: String,
        message: String,
        code: i32,
    },
}

/// Serialized bytes and host data contain no source-realm JavaScript handles.
/// Only the native serializer can construct this record.
pub struct SerializedValue {
    bytes: Vec<u8>,
    hosts: Vec<HostData>,
    buffers: Vec<v8::SharedRef<v8::BackingStore>>,
}

fn clone_error(scope: &mut v8::PinScope) {
    if let Ok(error) = dom_exception_value(scope, "DataCloneError") {
        scope.throw_exception(error);
    }
}

struct Serializer {
    hosts: Rc<RefCell<Vec<HostData>>>,
}
impl v8::ValueSerializerImpl for Serializer {
    fn throw_data_clone_error<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        _: v8::Local<'s, v8::String>,
    ) {
        clone_error(scope);
    }
    fn write_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
        writer: &dyn ValueSerializerHelper,
    ) -> Option<bool> {
        let data = (|| {
            if object.internal_field_count() == 0
                || !object
                    .get_internal_field(scope, 0)
                    .and_then(|v| v8::Local::<v8::Value>::try_from(v).ok())
                    .is_some_and(|v| v.is_number())
            {
                return None;
            }
            let state = binding_state_for_receiver(scope, object)?;
            let id = stateful_id(scope, object).ok()?;
            if !state
                .platform_wrappers
                .borrow()
                .get(&id)
                .and_then(|w| w.to_local(scope))
                .is_some_and(|w| w.strict_equals(object.into()))
            {
                return None;
            }
            let records = state.platform_objects.borrow();
            match records.get(&id)? {
                PlatformObjectRecord::Blob(blob) => Some(HostData::Blob(blob.clone())),
                PlatformObjectRecord::DomException {
                    name,
                    message,
                    code,
                } => Some(HostData::DomException {
                    name: name.clone(),
                    message: message.clone(),
                    code: *code,
                }),
                _ => None,
            }
        })();
        let Some(data) = data else {
            clone_error(scope);
            return None;
        };
        let mut hosts = self.hosts.borrow_mut();
        let Ok(id) = u32::try_from(hosts.len()) else {
            clone_error(scope);
            return None;
        };
        hosts.push(data);
        writer.write_uint32(id);
        Some(true)
    }
    fn get_shared_array_buffer_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        _: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        // Oryn has no cross-origin-isolated agent cluster yet.
        clone_error(scope);
        None
    }
    fn get_wasm_module_transfer_id(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        _: v8::Local<v8::WasmModuleObject>,
    ) -> Option<u32> {
        clone_error(scope);
        None
    }
}
struct Deserializer<'a> {
    hosts: &'a [HostData],
}
impl v8::ValueDeserializerImpl for Deserializer<'_> {
    fn read_host_object<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        reader: &dyn ValueDeserializerHelper,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let mut id = 0;
        if !reader.read_uint32(&mut id) {
            return None;
        }
        let data = self.hosts.get(id as usize)?;
        let state = scope.get_current_context().get_slot::<V8BindingState>()?;
        let (name, record) = match data {
            HostData::Blob(blob) => (
                if blob.file.is_some() { "File" } else { "Blob" },
                PlatformObjectRecord::Blob(blob.clone()),
            ),
            HostData::DomException {
                name,
                message,
                code,
            } => (
                "DOMException",
                PlatformObjectRecord::DomException {
                    name: name.clone(),
                    message: message.clone(),
                    code: *code,
                },
            ),
        };
        let id = register_platform_object(&state, record);
        new_platform_object(scope, &state, name, id).ok()
    }
}

/// Validate transfers before reading the value, then detach only after successful
/// serialization. Recheck at transfer time because getters can detach buffers.
pub fn serialize(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    transfers: &[v8::Local<v8::Value>],
) -> Option<SerializedValue> {
    let mut buffers: Vec<v8::Local<v8::ArrayBuffer>> = Vec::new();
    for value in transfers {
        let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(*value) else {
            clone_error(scope);
            return None;
        };
        if buffers.iter().any(|b| b.strict_equals(*value)) {
            clone_error(scope);
            return None;
        }
        buffers.push(buffer);
    }
    let hosts = Rc::new(RefCell::new(Vec::new()));
    let writer = v8::ValueSerializer::new(
        scope,
        Box::new(Serializer {
            hosts: hosts.clone(),
        }),
    );
    writer.write_header();
    for (id, buffer) in buffers.iter().enumerate() {
        writer.transfer_array_buffer(id as u32, *buffer);
    }
    if writer.write_value(scope.get_current_context(), value) != Some(true) {
        return None;
    }
    let mut stores = Vec::new();
    for buffer in buffers {
        if buffer.was_detached() || !buffer.is_detachable() {
            clone_error(scope);
            return None;
        }
        let store = buffer.get_backing_store();
        if buffer.detach(None) != Some(true) {
            clone_error(scope);
            return None;
        }
        stores.push(store);
    }
    let bytes = writer.release();
    drop(writer);
    let hosts = hosts.borrow().clone();
    Some(SerializedValue {
        bytes,
        hosts,
        buffers: stores,
    })
}

pub fn deserialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: SerializedValue,
) -> Option<v8::Local<'s, v8::Value>> {
    let reader = v8::ValueDeserializer::new(
        scope,
        Box::new(Deserializer {
            hosts: &value.hosts,
        }),
        &value.bytes,
    );
    if reader.read_header(scope.get_current_context()) != Some(true) {
        return None;
    }
    for (id, store) in value.buffers.iter().enumerate() {
        let buffer = v8::ArrayBuffer::with_backing_store(scope, store);
        reader.transfer_array_buffer(id as u32, buffer);
    }
    reader.read_value(scope.get_current_context())
}

fn transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<v8::Value>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    if options.is_null_or_undefined() {
        return Some(Vec::new());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(options) else {
        throw_type_error(scope, "StructuredSerializeOptions must be a dictionary");
        return None;
    };
    let key = v8::String::new(scope, "transfer")?;
    let value = options.get(scope, key.into())?;
    if value.is_undefined() {
        return Some(Vec::new());
    }
    let Ok(value) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(scope, "transfer requires a sequence of objects");
        return None;
    };
    let key = v8::Symbol::get_iterator(scope);
    let method = value.get(scope, key.into())?;
    let Ok(method) = v8::Local::<v8::Function>::try_from(method) else {
        throw_type_error(scope, "transfer requires an iterable");
        return None;
    };
    let iterator = method.call(scope, value.into(), &[])?;
    let Ok(iterator) = v8::Local::<v8::Object>::try_from(iterator) else {
        throw_type_error(scope, "iterator must return an object");
        return None;
    };
    let key = v8::String::new(scope, "next")?;
    let next = iterator.get(scope, key.into())?;
    let Ok(next) = v8::Local::<v8::Function>::try_from(next) else {
        throw_type_error(scope, "iterator next must be callable");
        return None;
    };
    let mut result = Vec::new();
    loop {
        let step = next.call(scope, iterator.into(), &[])?;
        let Ok(step) = v8::Local::<v8::Object>::try_from(step) else {
            throw_type_error(scope, "iterator result must be an object");
            return None;
        };
        let key = v8::String::new(scope, "done")?;
        if step.get(scope, key.into())?.boolean_value(scope) {
            break;
        }
        let key = v8::String::new(scope, "value")?;
        let item = step.get(scope, key.into())?;
        if !item.is_object() {
            {
                v8::tc_scope!(let tc, scope);
                let key = v8::String::new(tc, "return")?;
                if let Some(close) = iterator
                    .get(tc, key.into())
                    .and_then(|v| v8::Local::<v8::Function>::try_from(v).ok())
                {
                    close.call(tc, iterator.into(), &[]);
                }
            }
            throw_type_error(scope, "transfer sequence contains a non-object");
            return None;
        }
        result.push(item);
    }
    Some(result)
}
fn clone_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let receiver = args.this();
    let Some(context) = receiver.get_creation_context(scope) else {
        throw_type_error(scope, "structuredClone requires a global receiver");
        return;
    };
    let Some(state) = context.get_slot::<V8BindingState>() else {
        throw_type_error(scope, "structuredClone requires a global receiver");
        return;
    };
    if !global_events::is_global(scope, &state, receiver) {
        throw_type_error(scope, "structuredClone requires a global receiver");
        return;
    }
    if !global_events::check_window_access(scope, &state, receiver) {
        return;
    }
    if args.length() == 0 {
        throw_type_error(scope, "structuredClone requires a value");
        return;
    }
    if let Some(transfers) = transfer_list(scope, args.get(1))
        && let Some(data) = serialize(scope, args.get(0), &transfers)
    {
        let scope = &mut v8::ContextScope::new(scope, context);
        if let Some(result) = deserialize(scope, data) {
            rv.set(result);
        }
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(clone_callback)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("structuredClone"))?;
    let name = v8::String::new(scope, "structuredClone")
        .ok_or(V8BindingError::Allocation("structuredClone name"))?;
    function.set_name(name);
    context
        .global(scope)
        .set(scope, name.into(), function.into());
    Ok(())
}

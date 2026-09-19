//! Native PCM AudioData snapshots, independent of author buffers and realms.
use super::*;
use crate::message_wire::AudioDataRecord as Data;
pub(super) struct Record {
    pub(super) data: Option<Data>,
    timestamp: i64,
}
pub(super) type Backing = Rc<RefCell<Record>>;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "AudioData".into(),
        member: name.into(),
    }
}
pub(super) fn identify(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<Backing> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let state = binding_state_for_receiver(scope, object)?;
    let id = stateful_id(scope, object).ok()?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value))
    {
        return None;
    }
    let records = state.platform_objects.borrow();
    match records.get(&id) {
        Some(PlatformObjectRecord::AudioData(backing)) => Some(backing.clone()),
        _ => None,
    }
}
pub(super) fn allocate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    data: Data,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !data.valid() {
        return Err(error("InvalidStateError"));
    }
    let timestamp = data.timestamp;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::AudioData(Rc::new(RefCell::new(Record {
            data: Some(data),
            timestamp,
        }))),
    );
    new_platform_object(scope, state, "AudioData", id)
}
pub(super) fn buffer(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(v8::SharedRef<v8::BackingStore>, usize, usize), V8BindingError> {
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let buffer = view.buffer(scope).ok_or_else(|| error("TypeError"))?;
        if buffer.was_detached() {
            return Err(error("TypeError"));
        }
        return Ok((
            buffer.get_backing_store(),
            view.byte_offset(),
            view.byte_length(),
        ));
    }
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        if buffer.was_detached() {
            return Err(error("TypeError"));
        }
        return Ok((buffer.get_backing_store(), 0, buffer.byte_length()));
    }
    if let Ok(buffer) = v8::Local::<v8::SharedArrayBuffer>::try_from(value) {
        return Ok((buffer.get_backing_store(), 0, buffer.byte_length()));
    }
    Err(error("TypeError"))
}
fn uint(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<u32, V8BindingError> {
    let n = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !n.is_finite() || n.trunc() < 0. || n.trunc() > u32::MAX as f64 {
        return Err(error("TypeError"));
    }
    Ok(n.trunc() as u32)
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let init = args.get(0);
    let mut values = BTreeMap::new();
    let mut transfers = Vec::new();
    for key in [
        "data",
        "format",
        "numberOfChannels",
        "numberOfFrames",
        "sampleRate",
        "timestamp",
        "transfer",
    ] {
        let value = checked_dictionary_member(scope, init, key)?;
        if key == "transfer" {
            if let Some(value) = value {
                transfers = structured_clone::post_transfer_list(scope, value)
                    .ok_or(V8BindingError::JavaScriptException)?;
            }
            continue;
        }
        let value = value.ok_or_else(|| error("TypeError"))?;
        let value = match key {
            "data" => {
                if !value.is_array_buffer() && !value.is_array_buffer_view() {
                    return Err(error("TypeError"));
                }
                value
            }
            "format" => {
                let value = value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if Data::sample_bytes(&value.to_rust_string_lossy(scope)).is_none() {
                    return Err(error("TypeError"));
                }
                value.into()
            }
            "numberOfFrames" | "numberOfChannels" => {
                let value = uint(scope, value)?;
                v8::Integer::new_from_unsigned(scope, value).into()
            }
            _ => {
                let number = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !number.is_finite()
                    || (key == "timestamp"
                        && (number < i64::MIN as f64 || number >= i64::MAX as f64))
                    || (key == "sampleRate" && !(number as f32).is_finite())
                {
                    return Err(error("TypeError"));
                }
                v8::Number::new(scope, number).into()
            }
        };
        values.insert(key, value);
    }
    let format = values["format"]
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    let channels = uint(scope, values["numberOfChannels"])?;
    let frames = uint(scope, values["numberOfFrames"])?;
    let sample_rate = values["sampleRate"]
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)? as f32;
    let timestamp = values["timestamp"]
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !timestamp.is_finite() || timestamp < i64::MIN as f64 || timestamp >= i64::MAX as f64 {
        return Err(error("TypeError"));
    }
    let mut data = Data {
        format,
        sample_rate,
        frames,
        channels,
        timestamp: timestamp.trunc() as i64,
        bytes: Vec::new(),
    };
    let expected = data.expected_bytes().ok_or_else(|| error("TypeError"))?;
    if expected > crate::message_wire::MAX_BYTES {
        return Err(error("TypeError"));
    }
    let mut buffers = Vec::new();
    for (index, value) in transfers.iter().enumerate() {
        let buffer =
            v8::Local::<v8::ArrayBuffer>::try_from(*value).map_err(|_| error("TypeError"))?;
        if buffer.was_detached()
            || !buffer.is_detachable()
            || transfers[..index]
                .iter()
                .any(|other| other.strict_equals(*value))
        {
            return Err(error("DataCloneError"));
        }
        buffers.push(buffer);
    }
    let (store, offset, length) = buffer(scope, values["data"])?;
    if store.is_shared() || length < expected {
        return Err(error("TypeError"));
    }
    data.bytes = store
        .iter()
        .skip(offset)
        .take(expected)
        .map(Cell::get)
        .collect();
    if !data.valid() {
        return Err(error("TypeError"));
    }
    // All conversions and validations precede every native detachment.
    let timestamp = data.timestamp;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::AudioData(Rc::new(RefCell::new(Record {
            data: Some(data),
            timestamp,
        }))),
    );
    bind_platform_object(scope, state, id, args.this());
    for buffer in buffers {
        if buffer.detach(None) != Some(true) {
            return Err(error("DataCloneError"));
        }
    }
    Ok(())
}
struct CopyPlan {
    format: String,
    plane: u32,
    offset: u32,
    count: u32,
    bytes: usize,
}
fn plan(
    scope: &mut v8::PinScope,
    data: &Data,
    options: v8::Local<v8::Value>,
) -> Result<CopyPlan, V8BindingError> {
    let mut format = data.format.clone();
    let mut offset = 0;
    let mut count = None;
    let mut plane = None;
    for key in ["format", "frameCount", "frameOffset", "planeIndex"] {
        if let Some(value) = checked_dictionary_member(scope, options, key)? {
            match key {
                "format" => {
                    format = value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope)
                }
                "frameCount" => count = Some(uint(scope, value)?),
                "frameOffset" => offset = uint(scope, value)?,
                _ => plane = Some(uint(scope, value)?),
            }
        }
    }
    let plane = plane.ok_or_else(|| error("TypeError"))?;
    let sample = Data::sample_bytes(&format).ok_or_else(|| error("TypeError"))?;
    let planar = format.ends_with("-planar");
    if offset >= data.frames || plane >= if planar { data.channels } else { 1 } {
        return Err(error("RangeError"));
    }
    let count = count.unwrap_or(data.frames - offset);
    if count > data.frames - offset {
        return Err(error("RangeError"));
    }
    let bytes = (count as usize)
        .checked_mul(if planar { 1 } else { data.channels as usize })
        .and_then(|n| n.checked_mul(sample))
        .filter(|n| *n <= crate::message_wire::MAX_BYTES)
        .ok_or_else(|| error("RangeError"))?;
    Ok(CopyPlan {
        format,
        plane,
        offset,
        count,
        bytes,
    })
}
fn sample(data: &Data, frame: u32, channel: u32) -> f64 {
    let size = Data::sample_bytes(&data.format).unwrap();
    let index = if data.format.ends_with("-planar") {
        channel as usize * data.frames as usize + frame as usize
    } else {
        frame as usize * data.channels as usize + channel as usize
    };
    let bytes = &data.bytes[index * size..(index + 1) * size];
    match data.format.trim_end_matches("-planar") {
        "u8" => (bytes[0] as f64 - 128.) / 128.,
        "s16" => i16::from_le_bytes(bytes.try_into().unwrap()) as f64 / 32768.,
        "s32" => i32::from_le_bytes(bytes.try_into().unwrap()) as f64 / 2147483648.,
        _ => f32::from_le_bytes(bytes.try_into().unwrap()) as f64,
    }
}
fn copied(data: &Data, plan: &CopyPlan) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(plan.bytes);
    let output_size = Data::sample_bytes(&plan.format).unwrap();
    let same = data.format.trim_end_matches("-planar") == plan.format.trim_end_matches("-planar");
    for frame in plan.offset..plan.offset + plan.count {
        for channel in if plan.format.ends_with("-planar") {
            plan.plane..plan.plane + 1
        } else {
            0..data.channels
        } {
            if same {
                let index = if data.format.ends_with("-planar") {
                    channel as usize * data.frames as usize + frame as usize
                } else {
                    frame as usize * data.channels as usize + channel as usize
                };
                bytes
                    .extend_from_slice(&data.bytes[index * output_size..(index + 1) * output_size]);
                continue;
            }
            let value = sample(data, frame, channel);
            let value = if value.is_nan() {
                0.
            } else {
                value.clamp(-1., 1.)
            };
            match plan.format.trim_end_matches("-planar") {
                "u8" => bytes.push((value * 128. + 128.).round().clamp(0., 255.) as u8),
                "s16" => bytes.extend_from_slice(
                    &((value * 32768.)
                        .round()
                        .clamp(i16::MIN as f64, i16::MAX as f64) as i16)
                        .to_le_bytes(),
                ),
                "s32" => bytes.extend_from_slice(
                    &((value * 2147483648.)
                        .round()
                        .clamp(i32::MIN as f64, i32::MAX as f64) as i32)
                        .to_le_bytes(),
                ),
                _ => bytes.extend_from_slice(&(value as f32).to_le_bytes()),
            }
        }
    }
    bytes
}
fn member(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let Some(backing) = identify(scope, args.this().into()) else {
            return Err(error("TypeError"));
        };
        let code = args.data().int32_value(scope).unwrap_or(-1);
        if code == 9 {
            backing.borrow_mut().data.take();
            return Ok(None);
        }
        let data = backing.borrow().data.clone();
        if code >= 6 {
            let data = data.ok_or_else(|| error("InvalidStateError"))?;
            if code == 8 {
                let state = scope
                    .get_current_context()
                    .get_slot::<V8BindingState>()
                    .ok_or(V8BindingError::Allocation("audio realm"))?;
                return allocate(scope, &state, data).map(|value| Some(value.into()));
            }
            let plan = plan(scope, &data, args.get(if code == 6 { 0 } else { 1 }))?;
            if backing.borrow().data.is_none() {
                return Err(error("InvalidStateError"));
            }
            if code == 6 {
                return Ok(Some(v8::Number::new(scope, plan.bytes as f64).into()));
            }
            let (store, offset, length) = buffer(scope, args.get(0))?;
            if length < plan.bytes {
                return Err(error("RangeError"));
            }
            let bytes = copied(&data, &plan);
            for (cell, byte) in store.iter().skip(offset).zip(bytes) {
                cell.set(byte);
            }
            return Ok(None);
        }
        let value = match code {
            0 => {
                if let Some(data) = &data {
                    v8::String::new(scope, &data.format)
                        .ok_or(V8BindingError::Allocation("audio format"))?
                        .into()
                } else {
                    v8::null(scope).into()
                }
            }
            1 => v8::Number::new(
                scope,
                data.as_ref().map_or(0., |data| data.sample_rate as f64),
            )
            .into(),
            2 => v8::Integer::new_from_unsigned(scope, data.as_ref().map_or(0, |data| data.frames))
                .into(),
            3 => {
                v8::Integer::new_from_unsigned(scope, data.as_ref().map_or(0, |data| data.channels))
                    .into()
            }
            4 => v8::Number::new(
                scope,
                data.as_ref().map_or(0., |data| {
                    (data.frames as f64 / data.sample_rate as f64 * 1_000_000.).floor()
                }),
            )
            .into(),
            _ => v8::Number::new(scope, backing.borrow().timestamp as f64).into(),
        };
        Ok(Some(value))
    })();
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install_members(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    for (index, name) in [
        "format",
        "sampleRate",
        "numberOfFrames",
        "numberOfChannels",
        "duration",
        "timestamp",
        "allocationSize",
        "copyTo",
        "clone",
        "close",
    ]
    .iter()
    .enumerate()
    {
        let data = v8::Integer::new(scope, index as i32);
        let function = v8::Function::builder(member)
            .data(data.into())
            .length(if index == 6 {
                1
            } else if index == 7 {
                2
            } else {
                0
            })
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("audio member"))?;
        if index >= 6 {
            set_object_property(scope, prototype, name, function.into())?;
        } else {
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let key =
                v8::String::new(scope, name).ok_or(V8BindingError::Allocation("audio key"))?;
            if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
                return Err(V8BindingError::Allocation("audio getter"));
            }
        }
    }
    Ok(())
}

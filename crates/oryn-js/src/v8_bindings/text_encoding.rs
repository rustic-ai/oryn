//! Encoding Standard state and algorithms, shared by Window and worker bindings.
//! No author callback runs while a decoder or platform registry is borrowed.
use super::*;
use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding, REPLACEMENT};

pub(super) struct DecoderRecord {
    encoding: &'static Encoding,
    decoder: Decoder,
    fatal: bool,
    ignore_bom: bool,
    streaming: bool,
}

pub(super) fn collect(state: &V8BindingState) {
    let ids = std::mem::take(&mut *state.text_encoding_releases.borrow_mut());
    if !ids.is_empty() {
        let mut objects = state.platform_objects.borrow_mut();
        let mut wrappers = state.platform_wrappers.borrow_mut();
        for id in ids {
            objects.remove(&id);
            wrappers.remove(&id);
        }
    }
}

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Encoding".into(),
        member: name.into(),
    }
}

fn dictionary<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Object>>, V8BindingError> {
    if value.is_null_or_undefined() {
        Ok(None)
    } else {
        v8::Local::<v8::Object>::try_from(value)
            .map(Some)
            .map_err(|_| error("TypeError"))
    }
}

fn boolean_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: Option<v8::Local<'s, v8::Object>>,
    name: &str,
) -> Result<bool, V8BindingError> {
    event_init_member(scope, object, name)
        .map(|value| value.boolean_value(scope))
        .ok_or(V8BindingError::JavaScriptException)
}

impl DecoderRecord {
    fn new(encoding: &'static Encoding, fatal: bool, ignore_bom: bool) -> Self {
        Self {
            encoding,
            decoder: Self::decoder(encoding, ignore_bom),
            fatal,
            ignore_bom,
            streaming: false,
        }
    }

    fn decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
        if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        }
    }

    fn decode(&mut self, mut input: &[u8], stream: bool) -> Result<String, V8BindingError> {
        if !self.streaming {
            self.decoder = Self::decoder(self.encoding, self.ignore_bom);
        }
        self.streaming = stream;
        // An empty non-final chunk cannot advance the Encoding Standard's
        // input queue. encoding_rs 0.8.35 drops pending legacy lead bytes when
        // called with this input (upstream issue 126), so preserve the state.
        if stream && input.is_empty() {
            return Ok(String::new());
        }
        let mut output = String::new();
        // A fixed scratch buffer avoids reserving a worst-case expansion for
        // every input. Only the returned string grows with actual decoded text.
        let mut chunk = [0_u8; 8192];
        loop {
            let (done, read, written) = if self.fatal {
                let (result, read, written) = self
                    .decoder
                    .decode_to_utf8_without_replacement(input, &mut chunk, !stream);
                if matches!(result, DecoderResult::Malformed(..)) {
                    return Err(error("TypeError"));
                }
                (matches!(result, DecoderResult::InputEmpty), read, written)
            } else {
                let (result, read, written, _) =
                    self.decoder.decode_to_utf8(input, &mut chunk, !stream);
                (matches!(result, CoderResult::InputEmpty), read, written)
            };
            output.push_str(std::str::from_utf8(&chunk[..written]).expect("decoder emits UTF-8"));
            input = &input[read..];
            if done {
                return Ok(output);
            }
        }
    }
}

pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    name: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    collect(state);
    let record = if name == "TextEncoder" {
        PlatformObjectRecord::TextEncoder
    } else {
        let label = if args.get(0).is_undefined() {
            "utf-8".to_owned()
        } else {
            args.get(0)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
        };
        // WebIDL converts all arguments before running label validation.
        let options = dictionary(args.get(1))?;
        let fatal = boolean_member(scope, options, "fatal")?;
        let ignore_bom = boolean_member(scope, options, "ignoreBOM")?;
        let encoding = Encoding::for_label(label.as_bytes())
            .filter(|encoding| *encoding != REPLACEMENT)
            .ok_or_else(|| error("RangeError"))?;
        PlatformObjectRecord::TextDecoder(Box::new(DecoderRecord::new(encoding, fatal, ignore_bom)))
    };
    let id = register_platform_object(state, record);
    bind_platform_object(scope, state, id, args.this());
    let owner = Rc::downgrade(state);
    let weak = v8::Weak::with_finalizer(
        scope,
        args.this(),
        Box::new(move |_| {
            if let Some(state) = owner.upgrade() {
                // GC may run during an allocation with the platform registry
                // borrowed. Defer registry mutation to a normal host checkpoint.
                state.text_encoding_releases.borrow_mut().push(id);
            }
        }),
    );
    state.platform_wrappers.borrow_mut().insert(id, weak);
    Ok(())
}

pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match (&*platform_object(state, id)?, interface, member) {
        (PlatformObjectRecord::TextEncoder, "TextEncoder", "encoding") => v8_string(scope, "utf-8"),
        (PlatformObjectRecord::TextDecoder(record), "TextDecoder", "encoding") => {
            v8_string(scope, &record.encoding.name().to_ascii_lowercase())
        }
        (PlatformObjectRecord::TextDecoder(record), "TextDecoder", "fatal") => {
            Ok(v8::Boolean::new(scope, record.fatal).into())
        }
        (PlatformObjectRecord::TextDecoder(record), "TextDecoder", "ignoreBOM") => {
            Ok(v8::Boolean::new(scope, record.ignore_bom).into())
        }
        _ => Err(error("TypeError")),
    }
}

fn validate_input(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if value.is_undefined() {
        return Ok(());
    }
    let buffer = if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        view.buffer(scope).ok_or_else(|| error("TypeError"))?
    } else if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        buffer
    } else if let Ok(buffer) = v8::Local::<v8::SharedArrayBuffer>::try_from(value) {
        return if buffer.get_backing_store().is_resizable_by_user_javascript() {
            Err(error("TypeError"))
        } else {
            Ok(())
        };
    } else {
        return Err(error("TypeError"));
    };
    if buffer.was_detached() || buffer.get_backing_store().is_resizable_by_user_javascript() {
        return Err(error("TypeError"));
    }
    Ok(())
}

fn copy_input(value: v8::Local<v8::Value>) -> Vec<u8> {
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        view.copy_contents(&mut bytes);
        bytes
    } else if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        bytes_from_array_buffer(buffer)
    } else if let Ok(buffer) = v8::Local::<v8::SharedArrayBuffer>::try_from(value) {
        buffer.get_backing_store().iter().map(Cell::get).collect()
    } else {
        Vec::new()
    }
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    // Brand validation precedes conversions which can call back into this API.
    match (&*platform_object(state, id)?, interface) {
        (PlatformObjectRecord::TextEncoder, "TextEncoder")
        | (PlatformObjectRecord::TextDecoder(_), "TextDecoder") => {}
        _ => return Err(error("TypeError")),
    }
    if member == "decode" {
        validate_input(scope, args.get(0))?;
        let options = dictionary(args.get(1))?;
        let stream = boolean_member(scope, options, "stream")?;
        let bytes = copy_input(args.get(0));
        let output = {
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::TextDecoder(decoder) = &mut *record else {
                return Err(error("TypeError"));
            };
            decoder.decode(&bytes, stream)?
        };
        return v8_string(scope, &output).map(Some);
    }
    let input = if member == "encode" && args.get(0).is_undefined() {
        String::new()
    } else {
        args.get(0)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?
            .to_rust_string_lossy(scope)
    };
    if member == "encode" {
        let buffer = array_buffer_from_bytes(scope, input.as_bytes());
        let bytes = v8::Uint8Array::new(scope, buffer, 0, input.len())
            .ok_or(V8BindingError::Allocation("encoded bytes"))?;
        return Ok(Some(bytes.into()));
    }
    if member != "encodeInto" {
        return Err(error("TypeError"));
    }
    let destination =
        v8::Local::<v8::Uint8Array>::try_from(args.get(1)).map_err(|_| error("TypeError"))?;
    let buffer = destination
        .buffer(scope)
        .ok_or_else(|| error("TypeError"))?;
    let backing = buffer.get_backing_store();
    if backing.is_resizable_by_user_javascript() {
        return Err(error("TypeError"));
    }
    // A detached destination has zero available bytes; encodeInto performs no
    // writes and reports zero progress, unlike TextDecoder input conversion.
    let offset = destination.byte_offset();
    let mut read = 0;
    let mut written = 0;
    for scalar in input.chars() {
        let mut bytes = [0; 4];
        let bytes = scalar.encode_utf8(&mut bytes).as_bytes();
        if written + bytes.len() > destination.byte_length() {
            break;
        }
        for byte in bytes {
            backing[offset + written].set(*byte);
            written += 1;
        }
        read += scalar.len_utf16();
    }
    let result = v8::Object::new(scope);
    let read = v8::Number::new(scope, read as f64).into();
    set_object_property(scope, result, "read", read)?;
    let written = v8::Number::new(scope, written as f64).into();
    set_object_property(scope, result, "written", written)?;
    Ok(Some(result.into()))
}

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
    pending_input: Vec<u8>,
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
            pending_input: Vec::new(),
        }
    }

    fn decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
        if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        }
    }

    fn decode(&mut self, input: &[u8], stream: bool) -> Result<String, V8BindingError> {
        self.decode_input(input, stream, true)
    }

    fn decode_input(
        &mut self,
        input: &[u8],
        stream: bool,
        retain_after_error: bool,
    ) -> Result<String, V8BindingError> {
        if !self.streaming {
            self.decoder = Self::decoder(self.encoding, self.ignore_bom);
            self.pending_input.clear();
        }
        // Fatal streaming errors leave the unconsumed input queue intact. New
        // bytes are appended to that queue on the next call, not substituted for it.
        let mut queued = std::mem::take(&mut self.pending_input);
        let mut input = if queued.is_empty() {
            input
        } else {
            queued.extend_from_slice(input);
            queued.as_slice()
        };
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
                    if stream && retain_after_error {
                        self.pending_input.extend_from_slice(&input[read..]);
                    }
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
        PlatformObjectRecord::TextDecoder(Box::new(decoder_arguments(scope, args)?))
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
    allow_detached: bool,
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
    if (buffer.was_detached() && !allow_detached)
        || buffer.get_backing_store().is_resizable_by_user_javascript()
    {
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
        validate_input(scope, args.get(0), false)?;
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
        return v8_scalar_string(scope, &output).map(Some);
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

fn decoder_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<DecoderRecord, V8BindingError> {
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
    Ok(DecoderRecord::new(encoding, fatal, ignore_bom))
}

pub(super) struct EncodingStreamRecord {
    pub(super) readable_id: u64,
    pub(super) writable_id: u64,
    codec: StreamCodec,
}

enum StreamCodec {
    Encoder { pending_surrogate: Option<u16> },
    Decoder(Box<DecoderRecord>),
}

pub(super) fn construct_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    name: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    let codec = if name == "TextEncoderStream" {
        StreamCodec::Encoder {
            pending_surrogate: None,
        }
    } else {
        StreamCodec::Decoder(Box::new(decoder_arguments(scope, args)?))
    };
    let undefined = v8::undefined(scope).into();
    let (id, readable_id, writable_id) =
        transform_streams::create(scope, state, undefined, 1.0, 0.0)?;
    insert_platform_object(
        state,
        id,
        PlatformObjectRecord::EncodingStream(EncodingStreamRecord {
            readable_id,
            writable_id,
            codec,
        }),
    );
    if let PlatformObjectRecord::WritableStream(stream) =
        &mut *platform_object_mut(state, writable_id)?
        && let WritableKind::Transform { codec_id, .. } = &mut stream.kind
    {
        *codec_id = Some(id);
    }
    bind_platform_object(scope, state, id, args.this());
    writable_streams::start(scope, state, writable_id, None)
}

pub(super) fn get_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::EncodingStream(record) = &*record else {
        return Err(error("TypeError"));
    };
    if (interface == "TextEncoderStream") != matches!(record.codec, StreamCodec::Encoder { .. }) {
        return Err(error("TypeError"));
    }
    match member {
        "readable" => {
            Ok(new_platform_object(scope, state, "ReadableStream", record.readable_id)?.into())
        }
        "writable" => {
            Ok(new_platform_object(scope, state, "WritableStream", record.writable_id)?.into())
        }
        "encoding" => match &record.codec {
            StreamCodec::Encoder { .. } => v8_string(scope, "utf-8"),
            StreamCodec::Decoder(decoder) => {
                v8_string(scope, &decoder.encoding.name().to_ascii_lowercase())
            }
        },
        "fatal" => match &record.codec {
            StreamCodec::Decoder(decoder) => Ok(v8::Boolean::new(scope, decoder.fatal).into()),
            _ => Err(error("TypeError")),
        },
        "ignoreBOM" => match &record.codec {
            StreamCodec::Decoder(decoder) => Ok(v8::Boolean::new(scope, decoder.ignore_bom).into()),
            _ => Err(error("TypeError")),
        },
        _ => Err(error("TypeError")),
    }
}

pub(super) fn transform_chunk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    chunk: Option<v8::Local<'s, v8::Value>>,
) -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError> {
    let encoder = match &*platform_object(state, id)? {
        PlatformObjectRecord::EncodingStream(record) => {
            matches!(record.codec, StreamCodec::Encoder { .. })
        }
        _ => return Err(error("TypeError")),
    };
    // Conversion may invoke author code, so perform it without borrowing state.
    let (units, bytes) = if let Some(chunk) = chunk {
        if encoder {
            let string = chunk
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let mut units = vec![0u16; string.length()];
            string.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
            (units, Vec::new())
        } else {
            if chunk.is_undefined() {
                return Err(error("TypeError"));
            }
            validate_input(scope, chunk, true)?;
            (Vec::new(), copy_input(chunk))
        }
    } else {
        (Vec::new(), Vec::new())
    };
    let output = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::EncodingStream(record) = &mut *record else {
            return Err(error("TypeError"));
        };
        match &mut record.codec {
            // A transform error permanently errors both sides. Unlike a standalone
            // TextDecoder, this codec cannot resume, so do not retain unread bytes.
            StreamCodec::Decoder(decoder) => {
                decoder.decode_input(&bytes, chunk.is_some(), false)?
            }
            StreamCodec::Encoder { pending_surrogate } => {
                let mut units = units;
                if let Some(pending) = pending_surrogate.take() {
                    units.insert(0, pending);
                }
                if chunk.is_some() && units.last().is_some_and(|u| (0xd800..=0xdbff).contains(u)) {
                    *pending_surrogate = units.pop();
                }
                String::from_utf16_lossy(&units)
            }
        }
    };
    if output.is_empty() {
        return Ok(None);
    }
    if encoder {
        let buffer = array_buffer_from_bytes(scope, output.as_bytes());
        let bytes = v8::Uint8Array::new(scope, buffer, 0, output.len())
            .ok_or(V8BindingError::Allocation("encoded stream chunk"))?;
        Ok(Some(bytes.into()))
    } else {
        v8_scalar_string(scope, &output).map(Some)
    }
}

#[cfg(test)]
mod decoder_queue_tests {
    use super::*;

    #[test]
    fn decoders_and_blob_text_preserve_literal_dom_storage_prefix() {
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
        let source=v8::String::new(scope,r#"(async()=>{
            const literal='\u{f0000}oryn-wtf16:[55296]',bytes=new TextEncoder().encode(literal);
            if(new TextDecoder().decode(bytes)!==literal)throw Error('decoder changed scalar text');
            if(await new Blob([bytes]).text()!==literal)throw Error('Blob text changed scalar text');
            const stream=new TextDecoderStream(),reader=stream.readable.getReader(),writer=stream.writable.getWriter();
            const pending=reader.read();await writer.write(bytes);
            if((await pending).value!==literal)throw Error('stream decoder changed scalar text');
            await writer.close();return 'ok';
        })()"#).unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        assert_eq!(promise.result(scope).to_rust_string_lossy(scope), "ok");
    }

    #[test]
    fn fatal_streaming_error_preserves_unconsumed_input_in_order() {
        let mut decoder = DecoderRecord::new(encoding_rs::UTF_8, true, false);
        assert!(decoder.decode(&[0xff, b'A'], true).is_err());
        assert_eq!(decoder.decode(b"B", true).unwrap(), "AB");
        assert_eq!(decoder.decode(&[], false).unwrap(), "");
    }

    #[test]
    fn consecutive_fatal_errors_preserve_only_the_remaining_queue() {
        let mut decoder = DecoderRecord::new(encoding_rs::UTF_8, true, false);
        assert!(decoder.decode(&[0xff, b'A', 0xff, b'B'], true).is_err());
        assert!(decoder.decode(b"C", true).is_err());
        // Output produced by the throwing call is discarded; the remaining
        // queue still contains B followed by the newly appended C.
        assert_eq!(decoder.decode(&[], true).unwrap(), "BC");
    }

    #[test]
    fn terminal_stream_errors_do_not_retain_the_unread_chunk() {
        let mut decoder = DecoderRecord::new(encoding_rs::UTF_8, true, false);
        let mut chunk = vec![b'A'; 1024 * 1024];
        chunk[0] = 0xff;
        assert!(decoder.decode_input(&chunk, true, false).is_err());
        assert!(decoder.pending_input.is_empty());
        assert_eq!(decoder.pending_input.capacity(), 0);
        assert!(decoder.fatal);
        assert_eq!(decoder.encoding.name(), "UTF-8");
    }

    #[test]
    fn nonstreaming_error_resets_the_decoder_queue_on_the_next_call() {
        let mut decoder = DecoderRecord::new(encoding_rs::UTF_8, true, false);
        assert!(decoder.decode(&[0xff, b'A'], false).is_err());
        assert_eq!(decoder.decode(b"B", true).unwrap(), "B");
        assert!(decoder.decode(&[0xff, b'C'], true).is_err());
        assert_eq!(decoder.decode(b"D", false).unwrap(), "CD");
        assert_eq!(decoder.decode(b"E", false).unwrap(), "E");
    }
}

#[cfg(test)]
mod stream_tests {
    use super::super::writable_streams::tests::run;

    #[test]
    fn encoding_streams_retain_brand_attributes_and_read_write_pressure() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const [Stream,input,expected] of [[TextEncoderStream,'A',65],[TextDecoderStream,new Uint8Array([65]),'A']]){
                const s=new Stream();check(s.encoding==='utf-8','encoding');check(s.readable===s.readable&&s.writable===s.writable,'same stream identity');
                check(s.readable instanceof ReadableStream&&s.writable instanceof WritableStream,'native streams');
                const w=s.writable.getWriter(),r=s.readable.getReader();let written=false;const write=w.write(input).then(()=>written=true);
                for(let i=0;i<8;i++)await Promise.resolve();check(!written,'write waits for read');const output=await r.read();await write;
                check(Stream===TextEncoderStream?output.value[0]===expected:output.value===expected,'output');await w.close();check((await r.read()).done,'closed');
            }
            const d=new TextDecoderStream('LaTiN1',{fatal:true,ignoreBOM:true});check(d.encoding==='windows-1252'&&d.fatal&&d.ignoreBOM,'decoder options');return 'ok';
        })()"#);
    }

    #[test]
    fn encoder_preserves_surrogates_across_chunks_and_flushes_trailing_high_surrogate() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            const s=new TextEncoderStream(),r=s.readable.getReader(),w=s.writable.getWriter();const output=[];
            const drain=(async()=>{while(true){const x=await r.read();if(x.done)return;output.push(...x.value)}})();
            for(const chunk of ['\ud83d','','\ude00','\ud800','A','\udfff','\ud801'])await w.write(chunk);
            await w.close();await drain;check(new TextDecoder().decode(new Uint8Array(output))==='😀�A��','surrogate boundaries');return 'ok';
        })()"#);
    }

    #[test]
    fn decoder_preserves_split_multibyte_bom_legacy_state_and_fatal_flush_errors() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const [label,chunks,expected] of [
                ['utf-8',[[239],[187],[],[191,240,159],[152],[128]],'😀'],
                ['shift_jis',[[130],[],[160]],'あ'],
                ['windows-1252',[[128]],'€']
            ]){
                const s=new TextDecoderStream(label),r=s.readable.getReader(),w=s.writable.getWriter();let output='';
                const drain=(async()=>{while(true){const x=await r.read();if(x.done)return;output+=x.value}})();
                for(const chunk of chunks)await w.write(new Uint8Array(chunk));await w.close();await drain;check(output===expected,'decoder state '+label);
            }
            const s=new TextDecoderStream('utf-8',{fatal:true}),r=s.readable.getReader(),w=s.writable.getWriter();
            const read=r.read();await w.write(new Uint8Array([240,159]));const close=w.close();
            for(const p of [read,close,r.closed,w.closed]){let e;try{await p}catch(x){e=x}check(e instanceof TypeError,'fatal incomplete input')}
            return 'ok';
        })()"#);
    }

    #[test]
    fn encoding_conversion_errors_and_invalid_decoder_chunks_error_both_sides() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};const reason={conversion:1};
            for(const [Stream,input] of [[TextEncoderStream,{toString(){throw reason}}],...[undefined,null,42,{},[65]].map(x=>[TextDecoderStream,x])]){
                const s=new Stream(),r=s.readable.getReader(),w=s.writable.getWriter();const write=w.write(input),read=r.read();
                for(const p of [write,read,r.closed,w.closed]){let e;try{await p}catch(x){e=x}check(Stream===TextEncoderStream?e===reason:e instanceof TypeError,'error identity')}
            }
            return 'ok';
        })()"#);
    }

    #[test]
    fn encoding_pipeline_accepts_native_stream_pairs_and_non_string_inputs() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            const source=new ReadableStream({start(c){for(const x of [undefined,null,3.14,{},['hi']])c.enqueue(x);c.close()}});
            const output=source.pipeThrough(new TextEncoderStream()).pipeThrough(new TextDecoderStream());
            const r=output.getReader();let result='';while(true){const x=await r.read();if(x.done)break;result+=x.value}
            check(result==='undefinednull3.14[object Object]hi','pipeline output: '+result);return 'ok';
        })()"#);
    }
    #[test]
    fn decoder_stream_treats_transferred_chunks_as_empty_and_getters_check_brand() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const view of [false,true]){
                const buffer=new ArrayBuffer(3),chunk=view?new Uint8Array(buffer):buffer;buffer.transfer();
                const s=new TextDecoderStream(),r=s.readable.getReader(),w=s.writable.getWriter();const read=r.read();
                await w.write(chunk);await w.close();check((await read).done,'detached chunk is empty');
            }
            for(const [Getter,wrong] of [[TextEncoderStream,new TextDecoderStream()],[TextDecoderStream,new TextEncoderStream()]]){
                for(const member of ['encoding','readable','writable']){
                    let e;try{Object.getOwnPropertyDescriptor(Getter.prototype,member).get.call(wrong)}catch(x){e=x}check(e instanceof TypeError,'getter brand');
                }
            }return 'ok';
        })()"#);
    }

    #[test]
    fn stream_objects_results_and_errors_use_constructor_realm_with_foreign_methods() {
        use super::*;
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let owner = v8::Context::new(scope, Default::default());
        let (stream, decoder, constructors) = {
            let scope = &mut v8::ContextScope::new(scope, owner);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, owner, dom, RealmId(1), 1).unwrap();
            let source = v8::String::new(scope, "[new TextEncoderStream(),new TextDecoderStream(),{Object,Uint8Array,TypeError,ReadableStream,WritableStream}]").unwrap();
            let result = v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let values = v8::Local::<v8::Array>::try_from(result).unwrap();
            (
                values.get_index(scope, 0).unwrap(),
                values.get_index(scope, 1).unwrap(),
                values.get_index(scope, 2).unwrap(),
            )
        };
        let foreign = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, foreign);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        install_generated_bindings(scope, foreign, dom, RealmId(2), 1).unwrap();
        let global = foreign.global(scope);
        for (name, value) in [
            ("stream", stream),
            ("decoder", decoder),
            ("owner", constructors),
        ] {
            let key = v8::String::new(scope, name).unwrap();
            global.set(scope, key.into(), value);
        }
        let source = v8::String::new(scope, r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            const readable=Object.getOwnPropertyDescriptor(TextEncoderStream.prototype,'readable').get.call(stream);
            const writable=Object.getOwnPropertyDescriptor(TextEncoderStream.prototype,'writable').get.call(stream);
            check(readable.constructor===owner.ReadableStream,'readable realm');check(writable.constructor===owner.WritableStream,'writable realm');
            const r=readable.getReader(),w=writable.getWriter();
            const read=ReadableStreamDefaultReader.prototype.read.call(r);const write=WritableStreamDefaultWriter.prototype.write.call(w,'A');
            const value=await read;await write;check(value.constructor===owner.Object,'result realm');check(value.value.constructor===owner.Uint8Array,'chunk realm');
            await w.close();
            const dr=decoder.readable.getReader(),dw=decoder.writable.getWriter();const pending=ReadableStreamDefaultReader.prototype.read.call(dr);
            const bad=WritableStreamDefaultWriter.prototype.write.call(dw,null);
            for(const p of [pending,bad]){let e;try{await p}catch(x){e=x}check(e instanceof owner.TypeError,'error realm')}
            return 'ok';
        })()"#).unwrap();
        v8::tc_scope!(let scope, scope);
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
        scope.perform_microtask_checkpoint();
        assert_ne!(promise.state(), v8::PromiseState::Pending);
        assert_eq!(
            promise.state(),
            v8::PromiseState::Fulfilled,
            "{}",
            promise.result(scope).to_rust_string_lossy(scope)
        );
    }
}

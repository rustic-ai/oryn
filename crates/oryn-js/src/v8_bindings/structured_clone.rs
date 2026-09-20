//! Realm-independent structured serialization for native message delivery.
use super::*;
use v8::{ValueDeserializerHelper, ValueSerializerHelper};

#[derive(Clone)]
enum HostData {
    ModuleOrigin([u8; 32]),
    UserActivation([bool; 2]),
    MediaSourceHandle(u32),
    TransferredArrayBuffer {
        index: u32,
        bytes: Vec<u8>,
    },
    ImageBitmap(DecodedImage),
    VideoFrame(video_frame::Frame),
    AudioData(crate::message_wire::AudioDataRecord),
    OffscreenCanvas(DecodedImage),
    CryptoKey(crate::crypto_key::Key),
    Stream(stream_transfer::Descriptor),
    Quad,
    FileList,
    Geometry(GeometryClone),
    ImageData {
        width: u32,
        height: u32,
    },
    Port(u32),
    Blob(BlobRecord),
    DomException {
        stack: Vec<u16>,
        name: String,
        message: String,
        code: i32,
    },
}

/// Queued serialized bytes and host data contain no source-realm JS handles.
/// An immediate wire conversion temporarily retains its uncommitted transfer
/// plan so native transport validation can fail without detaching author data.
#[cfg(unix)]
#[derive(Clone)]
struct SharedBacking {
    backing: v8::SharedRef<v8::BackingStore>,
    lease: std::sync::Arc<crate::v8_shared_memory::MappingLease>,
}
struct PendingTransfers {
    audios: Vec<audio_data::Backing>,
    media_handles: Vec<media_source::Transfer>,
    frames: Vec<video_frame::Backing>,
    offscreens: Vec<offscreen_canvas::Source>,
    bitmaps: Vec<image_bitmap::Backing>,
    buffers: Vec<v8::Global<v8::ArrayBuffer>>,
    ports: Vec<(Rc<V8BindingState>, u64, message_ports::Port)>,
}
pub struct SerializedValue {
    stream_plans: Vec<stream_transfer::Plan>,
    pending: Option<PendingTransfers>,
    #[cfg(unix)]
    shared: Vec<SharedBacking>,
    #[cfg(unix)]
    offers: Option<SharedOffers>,
    wasm_modules: Vec<Vec<u8>>,
    bytes: Vec<u8>,
    hosts: Vec<HostData>,
    buffers: Vec<v8::SharedRef<v8::BackingStore>>,
    ports: Vec<message_ports::Port>,
}

impl Drop for SerializedValue {
    fn drop(&mut self) {
        // Until native deserialization or export commits, queued transferred
        // endpoints are owned by this payload, including failed decodes.
        if self.pending.is_none() {
            for port in std::mem::take(&mut self.ports) {
                message_ports::discard_unreceived(&port);
            }
        }
    }
}
struct WirePortGuard {
    transport: Option<std::sync::Arc<dyn crate::message_wire::MessageTransport>>,
    tokens: Vec<crate::message_wire::PortToken>,
}
impl WirePortGuard {
    fn new(state: &V8BindingState, tokens: &[crate::message_wire::PortToken]) -> Self {
        Self {
            transport: state.message_transport.borrow().clone(),
            tokens: tokens.to_vec(),
        }
    }
    fn commit(&mut self) {
        self.tokens.clear();
    }
}
impl Drop for WirePortGuard {
    fn drop(&mut self) {
        if let Some(transport) = &self.transport {
            for token in self.tokens.drain(..) {
                let _ = transport
                    .request(crate::message_wire::MessageRequest::ClosePort { port: token });
            }
        }
    }
}

impl SerializedValue {
    pub(super) fn user_activation(&self) -> Option<[bool; 2]> {
        self.hosts.iter().find_map(|host| {
            if let HostData::UserActivation(value) = host {
                Some(*value)
            } else {
                None
            }
        })
    }
    pub(super) fn set_user_activation(&mut self, value: Option<[bool; 2]>) {
        if let Some(value) = value {
            self.hosts.push(HostData::UserActivation(value));
        }
    }
    /// Tee clones serialized owning chunks before either branch deserializes.
    /// Transfer-only endpoint objects cannot be duplicated by structured clone.
    pub(super) fn clone_owned_chunk(&self, scope: &mut v8::PinScope) -> Option<Self> {
        if !self.ports.is_empty() || self.pending.is_some() {
            return None;
        }
        let mut buffers = Vec::new();
        for backing in &self.buffers {
            let buffer = v8::ArrayBuffer::with_backing_store(scope, backing);
            let encoded = serialize(scope, buffer.into(), &[])?;
            let decoded = deserialize(scope, encoded)?;
            buffers.push(
                v8::Local::<v8::ArrayBuffer>::try_from(decoded)
                    .ok()?
                    .get_backing_store(),
            );
        }
        Some(Self {
            stream_plans: Vec::new(),
            pending: None,
            #[cfg(unix)]
            shared: self.shared.clone(),
            #[cfg(unix)]
            offers: None,
            wasm_modules: self.wasm_modules.clone(),
            bytes: self.bytes.clone(),
            hosts: self.hosts.clone(),
            buffers,
            ports: Vec::new(),
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.hosts
            .iter()
            .fold(
                self.wasm_modules
                    .iter()
                    .fold(self.bytes.len(), |size, module| {
                        size.saturating_add(module.len())
                    }),
                |total, host| {
                    total.saturating_add(match host {
                        HostData::ModuleOrigin(_) => 32,
                        HostData::UserActivation(_) => 2,
                        HostData::MediaSourceHandle(_) => 4,
                        HostData::TransferredArrayBuffer { bytes, .. } => bytes.len(),
                        HostData::Stream(_) => 2 * std::mem::size_of::<u32>(),
                        HostData::CryptoKey(key) => key.retained_bytes(),
                        HostData::Quad | HostData::FileList => 0,
                        HostData::Geometry(record) => {
                            record.bits.len() * std::mem::size_of::<u64>()
                                + std::mem::size_of::<GeometryClone>()
                        }
                        HostData::AudioData(data) => data
                            .bytes
                            .len()
                            .saturating_add(data.format.len())
                            .saturating_add(32),
                        HostData::VideoFrame(frame) => frame.bitmap.as_ref().map_or(64, |bitmap| {
                            bitmap
                                .rgba
                                .len()
                                .saturating_add(frame.planes.as_ref().map_or(0, Vec::len))
                                .saturating_add(64)
                        }),
                        HostData::ImageBitmap(bitmap) | HostData::OffscreenCanvas(bitmap) => {
                            bitmap.rgba.len().saturating_add(16)
                        }
                        HostData::ImageData { .. } => 2 * std::mem::size_of::<u32>(),
                        HostData::Port(_) => std::mem::size_of::<message_ports::Port>(),
                        HostData::Blob(blob) => blob.bytes.len(),
                        HostData::DomException {
                            name,
                            message,
                            stack,
                            ..
                        } => name
                            .len()
                            .saturating_add(message.len())
                            .saturating_add(stack.len().saturating_mul(2)),
                    })
                },
            )
            .saturating_add(self.buffers.iter().fold(0usize, |total, store| {
                total.saturating_add(store.byte_length())
            }))
    }
}

fn clone_error(scope: &mut v8::PinScope) {
    if let Ok(error) = dom_exception_value(scope, "DataCloneError") {
        scope.throw_exception(error);
    }
}

struct Serializer {
    audios: Vec<(audio_data::Backing, u32)>,
    media_handles: Vec<(u64, u64, u32)>,
    frames: Vec<(video_frame::Backing, u32)>,
    offscreens: Vec<(offscreen_canvas::Source, u32)>,
    bitmaps: Vec<(image_bitmap::Backing, u32)>,
    streams: Vec<(u64, u64, u32)>,
    for_storage: bool,
    #[cfg(unix)]
    shared: Rc<RefCell<Vec<SharedBacking>>>,
    wasm_modules: Rc<RefCell<Vec<Vec<u8>>>>,
    hosts: Rc<RefCell<Vec<HostData>>>,
    ports: Vec<(u64, u64)>,
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
        if let Some(source) = media_source::identify(scope, object.into()) {
            if let Some((_, _, index)) = self
                .media_handles
                .iter()
                .find(|(owner, id, _)| *owner == source.owner.instance_id && *id == source.id)
            {
                writer.write_uint32(*index);
                return Some(true);
            }
            clone_error(scope);
            return None;
        }
        if let Some(identity) = message_ports::transfer_identity(scope, object.into())
            && let Some(index) = self.ports.iter().position(|key| *key == identity)
        {
            let mut hosts = self.hosts.borrow_mut();
            let id = u32::try_from(hosts.len()).ok()?;
            hosts.push(HostData::Port(index as u32));
            writer.write_uint32(id);
            return Some(true);
        }
        if let Some(stream) = stream_transfer::identify(scope, object.into()) {
            let Some((_, _, index)) = self
                .streams
                .iter()
                .find(|(owner, id, _)| *owner == stream.owner.instance_id && *id == stream.id)
            else {
                clone_error(scope);
                return None;
            };
            writer.write_uint32(*index);
            return Some(true);
        }
        match file_lists::entries(scope, object) {
            Ok(Some(entries)) => {
                let Ok(id) = u32::try_from(self.hosts.borrow().len()) else {
                    clone_error(scope);
                    return None;
                };
                self.hosts.borrow_mut().push(HostData::FileList);
                writer.write_uint32(id);
                // The same serializer handles each File, preserving graph
                // identity with references outside the list.
                writer.write_uint32(u32::try_from(entries.len()).ok()?);
                for entry in entries {
                    if writer.write_value(scope.get_current_context(), entry) != Some(true) {
                        return None;
                    }
                }
                return Some(true);
            }
            Err(_) => {
                clone_error(scope);
                return None;
            }
            Ok(None) => {}
        }
        if let Some(points) = quad_points(scope, object) {
            let Ok(id) = u32::try_from(self.hosts.borrow().len()) else {
                clone_error(scope);
                return None;
            };
            self.hosts.borrow_mut().push(HostData::Quad);
            writer.write_uint32(id);
            for index in 0..4 {
                let point = points.get_index(scope, index)?;
                if writer.write_value(scope.get_current_context(), point) != Some(true) {
                    return None;
                }
            }
            return Some(true);
        }
        if let Some(audio) = audio_data::identify(scope, object.into()) {
            if let Some((_, index)) = self
                .audios
                .iter()
                .find(|(candidate, _)| Rc::ptr_eq(candidate, &audio))
            {
                writer.write_uint32(*index);
                return Some(true);
            }
            let record = audio.borrow();
            let Some(data) = record.data.as_ref().filter(|data| data.valid()) else {
                clone_error(scope);
                return None;
            };
            let Ok(index) = u32::try_from(self.hosts.borrow().len()) else {
                clone_error(scope);
                return None;
            };
            self.hosts
                .borrow_mut()
                .push(HostData::AudioData(data.clone()));
            writer.write_uint32(index);
            return Some(true);
        }
        if let Some(frame) = video_frame::identify(scope, object.into()) {
            if let Some((_, index)) = self
                .frames
                .iter()
                .find(|(candidate, _)| Rc::ptr_eq(candidate, &frame))
            {
                writer.write_uint32(*index);
                return Some(true);
            }
            let frame = frame.borrow();
            if !video_frame::valid(&frame) {
                clone_error(scope);
                return None;
            }
            let Ok(index) = u32::try_from(self.hosts.borrow().len()) else {
                clone_error(scope);
                return None;
            };
            self.hosts
                .borrow_mut()
                .push(HostData::VideoFrame(frame.clone()));
            writer.write_uint32(index);
            return Some(true);
        }
        if let Some(source) = offscreen_canvas::identify(scope, object.into()) {
            if self.for_storage {
                clone_error(scope);
                return None;
            }
            let Some((_, index)) = self
                .offscreens
                .iter()
                .find(|(candidate, _)| Rc::ptr_eq(&candidate.backing, &source.backing))
            else {
                clone_error(scope);
                return None;
            };
            writer.write_uint32(*index);
            return Some(true);
        }
        if let Some(backing) = image_bitmap::identify(scope, object.into()) {
            if let Some((_, index)) = self
                .bitmaps
                .iter()
                .find(|(candidate, _)| Rc::ptr_eq(candidate, &backing))
            {
                writer.write_uint32(*index);
                return Some(true);
            }
            let bitmap = backing.borrow();
            let Some(bitmap) = bitmap
                .as_ref()
                .filter(|bitmap| bitmap.origin_clean && image_bitmap::valid(bitmap))
            else {
                clone_error(scope);
                return None;
            };
            let Ok(id) = u32::try_from(self.hosts.borrow().len()) else {
                clone_error(scope);
                return None;
            };
            self.hosts
                .borrow_mut()
                .push(HostData::ImageBitmap(bitmap.clone()));
            writer.write_uint32(id);
            return Some(true);
        }
        if let Some((width, height, pixels)) = native_image_data(scope, object) {
            if pixels.buffer(scope).is_some_and(|b| b.was_detached())
                || image_data_size(width, height).ok() != Some(pixels.byte_length())
            {
                clone_error(scope);
                return None;
            }
            let Ok(id) = u32::try_from(self.hosts.borrow().len()) else {
                clone_error(scope);
                return None;
            };
            self.hosts
                .borrow_mut()
                .push(HostData::ImageData { width, height });
            writer.write_uint32(id);
            // Serialize the view in the same graph, preserving references to its
            // buffer or data array elsewhere in the message and transfer list.
            return writer.write_value(scope.get_current_context(), pixels.into());
        }
        let data = (|| {
            if let Some(record) = native_geometry_clone(scope, object) {
                return Some(HostData::Geometry(record));
            }
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
                PlatformObjectRecord::MessagePort(_) => self
                    .ports
                    .iter()
                    .position(|key| *key == (state.instance_id, id))
                    .map(|index| HostData::Port(index as u32)),
                PlatformObjectRecord::CryptoKey(key) => Some(HostData::CryptoKey((**key).clone())),
                PlatformObjectRecord::Blob(blob) => Some(HostData::Blob(blob.clone())),
                PlatformObjectRecord::DomException {
                    name,
                    message,
                    code,
                } => Some(HostData::DomException {
                    stack: native_errors::stack(scope, object).unwrap_or_default(),
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
        buffer: v8::Local<'s, v8::SharedArrayBuffer>,
    ) -> Option<u32> {
        if self.for_storage {
            clone_error(scope);
            return None;
        }
        #[cfg(unix)]
        {
            let state = scope.get_current_context().get_slot::<V8BindingState>()?;
            let Some(lease) = state.native_shared_mapping(&buffer) else {
                clone_error(scope);
                return None;
            };
            let mut shared = self.shared.borrow_mut();
            if let Some(id) = shared
                .iter()
                .position(|entry| entry.lease.descriptor.token == lease.descriptor.token)
            {
                return Some(id as u32);
            }
            if shared.len() >= 1024 {
                clone_error(scope);
                return None;
            }
            // Open against the serializing realm, even when the source wrapper
            // was created by another live or retired realm in this isolate.
            let transport = state.worker_transport.borrow().clone()?;
            let creator = state.native_worker_creator().ok()?;
            let response =
                match transport.request(crate::worker_wire::AgentServiceRequest::SharedMemory {
                    creator,
                    request: crate::shared_memory::SharedMemoryRequest::Open {
                        descriptor: lease.descriptor,
                    },
                }) {
                    Ok(crate::worker_wire::AgentServiceResponse::SharedMemory(response)) => {
                        response
                    }
                    _ => {
                        clone_error(scope);
                        return None;
                    }
                };
            let buffer = match state.map_native_shared_memory(scope, response) {
                Ok(buffer) => buffer,
                Err(_) => {
                    clone_error(scope);
                    return None;
                }
            };
            let lease = state.native_shared_mapping(&buffer)?;
            let id = shared.len() as u32;
            shared.push(SharedBacking {
                backing: buffer.get_backing_store(),
                lease,
            });
            Some(id)
        }
        #[cfg(not(unix))]
        {
            let _ = buffer;
            clone_error(scope);
            None
        }
    }
    fn get_wasm_module_transfer_id(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        module: v8::Local<v8::WasmModuleObject>,
    ) -> Option<u32> {
        if !self
            .hosts
            .borrow()
            .iter()
            .any(|host| matches!(host, HostData::ModuleOrigin(_)))
        {
            let origin = module_origin(scope)?;
            self.hosts.borrow_mut().push(HostData::ModuleOrigin(origin));
        }
        let compiled = module.get_compiled_module();
        let bytes = compiled.get_wire_bytes_ref();
        let mut modules = self.wasm_modules.borrow_mut();
        if modules.len() >= 1024
            || modules.iter().fold(bytes.len(), |size, module| {
                size.saturating_add(module.len())
            }) > crate::message_wire::MAX_BYTES
        {
            clone_error(scope);
            return None;
        }
        let id = modules.len() as u32;
        modules.push(bytes.to_vec());
        Some(id)
    }
}
struct Deserializer<'a> {
    #[cfg(unix)]
    shared: &'a [SharedBacking],
    wasm_modules: &'a [Vec<u8>],
    hosts: &'a [HostData],
    ports: &'a [v8::Global<v8::Object>],
}
impl v8::ValueDeserializerImpl for Deserializer<'_> {
    fn get_shared_array_buffer_from_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: u32,
    ) -> Option<v8::Local<'s, v8::SharedArrayBuffer>> {
        #[cfg(unix)]
        {
            let shared = self.shared.get(id as usize)?;
            Some(v8::SharedArrayBuffer::with_backing_store(
                scope,
                &shared.backing,
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            clone_error(scope);
            None
        }
    }
    fn get_wasm_module_from_id<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        id: u32,
    ) -> Option<v8::Local<'s, v8::WasmModuleObject>> {
        let origin = module_origin(scope)?;
        if !self
            .hosts
            .iter()
            .any(|host| matches!(host,HostData::ModuleOrigin(expected)if *expected==origin))
        {
            clone_error(scope);
            return None;
        }
        let bytes = self.wasm_modules.get(id as usize)?;
        v8::WasmModuleObject::compile(scope, bytes)
    }

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
            HostData::ModuleOrigin(_) | HostData::UserActivation(_) => return None,
            HostData::MediaSourceHandle(index) => {
                return media_source::receive(scope, &state, *index, self.ports).ok();
            }
            HostData::TransferredArrayBuffer { .. } => return None,
            HostData::Stream(descriptor) => {
                return stream_transfer::receive(scope, &state, *descriptor, self.ports).ok();
            }
            HostData::CryptoKey(key) => {
                if !state.secure_context.get() || key.validate().is_err() {
                    clone_error(scope);
                    return None;
                }
                return web_crypto::materialize(scope, &state, key.clone()).ok();
            }
            HostData::FileList => {
                let mut count = 0;
                if !reader.read_uint32(&mut count) {
                    return None;
                }
                // Count is bounded by serialized input; no allocation based on
                // an untrusted count before entries have actually been decoded.
                let mut entries = Vec::new();
                for _ in 0..count {
                    entries.push(reader.read_value(scope.get_current_context())?);
                }
                return file_lists::restore_clone(scope, &state, &entries).ok();
            }
            HostData::Quad => {
                let points = v8::Array::new(scope, 4);
                for index in 0..4 {
                    let point = reader.read_value(scope.get_current_context())?;
                    let object = v8::Local::<v8::Object>::try_from(point).ok()?;
                    point_components(scope, object)?;
                    points.set_index(scope, index, point);
                }
                let template = state
                    .instance_templates
                    .borrow()
                    .get("DOMQuad")
                    .and_then(|template| template.to_global(scope))?;
                let object = v8::Local::new(scope, &template).new_instance(scope)?;
                set_quad_points(scope, object, points).ok()?;
                return Some(object);
            }
            HostData::AudioData(data) => {
                return audio_data::allocate(scope, &state, data.clone()).ok();
            }
            HostData::VideoFrame(frame) => {
                return video_frame::allocate(scope, &state, frame.clone()).ok();
            }
            HostData::OffscreenCanvas(bitmap) => {
                return offscreen_canvas::allocate(scope, &state, bitmap.clone()).ok();
            }
            HostData::ImageBitmap(bitmap) => {
                return image_bitmap::allocate(scope, &state, bitmap.clone()).ok();
            }
            HostData::ImageData { width, height } => {
                let value = reader.read_value(scope.get_current_context())?;
                let pixels = v8::Local::<v8::Uint8ClampedArray>::try_from(value).ok()?;
                if image_data_size(*width, *height).ok() != Some(pixels.byte_length()) {
                    return None;
                }
                let template = state
                    .instance_templates
                    .borrow()
                    .get("ImageData")
                    .and_then(|template| template.to_global(scope))?;
                let object = v8::Local::new(scope, &template).new_instance(scope)?;
                initialize_image_data(scope, object, *width, *height, pixels).ok()?;
                return Some(object);
            }
            HostData::Geometry(record) => {
                return restore_geometry_clone(scope, &state, record).ok();
            }
            HostData::Port(index) => {
                return self
                    .ports
                    .get(*index as usize)
                    .map(|port| v8::Local::new(scope, port));
            }
            HostData::Blob(blob) => (
                if blob.file.is_some() { "File" } else { "Blob" },
                PlatformObjectRecord::Blob(blob.clone()),
            ),
            HostData::DomException {
                name,
                message,
                code,
                stack,
            } => {
                let id = register_platform_object(
                    &state,
                    PlatformObjectRecord::DomException {
                        name: name.clone(),
                        message: message.clone(),
                        code: *code,
                    },
                );
                let object = new_platform_object(scope, &state, "DOMException", id).ok()?;
                if !stack.is_empty() {
                    native_errors::set(scope, object, stack)?;
                }
                return Some(object);
            }
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
    serialize_inner(scope, value, transfers, false, false)
}
fn serialize_inner(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    transfers: &[v8::Local<v8::Value>],
    wire_buffers: bool,
    for_storage: bool,
) -> Option<SerializedValue> {
    let mut buffers: Vec<v8::Local<v8::ArrayBuffer>> = Vec::new();
    let mut ports = Vec::new();
    let mut stream_sources = Vec::new();
    let mut media_handles = Vec::new();
    let mut port_keys = Vec::new();
    let mut detached_port = false;
    let mut bitmaps = Vec::new();
    let mut frames = Vec::new();
    let mut audios = Vec::new();
    let mut offscreens = Vec::new();
    for (index, value) in transfers.iter().enumerate() {
        if transfers[..index]
            .iter()
            .any(|previous| previous.strict_equals(*value))
        {
            clone_error(scope);
            return None;
        }
        if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(*value) {
            buffers.push(buffer);
        } else if let Some(audio) = audio_data::identify(scope, *value) {
            audios.push(audio);
        } else if let Some(frame) = video_frame::identify(scope, *value) {
            frames.push(frame);
        } else if let Some(source) = media_source::identify(scope, *value) {
            media_handles.push(source);
        } else if let Some(source) = offscreen_canvas::identify(scope, *value) {
            offscreens.push(source);
        } else if let Some(bitmap) = image_bitmap::identify(scope, *value) {
            bitmaps.push(bitmap);
        } else if let Some(port) = message_ports::transferable(scope, *value) {
            port_keys.push((port.0.instance_id, port.1));
            ports.push(port);
        } else if let Some(identity) = message_ports::transfer_identity(scope, *value) {
            port_keys.push(identity);
            detached_port = true;
        } else if let Some(stream) = stream_transfer::identify(scope, *value) {
            stream_sources.push(stream);
        } else {
            clone_error(scope);
            return None;
        }
    }
    #[cfg(unix)]
    let shared = Rc::new(RefCell::new(Vec::new()));
    let mut stream_plans = Vec::new();
    let mut stream_hosts = Vec::new();
    let mut initial_hosts = Vec::new();
    let mut media_hosts = Vec::new();
    for source in &media_handles {
        let index = source.append_port(&mut ports);
        media_hosts.push((
            source.owner.instance_id,
            source.id,
            initial_hosts.len() as u32,
        ));
        initial_hosts.push(HostData::MediaSourceHandle(index));
    }
    for source in stream_sources {
        let plan = match stream_transfer::Plan::prepare(source) {
            Ok(plan) => plan,
            Err(_) => {
                clone_error(scope);
                return None;
            }
        };
        let descriptor = plan.append_ports(&mut ports);
        let source = plan.source();
        stream_hosts.push((
            source.owner.instance_id,
            source.id,
            initial_hosts.len() as u32,
        ));
        initial_hosts.push(HostData::Stream(descriptor));
        stream_plans.push(plan);
    }
    // Transfer placeholders bypass ordinary platform serialization. Their
    // detachment/context checks occur only after all message getters finish.
    let mut audio_hosts = Vec::new();
    for audio in &audios {
        audio_hosts.push((audio.clone(), initial_hosts.len() as u32));
        initial_hosts.push(HostData::AudioData(crate::message_wire::AudioDataRecord {
            format: String::new(),
            sample_rate: 0.,
            frames: 0,
            channels: 0,
            timestamp: 0,
            bytes: Vec::new(),
        }));
    }
    let mut bitmap_hosts = Vec::new();
    let mut offscreen_hosts = Vec::new();
    let mut frame_hosts = Vec::new();
    for frame in &frames {
        frame_hosts.push((frame.clone(), initial_hosts.len() as u32));
        initial_hosts.push(HostData::VideoFrame(video_frame::Frame::empty()));
    }
    for backing in &bitmaps {
        bitmap_hosts.push((backing.clone(), initial_hosts.len() as u32));
        initial_hosts.push(HostData::ImageBitmap(DecodedImage {
            origin_clean: true,
            width: 0,
            height: 0,
            rgba: Vec::new(),
        }));
    }
    for source in &offscreens {
        offscreen_hosts.push((source.clone(), initial_hosts.len() as u32));
        initial_hosts.push(HostData::OffscreenCanvas(DecodedImage {
            origin_clean: true,
            width: 0,
            height: 0,
            rgba: Vec::new(),
        }));
    }
    let hosts = Rc::new(RefCell::new(initial_hosts));
    let wasm_modules = Rc::new(RefCell::new(Vec::new()));
    let writer = v8::ValueSerializer::new(
        scope,
        Box::new(Serializer {
            audios: audio_hosts.clone(),
            media_handles: media_hosts,
            frames: frame_hosts.clone(),
            offscreens: offscreen_hosts.clone(),
            bitmaps: bitmap_hosts.clone(),
            streams: stream_hosts,
            for_storage,
            #[cfg(unix)]
            shared: shared.clone(),
            wasm_modules: wasm_modules.clone(),
            hosts: hosts.clone(),
            ports: port_keys,
        }),
    );
    writer.write_header();
    for (id, buffer) in buffers.iter().enumerate() {
        writer.transfer_array_buffer(id as u32, *buffer);
    }
    if writer.write_value(scope.get_current_context(), value) != Some(true) {
        return None;
    }
    // A getter may have transferred a later item. Revalidate the entire list
    // before detaching anything here, preserving atomic failure for this call.
    if let Some(name) = offscreens.iter().find_map(|source| source.transfer_error()) {
        if let Ok(exception) = dom_exception_value(scope, name) {
            scope.throw_exception(exception);
        }
        return None;
    }
    if media_handles.iter().any(|source| !source.valid()) {
        clone_error(scope);
        return None;
    }
    if detached_port||audios.iter().any(|audio|audio.borrow().data.as_ref().is_none_or(|data|!data.valid()))||frames.iter().any(|frame|!video_frame::valid(&frame.borrow()))||bitmaps.iter().any(|bitmap|!bitmap.borrow().as_ref().is_some_and(|bitmap|bitmap.origin_clean&&image_bitmap::valid(bitmap)))||stream_plans.iter().any(|plan|!plan.source().valid())||buffers.iter().any(|buffer| buffer.was_detached() || !buffer.is_detachable())
        || ports.iter().any(|(state, id, port)| !message_ports::transfer_live(port) || !matches!(state.platform_objects.borrow().get(id), Some(PlatformObjectRecord::MessagePort(current)) if Rc::ptr_eq(current, port))) {
        clone_error(scope); return None;
    }
    for (audio, index) in &audio_hosts {
        hosts.borrow_mut()[*index as usize] = HostData::AudioData(audio.borrow().data.clone()?);
    }
    for (frame, index) in &frame_hosts {
        hosts.borrow_mut()[*index as usize] = HostData::VideoFrame(frame.borrow().clone());
    }
    for (backing, index) in &bitmap_hosts {
        let bitmap = backing.borrow().clone()?;
        hosts.borrow_mut()[*index as usize] = HostData::ImageBitmap(bitmap);
    }
    for (source, index) in &offscreen_hosts {
        let bitmap = match source.snapshot() {
            Ok(bitmap) => bitmap,
            Err(_) => {
                clone_error(scope);
                return None;
            }
        };
        hosts.borrow_mut()[*index as usize] = HostData::OffscreenCanvas(bitmap);
    }
    // The main graph always uses transfer IDs, including already detached
    // objects, so every author getter precedes final transfer validation.
    // A separate pure-buffer encoding retains resizable backing metadata on
    // the process wire without consulting properties or detaching the source.
    if wire_buffers {
        for (index, buffer) in buffers.iter().enumerate() {
            let mut encoded = serialize_inner(scope, (*buffer).into(), &[], false, false)?;
            hosts.borrow_mut().push(HostData::TransferredArrayBuffer {
                index: index as u32,
                bytes: std::mem::take(&mut encoded.bytes),
            });
        }
    }
    #[cfg(unix)]
    let offers = if wire_buffers {
        let mut offers = SharedOffers::new();
        for backing in shared.borrow().iter() {
            let reference =
                match backing
                    .lease
                    .request(crate::shared_memory::SharedMemoryRequest::Offer {
                        descriptor: backing.lease.descriptor,
                    }) {
                    Ok(crate::shared_memory::SharedMemoryResponse {
                        outcome: crate::shared_memory::SharedMemoryOutcome::Reference(reference),
                        ..
                    }) => reference,
                    _ => {
                        clone_error(scope);
                        return None;
                    }
                };
            offers.entries.push((backing.lease.clone(), reference));
        }
        if !offers.commit() {
            clone_error(scope);
            return None;
        }
        Some(offers)
    } else {
        None
    };
    let pending = wire_buffers.then(|| PendingTransfers {
        audios: audios.clone(),
        media_handles: media_handles.clone(),
        frames: frames.clone(),
        offscreens: offscreens.clone(),
        bitmaps: bitmaps.clone(),
        buffers: buffers
            .iter()
            .map(|buffer| v8::Global::new(scope, *buffer))
            .collect(),
        ports: ports.clone(),
    });
    if !wire_buffers && !commit_streams(scope, &mut stream_plans) {
        return None;
    }
    let mut stores = Vec::new();
    for buffer in buffers.into_iter().filter(|_| !wire_buffers) {
        if buffer.was_detached() || !buffer.is_detachable() {
            for plan in &mut stream_plans {
                plan.rollback(scope);
            }
            clone_error(scope);
            return None;
        }
        let store = buffer.get_backing_store();
        if buffer.detach(None) != Some(true) {
            for plan in &mut stream_plans {
                plan.rollback(scope);
            }
            clone_error(scope);
            return None;
        }
        if !wire_buffers {
            stores.push(store);
        }
    }
    if !wire_buffers {
        for handle in &media_handles {
            handle.detach();
        }
    }
    if !wire_buffers {
        for audio in audios {
            audio.borrow_mut().data.take();
        }
        for frame in frames {
            let mut frame = frame.borrow_mut();
            frame.bitmap.take();
            frame.planes.take();
        }
        for bitmap in bitmaps {
            bitmap.borrow_mut().take();
        }
        for source in offscreens {
            source.detach();
        }
    }
    let bytes = writer.release();
    drop(writer);
    let hosts = hosts.borrow().clone();
    if !wire_buffers {
        for (state, id, port) in &ports {
            message_ports::detach(state, *id, port);
        }
    }
    let wasm_modules = wasm_modules.borrow().clone();
    #[cfg(unix)]
    let shared = shared.borrow().clone();
    Some(SerializedValue {
        stream_plans,
        pending,
        #[cfg(unix)]
        shared,
        #[cfg(unix)]
        offers,
        wasm_modules,
        bytes,
        hosts,
        buffers: stores,
        ports: ports.into_iter().map(|(_, _, port)| port).collect(),
    })
}

pub fn deserialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: SerializedValue,
) -> Option<v8::Local<'s, v8::Value>> {
    deserialize_with_ports(scope, value).map(|(value, _)| value)
}

pub(super) fn deserialize_with_ports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut value: SerializedValue,
) -> Option<(v8::Local<'s, v8::Value>, Vec<v8::Local<'s, v8::Object>>)> {
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    #[cfg(unix)]
    let shared = {
        let mut received = Vec::with_capacity(value.shared.len());
        for entry in &value.shared {
            let transport = state.worker_transport.borrow().clone()?;
            let creator = state.native_worker_creator().ok()?;
            let response =
                match transport.request(crate::worker_wire::AgentServiceRequest::SharedMemory {
                    creator,
                    request: crate::shared_memory::SharedMemoryRequest::Open {
                        descriptor: entry.lease.descriptor,
                    },
                }) {
                    Ok(crate::worker_wire::AgentServiceResponse::SharedMemory(response)) => {
                        response
                    }
                    _ => {
                        clone_error(scope);
                        return None;
                    }
                };
            let buffer = match state.map_native_shared_memory(scope, response) {
                Ok(buffer) => buffer,
                Err(_) => {
                    clone_error(scope);
                    return None;
                }
            };
            let lease = state.native_shared_mapping(&buffer)?;
            received.push(SharedBacking {
                backing: buffer.get_backing_store(),
                lease,
            });
        }
        received
    };
    let mut stream_guard = stream_transfer::DecodeGuard::new(&state);
    let ports = value
        .ports
        .iter()
        .cloned()
        .map(|port| message_ports::receive(scope, &state, port))
        .collect::<Option<Vec<_>>>()?;
    let globals = ports
        .iter()
        .map(|port| v8::Global::new(scope, *port))
        .collect::<Vec<_>>();
    let reader = v8::ValueDeserializer::new(
        scope,
        Box::new(Deserializer {
            #[cfg(unix)]
            shared: &shared,
            wasm_modules: &value.wasm_modules,
            hosts: &value.hosts,
            ports: &globals,
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
    let decoded = reader.read_value(scope.get_current_context())?;
    drop(reader);
    native_errors::normalize_clone(scope, decoded)?;
    value.ports.clear();
    stream_guard.commit();
    let hidden = value
        .hosts
        .iter()
        .flat_map(|host| match host {
            HostData::Stream(descriptor) => descriptor.ports().collect::<Vec<_>>(),
            HostData::MediaSourceHandle(index) => vec![*index],
            _ => Vec::new(),
        })
        .collect::<BTreeSet<_>>();
    Some((
        decoded,
        ports
            .into_iter()
            .enumerate()
            .filter_map(|(index, port)| (!hidden.contains(&(index as u32))).then_some(port))
            .collect(),
    ))
}

pub fn transfer_list<'s>(
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
    transfer_sequence(scope, value, method)
}
/// WebIDL postMessage overload resolution accepts any iterable object, not
/// only Arrays. Read @@iterator once and preserve getter/call exceptions.
pub struct PostTransferOptions<'s> {
    pub transfers: Vec<v8::Local<'s, v8::Value>>,
    pub include_user_activation: bool,
}
pub fn include_user_activation(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<bool> {
    if value.is_null_or_undefined() {
        return Some(false);
    }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(
            scope,
            "postMessage options require a dictionary or sequence",
        );
        return None;
    };
    let key = v8::String::new(scope, "includeUserActivation")?;
    Some(object.get(scope, key.into())?.boolean_value(scope))
}
pub fn activation_snapshot(scope: &mut v8::PinScope, include: bool) -> Option<[bool; 2]> {
    if !include {
        return None;
    }
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    Some([
        state.has_transient_user_activation(),
        state.has_sticky_user_activation(),
    ])
}
pub fn post_transfer_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
) -> Option<PostTransferOptions<'s>> {
    if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        let key = v8::Symbol::get_iterator(scope);
        let method = object.get(scope, key.into())?;
        if !method.is_null_or_undefined() {
            return Some(PostTransferOptions {
                transfers: transfer_sequence(scope, object, method)?,
                include_user_activation: false,
            });
        }
    }
    let include_user_activation = include_user_activation(scope, value)?;
    Some(PostTransferOptions {
        transfers: transfer_list(scope, value)?,
        include_user_activation,
    })
}
pub fn post_transfer_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Value>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
    post_transfer_options(scope, value).map(|options| options.transfers)
}
fn transfer_sequence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<v8::Object>,
    method: v8::Local<v8::Value>,
) -> Option<Vec<v8::Local<'s, v8::Value>>> {
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

/// Wasm module deserialization is origin-restricted, including opaque native
/// identities. Query the authenticated parent for production process routes.
fn module_origin(scope: &mut v8::PinScope) -> Option<[u8; 32]> {
    use sha2::Digest;
    let context = scope.get_current_context();
    let state = context.get_slot::<V8BindingState>()?;
    if let Some(transport) = state.worker_transport.borrow().clone() {
        let creator = match state.native_worker_creator() {
            Ok(creator) => creator,
            Err(_) => {
                clone_error(scope);
                return None;
            }
        };
        return match transport.request(crate::worker_wire::AgentServiceRequest::SharedMemory {
            creator,
            request: crate::shared_memory::SharedMemoryRequest::CloneOrigin,
        }) {
            Ok(crate::worker_wire::AgentServiceResponse::SharedMemory(
                crate::shared_memory::SharedMemoryResponse {
                    outcome: crate::shared_memory::SharedMemoryOutcome::CloneOrigin(origin),
                    ..
                },
            )) => Some(origin),
            _ => {
                clone_error(scope);
                None
            }
        };
    }
    let authority = context
        .get_slot::<crate::realm::RealmDescriptor>()
        .map(|realm| realm.origin.security_token_key())
        .unwrap_or_else(|| {
            let origin = current_platform_origin(&state);
            if origin == "null" {
                format!("oryn:unbound-realm:{}", state.instance_id)
            } else {
                format!("oryn:origin:{origin}")
            }
        });
    Some(sha2::Sha256::digest(authority.as_bytes()).into())
}

/// Structured serialization for process transport. Engine bytes preserve graph
/// identity, UTF-16 strings, BigInts, Maps/Sets, dates, errors and typed views.
/// Only native host records and copied transferred storage supplement the bytes.
pub fn serialize_wire(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    transfers: &[v8::Local<v8::Value>],
) -> Option<crate::message_wire::WireValue> {
    serialize_wire_with_activation(scope, value, transfers, |_| None)
}
/// Capture activation after graph getters, before validating and committing transfers.
pub fn serialize_wire_with_activation(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    transfers: &[v8::Local<v8::Value>],
    activation: impl FnOnce(&mut v8::PinScope) -> Option<[bool; 2]>,
) -> Option<crate::message_wire::WireValue> {
    for value in transfers {
        if let Some((_, _, port)) = message_ports::transferable(scope, *value)
            && !message_ports::wire_transfer_supported(&port)
        {
            clone_error(scope);
            return None;
        }
    }
    // For IPC let V8 serialize the buffer itself (including resizable metadata
    // and graph aliases), then detach the source only after successful writing.
    let mut value = serialize_inner(scope, value, transfers, true, false)?;
    value.set_user_activation(activation(scope));
    to_wire(scope, value)
}
/// Storage serialization rejects shared memory before opening any native lease.
pub(super) fn serialize_storage(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<crate::message_wire::WireValue> {
    let value = serialize_inner(scope, value, &[], true, true)?;
    to_wire(scope, value)
}
pub(super) fn deserialize_storage<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wire: crate::message_wire::WireValue,
) -> Option<v8::Local<'s, v8::Value>> {
    if !wire.shared.is_empty()
        || !wire.ports.is_empty()
        || wire.shared_queue.is_some()
        || wire.shared_recipient.is_some()
    {
        clone_error(scope);
        return None;
    }
    deserialize_wire(scope, wire).map(|(value, _)| value)
}
fn commit_streams(scope: &mut v8::PinScope, plans: &mut [stream_transfer::Plan]) -> bool {
    for index in 0..plans.len() {
        if plans[index].commit(scope).is_err() {
            for plan in plans {
                plan.rollback(scope);
            }
            clone_error(scope);
            return false;
        }
    }
    true
}
fn to_wire(
    scope: &mut v8::PinScope,
    mut value: SerializedValue,
) -> Option<crate::message_wire::WireValue> {
    use crate::message_wire::{WireFile, WireHost, WireValue};
    #[cfg(unix)]
    let mut offers = value.offers.take().unwrap_or_else(SharedOffers::new);
    let hosts = std::mem::take(&mut value.hosts)
        .into_iter()
        .map(|host| match host {
            HostData::ModuleOrigin(origin) => WireHost::ModuleOrigin(origin),
            HostData::UserActivation([active, sticky]) => {
                WireHost::UserActivation { active, sticky }
            }
            HostData::MediaSourceHandle(index) => WireHost::MediaSourceHandle(index),
            HostData::TransferredArrayBuffer { index, bytes } => {
                WireHost::TransferredArrayBuffer { index, bytes }
            }
            HostData::Stream(descriptor) => WireHost::Stream {
                readable: descriptor.readable,
                writable: descriptor.writable,
            },
            HostData::CryptoKey(key) => WireHost::CryptoKey(key),
            HostData::FileList => WireHost::FileList,
            HostData::Quad => WireHost::Quad,
            HostData::Geometry(record) => WireHost::Geometry {
                rect: record.rect,
                matrix: record.matrix,
                readonly: record.readonly,
                is_2d: record.is_2d,
                bits: record.bits,
            },
            HostData::AudioData(data) => WireHost::AudioData(data),
            HostData::VideoFrame(frame) => WireHost::VideoFrame(frame.wire()),
            HostData::OffscreenCanvas(bitmap) => WireHost::OffscreenCanvas {
                width: bitmap.width,
                height: bitmap.height,
                rgba: bitmap.rgba,
                origin_clean: bitmap.origin_clean,
            },
            HostData::ImageBitmap(bitmap) => WireHost::ImageBitmap {
                width: bitmap.width,
                height: bitmap.height,
                rgba: bitmap.rgba,
            },
            HostData::ImageData { width, height } => WireHost::ImageData { width, height },
            HostData::Port(index) => WireHost::Port(index),
            HostData::Blob(blob) => WireHost::Blob {
                bytes: blob.bytes,
                media_type: blob.media_type,
                file: blob.file.map(|file| WireFile {
                    name: file.name,
                    last_modified: file.last_modified,
                }),
            },
            HostData::DomException {
                name,
                message,
                code,
                stack,
            } => WireHost::DomException {
                name,
                message,
                code,
                stack,
            },
        })
        .collect();
    let buffers = if let Some(pending) = &value.pending {
        vec![Vec::new(); pending.buffers.len()]
    } else {
        value
            .buffers
            .iter()
            .map(|store| store.iter().map(Cell::get).collect())
            .collect()
    };
    let mut wire = WireValue {
        shared: Vec::new(),
        shared_queue: None,
        shared_recipient: None,
        wasm_modules: std::mem::take(&mut value.wasm_modules),
        version: 1,
        bytes: std::mem::take(&mut value.bytes),
        hosts,
        buffers,
        ports: Vec::new(),
    };
    #[cfg(unix)]
    {
        wire.shared = offers
            .entries
            .iter()
            .map(|(_, reference)| *reference)
            .collect();
    }
    // Validate full wire size, host indices and transfer cardinality before the
    // parent rotates any live port capability or V8 detaches a backing store.
    wire.ports = (0..value.ports.len())
        .map(|id| crate::message_wire::PortToken {
            id: id as u64 + 1,
            secret: [0; 32],
        })
        .collect();
    if wire.validate().is_err() {
        clone_error(scope);
        return None;
    }
    if let Some(pending) = &value.pending {
        if pending.media_handles.iter().any(|source| !source.valid()) {
            clone_error(scope);
            return None;
        }
        if pending.audios.iter().any(|audio|audio.borrow().data.is_none())||pending.frames.iter().any(|frame|!video_frame::valid(&frame.borrow()))||pending.offscreens.iter().any(|source|!source.valid())||pending.bitmaps.iter().any(|bitmap|bitmap.borrow().is_none())||pending.buffers.iter().any(|buffer|{let buffer=v8::Local::new(scope,buffer);buffer.was_detached()||!buffer.is_detachable()})
            ||pending.ports.iter().any(|(state,id,port)|state.execution_retired()||!message_ports::transfer_live(port)||!matches!(state.platform_objects.borrow().get(id),Some(PlatformObjectRecord::MessagePort(current))if Rc::ptr_eq(current,port))){clone_error(scope);return None}
    }
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    if !commit_streams(scope, &mut value.stream_plans) {
        return None;
    }
    wire.ports = match message_ports::export_tokens(scope, &value.ports) {
        Some(ports) => ports,
        None => {
            for plan in &mut value.stream_plans {
                plan.rollback(scope);
            }
            return None;
        }
    };
    let mut port_guard = WirePortGuard::new(&state, &wire.ports);
    if wire.ports.len() != value.ports.len() || wire.validate().is_err() {
        for plan in &mut value.stream_plans {
            plan.rollback(scope);
        }
        clone_error(scope);
        return None;
    }
    if let Some(pending) = value.pending.take() {
        for handle in pending.media_handles {
            handle.detach();
        }
        // No author callback or V8 task runs after the complete revalidation.
        for audio in pending.audios {
            audio.borrow_mut().data.take();
        }
        for frame in pending.frames {
            let mut frame = frame.borrow_mut();
            frame.bitmap.take();
            frame.planes.take();
        }
        for bitmap in pending.bitmaps {
            bitmap.borrow_mut().take();
        }
        for source in pending.offscreens {
            source.detach();
        }
        for buffer in pending.buffers {
            if v8::Local::new(scope, &buffer).detach(None) != Some(true) {
                for plan in &mut value.stream_plans {
                    plan.rollback(scope);
                }
                clone_error(scope);
                return None;
            }
        }
        for (state, id, port) in pending.ports {
            message_ports::detach(&state, id, &port);
        }
    }
    #[cfg(unix)]
    {
        offers.committed = true;
    }
    port_guard.commit();
    value.ports.clear();
    Some(wire)
}
pub(super) fn from_wire(
    scope: &mut v8::PinScope,
    value: crate::message_wire::WireValue,
) -> Option<SerializedValue> {
    use crate::message_wire::WireHost;
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    let mut port_guard = WirePortGuard::new(&state, &value.ports);
    #[cfg(unix)]
    let deliveries = if value.shared.is_empty() {
        None
    } else {
        Some(SharedDeliveries::new(scope, &value.shared)?)
    };
    if value.validate().is_err() {
        clone_error(scope);
        return None;
    }
    if !state.secure_context.get()
        && value
            .hosts
            .iter()
            .any(|host| matches!(host, WireHost::CryptoKey(_)))
    {
        clone_error(scope);
        return None;
    }
    let state = scope.get_current_context().get_slot::<V8BindingState>()?;
    #[cfg(unix)]
    let mut shared = Vec::new();
    #[cfg(unix)]
    for reference in &value.shared {
        let response = match deliveries.as_ref()?.request(
            crate::shared_memory::SharedMemoryRequest::OpenDelivery {
                reference: *reference,
            },
        ) {
            Ok(response) => response,
            Err(_) => {
                clone_error(scope);
                return None;
            }
        };
        let buffer = match state.map_native_shared_memory(scope, response) {
            Ok(buffer) => buffer,
            Err(_) => {
                clone_error(scope);
                return None;
            }
        };
        let lease = state.native_shared_mapping(&buffer)?;
        shared.push(SharedBacking {
            backing: buffer.get_backing_store(),
            lease,
        });
    }
    #[cfg(not(unix))]
    if !value.shared.is_empty() {
        clone_error(scope);
        return None;
    }
    let mut encoded_buffers = BTreeMap::new();
    for host in &value.hosts {
        if let WireHost::TransferredArrayBuffer { index, bytes } = host {
            // The nested payload has no hosts or transfers, preventing recursive
            // metadata and native capability admission during backing decoding.
            let nested = crate::message_wire::WireValue {
                version: 1,
                bytes: bytes.clone(),
                hosts: Vec::new(),
                buffers: Vec::new(),
                ports: Vec::new(),
                shared: Vec::new(),
                shared_queue: None,
                shared_recipient: None,
                wasm_modules: Vec::new(),
            };
            let (decoded, _) = deserialize_wire(scope, nested)?;
            let buffer = v8::Local::<v8::ArrayBuffer>::try_from(decoded).ok()?;
            encoded_buffers.insert(*index, buffer.get_backing_store());
        }
    }
    let hosts = value
        .hosts
        .into_iter()
        .map(|host| match host {
            WireHost::ModuleOrigin(origin) => HostData::ModuleOrigin(origin),
            WireHost::UserActivation { active, sticky } => {
                HostData::UserActivation([active, sticky])
            }
            WireHost::MediaSourceHandle(index) => HostData::MediaSourceHandle(index),
            WireHost::TransferredArrayBuffer { index, bytes } => {
                HostData::TransferredArrayBuffer { index, bytes }
            }
            WireHost::Stream { readable, writable } => {
                HostData::Stream(stream_transfer::Descriptor { readable, writable })
            }
            WireHost::CryptoKey(key) => HostData::CryptoKey(key),
            WireHost::FileList => HostData::FileList,
            WireHost::Quad => HostData::Quad,
            WireHost::Geometry {
                rect,
                matrix,
                readonly,
                is_2d,
                bits,
            } => HostData::Geometry(GeometryClone {
                rect,
                matrix,
                readonly,
                is_2d,
                bits,
            }),
            WireHost::AudioData(data) => HostData::AudioData(data),
            WireHost::VideoFrame(frame) => {
                HostData::VideoFrame(video_frame::Frame::from_wire(frame))
            }
            WireHost::OffscreenCanvas {
                width,
                height,
                rgba,
                origin_clean,
            } => HostData::OffscreenCanvas(DecodedImage {
                origin_clean,
                width,
                height,
                rgba,
            }),
            WireHost::ImageBitmap {
                width,
                height,
                rgba,
            } => HostData::ImageBitmap(DecodedImage {
                origin_clean: true,
                width,
                height,
                rgba,
            }),
            WireHost::ImageData { width, height } => HostData::ImageData { width, height },
            WireHost::Port(index) => HostData::Port(index),
            WireHost::Blob {
                bytes,
                media_type,
                file,
            } => HostData::Blob(BlobRecord {
                bytes,
                media_type,
                file: file.map(|file| FileRecord {
                    name: file.name,
                    last_modified: file.last_modified,
                }),
            }),
            WireHost::DomException {
                name,
                message,
                code,
                stack,
            } => HostData::DomException {
                name,
                message,
                code,
                stack,
            },
        })
        .collect();
    let buffers = value
        .buffers
        .into_iter()
        .enumerate()
        .map(|(index, bytes)| {
            if let Some(backing) = encoded_buffers.remove(&(index as u32)) {
                return backing;
            }
            let store = v8::ArrayBuffer::new_backing_store(scope, bytes.len());
            for (cell, byte) in store.iter().zip(bytes) {
                cell.set(byte)
            }
            store.make_shared()
        })
        .collect();
    let ports = value
        .ports
        .into_iter()
        .map(|token| message_ports::import_token(&state, token))
        .collect::<Option<Vec<_>>>()?;
    port_guard.commit();
    Some(SerializedValue {
        stream_plans: Vec::new(),
        pending: None,
        #[cfg(unix)]
        shared,
        #[cfg(unix)]
        offers: None,
        wasm_modules: value.wasm_modules,
        bytes: value.bytes,
        hosts,
        buffers,
        ports,
    })
}
pub fn deserialize_wire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: crate::message_wire::WireValue,
) -> Option<(v8::Local<'s, v8::Value>, Vec<v8::Local<'s, v8::Object>>)> {
    let value = from_wire(scope, value)?;
    deserialize_with_ports(scope, value)
}

#[cfg(unix)]
struct SharedOffers {
    entries: Vec<(
        std::sync::Arc<crate::v8_shared_memory::MappingLease>,
        crate::shared_memory::SharedMemoryReference,
    )>,
    committed: bool,
}
#[cfg(unix)]
impl SharedOffers {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            committed: false,
        }
    }
    fn commit(&mut self) -> bool {
        for (lease, reference) in &self.entries {
            if lease
                .request(crate::shared_memory::SharedMemoryRequest::CommitOffers {
                    references: vec![*reference],
                })
                .is_err()
            {
                return false;
            }
        }
        true
    }
}
#[cfg(unix)]
impl Drop for SharedOffers {
    fn drop(&mut self) {
        if !self.committed {
            for (lease, reference) in &self.entries {
                let _ = lease.request(crate::shared_memory::SharedMemoryRequest::AbortOffer {
                    reference: *reference,
                });
            }
        }
    }
}
#[cfg(unix)]
struct SharedDeliveries {
    transport: std::sync::Arc<dyn crate::worker_wire::WorkerTransport>,
    creator: crate::worker_wire::Creator,
    references: Vec<crate::shared_memory::SharedMemoryReference>,
}
#[cfg(unix)]
impl SharedDeliveries {
    fn new(
        scope: &mut v8::PinScope,
        references: &[crate::shared_memory::SharedMemoryReference],
    ) -> Option<Self> {
        let state = scope.get_current_context().get_slot::<V8BindingState>()?;
        Some(Self {
            transport: state.worker_transport.borrow().clone()?,
            creator: state.native_worker_creator().ok()?,
            references: references.to_vec(),
        })
    }
    fn request(
        &self,
        request: crate::shared_memory::SharedMemoryRequest,
    ) -> Result<crate::shared_memory::SharedMemoryResponse, ()> {
        match self
            .transport
            .request(crate::worker_wire::AgentServiceRequest::SharedMemory {
                creator: self.creator.clone(),
                request,
            }) {
            Ok(crate::worker_wire::AgentServiceResponse::SharedMemory(response)) => Ok(response),
            _ => Err(()),
        }
    }
}
#[cfg(unix)]
impl Drop for SharedDeliveries {
    fn drop(&mut self) {
        for reference in &self.references {
            let _ = self.request(crate::shared_memory::SharedMemoryRequest::ReleaseDelivery {
                reference: *reference,
            });
        }
    }
}

/// Retain sender cleanup while an outbound call owns offered capabilities.
/// Synchronous parent adoption consumes them; abort then becomes idempotent.
/// An event mailbox must explicitly transfer custody after successful enqueue.
pub struct SharedSendGuard {
    #[cfg(unix)]
    delivery: Option<SharedDeliveries>,
}
impl SharedSendGuard {
    pub fn transfer_custody(&mut self) {
        #[cfg(unix)]
        {
            if let Some(mut delivery) = self.delivery.take() {
                delivery.references.clear();
            }
        }
    }
}
pub fn shared_send_guard(
    scope: &mut v8::PinScope,
    value: &crate::message_wire::WireValue,
) -> SharedSendGuard {
    SharedSendGuard {
        #[cfg(unix)]
        delivery: if value.shared.is_empty() {
            None
        } else {
            SharedDeliveries::new(scope, &value.shared)
        },
    }
}
impl Drop for SharedSendGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(mut delivery) = self.delivery.take() {
            for reference in &delivery.references {
                let _ = delivery.request(crate::shared_memory::SharedMemoryRequest::AbortOffer {
                    reference: *reference,
                });
            }
            // These were source offers, never receiving claims.
            delivery.references.clear();
        }
    }
}

/// Own delivery cleanup before other native work (such as resolving a message
/// source Window) can fail. Opening a claim consumes it, making final cleanup
/// idempotent. Cleanup does not require the receiving document still be live.
pub struct SharedReceiveGuard {
    ports: WirePortGuard,
    #[cfg(unix)]
    _delivery: Option<SharedDeliveries>,
}
impl SharedReceiveGuard {
    /// Transfer raw port custody to the next native decoding/dispatch owner.
    pub fn handoff_ports(&mut self) {
        self.ports.commit();
    }
}
pub fn shared_receive_guard(
    state: &V8BindingState,
    value: &crate::message_wire::WireValue,
) -> SharedReceiveGuard {
    SharedReceiveGuard {
        ports: WirePortGuard::new(state, &value.ports),
        #[cfg(unix)]
        _delivery: if value.shared.is_empty() {
            None
        } else {
            state
                .worker_transport
                .borrow()
                .clone()
                .map(|transport| SharedDeliveries {
                    transport,
                    creator: state.worker_creator.borrow().clone().unwrap_or_else(|| {
                        crate::worker_wire::Creator::Window {
                            target: state.message_target.borrow().clone(),
                            document_generation: state.document_generation(),
                            stream_epoch: state.document_authority_epoch(),
                        }
                    }),
                    references: value.shared.clone(),
                })
        },
    }
}

#[cfg(test)]
mod delivery_custody_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Transport(Mutex<Vec<crate::message_wire::MessageRequest>>);
    impl crate::message_wire::MessageTransport for Transport {
        fn request(
            &self,
            request: crate::message_wire::MessageRequest,
        ) -> Result<crate::message_wire::MessageResponse, String> {
            self.0.lock().unwrap().push(request);
            Ok(crate::message_wire::MessageResponse::Done)
        }
    }
    #[test]
    fn rejected_native_port_admission_leaves_the_entire_transfer_list_attached() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport(Mutex::new(Vec::new())));
        state.set_message_transport(transport.clone());
        let endpoint = message_ports::import_token(
            &state,
            crate::message_wire::PortToken {
                id: 991,
                secret: [3; 32],
            },
        )
        .unwrap();
        let port = message_ports::receive(scope, &state, endpoint).unwrap();
        let buffer = v8::ArrayBuffer::new(scope, 8);
        let value = v8::Array::new(scope, 2);
        value.set_index(scope, 0, buffer.into());
        value.set_index(scope, 1, port.into());
        {
            v8::tc_scope!(let tc,scope);
            assert!(serialize_wire(tc, value.into(), &[buffer.into(), port.into()]).is_none());
        }
        assert_eq!(buffer.byte_length(), 8);
        assert!(!buffer.was_detached());
        assert!(message_ports::transferable(scope, port.into()).is_some());
        let requests = transport.0.lock().unwrap();
        assert_eq!(requests.iter().filter(|request|matches!(request,crate::message_wire::MessageRequest::InDocument{request,..} if matches!(request.as_ref(),crate::message_wire::MessageRequest::Transfer{..}))).count(),1);
        assert!(!requests.iter().any(|request| matches!(
            request,
            crate::message_wire::MessageRequest::ClosePort { .. }
        )));
    }
    #[test]
    fn malformed_wire_and_undelivered_payload_dispose_transferred_ports() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let transport = Arc::new(Transport(Mutex::new(Vec::new())));
        state.set_message_transport(transport.clone());
        let token = crate::message_wire::PortToken {
            id: 123,
            secret: [7; 32],
        };
        let wire = crate::message_wire::WireValue {
            shared: Vec::new(),
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: Vec::new(),
            version: 1,
            bytes: vec![0],
            hosts: Vec::new(),
            buffers: Vec::new(),
            ports: vec![token.clone()],
        };
        let mut invalid = wire.clone();
        invalid.version = u32::MAX;
        {
            v8::tc_scope!(let tc,scope);
            assert!(from_wire(tc, invalid).is_none());
        }
        let guard = shared_receive_guard(&state, &wire);
        drop(guard);
        let payload = from_wire(scope, wire).unwrap();
        drop(payload);
        assert!(transport.0.lock().unwrap().iter().any(|request|matches!(request,crate::message_wire::MessageRequest::ClosePort{port}if port==&token)));
    }
}

#[cfg(test)]
mod file_list_tests {
    use super::*;
    #[test]
    fn file_list_local_and_wire_graph_identity_and_snapshot() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let source = v8::Context::new(scope, Default::default());
        let payload = {
            let scope = &mut v8::ContextScope::new(scope, source);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<input type=file>")
                    .into_runtime()
                    .unwrap(),
            ));
            install_generated_bindings(scope, source, dom, RealmId(0), 1).unwrap();
            let script=v8::String::new(scope,r#"
                const check=(x,m)=>{if(!x)throw Error(m)};
                const transfer=new DataTransfer();transfer.items.add(new File(['abc'],'a.txt',{type:'text/plain',lastModified:42}));
                const list=transfer.files, file=list[0];
                const payload={list,again:list,file};
                const local=structuredClone(payload);
                check(local.list instanceof FileList&&local.list!==list&&local.list===local.again,'list identity');
                check(local.list[0]===local.file&&local.list.item(0)===local.file,'file graph identity');
                check(local.file.name==='a.txt'&&local.file.size===3&&local.file.lastModified===42,'metadata');
                check(local.list.item(99)===null&&local.list[99]===undefined,'index semantics');
                transfer.items.clear();check(local.list.length===1,'snapshot');
                const empty=structuredClone(document.querySelector('input').files);
                check(empty instanceof FileList&&empty.length===0,'empty list');
                globalThis.saved=local;
            "#).unwrap();
            v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap();
            let key = v8::String::new(scope, "saved").unwrap();
            let value = source.global(scope).get(scope, key.into()).unwrap();
            serialize_wire(scope, value, &[]).unwrap()
        };
        assert!(
            payload
                .hosts
                .iter()
                .any(|host| matches!(host, crate::message_wire::WireHost::FileList))
        );
        payload.validate().unwrap();
        let destination = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, destination);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let destination_state =
            install_generated_bindings(scope, destination, dom, RealmId(1), 1).unwrap();
        let (value, _) = deserialize_wire(scope, payload).unwrap();
        // Exercise the actual IndexedDB storage envelope (including the Blob
        // base64 side table), rather than only the shared wire helper.
        let stored = idb_value_to_json(scope, &destination_state, value).unwrap();
        let restored = idb_json_to_value(scope, &destination_state, &stored).unwrap();
        let key = v8::String::new(scope, "storedCopy").unwrap();
        destination.global(scope).set(scope, key.into(), restored);
        let key = v8::String::new(scope, "copy").unwrap();
        destination.global(scope).set(scope, key.into(), value);
        let script=v8::String::new(scope,r#"
            if(!(copy.list instanceof FileList)||!(copy.file instanceof File)||copy.list!==copy.again||copy.list[0]!==copy.file||copy.file.name!=='a.txt'||copy.file.size!==3)throw Error('destination graph');
            const d=Object.getOwnPropertyDescriptor(copy.list,'0');if(!d.enumerable||d.writable)throw Error('index descriptor');
            if(!(storedCopy.list instanceof FileList)||storedCopy.list===copy.list||storedCopy.list!==storedCopy.again||storedCopy.list[0]!==storedCopy.file||storedCopy.file.size!==3||storedCopy.file.type!=='text/plain')throw Error('storage graph');
            let failed=false;try{structuredClone(document)}catch(e){failed=e.name==='DataCloneError'}if(!failed)throw Error('node rejection exception');
            if(structuredClone(new Blob(['after'])).size!==5)throw Error('probe left exception');
        "#).unwrap();
        v8::Script::compile(scope, script, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
    #[test]
    fn cloned_list_rejects_non_file_and_proxy_entries() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        for expression in [
            "new Blob(['x'])",
            "({name:'a',size:1})",
            "new Proxy(new File(['x'],'a'),{})",
        ] {
            let script = v8::String::new(scope, expression).unwrap();
            let value = v8::Script::compile(scope, script, None)
                .unwrap()
                .run(scope)
                .unwrap();
            assert!(file_lists::restore_clone(scope, &state, &[value]).is_err());
        }
    }
}

#[cfg(test)]
#[path = "structured_clone_transfer_tests.rs"]
mod transfer_tests;

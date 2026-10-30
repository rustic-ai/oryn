//! Shared serialization settings; the raster snapshot remains native-owned.
use super::*;

type CustodyEntry = (u64, Option<Arc<dyn Send + Sync>>);
#[derive(Default)]
pub(super) struct Custody {
    entries: Vec<CustodyEntry>,
    storage: Option<Arc<dyn Send + Sync>>,
}
impl Custody {
    pub(super) fn contains_key(&self, id: &u64) -> bool {
        self.entries.binary_search_by_key(id, |(key, _)| *key).ok()
            .is_some_and(|index| self.entries[index].1.is_some())
    }
    pub(super) fn remove(&mut self, id: &u64) -> Option<Arc<dyn Send + Sync>> {
        let index = self.entries.binary_search_by_key(id, |(key, _)| *key).ok()?;
        self.entries[index].1.take()
    }
    pub(super) fn reserve(&mut self, allocation: &BindingRetainedAllocation) -> Result<(), V8BindingError> {
        if self.entries.len() < self.entries.capacity() { return Ok(()); }
        // Collected slots retain only numeric IDs. Compact without allocation
        // when growth is needed, keeping ordinary release logarithmic.
        self.entries.retain(|(_, charge)| charge.is_some());
        if self.entries.len() < self.entries.capacity() { return Ok(()); }
        let capacity = self.entries.len().checked_add(1).and_then(usize::checked_next_power_of_two)
            .ok_or(V8BindingError::Allocation("canvas Blob custody capacity"))?;
        let bytes = std::alloc::Layout::array::<CustodyEntry>(capacity)
            .map_err(|_| V8BindingError::Allocation("canvas Blob custody layout"))?.size();
        let storage = allocation.reserve(bytes)
            .map_err(|_| V8BindingError::Allocation("canvas Blob custody storage admission"))?;
        let mut entries = Vec::new();
        entries.try_reserve_exact(capacity)
            .map_err(|_| V8BindingError::Allocation("canvas Blob custody storage allocation"))?;
        if entries.capacity() != capacity {
            return Err(V8BindingError::Allocation("canvas Blob custody allocation extent"));
        }
        // Admission and allocation both succeed before moving any live custody.
        entries.append(&mut self.entries);
        self.entries = entries;
        self.storage = Some(storage);
        Ok(())
    }
    pub(super) fn insert_reserved(&mut self, id: u64, charge: Arc<dyn Send + Sync>) {
        assert!(self.entries.len() < self.entries.capacity());
        assert!(self.entries.last().is_none_or(|(last, _)| *last < id));
        self.entries.push((id, Some(charge)));
    }
}

/// All encoder writes obey the existing 32 MiB serialized-payload ceiling.
/// Growth is fallible and rejected before mutation, including codec headers.
pub(super) struct Encoded {
    pub bytes: Vec<u8>,
    pub charge: Option<Arc<dyn Send + Sync>>,
}
pub(super) struct Output<'a> {
    bytes: Vec<u8>,
    limit: usize,
    allocation: Option<&'a BindingRetainedAllocation>,
    charge: Option<Arc<dyn Send + Sync>>,
}
impl<'a> Output<'a> {
    pub(super) fn new(allocation: Option<&'a BindingRetainedAllocation>) -> Self {
        Self { bytes: Vec::new(), limit: crate::message_wire::MAX_BYTES, allocation, charge: None }
    }
    pub(super) fn finish(self) -> Encoded { Encoded { bytes: self.bytes, charge: self.charge } }
    pub(super) fn len(&self) -> usize { self.bytes.len() }
    pub(super) fn clear(&mut self) { self.bytes.clear(); }
}
impl std::io::Write for Output<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let required = self.bytes.len().checked_add(bytes.len())
            .filter(|size| *size <= self.limit)
            .ok_or_else(|| std::io::Error::other("canvas encoding output limit"))?;
        if required > self.bytes.capacity() {
            let capacity = required.max(self.bytes.capacity().saturating_mul(2)).min(self.limit);
            let charge = self.allocation.map(|allocation| allocation.reserve(capacity))
                .transpose().map_err(|_| std::io::Error::other("canvas encoding output admission"))?;
            self.bytes.try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| std::io::Error::other("canvas encoding output allocation"))?;
            if self.bytes.capacity() != capacity {
                return Err(std::io::Error::other("canvas encoding output allocation extent"));
            }
            self.charge = charge;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Settings {
    pub media_type: &'static str,
    pub quality: u8,
}
impl Settings {
    pub(super) fn new(media_type: &str, quality: Option<f64>) -> Self {
        let media_type = match media_type.to_ascii_lowercase().as_str() {
            "image/jpeg" => "image/jpeg",
            "image/webp" => "image/webp",
            _ => "image/png",
        };
        Self { media_type, quality: quality.filter(|q| q.is_finite() && (0.0..=1.0).contains(q))
            .map_or(92, |q| (q * 100.0).round() as u8) }
    }
}

pub(super) fn options(scope: &mut v8::PinScope, value: v8::Local<v8::Value>)
    -> Result<Settings, V8BindingError>
{
    if value.is_null_or_undefined() { return Ok(Settings::new("image/png", None)); }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        throw_type_error(scope, "ImageEncodeOptions must be a dictionary");
        return Err(V8BindingError::JavaScriptException);
    };
    // Dictionary member conversion is lexicographic, including inherited getters.
    let key = v8::String::new(scope, "quality").ok_or(V8BindingError::Allocation("encoding quality"))?;
    let value = object.get(scope, key.into()).ok_or(V8BindingError::JavaScriptException)?;
    let quality = if value.is_undefined() { None } else {
        let value = value.number_value(scope).ok_or(V8BindingError::JavaScriptException)?;
        if !value.is_finite() {
            throw_type_error(scope, "ImageEncodeOptions quality must be finite");
            return Err(V8BindingError::JavaScriptException);
        }
        Some(value)
    };
    let key = v8::String::new(scope, "type").ok_or(V8BindingError::Allocation("encoding type"))?;
    let value = object.get(scope, key.into()).ok_or(V8BindingError::JavaScriptException)?;
    let media_type = if value.is_undefined() { "image/png".into() } else {
        value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?.to_rust_string_lossy(scope)
    };
    Ok(Settings::new(&media_type, quality))
}

pub(super) fn publish_blob<'s>(scope: &mut v8::PinScope<'s, '_>, state: &V8BindingState,
    bytes: Vec<u8>, media_type: &'static str, charge: Option<Arc<dyn Send + Sync>>)
    -> Result<v8::Local<'s, v8::Object>, V8BindingError>
{
    if state.execution_retired() { return Err(V8BindingError::Allocation("retired canvas Blob owner")); }
    let allocation = state.capture_retained_allocation(scope,
        Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
    let charge = match charge {
        Some(charge) => charge,
        None => {
            allocation.reserve(bytes.capacity())
                .map_err(|_| V8BindingError::Allocation("canvas Blob custody admission"))?
        }
    };
    state.canvas_blob_custody.borrow_mut().reserve(&allocation)?;
    let id = register_platform_object(state, PlatformObjectRecord::Blob(BlobRecord {
        bytes, media_type: media_type.into(), file: None,
    }));
    state.canvas_blob_custody.borrow_mut().insert_reserved(id, charge);
    match new_platform_object(scope, state, "Blob", id) {
        Ok(object) => Ok(object),
        Err(error) => {
            state.platform_objects.borrow_mut().remove(&id);
            state.canvas_blob_custody.borrow_mut().remove(&id);
            state.platform_edges.borrow_mut().discard_unpublished(id);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn encoding_output_limit_rejects_before_mutating_and_keeps_fallback_policy() {
        let mut output = Output { bytes: Vec::new(), limit: 4, allocation: None, charge: None };
        output.write_all(&[1, 2, 3]).unwrap();
        assert!(output.write_all(&[4, 5]).is_err());
        assert_eq!(output.bytes, [1, 2, 3]);
        output.write_all(&[4]).unwrap();
        assert!(output.write_all(&[5]).is_err());
        assert_eq!(output.finish().bytes, [1, 2, 3, 4]);
        assert_eq!(Settings::new("IMAGE/JPEG", Some(0.5)).quality, 50);
        assert_eq!(Settings::new("image/jpeg", Some(f64::NAN)).quality, 92);
        assert_eq!(Settings::new("image/jpeg", Some(2.0)).quality, 92);
        assert_eq!(Settings::new("unrecognized", None).media_type, "image/png");
    }

    #[test]
    fn output_admission_keeps_bytes_owned_and_rejects_retired_growth_atomically() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let allocation = state.capture_retained_allocation(scope,
            Arc::new(std::sync::atomic::AtomicBool::new(false))).unwrap();
        let mut output = Output::new(Some(&allocation));
        output.write_all(&[1, 2, 3]).unwrap();
        assert!(output.charge.is_some());
        state.retire_execution();
        assert!(output.write_all(&[4, 5, 6, 7]).is_err());
        assert_eq!(output.bytes, [1, 2, 3]);
        let encoded = output.finish();
        let charged = state.idb_payload_ledger.live_bytes();
        assert!(encoded.charge.is_some());
        drop(encoded);
        assert!(state.idb_payload_ledger.live_bytes() < charged);
    }

    #[test]
    fn html_encoding_defers_snapshot_delivery_and_preserves_format() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let script = v8::String::new(scope, r#"(()=>{
            globalThis.encoded=[];
            const c=document.createElement('canvas');c.width=1;c.height=1;
            const ctx=c.getContext('2d');ctx.fillStyle='red';ctx.fillRect(0,0,1,1);
            c.toBlob(blob=>encoded.push(blob),'IMAGE/JPEG',1);
            c.toBlob(blob=>encoded.push(blob),'image/webp');
            if(encoded.length)throw Error('inline callback');
            if(!c.toDataURL('image/jpeg',1).startsWith('data:image/jpeg;base64,'))throw Error('JPEG data URL');
            if(!c.toDataURL('image/webp').startsWith('data:image/webp;base64,'))throw Error('WebP data URL');
            c.width=0;
            return true;
        })()"#).unwrap();
        assert!(v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap().is_true());
        let queued_bytes: usize = state.deferred_platform_tasks.borrow().iter().filter_map(|task| {
            match task { DeferredPlatformTask::CanvasBlob { bytes: Some(bytes), _bytes_charge, .. } => {
                assert!(_bytes_charge.is_some()); Some(bytes.capacity())
            }, _ => None }
        }).sum();
        assert!(queued_bytes > 0 && state.idb_payload_ledger.live_bytes() >= queued_bytes);
        state.drain_platform_completions(scope, 16).unwrap();
        let script = v8::String::new(scope, "encoded.length===2 && encoded[0].type==='image/jpeg' && encoded[1].type==='image/webp'").unwrap();
        assert!(v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap().is_true());
        let key = v8::String::new(scope, "encoded").unwrap();
        let root = context.global(scope);
        let values = root.get(scope, key.into()).unwrap().try_cast::<v8::Array>().unwrap();
        for index in 0..2 {
            let object = values.get_index(scope, index).unwrap().to_object(scope).unwrap();
            let owner = binding_state_for_receiver(scope, object).unwrap();
            let id = stateful_id(scope, object).unwrap();
            let objects = owner.platform_objects.borrow();
            let Some(PlatformObjectRecord::Blob(blob)) = objects.get(&id) else { panic!("native Blob delivery"); };
            let image = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
            assert_eq!(image.dimensions(), (1, 1));
            assert!(image.get_pixel(0, 0).0[0] >= 250);
        }
    }

    #[test]
    fn encoded_blob_custody_survives_retirement_until_wrapper_collection() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let before = state.idb_payload_ledger.live_bytes();
        let (id, root) = {
            v8::scope!(let inner, scope);
            let object = publish_blob(inner, &state, vec![1, 2, 3], "image/png", None).unwrap();
            (stateful_id(inner, object).unwrap(), v8::Global::new(inner, object))
        };
        let charged = state.idb_payload_ledger.live_bytes();
        assert!(charged > before && state.canvas_blob_custody.borrow().contains_key(&id));
        state.retire_execution();
        state.process_weak_releases().unwrap();
        assert!(state.platform_objects.borrow().contains_key(&id));
        assert!(state.canvas_blob_custody.borrow().contains_key(&id));
        drop(root);
        for _ in 0..8 {
            scope.low_memory_notification();
            state.process_weak_releases().unwrap();
            if !state.canvas_blob_custody.borrow().contains_key(&id) { break; }
        }
        assert!(!state.canvas_blob_custody.borrow().contains_key(&id));
        assert!(!state.platform_objects.borrow().contains_key(&id));
        assert!(state.idb_payload_ledger.live_bytes() < charged);
    }
}

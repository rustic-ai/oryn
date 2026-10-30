//! Offscreen canvases share the native canvas raster engine through a private,
//! leased canvas surface. The surface never enters the authored DOM or exposes
//! an HTMLElement wrapper; ownership is the OffscreenCanvas platform object.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ContextKind {
    None,
    TwoD,
    BitmapRenderer,
}
pub(super) struct Record {
    pub(super) node: Cell<Option<NodeId>>,
    pub(super) context: Cell<ContextKind>,
    lease: RefCell<Option<oryn_dom::TrackedNodeLease>>,
}
pub(super) type Backing = Rc<Record>;
#[derive(Clone)]
pub(super) struct Source {
    pub(super) owner: Rc<V8BindingState>,
    pub(super) backing: Backing,
}
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "OffscreenCanvas".into(),
        member: name.into(),
    }
}
impl Source {
    pub(super) fn transfer_error(&self) -> Option<&'static str> {
        if self.backing.node.get().is_none() || self.owner.execution_retired() {
            Some("DataCloneError")
        } else if self.backing.context.get() != ContextKind::None {
            Some("InvalidStateError")
        } else {
            None
        }
    }
    pub(super) fn valid(&self) -> bool {
        self.backing.node.get().is_some()
            && self.backing.context.get() == ContextKind::None
            && !self.owner.execution_retired()
    }
    pub(super) fn snapshot(&self) -> Result<DecodedImage, V8BindingError> {
        if self.owner.execution_retired() {
            return Err(error("InvalidStateError"));
        }
        let node = self
            .backing
            .node
            .get()
            .ok_or_else(|| error("InvalidStateError"))?;
        let (width, height) = canvas_dimensions(&self.owner, node)?;
        let renderer = self.owner.bitmap_renderer_surfaces.borrow().image(&node).cloned();
        let mut bitmap = renderer.or_else(|| self
            .owner
            .canvas_images
            .borrow()
            .get(&node)
            .cloned())
            .map(Ok)
            .unwrap_or_else(|| canvas_context_settings::empty(&self.owner, node, width, height))?;
        bitmap.origin_clean &= !canvas_tainted(&self.owner, node);
        Ok(bitmap)
    }
    pub(super) fn charged_snapshot(&self, allocation: &BindingRetainedAllocation)
        -> Result<image_bitmap::Snapshot, V8BindingError>
    {
        if self.owner.execution_retired() { return Err(error("InvalidStateError")); }
        let node = self.backing.node.get().ok_or_else(|| error("InvalidStateError"))?;
        let clean = !canvas_tainted(&self.owner, node);
        let renderers = self.owner.bitmap_renderer_surfaces.borrow();
        let canvases = self.owner.canvas_images.borrow();
        if let Some(image) = renderers.image(&node).or_else(|| canvases.get(&node)) {
            image_bitmap::Snapshot::copy(allocation, image, clean)
        } else {
            let (width, height) = canvas_dimensions(&self.owner, node)?;
            if width == 0 || height == 0 {
                return Err(error("InvalidStateError"));
            }
            let pixels = allocation.reserve(image_data_size(width, height)?)
                .map_err(|_| V8BindingError::Allocation("offscreen source snapshot admission"))?;
            let mut image = canvas_context_settings::empty(&self.owner, node, width, height)?;
            image.origin_clean &= clean;
            image_bitmap::Snapshot::owned(allocation, image, pixels)
        }
    }
    pub(super) fn detach(&self) {
        if let Some(node) = self.backing.node.take() {
            reset_canvas_state(&self.owner, node);
        }
        self.backing.lease.borrow_mut().take();
    }
}
pub(super) fn identify(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<Source> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let owner = binding_state_for_receiver(scope, object)?;
    // DOM wrappers use a BigInt node identity. This is a non-coercing brand
    // probe: passing those identities to stateful_id would leave a pending
    // ToNumber exception even when this probe subsequently returns None.
    if object.internal_field_count() == 0
        || !object.get_internal_field(scope, 0)?.is_number()
    {
        return None;
    }
    let id = stateful_id(scope, object).ok()?;
    if !owner
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value))
    {
        return None;
    }
    let backing = match owner.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::OffscreenCanvas(backing)) => backing.clone(),
        _ => return None,
    };
    Some(Source { owner, backing })
}
fn record(state: &V8BindingState, width: u32, height: u32) -> Result<Backing, V8BindingError> {
    let node = state.dom.borrow_mut().create_element("canvas");
    state
        .dom
        .borrow_mut()
        .set_attribute(node, "width", width.to_string())?;
    state
        .dom
        .borrow_mut()
        .set_attribute(node, "height", height.to_string())?;
    let lease = oryn_dom::TrackedNodeLease::new(&state.dom, node)?;
    Ok(Rc::new(Record {
        node: Cell::new(Some(node)),
        context: Cell::new(ContextKind::None),
        lease: RefCell::new(Some(lease)),
    }))
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    if args.length() < 2 {
        throw_type_error(scope, "OffscreenCanvas requires width and height");
        return Err(V8BindingError::JavaScriptException);
    }
    let width = dimension(scope, args.get(0))?;
    let height = dimension(scope, args.get(1))?;
    let backing = record(state, width, height)?;
    let id = register_platform_object(state, PlatformObjectRecord::OffscreenCanvas(backing));
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn dimension(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<u32, V8BindingError> {
    let value = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !value.is_finite() || value.trunc() < 0. || value.trunc() > u32::MAX as f64 {
        throw_type_error(
            scope,
            "OffscreenCanvas dimension is outside the unsigned range",
        );
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(value.trunc() as u32)
}
pub(super) fn allocate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    bitmap: DecodedImage,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if (bitmap.width as usize)
        .checked_mul(bitmap.height as usize)
        .and_then(|n| n.checked_mul(4))
        != Some(bitmap.rgba.len())
        || bitmap.rgba.len() > crate::message_wire::MAX_BYTES
    {
        return Err(error("InvalidStateError"));
    }
    let backing = record(state, bitmap.width, bitmap.height)?;
    let node = backing.node.get().unwrap();
    if !bitmap.origin_clean {
        state.tainted_canvases.borrow_mut().insert(node);
    }
    state.canvas_images.borrow_mut().insert(node, bitmap);
    let id = register_platform_object(state, PlatformObjectRecord::OffscreenCanvas(backing));
    new_platform_object(scope, state, "OffscreenCanvas", id)
}
fn member(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    if args.data().int32_value(scope) == Some(6) {
        convert_to_blob(scope, args, rv);
        return;
    }
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let Some(source) = identify(scope, args.this().into()) else {
            throw_type_error(scope, "OffscreenCanvas receiver required");
            return Err(V8BindingError::JavaScriptException);
        };
        let operation = args.data().int32_value(scope).unwrap_or(-1);
        if matches!(operation, 0 | 1) {
            let dimensions = source
                .backing
                .node
                .get()
                .map(|node| canvas_dimensions(&source.owner, node))
                .transpose()?
                .unwrap_or((0, 0));
            return Ok(Some(
                v8::Integer::new_from_unsigned(
                    scope,
                    if operation == 0 {
                        dimensions.0
                    } else {
                        dimensions.1
                    },
                )
                .into(),
            ));
        }
        let node = source
            .backing
            .node
            .get()
            .ok_or_else(|| error("InvalidStateError"))?;
        match operation {
            2 | 3 => {
                let dimension = dimension(scope, args.get(0))?;
                // Numeric conversion may run author code that transfers the
                // source or retires its execution. Never write the pre-conversion
                // leased NodeId after that ownership has disappeared.
                if source.owner.execution_retired() || source.backing.node.get() != Some(node) {
                    return Err(error("InvalidStateError"));
                }
                source.owner.dom.borrow_mut().set_attribute(
                    node,
                    if operation == 2 { "width" } else { "height" },
                    dimension.to_string(),
                )?;
                reset_canvas_state(&source.owner, node);
                Ok(None)
            }
            4 => {
                let kind = args
                    .get(0)
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                if kind != "2d" && kind != "bitmaprenderer" {
                    return Ok(Some(v8::null(scope).into()));
                }
                if kind == "bitmaprenderer" {
                    if source.backing.context.get() == ContextKind::TwoD {
                        return Ok(Some(v8::null(scope).into()));
                    }
                    let owner = args.this();
                    if let Some(context) = bitmap_renderer::cached(scope, owner)? {
                        return Ok(Some(context));
                    }
                    // Convert the sole dictionary member before committing
                    // ownership; getters may create a competing 2D context.
                    let alpha = checked_dictionary_member(scope, args.get(1), "alpha")?
                        .map_or(true, |value| value.boolean_value(scope));
                    let source = identify(scope, owner.into()).ok_or_else(|| error("InvalidStateError"))?;
                    if source.backing.node.get().is_none() || source.owner.execution_retired() {
                        return Err(error("InvalidStateError"));
                    }
                    if source.backing.context.get() == ContextKind::TwoD {
                        return Ok(Some(v8::null(scope).into()));
                    }
                    if let Some(context) = bitmap_renderer::cached(scope, owner)? {
                        return Ok(Some(context));
                    }
                    let context = bitmap_renderer::create(scope, &source.owner, owner, alpha)?;
                    source.backing.context.set(ContextKind::BitmapRenderer);
                    return Ok(Some(context.into()));
                }
                if source.backing.context.get() == ContextKind::BitmapRenderer {
                    return Ok(Some(v8::null(scope).into()));
                }
                let owner = args.this();
                let name = v8::String::new(scope, "oryn.canvas.context2d")
                    .ok_or(V8BindingError::Allocation("offscreen cache"))?;
                let key = v8::Private::for_api(scope, Some(name));
                if let Some(context) = owner
                    .get_private(scope, key)
                    .filter(|value| value.is_object())
                {
                    return Ok(Some(context));
                }
                let settings = canvas_context_settings::parse(scope, args.get(1))?;
                let source = identify(scope, owner.into()).ok_or_else(|| error("InvalidStateError"))?;
                let node = source.backing.node.get().ok_or_else(|| error("InvalidStateError"))?;
                // Settings getters can create another context reentrantly.
                // Honor the kind that won before constructing any new wrapper.
                if source.backing.context.get() == ContextKind::BitmapRenderer {
                    return Ok(Some(v8::null(scope).into()));
                }
                if let Some(context) = owner.get_private(scope, key).filter(|value| value.is_object()) {
                    return Ok(Some(context));
                }
                let template = source
                    .owner
                    .instance_templates
                    .borrow()
                    .get("OffscreenCanvasRenderingContext2D")
                    .and_then(|template| template.to_global(scope))
                    .ok_or(V8BindingError::MissingInterface(
                        "OffscreenCanvasRenderingContext2D",
                    ))?;
                let context = v8::Local::new(scope, &template)
                    .new_instance(scope)
                    .ok_or(V8BindingError::Allocation("offscreen context"))?;
                let name = v8::String::new(scope, "oryn.canvas.owner")
                    .ok_or(V8BindingError::Allocation("offscreen owner"))?;
                let owner_key = v8::Private::for_api(scope, Some(name));
                if context.set_private(scope, owner_key, owner.into()) != Some(true)
                    || owner.set_private(scope, key, context.into()) != Some(true)
                {
                    return Err(V8BindingError::Allocation("offscreen context ownership"));
                }
                source.backing.context.set(ContextKind::TwoD);
                source.owner.canvas_context_settings.borrow_mut().insert(node, settings);
                Ok(Some(context.into()))
            }
            5 => {
                if source.backing.context.get() == ContextKind::None {
                    return Err(error("InvalidStateError"));
                }
                let allocation = source.owner.capture_retained_allocation(scope,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
                let bitmap = source.charged_snapshot(&allocation)?;
                if bitmap.width == 0 || bitmap.height == 0 {
                    return Err(error("InvalidStateError"));
                }
                let object = image_bitmap::allocate_snapshot(scope, &source.owner, bitmap, &allocation)?;
                source.owner.canvas_images.borrow_mut().remove(&node);
                source.owner.bitmap_renderer_surfaces.borrow_mut().clear_pixels(&node);
                source.owner.tainted_canvases.borrow_mut().remove(&node);
                Ok(Some(object.into()))
            }
            _ => Ok(None),
        }
    })();
    match result {
        Ok(Some(value)) => rv.set(value),
        Ok(None) => {}
        Err(error) => throw_binding_error(scope, &error),
    }
}
// Promise-returning WebIDL operations reject even for an invalid receiver.
// Create the promise in the calling realm before entering the conversion scope;
// preserve author exceptions rather than relabeling every failure SecurityError.
fn convert_to_blob(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else { return; };
    rv.set(resolver.get_promise(scope).into());
    v8::tc_scope!(let tc, scope);
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let Some(source) = identify(tc, args.this().into()) else {
            throw_type_error(tc, "OffscreenCanvas receiver required");
            return Err(V8BindingError::JavaScriptException);
        };
        if source.owner.execution_retired() {
            return Err(error("InvalidStateError"));
        }
        let settings = canvas_encoding::options(tc, args.get(0))?;
        // Option getters can transfer or retire the source. Revalidate before
        // reading native pixels, taint, dimensions or allocating encoded output.
        if source.owner.execution_retired() { return Err(error("InvalidStateError")); }
        let node = source.backing.node.get().ok_or_else(|| error("InvalidStateError"))?;
        let (width, height) = canvas_output_dimensions(&source.owner, node)?;
        if canvas_tainted(&source.owner, node) {
            return Err(error("SecurityError"));
        }
        if width == 0 || height == 0 {
            return Err(error("IndexSizeError"));
        }
        let allocation = source.owner.capture_retained_allocation(tc,
            Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
        let encoded = canvas_encode(&source.owner, node, settings, Some(&allocation))?
            .ok_or_else(|| error("EncodingError"))?;
        canvas_encoding::publish_blob(tc, &source.owner, encoded.bytes, settings.media_type, encoded.charge)
    })();
    match result {
        Ok(blob) => { resolver.resolve(tc, blob.into()); }
        Err(error) => {
            let exception = if tc.has_caught() {
                tc.exception()
            } else {
                match &error {
                    V8BindingError::Unsupported { member, .. } => dom_exception_value(tc, member).ok(),
                    _ => dom_exception_value(tc, "EncodingError").ok(),
                }
            };
            tc.reset();
            if let Some(exception) = exception { resolver.reject(tc, exception); }
        }
    }
}
pub(super) fn install_members(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    for (index, name) in [(0, "width"), (1, "height")] {
        let data = v8::Integer::new(scope, index);
        let getter = v8::Function::builder(member)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("offscreen getter"))?;
        let data = v8::Integer::new(scope, index + 2);
        let setter = v8::Function::builder(member)
            .data(data.into())
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("offscreen setter"))?;
        let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        let key =
            v8::String::new(scope, name).ok_or(V8BindingError::Allocation("offscreen key"))?;
        if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(V8BindingError::Allocation("offscreen descriptor"));
        }
    }
    for (index, name, arity) in [
        (4, "getContext", 1),
        (5, "transferToImageBitmap", 0),
        (6, "convertToBlob", 0),
    ] {
        let data = v8::Integer::new(scope, index);
        let function = v8::Function::builder(member)
            .data(data.into())
            .length(arity)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("offscreen method"))?;
        set_object_property(scope, prototype, name, function.into())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_kinds_block_transfer_without_coercing_other_interfaces() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<canvas id='surface'></canvas>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let script = v8::String::new(scope, "new OffscreenCanvas(2,3)").unwrap();
        let object = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        let source = identify(scope, object).unwrap();
        assert!(source.valid());
        assert_eq!(source.transfer_error(), None);
        for kind in [ContextKind::TwoD, ContextKind::BitmapRenderer] {
            source.backing.context.set(kind);
            assert!(!source.valid());
            assert_eq!(source.transfer_error(), Some("InvalidStateError"));
        }
        source.backing.context.set(ContextKind::None);
        source.detach();
        assert!(!source.valid());
        assert_eq!(source.transfer_error(), Some("DataCloneError"));

        // A DOM canvas has a BigInt node identity, not a platform-object ID.
        // The wrong-interface probe must not invoke its ToNumber conversion.
        let script = v8::String::new(scope, "document.getElementById('surface')").unwrap();
        let object = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        v8::tc_scope!(let tc, scope);
        assert!(image_bitmap::identify(tc, object).is_none());
        assert!(!tc.has_caught());
        assert!(identify(tc, object).is_none());
        assert!(!tc.has_caught());
    }

    #[test]
    fn dimension_conversion_cannot_mutate_a_transferred_source() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let script = v8::String::new(scope, r#"(()=>{
            for(const key of ['width','height']) {
                const source=new OffscreenCanvas(2,3);let moved;
                try {
                    source[key]={valueOf(){moved=structuredClone(source,{transfer:[source]});return 9}};
                    throw Error('detached setter accepted');
                } catch(e) { if(e.name!=='InvalidStateError')throw e; }
                if(source.width!==0 || source.height!==0 || moved.width!==2 || moved.height!==3)
                    throw Error('conversion changed transferred dimensions');
            }
            return true;
        })()"#).unwrap();
        let result = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(result.is_true());
    }

    #[test]
    fn offscreen_encoding_rejects_receiver_detachment_and_empty_dimensions() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let script = v8::String::new(scope, r#"(()=>{
            const encode=OffscreenCanvas.prototype.convertToBlob;
            const detached=new OffscreenCanvas(1,1);
            structuredClone(detached,{transfer:[detached]});
            const surface=new OffscreenCanvas(1,1),ctx=surface.getContext('2d');
            ctx.fillStyle='red';ctx.fillRect(0,0,1,1);
            const moving=new OffscreenCanvas(1,1),order=[];
            const promises=[encode.call({}),new OffscreenCanvas(0,1).convertToBlob(),
                detached.convertToBlob(),surface.convertToBlob(),
                surface.convertToBlob({type:'IMAGE/JPEG',quality:1}),
                surface.convertToBlob({type:'image/webp'}),
                surface.convertToBlob({type:'unsupported'}),
                surface.convertToBlob({quality:Infinity}),
                moving.convertToBlob({get quality(){order.push('quality');return 0.5},
                    get type(){order.push('type');structuredClone(moving,{transfer:[moving]});return 'image/png'}})];
            if(order.join()!=='quality,type')throw Error('dictionary conversion order');
            for(const promise of promises) {
                if(!(promise instanceof Promise))throw Error('encoding did not return caller Promise');
                promise.catch(()=>{});
            }
            surface.width=0;
            return promises;
        })()"#).unwrap();
        let values = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        let values = v8::Local::<v8::Array>::try_from(values).unwrap();
        for (index, expected) in ["TypeError", "IndexSizeError", "InvalidStateError"].into_iter().enumerate() {
            let value = values.get_index(scope, index as u32).unwrap();
            let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
            assert_eq!(promise.state(), v8::PromiseState::Rejected);
            let reason = promise.result(scope).to_object(scope).unwrap();
            let key = v8::String::new(scope, "name").unwrap();
            let name = reason.get(scope, key.into()).unwrap();
            assert_eq!(name.to_rust_string_lossy(scope), expected);
        }
        let value = values.get_index(scope, 3).unwrap();
        let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        let blob = promise.result(scope).to_object(scope).unwrap();
        let owner = binding_state_for_receiver(scope, blob).unwrap();
        let id = stateful_id(scope, blob).unwrap();
        let objects = owner.platform_objects.borrow();
        let Some(PlatformObjectRecord::Blob(blob)) = objects.get(&id) else { panic!("native encoding Blob"); };
        assert_eq!(blob.media_type, "image/png");
        let decoded = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (1, 1));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        drop(objects);
        for (index, media_type) in [(4, "image/jpeg"), (5, "image/webp"), (6, "image/png")] {
            let value = values.get_index(scope, index).unwrap();
            let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
            assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
            let object = promise.result(scope).to_object(scope).unwrap();
            let owner = binding_state_for_receiver(scope, object).unwrap();
            let id = stateful_id(scope, object).unwrap();
            let objects = owner.platform_objects.borrow();
            let Some(PlatformObjectRecord::Blob(blob)) = objects.get(&id) else { panic!("encoded Blob"); };
            assert_eq!(blob.media_type, media_type);
            let decoded = image::load_from_memory(&blob.bytes).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (1, 1));
            let pixel = decoded.get_pixel(0, 0).0;
            assert!(pixel[0] >= 250 && pixel[1] <= 5 && pixel[2] <= 5);
        }
        for (index, expected) in [(7, "TypeError"), (8, "InvalidStateError")] {
            let value = values.get_index(scope, index).unwrap();
            let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
            assert_eq!(promise.state(), v8::PromiseState::Rejected);
            let reason = promise.result(scope).to_object(scope).unwrap();
            let key = v8::String::new(scope, "name").unwrap();
            let name = reason.get(scope, key.into()).unwrap();
            assert_eq!(name.to_rust_string_lossy(scope), expected);
        }
    }
}

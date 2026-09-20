//! Offscreen canvases share the native canvas raster engine through a private,
//! leased canvas surface. The surface never enters the authored DOM or exposes
//! an HTMLElement wrapper; ownership is the OffscreenCanvas platform object.
use super::*;
pub(super) struct Record {
    pub(super) node: Cell<Option<NodeId>>,
    context: Cell<bool>,
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
        } else if self.backing.context.get() {
            Some("InvalidStateError")
        } else {
            None
        }
    }
    pub(super) fn valid(&self) -> bool {
        self.backing.node.get().is_some()
            && !self.backing.context.get()
            && !self.owner.execution_retired()
    }
    pub(super) fn snapshot(&self) -> Result<DecodedImage, V8BindingError> {
        let node = self
            .backing
            .node
            .get()
            .ok_or_else(|| error("InvalidStateError"))?;
        let (width, height) = canvas_dimensions(&self.owner, node)?;
        let mut bitmap = self
            .owner
            .canvas_images
            .borrow()
            .get(&node)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| empty_canvas_bitmap(width, height))?;
        bitmap.origin_clean &= !self.owner.tainted_canvases.borrow().contains(&node);
        Ok(bitmap)
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
        context: Cell::new(false),
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
                if kind != "2d" {
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
                source.backing.context.set(true);
                Ok(Some(context.into()))
            }
            5 => {
                if !source.backing.context.get() {
                    return Err(error("InvalidStateError"));
                }
                let bitmap = source.snapshot()?;
                if bitmap.width == 0 || bitmap.height == 0 {
                    return Err(error("InvalidStateError"));
                }
                let object = image_bitmap::allocate(scope, &source.owner, bitmap)?;
                source.owner.canvas_images.borrow_mut().remove(&node);
                source.owner.tainted_canvases.borrow_mut().remove(&node);
                Ok(Some(object.into()))
            }
            6 => {
                let resolver = v8::PromiseResolver::new(scope)
                    .ok_or(V8BindingError::Allocation("offscreen encoding"))?;
                let promise = resolver.get_promise(scope);
                match canvas_png(&source.owner, node) {
                    Ok(Some(bytes)) => {
                        let id = register_platform_object(
                            &source.owner,
                            PlatformObjectRecord::Blob(BlobRecord {
                                bytes,
                                media_type: "image/png".into(),
                                file: None,
                            }),
                        );
                        let blob = new_platform_object(scope, &source.owner, "Blob", id)?;
                        resolver.resolve(scope, blob.into());
                    }
                    Ok(None) => {
                        let exception = dom_exception_value(scope, "IndexSizeError")?;
                        resolver.reject(scope, exception);
                    }
                    Err(_) => {
                        let exception = dom_exception_value(scope, "SecurityError")?;
                        resolver.reject(scope, exception);
                    }
                }
                Ok(Some(promise.into()))
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

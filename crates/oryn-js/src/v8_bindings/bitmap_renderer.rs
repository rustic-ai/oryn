//! Bitmap rendering moves the immutable native image into the canvas surface.
//! Context and owner keep each other alive through private V8 edges, not roots.
use super::*;

pub(super) type SurfaceEntry = (NodeId, Option<DecodedImage>, Option<Arc<dyn Send + Sync>>, bool);
#[derive(Default)]
pub(super) struct Surfaces {
    entries: Vec<SurfaceEntry>,
    storage: Option<Arc<dyn Send + Sync>>,
    revision: u64,
}
impl Surfaces {
    pub(super) fn contains(&self, node: &NodeId) -> bool {
        self.entries.iter().any(|(key, _, _, _)| key == node)
    }
    pub(super) fn reserve(&mut self, allocation: &BindingRetainedAllocation, additional: usize)
        -> Result<(), V8BindingError>
    {
        let required = self.entries.len().checked_add(additional)
            .ok_or(V8BindingError::Allocation("renderer surface count"))?;
        if required <= self.entries.capacity() { return Ok(()); }
        let capacity = required.checked_next_power_of_two()
            .ok_or(V8BindingError::Allocation("renderer surface capacity"))?;
        let bytes = std::alloc::Layout::array::<SurfaceEntry>(capacity)
            .map_err(|_| V8BindingError::Allocation("renderer surface layout"))?.size();
        let storage = allocation.reserve(bytes)
            .map_err(|_| V8BindingError::Allocation("renderer surface storage admission"))?;
        let mut entries = Vec::new();
        entries.try_reserve_exact(capacity)
            .map_err(|_| V8BindingError::Allocation("renderer surface storage"))?;
        if entries.capacity() != capacity {
            return Err(V8BindingError::Allocation("renderer surface storage extent"));
        }
        entries.append(&mut self.entries);
        self.entries = entries;
        self.storage = Some(storage);
        Ok(())
    }
    pub(super) fn insert_reserved(&mut self, node: NodeId, image: Option<DecodedImage>, charge: Option<Arc<dyn Send + Sync>>, alpha: bool) {
        self.revision = self.revision.saturating_add(1);
        if let Some((_, current_image, current_charge, _)) = self.entries.iter_mut().find(|(key, _, _, _)| *key == node) {
            *current_image = image;
            *current_charge = charge;
        } else {
            assert!(self.entries.len() < self.entries.capacity());
            self.entries.push((node, image, charge, alpha));
        }
    }
    pub(super) fn take(&mut self, node: &NodeId) -> Option<SurfaceEntry> {
        let index = self.entries.iter().position(|(key, _, _, _)| key == node)?;
        self.revision = self.revision.saturating_add(1);
        Some(self.entries.swap_remove(index))
    }
    pub(super) fn clear_pixels(&mut self, node: &NodeId) {
        if let Some((_, image, charge, _)) = self.entries.iter_mut().find(|(key, _, _, _)| key == node) {
            self.revision = self.revision.saturating_add(1);
            image.take();
            charge.take();
        }
    }
    pub(super) fn has_pixels(&self, node: &NodeId) -> bool {
        self.entries.iter().any(|(key, _, charge, _)| key == node && charge.is_some())
    }
    pub(super) fn image(&self, node: &NodeId) -> Option<&DecodedImage> {
        self.entries.iter().find(|(key, _, _, _)| key == node)?.1.as_ref()
    }
    pub(super) fn images(&self) -> impl Iterator<Item = (&NodeId, &DecodedImage)> {
        self.entries.iter().filter_map(|(node, image, _, _)| image.as_ref().map(|image| (node, image)))
    }
    pub(super) fn opaque(&self, node: &NodeId) -> bool {
        self.entries.iter().any(|(key, _, _, alpha)| key == node && !alpha)
    }
    pub(super) fn retain(&mut self, mut keep: impl FnMut(&NodeId) -> bool) {
        let count = self.entries.len();
        self.entries.retain(|(node, _, _, _)| keep(node));
        if self.entries.len() != count { self.revision = self.revision.saturating_add(1); }
    }
    pub(super) fn paint_revision(&self) -> Result<u64, V8BindingError> {
        if self.revision == u64::MAX {
            return Err(V8BindingError::Allocation("renderer paint revision exhausted"));
        }
        Ok(self.revision)
    }
}

fn key<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str)
    -> Result<v8::Local<'s, v8::Private>, V8BindingError>
{
    let name = v8::String::new(scope, name)
        .ok_or(V8BindingError::Allocation("bitmap renderer key"))?;
    Ok(v8::Private::for_api(scope, Some(name)))
}

pub(super) fn cached<'s>(scope: &mut v8::PinScope<'s, '_>, owner: v8::Local<v8::Object>)
    -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError>
{
    let cache = key(scope, "oryn.canvas.bitmaprenderer")?;
    Ok(owner.get_private(scope, cache).filter(|value| value.is_object()))
}

pub(super) fn create<'s>(scope: &mut v8::PinScope<'s, '_>, state: &V8BindingState,
    owner: v8::Local<v8::Object>, alpha: bool) -> Result<v8::Local<'s, v8::Object>, V8BindingError>
{
    let allocation = state.capture_retained_allocation(scope,
        Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
    state.bitmap_renderer_surfaces.borrow_mut().reserve(&allocation, 1)?;
    let template = state.instance_templates.borrow().get("ImageBitmapRenderingContext")
        .and_then(|template| template.to_global(scope))
        .ok_or(V8BindingError::MissingInterface("ImageBitmapRenderingContext"))?;
    let context = v8::Local::new(scope, &template).new_instance(scope)
        .ok_or(V8BindingError::Allocation("bitmap renderer context"))?;
    let owner_key = key(scope, "oryn.canvas.owner")?;
    let cache_key = key(scope, "oryn.canvas.bitmaprenderer")?;
    if context.set_private(scope, owner_key, owner.into()) != Some(true)
        || owner.set_private(scope, cache_key, context.into()) != Some(true)
    {
        return Err(V8BindingError::Allocation("bitmap renderer ownership"));
    }
    let node = if let Some(source) = offscreen_canvas::identify(scope, owner.into()) {
        source.backing.node.get().ok_or(V8BindingError::Allocation("bitmap renderer surface"))?
    } else {
        node_identity::argument(scope, owner.into())?
            .ok_or(V8BindingError::Allocation("bitmap renderer surface owner"))?.1
    };
    state.bitmap_renderer_surfaces.borrow_mut().insert_reserved(node, None, None, alpha);
    Ok(context)
}

fn member(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let owner = canvas_context_owner(scope, args.this()).ok_or_else(|| {
            throw_type_error(scope, "ImageBitmapRenderingContext receiver required");
            V8BindingError::JavaScriptException
        })?;
        // A 2D context has the same owner edge. Only the exact cached renderer
        // wrapper is entitled to consume a bitmap or access this getter.
        if !cached(scope, owner)?.is_some_and(|value| value.strict_equals(args.this().into())) {
            throw_type_error(scope, "ImageBitmapRenderingContext receiver required");
            return Err(V8BindingError::JavaScriptException);
        }
        if args.data().int32_value(scope) == Some(0) {
            return Ok(Some(owner.into()));
        }
        if args.length() == 0 {
            throw_type_error(scope, "transferFromImageBitmap requires an argument");
            return Err(V8BindingError::JavaScriptException);
        }
        let backing = if args.get(0).is_null_or_undefined() { None } else {
            Some(image_bitmap::identify(scope, args.get(0)).ok_or_else(|| {
                throw_type_error(scope, "ImageBitmap argument required");
                V8BindingError::JavaScriptException
            })?)
        };
        let (state, node) = if let Some(source) = offscreen_canvas::identify(scope, owner.into()) {
            let node = source.backing.node.get().ok_or(V8BindingError::Unsupported {
                interface: "OffscreenCanvas".into(), member: "InvalidStateError".into()
            })?;
            (source.owner, node)
        } else {
            node_identity::argument(scope, owner.into())?
                .ok_or(V8BindingError::Allocation("bitmap renderer owner"))?
        };
        if state.execution_retired() {
            return Err(V8BindingError::Unsupported {
                interface: "ImageBitmapRenderingContext".into(), member: "InvalidStateError".into()
            });
        }
        if let Some(backing) = backing {
            // Validate before taking: detached inputs leave the prior canvas
            // bitmap and taint unchanged. No pixel allocation or copy occurs.
            let mut source = backing.borrow_mut();
            if source.as_ref().is_none_or(|image| !image_bitmap::valid(image)) {
                return Err(V8BindingError::Unsupported {
                    interface: "ImageBitmapRenderingContext".into(), member: "InvalidStateError".into()
                });
            }
            let allocation = state.capture_retained_allocation(scope,
                Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
            let charge = allocation.reserve(source.as_ref().unwrap().rgba.capacity())
                .map_err(|_| V8BindingError::Allocation("bitmap renderer pixel admission"))?;
            // Keep the prior destination charge live until its bytes have been
            // replaced. Admission failure above cannot detach the input.
            let image = source.take().unwrap();
            state.bitmap_renderer_surfaces.borrow_mut().insert_reserved(node, Some(image), Some(charge), true);
            drop(source);
            image_bitmap::release_consumed(scope, args.get(0));
        } else {
            // Null restores the transparent bitmap at authored dimensions;
            // the shared source path supplies that empty surface lazily.
            state.canvas_images.borrow_mut().remove(&node);
            state.bitmap_renderer_surfaces.borrow_mut().clear_pixels(&node);
            state.tainted_canvases.borrow_mut().remove(&node);
        }
        Ok(None)
    })();
    match result { Ok(Some(value)) => rv.set(value), Ok(None) => {},
        Err(error) => throw_binding_error(scope, &error) }
}

pub(super) fn install_members(scope: &mut v8::PinScope, prototype: v8::Local<v8::Object>)
    -> Result<(), V8BindingError>
{
    for (index, name, arity) in [(0, "canvas", 0), (1, "transferFromImageBitmap", 1)] {
        let data = v8::Integer::new(scope, index);
        let function = v8::Function::builder(member).data(data.into()).length(arity)
            .constructor_behavior(v8::ConstructorBehavior::Throw).build(scope)
            .ok_or(V8BindingError::Allocation("bitmap renderer member"))?;
        let function_name = v8::String::new(scope, if index == 0 { "get canvas" } else { name })
            .ok_or(V8BindingError::Allocation("bitmap renderer function name"))?;
        function.set_name(function_name);
        if index == 0 {
            let undefined = v8::undefined(scope);
            let mut descriptor = v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let name = v8::String::new(scope, name)
                .ok_or(V8BindingError::Allocation("bitmap renderer member name"))?;
            if prototype.define_property(scope, name.into(), &descriptor) != Some(true) {
                return Err(V8BindingError::Allocation("bitmap renderer getter"));
            }
        } else { set_object_property(scope, prototype, name, function.into())?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_transfer_preserves_intrinsic_pixels_and_exclusive_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let state = scope.get_current_context().get_slot::<V8BindingState>().unwrap();
        let allocation = Arc::new(state.capture_retained_allocation(scope,
            Arc::new(std::sync::atomic::AtomicBool::new(false))).unwrap());
        state.install_retained_allocation(allocation.clone()).unwrap();
        let _allocation_scope = allocation.scope_guard();
        let bitmap = image_bitmap::allocate(scope, &state, DecodedImage {
            width: 2, height: 1, origin_clean: true,
            rgba: vec![0,255,0,128,255,0,0,255],
        }).unwrap();
        let bitmap_id = stateful_id(scope, bitmap).unwrap();
        assert!(state.image_bitmap_custody.borrow().contains_key(&bitmap_id));
        let global = context.global(scope);
        set_object_property(scope, global, "inputBitmap", bitmap.into()).unwrap();
        let script = v8::String::new(scope, r#"(()=>{
            function check(value,reason){if(!value)throw Error(reason)}
            const c=document.createElement('canvas');c.width=20;c.height=30;
            const ctx=c.getContext('bitmaprenderer',{alpha:false});
            check(ctx instanceof ImageBitmapRenderingContext,'native interface');
            check(ctx.canvas===c,'owner');
            try{CanvasRenderingContext2D.prototype.fillRect.call(ctx,0,0,1,1);throw Error('2d brand accepted')}
            catch(e){check(e.name==='TypeError','2d brand error')}
            const canvasGetter=Object.getOwnPropertyDescriptor(CanvasRenderingContext2D.prototype,'canvas').get;
            try{canvasGetter.call(ctx);throw Error('2d getter brand accepted')}
            catch(e){check(e.name==='TypeError','2d getter brand error')}
            check(c.getContext('bitmaprenderer')===ctx,'identity');
            check(c.getContext('2d')===null,'exclusive renderer');
            ctx.transferFromImageBitmap(inputBitmap);
            globalThis.transferredCanvas=c;
            check(inputBitmap.width===0 && inputBitmap.height===0,'consume');
            check(c.width===20 && c.height===30,'author dimensions unchanged');
            const d=document.createElement('canvas');d.width=3;d.height=2;
            const draw=d.getContext('2d');draw.drawImage(c,0,0);
            check(Array.from(draw.getImageData(0,0,1,1).data).join()==='0,255,0,128','source alpha');
            check(Array.from(draw.getImageData(2,0,1,1).data).join()==='0,0,0,0','intrinsic extent');
            try{ctx.transferFromImageBitmap(inputBitmap);throw Error('closed accepted')}
            catch(e){check(e.name==='InvalidStateError','closed error')}
            try{ImageBitmapRenderingContext.prototype.transferFromImageBitmap.call(draw,null);throw Error('brand accepted')}
            catch(e){check(e.name==='TypeError','brand error')}
            const two=document.createElement('canvas');two.getContext('2d');
            check(two.getContext('bitmaprenderer')===null,'exclusive 2d');
            const reentrant=document.createElement('canvas');
            check(reentrant.getContext('bitmaprenderer',{get alpha(){reentrant.getContext('2d');return true}})===null,'reentrant kind');
            const off=new OffscreenCanvas(4,5),oc=off.getContext('bitmaprenderer');
            check(oc.canvas===off && off.getContext('2d')===null,'offscreen ownership');
            check(off.getContext('bitmaprenderer')===oc,'offscreen identity');
            return true;
        })()"#).unwrap();
        let value = v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(value.is_true());
        let name = v8::String::new(scope, "transferredCanvas").unwrap();
        let canvas = global.get(scope, name.into()).unwrap();
        let (owner, node) = node_identity::argument(scope, canvas).unwrap().unwrap();
        assert!(owner.bitmap_renderer_surfaces.borrow().has_pixels(&node));
        assert!(!state.image_bitmap_custody.borrow().contains_key(&bitmap_id));
        let bytes = canvas_png(&owner, node).unwrap().unwrap();
        let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (2, 1));
        assert_eq!(pixels.into_raw(), vec![0,255,0,128,255,0,0,255]);
        assert!(owner.bitmap_renderer_surfaces.borrow().opaque(&node));
        let painted_revision = owner.bitmap_renderer_surfaces.borrow().paint_revision().unwrap();
        let installed = owner.retained_allocation.borrow_mut().take();
        assert!(native_drag_image::snapshot_native_paint_resources(&owner, node).is_err());
        *owner.retained_allocation.borrow_mut() = installed;
        let resources = native_drag_image::snapshot_native_paint_resources(&owner, node).unwrap();
        let resource = resources.iter().find(|resource| resource.node == node).unwrap();
        let pixels = image::load_from_memory(&resource.png).unwrap().to_rgba8();
        assert_eq!(pixels.into_raw(), vec![0,128,0,255,255,0,0,255]);
        let custody = resource.native_custody.clone().unwrap();
        let weak = Arc::downgrade(&custody.0);
        drop(resources);
        assert!(weak.upgrade().is_some());
        drop(custody);
        assert!(weak.upgrade().is_none());
        // Presentation capture must not alter subsequent source/serialization.
        let bytes = canvas_png(&owner, node).unwrap().unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8().into_raw(),
            vec![0,255,0,128,255,0,0,255]);
        assert_eq!(owner.bitmap_renderer_surfaces.borrow().paint_revision().unwrap(), painted_revision);
        let tainted = image_bitmap::allocate(scope, &state, DecodedImage {
            width: 1, height: 1, origin_clean: false, rgba: vec![9,8,7,255],
        }).unwrap();
        set_object_property(scope, global, "taintedBitmap", tainted.into()).unwrap();
        let script = v8::String::new(scope,
            "transferredCanvas.getContext('bitmaprenderer').transferFromImageBitmap(taintedBitmap)").unwrap();
        v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(canvas_tainted(&owner, node));
        assert!(owner.bitmap_renderer_surfaces.borrow().paint_revision().unwrap() > painted_revision);
        assert!(canvas_png(&owner, node).is_err());
        assert!(!owner.canvas_images.borrow().contains_key(&node));
        let script = v8::String::new(scope,
            "transferredCanvas.getContext('bitmaprenderer').transferFromImageBitmap(null)").unwrap();
        v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(!canvas_tainted(&owner, node));
        let script = v8::String::new(scope,
            "transferredCanvas.getContext('bitmaprenderer').transferFromImageBitmap(null)").unwrap();
        v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(!owner.bitmap_renderer_surfaces.borrow().has_pixels(&node));
        let bytes = canvas_png(&owner, node).unwrap().unwrap();
        let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(pixels.dimensions(), (20, 30));
        assert!(pixels.into_raw().iter().all(|byte| *byte == 0));
        let script = v8::String::new(scope, "inputBitmap.close()").unwrap();
        v8::Script::compile(scope, script, None).unwrap().run(scope).unwrap();
        assert!(!state.image_bitmap_custody.borrow().contains_key(&bitmap_id));
    }
}

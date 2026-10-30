//! Native immutable pixel snapshots. Detachment lives with the bitmap backing,
//! independently of the source canvas, realm wrappers, and author properties.
use super::*;
pub(super) type Backing = Rc<RefCell<Option<DecodedImage>>>;
struct SnapshotBacking {
    image: Rc<DecodedImage>,
    _pixels: Arc<dyn Send + Sync>,
    _metadata: Arc<dyn Send + Sync>,
}
/// Immutable source copies keep pixels and their independent admission together.
/// Cloning a retained pattern shares the immutable snapshot, not its pixel Vec.
#[derive(Clone)]
pub(super) struct Snapshot(Rc<SnapshotBacking>);
impl std::ops::Deref for Snapshot {
    type Target = DecodedImage;
    fn deref(&self) -> &DecodedImage { &self.0.image }
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0.image, formatter)
    }
}
impl Snapshot {
    pub(super) fn frame_parts(self) -> (Rc<DecodedImage>, Rc<dyn std::any::Any>) {
        (self.0.image.clone(), self.0)
    }
    pub(super) fn owned(allocation: &BindingRetainedAllocation, image: DecodedImage,
        pixels: Arc<dyn Send + Sync>) -> Result<Self, V8BindingError>
    {
        let metadata = match allocation.reserve(std::mem::size_of::<SnapshotBacking>()
            + std::mem::size_of::<DecodedImage>()) {
            Ok(metadata) => metadata,
            Err(_) => {
                drop(image);
                drop(pixels);
                return Err(V8BindingError::Allocation("image source snapshot metadata"));
            }
        };
        Ok(Self(Rc::new(SnapshotBacking { image: Rc::new(image), _pixels: pixels, _metadata: metadata })))
    }
    pub(super) fn copy(allocation: &BindingRetainedAllocation, image: &DecodedImage, origin_clean: bool)
        -> Result<Self, V8BindingError>
    {
        if !valid(image) { return Err(error("InvalidStateError")); }
        let pixels = allocation.reserve(image.rgba.len())
            .map_err(|_| V8BindingError::Allocation("image source snapshot pixels"))?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(image.rgba.len())
            .map_err(|_| V8BindingError::Allocation("image source snapshot allocation"))?;
        if rgba.capacity() != image.rgba.len() {
            return Err(V8BindingError::Allocation("image source snapshot extent"));
        }
        rgba.extend_from_slice(&image.rgba);
        Self::owned(allocation, DecodedImage { width: image.width, height: image.height,
            origin_clean: image.origin_clean && origin_clean, rgba }, pixels)
    }
    fn existing(allocation: &BindingRetainedAllocation, image: DecodedImage)
        -> Result<Self, V8BindingError>
    {
        let pixels = allocation.reserve(image.rgba.capacity())
            .map_err(|_| V8BindingError::Allocation("decoded image source custody"))?;
        Self::owned(allocation, image, pixels)
    }
}
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "ImageBitmap".into(),
        member: name.into(),
    }
}
pub(super) fn identify(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<Backing> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let state = binding_state_for_receiver(scope, object)?;
    // Interface conversion is a non-coercing brand probe. DOM wrappers carry
    // BigInt identities; asking stateful_id to convert those to Number leaves
    // a pending exception rather than simply rejecting the wrong interface.
    if object.internal_field_count() == 0
        || !object.get_internal_field(scope, 0)?.is_number()
    {
        return None;
    }
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
        Some(PlatformObjectRecord::ImageBitmap(backing)) => Some(backing.clone()),
        _ => None,
    }
}
pub(super) fn valid(bitmap: &DecodedImage) -> bool {
    image_data_size(bitmap.width, bitmap.height).ok() == Some(bitmap.rgba.len())
}
/// Release only the exact wrapper's final-output pixel charge after the caller
/// has consumed or closed its backing. Wrong-interface probes never coerce IDs.
pub(super) fn release_consumed(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) {
    let Some(backing) = identify(scope, value) else { return; };
    if backing.borrow().is_some() { return; }
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else { return; };
    if let Some(state) = binding_state_for_receiver(scope, object)
        && let Ok(id) = stateful_id(scope, object)
    {
        state.image_bitmap_custody.borrow_mut().remove(&id);
    }
}
pub(super) fn allocate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    bitmap: DecodedImage,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !valid(&bitmap) {
        return Err(error("InvalidStateError"));
    }
    let allocation = state.capture_retained_allocation(scope,
        Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
    let charge = allocation.reserve(bitmap.rgba.capacity())
        .map_err(|_| V8BindingError::Allocation("ImageBitmap pixel custody admission"))?;
    allocate_admitted(scope, state, bitmap, &allocation, charge)
}
/// Transfer a private immutable source snapshot into a native bitmap without
/// cloning pixels or releasing their admission before wrapper publication.
pub(super) fn allocate_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    snapshot: Snapshot,
    allocation: &BindingRetainedAllocation,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let backing = Rc::try_unwrap(snapshot.0)
        .map_err(|_| V8BindingError::Allocation("shared image snapshot handoff"))?;
    let image = Rc::try_unwrap(backing.image)
        .map_err(|_| V8BindingError::Allocation("shared image snapshot pixels"))?;
    allocate_admitted(scope, state, image, allocation, backing._pixels)
}
/// Publish already-admitted output without releasing its charge between the
/// source transformation and the final native wrapper's custody.
fn allocate_admitted<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    bitmap: DecodedImage,
    allocation: &BindingRetainedAllocation,
    charge: Arc<dyn Send + Sync>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !valid(&bitmap) {
        return Err(error("InvalidStateError"));
    }
    state.image_bitmap_custody.borrow_mut().reserve(allocation)?;
    let id = register_platform_object(
        state,
        PlatformObjectRecord::ImageBitmap(Rc::new(RefCell::new(Some(bitmap)))),
    );
    state.image_bitmap_custody.borrow_mut().insert_reserved(id, charge);
    match new_platform_object(scope, state, "ImageBitmap", id) {
        Ok(object) => Ok(object),
        Err(error) => {
            state.platform_objects.borrow_mut().remove(&id);
            state.image_bitmap_custody.borrow_mut().remove(&id);
            state.platform_edges.borrow_mut().discard_unpublished(id);
            Err(error)
        }
    }
}
pub(super) fn source(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    extended: bool,
) -> Result<Option<Snapshot>, V8BindingError> {
    source_with_orientation(scope, state, value, extended, true)
}
fn source_with_orientation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    extended: bool,
    orient: bool,
) -> Result<Option<Snapshot>, V8BindingError> {
    let allocation = state.capture_retained_allocation(scope,
        Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
    if let Some(frame) = video_frame::identify(scope, value) {
        let frame = frame.borrow();
        if !video_frame::valid(&frame) {
            return Err(error("InvalidStateError"));
        }
        let size = image_data_size(frame.display[0], frame.display[1])?;
        let pixels = allocation.reserve(size)
            .map_err(|_| V8BindingError::Allocation("VideoFrame source output admission"))?;
        // Rotation allocates a second full-size buffer while the unrotated
        // raster remains live. Flipping alone is in-place and needs no copy.
        let scratch = if matches!(frame.rotation, 90 | 180 | 270) {
            Some(allocation.reserve(size)
                .map_err(|_| V8BindingError::Allocation("VideoFrame rotation scratch admission"))?)
        } else { None };
        let image = video_frame::image(&frame)?;
        if image.rgba.capacity() != size {
            return Err(V8BindingError::Allocation("VideoFrame source output extent"));
        }
        // image() has disposed of its intermediate raster before returning.
        drop(scratch);
        return Snapshot::owned(&allocation, image, pixels).map(Some);
    }
    if let Some(source) = offscreen_canvas::identify(scope, value) {
        return source.charged_snapshot(&allocation).map(Some);
    }
    if let Some(backing) = identify(scope, value) {
        let backing = backing.borrow();
        let image = backing.as_ref().ok_or_else(|| error("InvalidStateError"))?;
        return Snapshot::copy(&allocation, image, true).map(Some);
    }
    if extended && let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
        if let Some((width, height, pixels)) = native_image_data(scope, object) {
            if pixels
                .buffer(scope)
                .is_none_or(|buffer| buffer.was_detached())
            {
                return Err(error("InvalidStateError"));
            }
            let size = image_data_size(width, height)?;
            if size != pixels.byte_length() { return Err(error("InvalidStateError")); }
            let pixel_charge = allocation.reserve(size)
                .map_err(|_| V8BindingError::Allocation("ImageData source snapshot admission"))?;
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(size)
                .map_err(|_| V8BindingError::Allocation("ImageData source snapshot allocation"))?;
            if rgba.capacity() != size { return Err(V8BindingError::Allocation("ImageData source snapshot extent")); }
            rgba.resize(size, 0);
            pixels.copy_contents(&mut rgba);
            let bitmap = DecodedImage {
                origin_clean: true,
                width,
                height,
                rgba,
            };
            return if valid(&bitmap) {
                Snapshot::owned(&allocation, bitmap, pixel_charge).map(Some)
            } else {
                Err(error("InvalidStateError"))
            };
        }
        if let Some(owner) = binding_state_for_receiver(scope, object)
            && let Ok(id) = stateful_id(scope, object)
        {
            let blob = match owner.platform_objects.borrow().get(&id) {
                Some(PlatformObjectRecord::Blob(blob)) => Some(blob.clone()),
                _ => None,
            };
            if let Some(blob) = blob {
                let mut reader = image::ImageReader::new(std::io::Cursor::new(&blob.bytes))
                    .with_guessed_format()
                    .map_err(|_| error("InvalidStateError"))?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(32 * 1024 * 1024);
                reader.limits(limits);
                use image::ImageDecoder;
                let mut decoder = reader
                    .into_decoder()
                    .map_err(|_| error("InvalidStateError"))?;
                let orientation = decoder
                    .orientation()
                    .map_err(|_| error("InvalidStateError"))?;
                let (width, height) = decoder.dimensions();
                let output_size = image_data_size(width, height)?;
                let decoded_size = usize::try_from(decoder.total_bytes())
                    .ok().filter(|size| *size <= 32 * 1024 * 1024)
                    .ok_or(V8BindingError::Allocation("encoded image decoded extent"))?;
                let decoded_charge = allocation.reserve(decoded_size)
                    .map_err(|_| V8BindingError::Allocation("encoded image decoded pixels admission"))?;
                let mut decoded = image::DynamicImage::from_decoder(decoder)
                    .map_err(|_| error("InvalidStateError"))?;
                if orient {
                    use image::metadata::Orientation;
                    let scratch = if matches!(orientation, Orientation::Rotate90
                        | Orientation::Rotate270 | Orientation::Rotate90FlipH
                        | Orientation::Rotate270FlipH) {
                        Some(allocation.reserve(decoded_size)
                            .map_err(|_| V8BindingError::Allocation("encoded image orientation scratch admission"))?)
                    } else { None };
                    decoded.apply_orientation(orientation);
                    drop(scratch);
                }
                let output_charge = allocation.reserve(output_size)
                    .map_err(|_| V8BindingError::Allocation("encoded image RGBA output admission"))?;
                let rgba = decoded.to_rgba8();
                drop(decoded);
                drop(decoded_charge);
                let bitmap = DecodedImage {
                    origin_clean: true,
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: rgba.into_raw(),
                };
                if bitmap.rgba.capacity() != output_size {
                    return Err(V8BindingError::Allocation("encoded image RGBA output extent"));
                }
                return if valid(&bitmap) {
                    Snapshot::owned(&allocation, bitmap, output_charge).map(Some)
                } else {
                    Err(error("InvalidStateError"))
                };
            }
        }
    }
    let node = argument_node(scope, state, value)?;
    let owner = active_document_state_for_node(scope, state, node).unwrap_or_else(|| state.clone());
    let dom = owner.dom.borrow();
    let canvas = dom.is_element(node, "http://www.w3.org/1999/xhtml", "canvas");
    let image = dom.is_element(node, "http://www.w3.org/1999/xhtml", "img")
        || dom.is_element(node, "http://www.w3.org/2000/svg", "image")
        || dom.is_element(node, "http://www.w3.org/1999/xhtml", "video");
    drop(dom);
    if !canvas && !image {
        throw_type_error(scope, "source is not an image source");
        return Err(V8BindingError::JavaScriptException);
    }
    if canvas {
        let (width, height) = canvas_output_dimensions(&owner, node)?;
        if width == 0 || height == 0 {
            return Err(error("InvalidStateError"));
        }
        let clean = !canvas_tainted(&owner, node);
        let renderers = owner.bitmap_renderer_surfaces.borrow();
        let canvases = owner.canvas_images.borrow();
        if let Some(image) = renderers.image(&node).or_else(|| canvases.get(&node)) {
            Snapshot::copy(&allocation, image, clean).map(Some)
        } else {
            let size = image_data_size(width, height)?;
            let pixels = allocation.reserve(size)
                .map_err(|_| V8BindingError::Allocation("empty image source snapshot admission"))?;
            let mut image = canvas_context_settings::empty(&owner, node, width, height)?;
            image.origin_clean &= clean;
            Snapshot::owned(&allocation, image, pixels).map(Some)
        }
    } else {
        if owner.failed_images.borrow().contains(&node) {
            return Err(error("InvalidStateError"));
        }
        if let Some(image) = owner.decoded_images.borrow().get(&node) {
            return Snapshot::copy(&allocation, image, true).map(Some);
        }
        let svg = owner.decoded_svg_images.borrow().get(&node).cloned();
        if let Some(bytes) = svg {
            // A decoded zero-size SVG is an unavailable canvas image source,
            // not a broken resource. createPattern returns null for this case.
            if image_dimensions::svg_size(&bytes).is_some_and(|(width, height)| width == 0 || height == 0) {
                return Ok(None);
            }
            let resource = oryn_common::v4::PaintImageResource {
                svg_document_snapshot: false, svg_instances: Vec::new(), content_type: None,
                url_aliases: Vec::new(), media_type: "image/svg+xml".into(), bytes,
                url: owner.image_response_urls.borrow().get(&node).cloned().unwrap_or_default(),
            };
            let rgba = oryn_render_core::decode_svg_bitmap(&resource, 4 * 1024 * 1024)
                .map_err(|_| error("InvalidStateError"))?;
            let bitmap = DecodedImage {
                width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw(),
                // Missing origin provenance is unclean, never an implicit grant.
                origin_clean: owner.svg_image_origin_clean.borrow().get(&node).copied().unwrap_or(false),
            };
            owner.decoded_images.borrow_mut().insert(node, bitmap.clone());
            return Snapshot::existing(&allocation, bitmap).map(Some);
        }
        owner.decoded_images.borrow().get(&node)
            .map(|image| Snapshot::copy(&allocation, image, true)).transpose()
    }
}
fn member(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(backing) = identify(scope, args.this().into()) else {
        throw_type_error(scope, "ImageBitmap receiver required");
        return;
    };
    match args.data().int32_value(scope).unwrap_or(-1) {
        0 => rv.set_uint32(backing.borrow().as_ref().map_or(0, |image| image.width)),
        1 => rv.set_uint32(backing.borrow().as_ref().map_or(0, |image| image.height)),
        2 => {
            backing.borrow_mut().take();
            release_consumed(scope, args.this().into());
        }
        _ => {}
    }
}
pub(super) fn install_members(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    for (index, name) in [(0, "width"), (1, "height"), (2, "close")] {
        let data = v8::Integer::new(scope, index);
        let function = v8::Function::builder(member)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("ImageBitmap member"))?;
        let function_name = if index == 2 {
            name.to_owned()
        } else {
            format!("get {name}")
        };
        let function_name = v8::String::new(scope, &function_name)
            .ok_or(V8BindingError::Allocation("ImageBitmap member name"))?;
        function.set_name(function_name);
        if index == 2 {
            set_object_property(scope, prototype, name, function.into())?;
        } else {
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let key = v8::String::new(scope, name)
                .ok_or(V8BindingError::Allocation("ImageBitmap key"))?;
            if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
                return Err(V8BindingError::Allocation("ImageBitmap getter"));
            }
        }
    }
    Ok(())
}
fn option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<'s, v8::Object>>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let Some(options) = options else {
        return Ok(v8::undefined(scope).into());
    };
    let key = v8::String::new(scope, key).ok_or(V8BindingError::Allocation("bitmap option"))?;
    options
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn create(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    v8::tc_scope!(let tc,scope);
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let crop = args.length() >= 5;
        let crop_rect = if crop {
            Some((
                args.get(1)
                    .int32_value(tc)
                    .ok_or(V8BindingError::JavaScriptException)? as i64,
                args.get(2)
                    .int32_value(tc)
                    .ok_or(V8BindingError::JavaScriptException)? as i64,
                args.get(3)
                    .int32_value(tc)
                    .ok_or(V8BindingError::JavaScriptException)? as i64,
                args.get(4)
                    .int32_value(tc)
                    .ok_or(V8BindingError::JavaScriptException)? as i64,
            ))
        } else {
            None
        };
        let options = args.get(if crop { 5 } else { 1 });
        let options = if options.is_null_or_undefined() {
            None
        } else {
            Some(
                options
                    .to_object(tc)
                    .ok_or(V8BindingError::JavaScriptException)?,
            )
        };

        let mut orient = true;
        let mut flip = false;
        let mut smooth = true;
        let mut resize_width = None;
        let mut resize_height = None;
        // Dictionary members are read in WebIDL lexicographic order.
        for key in [
            "colorSpaceConversion",
            "imageOrientation",
            "premultiplyAlpha",
            "resizeHeight",
            "resizeQuality",
            "resizeWidth",
        ] {
            let value = option(tc, options, key)?;
            if value.is_undefined() {
                continue;
            }
            if key == "resizeWidth" || key == "resizeHeight" {
                let value = value
                    .number_value(tc)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !value.is_finite() || value.trunc() < 1. || value.trunc() > u32::MAX as f64 {
                    return Err(error("RangeError"));
                }
                if key == "resizeWidth" {
                    resize_width = Some(value.trunc() as u32)
                } else {
                    resize_height = Some(value.trunc() as u32)
                }
                continue;
            }
            let value = value
                .to_string(tc)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(tc);
            let valid = match key {
                "colorSpaceConversion" => matches!(value.as_str(), "none" | "default"),
                "imageOrientation" => {
                    orient = value != "none";
                    flip = value == "flipY";
                    matches!(value.as_str(), "from-image" | "flipY" | "none")
                }
                "premultiplyAlpha" => matches!(value.as_str(), "none" | "premultiply" | "default"),
                _ => {
                    smooth = value != "pixelated";
                    matches!(value.as_str(), "pixelated" | "low" | "medium" | "high")
                }
            };
            if !valid {
                throw_type_error(tc, "Invalid ImageBitmap option");
                return Err(V8BindingError::JavaScriptException);
            }
        }
        let source = source_with_orientation(tc, &state, args.get(0), true, orient)?
            .ok_or_else(|| error("InvalidStateError"))?;
        let (mut x, mut y, mut width, mut height) =
            crop_rect.unwrap_or((0, 0, i64::from(source.width), i64::from(source.height)));
        if width == 0 || height == 0 {
            return Err(error("RangeError"));
        }
        if width < 0 {
            x += width;
            width = -width;
        }
        if height < 0 {
            y += height;
            height = -height;
        }
        let out_width = resize_width.unwrap_or_else(|| {
            resize_height.map_or(width as u32, |h| {
                ((h as f64 * width as f64 / height as f64).ceil().max(1.)) as u32
            })
        });
        let out_height = resize_height.unwrap_or_else(|| {
            resize_width.map_or(height as u32, |w| {
                ((w as f64 * height as f64 / width as f64).ceil().max(1.)) as u32
            })
        });
        let output_size = image_data_size(out_width, out_height)?;
        let allocation = state.capture_retained_allocation(tc,
            Arc::new(std::sync::atomic::AtomicBool::new(false)))?;
        let output_charge = allocation.reserve(output_size)
            .map_err(|_| V8BindingError::Allocation("ImageBitmap transformed output admission"))?;
        let mut output = empty_canvas_bitmap(out_width, out_height)?;
        if output.rgba.capacity() != output_size {
            return Err(V8BindingError::Allocation("ImageBitmap transformed output extent"));
        }
        if i64::from(out_width) == width && i64::from(out_height) == height {
            for row in 0..out_height {
                let source_y = y + i64::from(row);
                if source_y < 0 || source_y >= i64::from(source.height) {
                    continue;
                }
                let start = x.max(0);
                let end = (x + width).min(i64::from(source.width));
                if end <= start {
                    continue;
                }
                let from = (source_y as usize * source.width as usize + start as usize) * 4;
                let to = (row as usize * out_width as usize + (start - x) as usize) * 4;
                let count = (end - start) as usize * 4;
                output.rgba[to..to + count].copy_from_slice(&source.rgba[from..from + count]);
            }
        } else {
            raster_canvas_image_transformed(
                &mut output,
                &source,
                CanvasImageInputs {
                    rect: [
                        x as f64,
                        y as f64,
                        width as f64,
                        height as f64,
                        0.,
                        0.,
                        out_width as f64,
                        out_height as f64,
                    ],
                    smooth,
                    global_alpha: 1.,
                    matrix: [1., 0., 0., 1., 0., 0.],
                    composite: CanvasDrawState::default().composite,
                    clips: &[],
                },
            );
        }
        if flip {
            let row = out_width as usize * 4;
            for y in 0..out_height as usize / 2 {
                let split = (out_height as usize - 1 - y) * row;
                let (top, bottom) = output.rgba.split_at_mut(split);
                top[y * row..(y + 1) * row].swap_with_slice(&mut bottom[..row]);
            }
        }
        output.origin_clean = source.origin_clean;
        allocate_admitted(tc, &state, output, &allocation, output_charge)
    })();
    match result {
        Ok(bitmap) => {
            resolver.resolve(tc, bitmap.into());
        }
        Err(error) => {
            let exception = if tc.has_caught() {
                tc.exception()
            } else {
                match &error {
                    V8BindingError::Unsupported { member, .. } if member == "RangeError" => {
                        v8::String::new(tc, "ImageBitmap dimensions must be nonzero")
                            .map(|message| v8::Exception::range_error(tc, message))
                    }
                    V8BindingError::Unsupported { member, .. } => {
                        dom_exception_value(tc, member).ok()
                    }
                    _ => dom_exception_value(tc, "InvalidStateError").ok(),
                }
            };
            tc.reset();
            if let Some(exception) = exception {
                resolver.reject(tc, exception);
            }
        }
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let function = v8::Function::builder(create)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("createImageBitmap"))?;
    let global = context.global(scope);
    set_object_property(scope, global, "createImageBitmap", function.into())
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn bitmap_handoff_moves_pixels_and_keeps_admission_until_close() {
        use super::super::tests::initialize_v8;
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let allocation = state.capture_retained_allocation(scope,
            Arc::new(std::sync::atomic::AtomicBool::new(false))).unwrap();
        let image = DecodedImage { width: 1, height: 1, origin_clean: false,
            rgba: vec![7, 8, 9, 255] };
        let snapshot = Snapshot::copy(&allocation, &image, true).unwrap();
        let retained = snapshot.clone();
        let records = state.platform_objects.borrow().len();
        assert!(allocate_snapshot(scope, &state, snapshot, &allocation).is_err());
        assert_eq!(state.platform_objects.borrow().len(), records);
        assert_eq!(retained.rgba, image.rgba);
        let pointer = retained.rgba.as_ptr();
        let pixels = Arc::downgrade(&retained.0._pixels);
        let metadata = Arc::downgrade(&retained.0._metadata);
        let object = allocate_snapshot(scope, &state, retained, &allocation).unwrap();
        let backing = identify(scope, object.into()).unwrap();
        {
            let borrowed = backing.borrow();
            let bitmap = borrowed.as_ref().unwrap();
            assert_eq!(bitmap.rgba.as_ptr(), pointer);
            assert!(!bitmap.origin_clean);
        }
        assert!(metadata.upgrade().is_none());
        assert!(pixels.upgrade().is_some());
        backing.borrow_mut().take();
        release_consumed(scope, object.into());
        assert!(pixels.upgrade().is_none());
    }
    #[test]
    fn snapshot_copy_is_admitted_independent_shared_and_released_after_last_owner() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let handles, &mut isolate);
        let context = v8::Context::new(handles, Default::default());
        let scope = &mut v8::ContextScope::new(handles, context);
        let dom = Rc::new(RefCell::new(html::parse("<body></body>").into_runtime().unwrap()));
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let retired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let allocation = state.capture_retained_allocation(scope, retired.clone()).unwrap();
        let mut image = DecodedImage { width: 1, height: 1, origin_clean: true, rgba: vec![1,2,3,255] };
        let snapshot = Snapshot::copy(&allocation, &image, true).unwrap();
        let retained = snapshot.clone();
        assert!(Rc::ptr_eq(&snapshot.0, &retained.0));
        assert_ne!(snapshot.rgba.as_ptr(), image.rgba.as_ptr());
        image.rgba[0] = 99;
        assert_eq!(snapshot.rgba, [1,2,3,255]);
        let pixels = Arc::downgrade(&snapshot.0._pixels);
        let metadata = Arc::downgrade(&snapshot.0._metadata);
        retired.store(true, std::sync::atomic::Ordering::Release);
        assert!(Snapshot::copy(&allocation, &image, true).is_err());
        assert_eq!(retained.rgba, [1,2,3,255]);
        drop(snapshot);
        assert!(pixels.upgrade().is_some() && metadata.upgrade().is_some());
        let (frame_image, frame_custody) = retained.frame_parts();
        let cloned_image = frame_image.clone();
        let cloned_custody = frame_custody.clone();
        assert!(Rc::ptr_eq(&frame_image, &cloned_image));
        drop(frame_image);
        drop(frame_custody);
        assert!(pixels.upgrade().is_some() && metadata.upgrade().is_some());
        drop(cloned_image);
        drop(cloned_custody);
        assert!(pixels.upgrade().is_none() && metadata.upgrade().is_none());
    }
}

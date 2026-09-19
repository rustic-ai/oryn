//! Native image-backed VideoFrame custody for canvas capture and structured
//! messages. Codec/encoder support is independent of these frame resources.
use super::*;
#[derive(Clone)]
pub(super) struct Frame {
    pub(super) bitmap: Option<DecodedImage>,
    pub(super) planes: Option<Vec<u8>>,
    pub(super) timestamp: i64,
    pub(super) duration: Option<u64>,
    pub(super) visible: [u32; 4],
    pub(super) display: [u32; 2],
    pub(super) rotation: u16,
    pub(super) flip: bool,
}
pub(super) type Backing = Rc<RefCell<Frame>>;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "VideoFrame".into(),
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
        Some(PlatformObjectRecord::VideoFrame(backing)) => Some(backing.clone()),
        _ => None,
    }
}
pub(super) fn valid(frame: &Frame) -> bool {
    if let Some(planes) = &frame.planes {
        if !frame.bitmap.as_ref().is_some_and(|bitmap| {
            crate::message_wire::VideoFrameData::i420_size(bitmap.width, bitmap.height)
                == Some(planes.len())
        }) {
            return false;
        }
    }
    frame.bitmap.as_ref().is_some_and(|bitmap| {
        image_bitmap::valid(bitmap)
            && bitmap.origin_clean
            && frame.visible[2] > 0
            && frame.visible[3] > 0
            && frame.visible[0]
                .checked_add(frame.visible[2])
                .is_some_and(|end| end <= bitmap.width)
            && frame.visible[1]
                .checked_add(frame.visible[3])
                .is_some_and(|end| end <= bitmap.height)
            && frame.display.iter().all(|value| *value > 0)
    })
}
impl Frame {
    pub(super) fn empty() -> Self {
        Self {
            bitmap: None,
            planes: None,
            timestamp: 0,
            duration: None,
            visible: [0; 4],
            display: [0; 2],
            rotation: 0,
            flip: false,
        }
    }
    pub(super) fn wire(self) -> crate::message_wire::VideoFrameData {
        let bitmap = self.bitmap.unwrap_or(DecodedImage {
            origin_clean: true,
            width: 0,
            height: 0,
            rgba: Vec::new(),
        });
        crate::message_wire::VideoFrameData {
            width: bitmap.width,
            height: bitmap.height,
            rgba: bitmap.rgba,
            planes: self.planes,
            timestamp: self.timestamp,
            duration: self.duration,
            visible: self.visible,
            display: self.display,
            rotation: self.rotation,
            flip: self.flip,
        }
    }
    pub(super) fn from_wire(value: crate::message_wire::VideoFrameData) -> Self {
        Self {
            bitmap: Some(DecodedImage {
                origin_clean: true,
                width: value.width,
                height: value.height,
                rgba: value.rgba,
            }),
            planes: value.planes,
            timestamp: value.timestamp,
            duration: value.duration,
            visible: value.visible,
            display: value.display,
            rotation: value.rotation,
            flip: value.flip,
        }
    }
}
pub(super) fn allocate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    frame: Frame,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !valid(&frame) {
        return Err(error("InvalidStateError"));
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::VideoFrame(Rc::new(RefCell::new(frame))),
    );
    new_platform_object(scope, state, "VideoFrame", id)
}
pub(super) fn image(frame: &Frame) -> Result<DecodedImage, V8BindingError> {
    let bitmap = frame
        .bitmap
        .as_ref()
        .ok_or_else(|| error("InvalidStateError"))?;
    let mut output = empty_canvas_bitmap(frame.display[0], frame.display[1])?;
    let [x, y, w, h] = frame.visible;
    raster_canvas_image_transformed(
        &mut output,
        bitmap,
        [
            x as f64,
            y as f64,
            w as f64,
            h as f64,
            0.,
            0.,
            frame.display[0] as f64,
            frame.display[1] as f64,
        ],
        true,
        1.,
        CANVAS_IDENTITY,
        CanvasDrawState::default().composite,
        &[],
    );
    if frame.rotation != 0 || frame.flip {
        let pixels = image::RgbaImage::from_raw(output.width, output.height, output.rgba)
            .ok_or_else(|| error("InvalidStateError"))?;
        let mut pixels = match frame.rotation {
            90 => image::imageops::rotate90(&pixels),
            180 => image::imageops::rotate180(&pixels),
            270 => image::imageops::rotate270(&pixels),
            _ => pixels,
        };
        if frame.flip {
            image::imageops::flip_horizontal_in_place(&mut pixels);
        }
        output = DecodedImage {
            origin_clean: true,
            width: pixels.width(),
            height: pixels.height(),
            rgba: pixels.into_raw(),
        };
    }
    Ok(output)
}
pub(super) fn construct(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    if args.length() == 0 {
        throw_type_error(scope, "VideoFrame requires an image");
        return Err(V8BindingError::JavaScriptException);
    }
    let init = args.get(1);
    let init = if init.is_null_or_undefined() {
        None
    } else {
        Some(
            init.to_object(scope)
                .ok_or(V8BindingError::JavaScriptException)?,
        )
    };
    let raw = args.get(0).is_array_buffer()
        || args.get(0).is_array_buffer_view()
        || args.get(0).is_shared_array_buffer();
    let mut coded = [None, None];
    let mut format = None;
    let mut timestamp = None;
    let mut duration = None;
    let mut display = [None, None];
    let mut visible = None;
    let mut discard_alpha = false;
    let mut rotation = None;
    let mut flip = None;
    // Convert all options before taking the native source snapshot.
    for key in [
        "alpha",
        "codedHeight",
        "codedWidth",
        "displayHeight",
        "displayWidth",
        "duration",
        "flip",
        "format",
        "rotation",
        "timestamp",
        "visibleRect",
    ] {
        let Some(init) = init else { continue };
        let Some(value) = checked_dictionary_member(scope, init.into(), key)? else {
            continue;
        };
        match key {
            "format" => {
                format = Some(
                    value
                        .to_string(scope)
                        .ok_or(V8BindingError::JavaScriptException)?
                        .to_rust_string_lossy(scope),
                );
            }
            "codedWidth" | "codedHeight" => {
                let number = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !number.is_finite() || number < 1. || number > u32::MAX as f64 {
                    return Err(error("TypeError"));
                }
                coded[usize::from(key == "codedHeight")] = Some(number.trunc() as u32);
            }
            "alpha" => {
                let alpha = value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
                    .to_rust_string_lossy(scope);
                if !matches!(alpha.as_str(), "keep" | "discard") {
                    throw_type_error(scope, "Invalid VideoFrame alpha mode");
                    return Err(V8BindingError::JavaScriptException);
                }
                discard_alpha = alpha == "discard";
            }
            "flip" => {
                flip = Some(value.boolean_value(scope));
            }
            "rotation" => {
                let value = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !value.is_finite() || value % 90. != 0. {
                    return Err(error("TypeError"));
                }
                rotation = Some(value.rem_euclid(360.) as u16);
            }
            "visibleRect" => {
                let value = value
                    .to_object(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                let mut rect = [0; 4];
                for (index, name) in ["x", "y", "width", "height"].iter().enumerate() {
                    if let Some(value) = checked_dictionary_member(scope, value.into(), name)? {
                        let number = value
                            .number_value(scope)
                            .ok_or(V8BindingError::JavaScriptException)?;
                        if !number.is_finite() || number < 0. || number > u32::MAX as f64 {
                            return Err(error("TypeError"));
                        }
                        rect[index] = number.trunc() as u32;
                    }
                }
                visible = Some(rect);
            }
            _ => {
                let number = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !number.is_finite() {
                    return Err(error("TypeError"));
                }
                match key {
                    "timestamp" => {
                        if number < i64::MIN as f64 || number >= i64::MAX as f64 {
                            return Err(error("TypeError"));
                        }
                        timestamp = Some(number.trunc() as i64);
                    }
                    "duration" => {
                        if number < 0. || number >= u64::MAX as f64 {
                            return Err(error("TypeError"));
                        }
                        duration = Some(number.trunc() as u64);
                    }
                    _ => {
                        if number < 1. || number > u32::MAX as f64 {
                            return Err(error("TypeError"));
                        }
                        display[usize::from(key == "displayHeight")] = Some(number.trunc() as u32);
                    }
                }
            }
        }
    }
    let mut frame = if raw {
        if format.as_deref() != Some("I420") {
            return Err(error("NotSupportedError"));
        }
        let width = coded[0].ok_or_else(|| error("TypeError"))?;
        let height = coded[1].ok_or_else(|| error("TypeError"))?;
        let expected = crate::message_wire::VideoFrameData::i420_size(width, height)
            .ok_or_else(|| error("TypeError"))?;
        let (store, offset, length) = audio_data::buffer(scope, args.get(0))?;
        if length < expected {
            return Err(error("TypeError"));
        }
        let planes: Vec<u8> = store
            .iter()
            .skip(offset)
            .take(expected)
            .map(Cell::get)
            .collect();
        let bitmap = i420_bitmap(width, height, &planes)?;
        Frame {
            bitmap: Some(bitmap),
            planes: Some(planes),
            timestamp: timestamp.ok_or_else(|| error("TypeError"))?,
            duration: None,
            visible: [0, 0, width, height],
            display: [width, height],
            rotation: 0,
            flip: false,
        }
    } else if let Some(source) = identify(scope, args.get(0)) {
        let frame = source.borrow().clone();
        if !valid(&frame) {
            return Err(error("InvalidStateError"));
        }
        frame
    } else {
        let bitmap = image_bitmap::source(scope, state, args.get(0), false)?
            .ok_or_else(|| error("InvalidStateError"))?;
        if !bitmap.origin_clean {
            return Err(error("SecurityError"));
        }
        let timestamp = timestamp.ok_or_else(|| error("TypeError"))?;
        Frame {
            visible: [0, 0, bitmap.width, bitmap.height],
            display: [bitmap.width, bitmap.height],
            bitmap: Some(bitmap),
            planes: None,
            timestamp,
            duration: None,
            rotation: 0,
            flip: false,
        }
    };
    if let Some(rotation) = rotation {
        frame.rotation = rotation;
    }
    if let Some(flip) = flip {
        frame.flip = flip;
    }
    if let Some(timestamp) = timestamp {
        frame.timestamp = timestamp;
    }
    if duration.is_some() {
        frame.duration = duration;
    }
    if let Some(visible) = visible {
        frame.visible = visible;
        frame.display = [visible[2], visible[3]];
    }
    if display[0].is_some() != display[1].is_some() {
        return Err(error("TypeError"));
    }
    if let (Some(width), Some(height)) = (display[0], display[1]) {
        frame.display = [width, height];
    }
    if discard_alpha {
        if let Some(bitmap) = &mut frame.bitmap {
            for pixel in bitmap.rgba.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
    }
    if !valid(&frame) {
        return Err(error("TypeError"));
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::VideoFrame(Rc::new(RefCell::new(frame))),
    );
    bind_platform_object(scope, state, id, args.this());
    Ok(())
}
fn member(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        let Some(backing) = identify(scope, args.this().into()) else {
            throw_type_error(scope, "VideoFrame receiver required");
            return Err(V8BindingError::JavaScriptException);
        };
        let code = args.data().int32_value(scope).unwrap_or(-1);
        if code == 12 {
            let mut frame = backing.borrow_mut();
            frame.bitmap.take();
            frame.planes.take();
            return Ok(None);
        }
        let frame = backing.borrow();
        let alive = frame.bitmap.is_some();
        if code == 11 {
            let state = scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .ok_or(V8BindingError::Allocation("VideoFrame realm"))?;
            return allocate(scope, &state, frame.clone()).map(|object| Some(object.into()));
        }
        let value = match code {
            0 => {
                if alive {
                    v8::String::new(
                        scope,
                        if frame.planes.is_some() {
                            "I420"
                        } else {
                            "RGBA"
                        },
                    )
                    .ok_or(V8BindingError::Allocation("frame format"))?
                    .into()
                } else {
                    v8::null(scope).into()
                }
            }
            1 | 2 => v8::Integer::new_from_unsigned(
                scope,
                frame.bitmap.as_ref().map_or(0, |bitmap| {
                    if code == 1 {
                        bitmap.width
                    } else {
                        bitmap.height
                    }
                }),
            )
            .into(),
            3 | 4 => {
                if !alive {
                    v8::null(scope).into()
                } else {
                    let rect = if code == 3 {
                        let bitmap = frame.bitmap.as_ref().unwrap();
                        [0, 0, bitmap.width, bitmap.height]
                    } else {
                        frame.visible
                    };
                    let state = scope
                        .get_current_context()
                        .get_slot::<V8BindingState>()
                        .ok_or(V8BindingError::Allocation("frame realm"))?;
                    let template = state
                        .instance_templates
                        .borrow()
                        .get("DOMRectReadOnly")
                        .cloned()
                        .ok_or(V8BindingError::MissingInterface("DOMRectReadOnly"))?;
                    let object = v8::Local::new(scope, &template)
                        .new_instance(scope)
                        .ok_or(V8BindingError::Allocation("frame rect"))?;
                    initialize_rect_components(scope, object, rect.map(f64::from), true)?;
                    object.into()
                }
            }
            5 | 6 => v8::Integer::new_from_unsigned(
                scope,
                if alive {
                    frame.display[(code - 5) as usize]
                } else {
                    0
                },
            )
            .into(),
            7 => frame
                .duration
                .map(|duration| v8::Number::new(scope, duration as f64).into())
                .unwrap_or_else(|| v8::null(scope).into()),
            8 => v8::Number::new(scope, frame.timestamp as f64).into(),
            9 => v8::Number::new(scope, frame.rotation as f64).into(),
            10 => v8::Boolean::new(scope, frame.flip).into(),
            13 => {
                if !alive {
                    return Err(error("InvalidStateError"));
                }
                v8::Object::new(scope).into()
            }
            _ => return Ok(None),
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
        "codedWidth",
        "codedHeight",
        "codedRect",
        "visibleRect",
        "displayWidth",
        "displayHeight",
        "duration",
        "timestamp",
        "rotation",
        "flip",
        "clone",
        "close",
        "metadata",
    ]
    .iter()
    .enumerate()
    {
        let data = v8::Integer::new(scope, index as i32);
        let function = v8::Function::builder(member)
            .data(data.into())
            .length(0)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("VideoFrame member"))?;
        if index >= 11 {
            set_object_property(scope, prototype, name, function.into())?;
        } else {
            let undefined = v8::undefined(scope);
            let mut descriptor =
                v8::PropertyDescriptor::new_from_get_set(function.into(), undefined.into());
            descriptor.set_enumerable(true);
            descriptor.set_configurable(true);
            let key =
                v8::String::new(scope, name).ok_or(V8BindingError::Allocation("frame key"))?;
            if prototype.define_property(scope, key.into(), &descriptor) != Some(true) {
                return Err(V8BindingError::Allocation("frame getter"));
            }
        }
    }
    Ok(())
}

// Preserve original planar bytes for clone/transfer; canvas consumers use this
// independently owned BT.601 limited-range conversion.
fn i420_bitmap(width: u32, height: u32, planes: &[u8]) -> Result<DecodedImage, V8BindingError> {
    if crate::message_wire::VideoFrameData::i420_size(width, height) != Some(planes.len()) {
        return Err(error("TypeError"));
    }
    let mut bitmap = empty_canvas_bitmap(width, height)?;
    let w = width as usize;
    let h = height as usize;
    let cw = w.div_ceil(2);
    let chroma = cw * h.div_ceil(2);
    let y_size = w * h;
    for y in 0..h {
        for x in 0..w {
            let l = planes[y * w + x] as i32 - 16;
            let u = planes[y_size + (y / 2) * cw + x / 2] as i32 - 128;
            let v = planes[y_size + chroma + (y / 2) * cw + x / 2] as i32 - 128;
            let pixel = &mut bitmap.rgba[(y * w + x) * 4..][..4];
            pixel.copy_from_slice(&[
                ((298 * l + 409 * v + 128) >> 8).clamp(0, 255) as u8,
                ((298 * l - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8,
                ((298 * l + 516 * u + 128) >> 8).clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    Ok(bitmap)
}

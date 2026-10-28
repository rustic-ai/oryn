//! Opaque native canvas gradient identity and factories; paint integration uses
//! the same live record rather than a snapshot of its stops.
use super::*;
use oryn_common::canvas_gradient::{GradientGeometry, GradientStops};

#[derive(Debug, Clone)]
pub(super) struct Gradient {
    pub(super) geometry: GradientGeometry,
    pub(super) stops: GradientStops,
}

#[derive(Debug, Clone)]
pub(super) struct Paint {
    pub(super) gradient: Rc<RefCell<Gradient>>,
    pub(super) object: v8::Global<v8::Object>,
}
impl Paint {
    pub(super) fn from_value(
        scope: &mut v8::PinScope,
        value: v8::Local<v8::Value>,
    ) -> Option<Self> {
        let gradient = identify(scope, value)?;
        let object = v8::Local::<v8::Object>::try_from(value).ok()?;
        Some(Self {
            gradient,
            object: v8::Global::new(scope, object),
        })
    }
}

/// Shared path rasterization for color and live gradient fills/strokes.
pub(super) fn paint_path(
    target: &mut DecodedImage,
    path: &kurbo::BezPath,
    draw: &CanvasDrawState,
    stroking: bool,
    evenodd: bool,
) {
    use kurbo::Shape;
    let (width, height) = (target.width, target.height);
    let bounds = path.bounding_box();
    let (left, top, right, bottom) = if draw.composite.affects_outside_source() {
        (0, 0, width, height)
    } else {
        (
            bounds.x0.floor().clamp(0.0, width as f64) as u32,
            bounds.y0.floor().clamp(0.0, height as f64) as u32,
            bounds.x1.ceil().clamp(0.0, width as f64) as u32,
            bounds.y1.ceil().clamp(0.0, height as f64) as u32,
        )
    };
    let paint = if stroking {
        &draw.stroke_gradient
    } else {
        &draw.fill_gradient
    };
    let gradient = paint.as_ref().map(|paint| paint.gradient.borrow());
    let pattern_paint = if stroking { &draw.stroke_pattern } else { &draw.fill_pattern };
    let pattern = pattern_paint.as_ref().map(|paint| paint.pattern.borrow());
    let pattern_sample = pattern.as_ref().and_then(|pattern| {
        Some((
            oryn_common::canvas_pattern::Bitmap::new(pattern.image.width, pattern.image.height, &pattern.image.rgba)?,
            canvas_matrix_inverse(pattern.transform)?,
            pattern.repeat,
        ))
    });
    let inverse = canvas_matrix_inverse(draw.transform);
    for y in top..bottom {
        for x in left..right {
            let (coverage, clip_coverage) =
                canvas_path::coverage_in_clip(path, x, y, evenodd, &draw.clips);
            let color = if let Some(gradient) = &gradient {
                inverse.map_or([0; 4], |inverse| {
                    let point = canvas_matrix_point(inverse, x as f64 + 0.5, y as f64 + 0.5);
                    gradient
                        .geometry
                        .sample(&gradient.stops, [point.0, point.1])
                })
            } else if stroking {
                draw.stroke_color
            } else {
                draw.fill_color
            };
            let alpha = color[3] as f64 / 255.0 * draw.global_alpha * coverage;
            let mut pixel = [
                color[0] as f64 * alpha,
                color[1] as f64 * alpha,
                color[2] as f64 * alpha,
                alpha,
            ];
            if pattern.is_some() {
                let sample = inverse.and_then(|inverse| {
                    let point = canvas_matrix_point(inverse, x as f64 + 0.5, y as f64 + 0.5);
                    let (bitmap, inverse_pattern, repeat) = pattern_sample?;
                    let point = canvas_matrix_point(inverse_pattern, point.0, point.1);
                    Some(bitmap.sample(repeat, point.0, point.1, draw.image_smoothing))
                }).unwrap_or([0.0; 4]);
                pixel = sample.map(|channel| channel * draw.global_alpha * coverage);
            }
            let offset = (y as usize * width as usize + x as usize) * 4;
            draw.composite.apply_clipped(
                &mut target.rgba[offset..offset + 4],
                pixel,
                clip_coverage,
            );
        }
    }
}

pub(super) fn identify(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<Rc<RefCell<Gradient>>> {
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
    match owner.platform_objects.borrow().get(&id)? {
        PlatformObjectRecord::CanvasGradient(gradient) => Some(gradient.clone()),
        _ => None,
    }
}

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CanvasGradient".into(),
        member: name.into(),
    }
}

pub(super) fn add_stop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let gradient = identify(scope, object.into()).ok_or_else(|| error("TypeError"))?;
    if args.length() < 2 {
        return Err(error("TypeError"));
    }
    let offset = args
        .get(0)
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if !offset.is_finite() {
        return Err(error("TypeError"));
    }
    let color = args
        .get(1)
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope);
    if !(0.0..=1.0).contains(&offset) {
        return Err(error("IndexSizeError"));
    }
    let color = oryn_common::color::parse_color(&color).ok_or_else(|| error("SyntaxError"))?;
    gradient.borrow_mut().stops.insert(offset, color);
    Ok(v8::undefined(scope).into())
}

pub(super) fn install_factories(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    for (name, length) in [
        ("createLinearGradient", 4),
        ("createRadialGradient", 6),
        ("createConicGradient", 3),
    ] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("gradient factory name"))?;
        let function = v8::Function::builder(factory)
            .data(key.into())
            .length(length)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or(V8BindingError::Allocation("gradient factory"))?;
        function.set_name(key);
        set_object_property(scope, prototype, name, function.into())?;
    }
    Ok(())
}

fn factory(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let canvas = canvas_context_owner(scope, args.this()).ok_or_else(|| error("TypeError"))?;
        let owner = binding_state_for_receiver(scope, canvas).ok_or_else(|| error("TypeError"))?;
        canvas_receiver_node(scope, &owner, &args)?;
        let name = args.data().to_rust_string_lossy(scope);
        let count = match name.as_str() {
            "createLinearGradient" => 4,
            "createRadialGradient" => 6,
            _ => 3,
        };
        if args.length() < count {
            return Err(error("TypeError"));
        }
        let mut values = [0.0; 6];
        for index in 0..count {
            values[index as usize] = args
                .get(index)
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            if !values[index as usize].is_finite() {
                return Err(error("TypeError"));
            }
        }
        let geometry = match name.as_str() {
            "createLinearGradient" => GradientGeometry::Linear {
                start: [values[0], values[1]],
                end: [values[2], values[3]],
            },
            "createRadialGradient" => {
                if values[2] < 0.0 || values[5] < 0.0 {
                    return Err(error("IndexSizeError"));
                }
                GradientGeometry::Radial {
                    start: [values[0], values[1]],
                    start_radius: values[2],
                    end: [values[3], values[4]],
                    end_radius: values[5],
                }
            }
            _ => GradientGeometry::Conic {
                center: [values[1], values[2]],
                angle: values[0],
            },
        };
        let id = register_platform_object(
            &owner,
            PlatformObjectRecord::CanvasGradient(Rc::new(RefCell::new(Gradient {
                geometry,
                stops: GradientStops::default(),
            }))),
        );
        new_platform_object(scope, &owner, "CanvasGradient", id)
    })();
    match result {
        Ok(object) => rv.set(object.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

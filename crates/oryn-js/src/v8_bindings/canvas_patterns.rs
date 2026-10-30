//! Native pattern identity retains a source snapshot and a live transform.
use super::*;
use oryn_common::canvas_pattern::Repeat;

#[derive(Debug, Clone)]
pub(super) struct Pattern {
    pub(super) image: image_bitmap::Snapshot,
    pub(super) repeat: Repeat,
    pub(super) transform: [f64; 6],
}

#[derive(Debug, Clone)]
pub(super) struct Paint {
    pub(super) pattern: Rc<RefCell<Pattern>>,
    pub(super) object: v8::Global<v8::Object>,
}
impl Paint {
    pub(super) fn from_value(
        scope: &mut v8::PinScope,
        value: v8::Local<v8::Value>,
    ) -> Option<Self> {
        let pattern = identify(scope, value)?;
        let object = v8::Local::<v8::Object>::try_from(value).ok()?;
        Some(Self {
            pattern,
            object: v8::Global::new(scope, object),
        })
    }
}

fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "CanvasPattern".into(),
        member: name.into(),
    }
}

pub(super) fn install_factory(
    scope: &mut v8::PinScope,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "createPattern")
        .ok_or(V8BindingError::Allocation("pattern factory name"))?;
    let function = v8::Function::builder(factory)
        .length(2)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("pattern factory"))?;
    function.set_name(name);
    set_object_property(scope, prototype, "createPattern", function.into())
}

fn factory(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let canvas = canvas_context_owner(scope, args.this()).ok_or_else(|| error("TypeError"))?;
        let owner = binding_state_for_receiver(scope, canvas).ok_or_else(|| error("TypeError"))?;
        canvas_receiver_node(scope, &owner, &args)?;
        if args.length() < 2 {
            return Err(error("TypeError"));
        }
        let value = args.get(0);
        // Brand conversion precedes repetition string conversion, but source
        // usability and pixel snapshotting follow all argument conversions.
        let native = video_frame::identify(scope, value).is_some()
            || offscreen_canvas::identify(scope, value).is_some()
            || image_bitmap::identify(scope, value).is_some();
        if !native {
            let node = argument_node(scope, &owner, value)?;
            let source_owner = active_document_state_for_node(scope, &owner, node)
                .unwrap_or_else(|| owner.clone());
            let dom = source_owner.dom.borrow();
            if !["canvas", "img", "video"]
                .iter()
                .any(|name| dom.is_element(node, "http://www.w3.org/1999/xhtml", name))
                && !dom.is_element(node, "http://www.w3.org/2000/svg", "image")
            {
                return Err(error("TypeError"));
            }
        }
        let repetition = if args.get(1).is_null() {
            String::new()
        } else {
            args.get(1)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope)
        };
        let repeat = Repeat::parse(&repetition).ok_or_else(|| error("SyntaxError"))?;
        let Some(image) = image_bitmap::source(scope, &owner, value, false)? else {
            return Ok(v8::null(scope).into());
        };
        if image.width == 0 || image.height == 0 {
            return Err(error("InvalidStateError"));
        }
        let id = register_platform_object(
            &owner,
            PlatformObjectRecord::CanvasPattern(Rc::new(RefCell::new(Pattern {
                image,
                repeat,
                transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }))),
        );
        new_platform_object(scope, &owner, "CanvasPattern", id).map(Into::into)
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}

pub(super) fn identify(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Option<Rc<RefCell<Pattern>>> {
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
        PlatformObjectRecord::CanvasPattern(pattern) => Some(pattern.clone()),
        _ => None,
    }
}

pub(super) fn set_transform<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let pattern = identify(scope, object.into()).ok_or_else(|| V8BindingError::Unsupported {
        interface: "CanvasPattern".into(),
        member: "TypeError".into(),
    })?;
    // Conversion and alias validation finish before any mutation. Nonfinite
    // components are ignored, rather than resetting an existing transform.
    let (matrix, _) = matrix_dictionary_components_mode(scope, args.get(0), false)?;
    let transform = [
        matrix[0], matrix[1], matrix[4], matrix[5], matrix[12], matrix[13],
    ];
    if transform.iter().all(|component| component.is_finite()) {
        pattern.borrow_mut().transform = transform;
    }
    Ok(v8::undefined(scope).into())
}

//! Native immutable Canvas metrics and the public measurement boundary.
use super::*;

pub(super) struct Record {
    values: [f64; 12],
    _allocation: std::sync::Arc<dyn Send + Sync>,
}
const NAMES: [&str; 12] = ["width", "actualBoundingBoxLeft", "actualBoundingBoxRight",
    "fontBoundingBoxAscent", "fontBoundingBoxDescent", "actualBoundingBoxAscent",
    "actualBoundingBoxDescent", "emHeightAscent", "emHeightDescent", "hangingBaseline",
    "alphabeticBaseline", "ideographicBaseline"];

pub(super) fn get<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>, member: &str)
    -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let state = binding_state_for_receiver(scope, object).ok_or(V8BindingError::Message("TypeError".into()))?;
    let id = stateful_id(scope, object)?;
    if !state.platform_wrappers.borrow().get(&id).and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(object.into())) {
        return Err(V8BindingError::Message("TypeError".into()));
    }
    let index = NAMES.iter().position(|name| *name == member).ok_or(V8BindingError::Message("TypeError".into()))?;
    let value = match &*platform_object(&state, id)? {
        PlatformObjectRecord::CanvasTextMetrics(record) => record.values[index],
        _ => return Err(V8BindingError::Unsupported { interface: "TextMetrics".into(), member: "TypeError".into() }),
    };
    Ok(v8::Number::new(scope, value).into())
}

fn create<'s>(scope: &mut v8::PinScope<'s, '_>, state: &Rc<V8BindingState>, metrics: oryn_layout::canvas_text::metrics::Metrics)
    -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let values = [metrics.width, metrics.actual_bounding_box_left, metrics.actual_bounding_box_right,
        metrics.font_bounding_box_ascent, metrics.font_bounding_box_descent,
        metrics.actual_bounding_box_ascent, metrics.actual_bounding_box_descent,
        metrics.em_height_ascent, metrics.em_height_descent, metrics.hanging_baseline,
        metrics.alphabetic_baseline, metrics.ideographic_baseline];
    let allocation = font_loading::reserve_font_container(scope, state, std::mem::size_of::<Record>())?
        .ok_or(V8BindingError::Allocation("TextMetrics metadata admission unavailable"))?;
    let id = register_platform_object(state, PlatformObjectRecord::CanvasTextMetrics(Record { values, _allocation: allocation }));
    match new_platform_object(scope, state, "TextMetrics", id) {
        Ok(object) => Ok(object),
        Err(error) => { state.platform_objects.borrow_mut().remove(&id); Err(error) }
    }
}

pub(super) fn install(scope: &mut v8::PinScope, prototype: v8::Local<v8::Object>) -> Result<(), V8BindingError> {
    let name = v8::String::new(scope, "measureText").ok_or(V8BindingError::Allocation("Canvas measurement name"))?;
    let function = v8::Function::builder(measure).length(1).constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope).ok_or(V8BindingError::Allocation("Canvas measurement method"))?;
    function.set_name(name);
    set_object_property(scope, prototype, "measureText", function.into())
}

fn measure(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| {
        let owner = canvas_context_owner(scope, args.this()).ok_or(V8BindingError::Unsupported {
            interface: "CanvasRenderingContext2D".into(), member: "TypeError".into() })?;
        let state = binding_state_for_receiver(scope, owner).ok_or(V8BindingError::Unsupported {
            interface: "CanvasRenderingContext2D".into(), member: "TypeError".into() })?;
        let canvas = canvas_receiver_node(scope, &state, &args)?;
        if args.length() < 1 { return Err(V8BindingError::Unsupported { interface: "CanvasRenderingContext2D".into(), member: "TypeError".into() }); }
        // Brand precedes ToString. Capture drawing state only after author
        // conversion, which may reset/resize the canvas or replace its stream.
        let value = args.get(0).to_string(scope).ok_or(V8BindingError::JavaScriptException)?;
        if canvas_receiver_node(scope, &state, &args)? != canvas { return Err(V8BindingError::Message("InvalidStateError".into())); }
        let _entry = state.enter_retained_allocation(scope)?;
        let text = canvas_text_state::owned_string(scope, &state, value)?;
        refresh_layout_in_scope(scope, &state)?;
        let draw = state.canvas_draw_states.borrow().get(&canvas).map(|draw| draw.text.clone()).unwrap_or_default();
        let provider = state.native_layout_provider().ok_or(V8BindingError::Allocation("Canvas measurement native provider"))?;
        let request = BindingCanvasFontRequest { document: state.document_root(), canvas,
            document_generation: state.document_generation(), stream_epoch: state.document_stream_epoch(),
            style_generation: provider.layout_revision(), source_element: offscreen_canvas::identify(scope, owner.into()).is_none() };
        let inputs = state.native_font_layout_inputs()?;
        let count = inputs.states.len().checked_add(state.scroll_offsets.borrow().len()).ok_or(V8BindingError::Allocation("Canvas measurement maps overflow"))?;
        let mut bytes = count.checked_mul(128).ok_or(V8BindingError::Allocation("Canvas measurement maps overflow"))?;
        for (_, values) in &inputs.states {
            for value in values { bytes = bytes.checked_add(128).and_then(|n| n.checked_add(value.len())).ok_or(V8BindingError::Allocation("Canvas measurement states overflow"))?; }
        }
        let _maps = font_loading::reserve_font_container(scope, &state, bytes)?;
        let states = inputs.states.iter().map(|(node, values)| (*node, values.iter().cloned().collect())).collect();
        let scrolls = state.element_scroll_offsets();
        // Resolve the current source independently of stored drawing state so
        // direction:inherit observes current CSS, not font-setter-time CSS.
        let current_font = provider.resolve_canvas_font(&state.dom, request,
            draw.font.as_ref().map_or("10px sans-serif", |font| font.css.value.as_str()), &states,
            state.document_scroll_offset(), &scrolls).ok_or(V8BindingError::Allocation("Canvas measurement font provider unavailable"))?
            .map_err(|error| match error { BindingCanvasFontError::Syntax => V8BindingError::Message("Canvas stored font became invalid".into()), BindingCanvasFontError::Native(error) => V8BindingError::Message(error) })?;
        let rtl = draw.rtl(current_font.inherited_rtl());
        let font = draw.font.as_deref().unwrap_or(&current_font);
        let projection_bytes = font.shaping.declarations.iter().try_fold(0usize, |n, (key, value)| n.checked_add(128).and_then(|n| n.checked_add(key.len())).and_then(|n| n.checked_add(value.len())))
            .ok_or(V8BindingError::Allocation("Canvas shaping projection overflow"))?;
        let _projection = font_loading::reserve_font_container(scope, &state, projection_bytes.checked_add(256)
            .and_then(|bytes| bytes.checked_add(draw.shaping_projection_bytes()?)).ok_or(V8BindingError::Allocation("Canvas shaping projection overflow"))?)?;
        let mut style: BTreeMap<String, String> = font.shaping.declarations.iter().cloned().collect();
        style.insert("direction".into(), if rtl { "rtl" } else { "ltr" }.into());
        draw.apply_shaping(&mut style);
        let input = provider.prepare_canvas_text_input(&state.dom, request, &states, state.document_scroll_offset(), &scrolls)
            .ok_or(V8BindingError::Allocation("Canvas measurement capture unavailable"))?.map_err(V8BindingError::Message)?;
        let shape = oryn_layout::canvas_text::shape_font_bytes_admitted(&text.value, &style, input.viewport, &input.fonts.fonts,
            Some(input.cancelled.clone()), input.font_byte_admission.clone()).map_err(|error| V8BindingError::Message(error.to_string()))?;
        let metrics = shape.measure_text(&text.value, draw.baseline(), shape.width * draw.alignment().fraction(rtl))
            .map_err(|error| V8BindingError::Message(error.to_string()))?;
        provider.validate_canvas_text_input(&state.dom, &input).map_err(V8BindingError::Message)?;
        if state.execution_tasks_stopped() || state.document_root() != request.document || state.document_generation() != request.document_generation
            || state.document_stream_epoch() != request.stream_epoch || canvas_receiver_node(scope, &state, &args)? != canvas {
            return Err(V8BindingError::Message("InvalidStateError".into()));
        }
        create(scope, &state, metrics)
    })();
    match result { Ok(object) => rv.set(object.into()), Err(error) => throw_binding_error(scope, &error) }
}

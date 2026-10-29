//! Native conversion of CanvasRenderingContext2DSettings. Surface selection
//! must consume this result atomically; parsing alone does not create a context.
use super::*;

pub(super) fn opaque(state: &V8BindingState, node: NodeId) -> bool {
    state.canvas_context_settings.borrow().get(&node).is_some_and(|settings| !settings.alpha)
}

pub(super) fn finish(state: &V8BindingState, node: NodeId, bitmap: &mut DecodedImage) {
    if !opaque(state, node) { return; }
    for pixel in bitmap.rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
        pixel[3] = 255;
    }
}

pub(super) fn empty(state: &V8BindingState, node: NodeId, width: u32, height: u32)
    -> Result<DecodedImage, V8BindingError>
{
    let mut bitmap = empty_canvas_bitmap(width, height)?;
    finish(state, node, &mut bitmap);
    Ok(bitmap)
}

fn attributes(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let result = (|| -> Result<v8::Local<v8::Object>, V8BindingError> {
        let owner = canvas_context_owner(scope, args.this()).ok_or_else(|| {
            throw_type_error(scope, "Canvas context receiver required");
            V8BindingError::JavaScriptException
        })?;
        let state = binding_state_for_receiver(scope, owner)
            .ok_or(V8BindingError::Allocation("canvas context owner"))?;
        let node = canvas_receiver_node(scope, &state, &args)?;
        let requested = state.canvas_context_settings.borrow().get(&node).copied()
            .ok_or(V8BindingError::Allocation("canvas native settings"))?;
        let object = v8::Object::new(scope);
        // Native pixels and sampling are currently synchronous straight RGBA8
        // in sRGB. Report the actual backing, never an unsupported request.
        for name in ["alpha", "colorSpace", "colorType", "desynchronized", "willReadFrequently"] {
            let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("canvas settings key"))?;
            let value: v8::Local<v8::Value> = match name {
                "colorSpace" | "colorType" => v8::String::new(scope, if name == "colorSpace" { "srgb" } else { "unorm8" })
                    .ok_or(V8BindingError::Allocation("canvas settings value"))?.into(),
                _ => v8::Boolean::new(scope, match name {
                    "alpha" => requested.alpha,
                    "willReadFrequently" => requested.will_read_frequently,
                    _ => false,
                }).into(),
            };
            if object.create_data_property(scope, key.into(), value) != Some(true) {
                return Err(V8BindingError::Allocation("canvas settings result"));
            }
        }
        Ok(object)
    })();
    match result { Ok(value) => rv.set(value.into()), Err(error) => throw_binding_error(scope, &error) }
}

pub(super) fn install(scope: &mut v8::PinScope, prototype: v8::Local<v8::Object>) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, "getContextAttributes").ok_or(V8BindingError::Allocation("canvas settings method"))?;
    let function = v8::Function::builder(attributes).length(0).build(scope)
        .ok_or(V8BindingError::Allocation("canvas settings function"))?;
    if prototype.define_own_property(scope, key.into(), function.into(), v8::PropertyAttribute::NONE) != Some(true) {
        return Err(V8BindingError::Allocation("canvas settings method"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Requested {
    pub(super) alpha: bool,
    pub(super) color_space: &'static str,
    pub(super) color_type: &'static str,
    pub(super) desynchronized: bool,
    pub(super) will_read_frequently: bool,
}

fn member<'s>(scope: &mut v8::PinScope<'s, '_>, value: v8::Local<v8::Value>, name: &str)
    -> Result<Option<v8::Local<'s, v8::Value>>, V8BindingError>
{
    let result = checked_dictionary_member(scope, value, name)?;
    if !crate::v8_termination::checkpoint(scope) {
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(result)
}

fn boolean(scope: &mut v8::PinScope, value: v8::Local<v8::Value>, name: &str, default: bool)
    -> Result<bool, V8BindingError>
{
    Ok(member(scope, value, name)?.map_or(default, |value| value.boolean_value(scope)))
}

fn enumeration(scope: &mut v8::PinScope, value: v8::Local<v8::Value>, name: &str,
    default: &'static str, values: &[&'static str]) -> Result<&'static str, V8BindingError>
{
    let Some(value) = member(scope, value, name)? else { return Ok(default); };
    let string = value.to_string(scope).ok_or(V8BindingError::JavaScriptException)?;
    if !crate::v8_termination::checkpoint(scope) {
        return Err(V8BindingError::JavaScriptException);
    }
    let string = string.to_rust_string_lossy(scope);
    if let Some(value) = values.iter().copied().find(|value| *value == string) {
        return Ok(value);
    }
    throw_type_error(scope, &format!("invalid CanvasRenderingContext2DSettings {name}"));
    Err(V8BindingError::JavaScriptException)
}

pub(super) fn parse(scope: &mut v8::PinScope, value: v8::Local<v8::Value>)
    -> Result<Requested, V8BindingError>
{
    if !value.is_null_or_undefined() && !value.is_object() {
        throw_type_error(scope, "CanvasRenderingContext2DSettings must be a dictionary");
        return Err(V8BindingError::JavaScriptException);
    }
    // Observable Get/conversion order is alphabetical, not IDL source order.
    let alpha = boolean(scope, value, "alpha", true)?;
    let color_space = enumeration(scope, value, "colorSpace", "srgb",
        &["srgb", "srgb-linear", "display-p3", "display-p3-linear"])?;
    let color_type = enumeration(scope, value, "colorType", "unorm8", &["unorm8", "float16"])?;
    let desynchronized = boolean(scope, value, "desynchronized", false)?;
    let will_read_frequently = boolean(scope, value, "willReadFrequently", false)?;
    Ok(Requested { alpha, color_space, color_type, desynchronized, will_read_frequently })
}

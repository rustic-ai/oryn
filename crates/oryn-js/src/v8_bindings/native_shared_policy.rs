//! Isolation exposure derived from the parent broker and native ancestry.
use super::*;
use crate::shared_memory::SharedMemoryRequest;
use crate::worker_wire::{AgentServiceRequest, AgentServiceResponse, Creator};

pub(super) fn bind(scope: &mut v8::PinScope, state: &V8BindingState) -> bool {
    bind_at(scope, state, 0)
}
fn bind_at(scope: &mut v8::PinScope, state: &V8BindingState, depth: usize) -> bool {
    if depth >= 256 || state.execution_retired() {
        return false;
    }
    if state.refresh_native_container_policy().is_err() {
        return false;
    }

    let Ok(creator) = state.native_worker_creator() else {
        return false;
    };
    let Some(transport) = state.worker_transport.borrow().clone() else {
        return false;
    };
    let parent = if matches!(
        &creator,
        Creator::Window {
            target: crate::message_wire::WindowTarget::Frame(_),
            ..
        }
    ) {
        let Some(global) = state.global_object.to_local(scope) else {
            return false;
        };
        let parent = window_document::native_parent(scope, global);
        if parent.strict_equals(global.into()) {
            return false;
        }
        let Some(context) = parent.get_creation_context(scope) else {
            return false;
        };
        let Some(parent_state) = context.get_slot::<V8BindingState>() else {
            return false;
        };
        if !bind_at(scope, &parent_state, depth + 1) {
            return false;
        }
        let Ok(parent) = parent_state.native_worker_creator() else {
            return false;
        };
        Some(parent)
    } else {
        None
    };
    if workers::publish_secure_context(state).is_err() {
        return false;
    }
    matches!(transport.request(AgentServiceRequest::SharedMemory{creator,request:SharedMemoryRequest::Bind{parent}}),
        Ok(AgentServiceResponse::SharedMemory(response)) if response.isolated)
}
fn getter(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue) {
    let context = scope.get_current_context();
    let Some(state) = context.get_slot::<V8BindingState>() else {
        rv.set_bool(false);
        return;
    };
    if !global_events::is_global(scope, &state, args.this()) {
        throw_type_error(scope, "crossOriginIsolated receiver is not a native global");
        return;
    }
    rv.set_bool(bind(scope, &state));
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
) -> Result<(), V8BindingError> {
    let getter = v8::Function::builder(getter)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("crossOriginIsolated getter"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key = v8::String::new(scope, "crossOriginIsolated")
        .ok_or(V8BindingError::Allocation("isolation name"))?;
    if context
        .global(scope)
        .define_property(scope, key.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("isolation exposure"));
    }
    Ok(())
}

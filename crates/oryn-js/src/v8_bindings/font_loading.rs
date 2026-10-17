use super::*;
#[path="font_cycles.rs"] mod font_cycles;
#[path="font_layout_readiness.rs"] mod font_layout_readiness;
include!("font_promise_state.rs");
include!("font_receiver.rs");
include!("font_registry.rs");
include!("font_container_capacity.rs");
include!("font_loading_cycles.rs");
include!("font_loading_bindings.rs");
include!("font_loading_operations.rs");
#[cfg(test)]
include!("font_load_settlement_controls.rs");
include!("font_loading_membership.rs");
include!("font_set_iterators.rs");
include!("font_loading_retirement.rs");
include!("font_loading_native_bridge.rs");
include!("font_loading_events.rs");
include!("font_source_input.rs");
include!("font_owned_input.rs");

// Scalar bridge operations keep State fields private to this compilation unit.
pub(super) fn provider_matches(state:&V8BindingState,identity:usize)->bool{
    let provider=state.font_loading.borrow().provider.clone();
    provider.is_some_and(|provider|provider.same_provider(identity))
}
pub(super) fn loading_cycle(state:&V8BindingState)->u64{state.font_loading.borrow().cycles.cycle()}
pub(super) fn loading_materialized(state:&V8BindingState)->bool{
    let fonts=state.font_loading.borrow();fonts.document_set.is_some()||!fonts.face_objects.is_empty()
}

pub mod backend_adapter;
pub mod engine;
pub mod result;

// Re-export resolution logic from oryn-core
pub use oryn_core::resolution::{
    AssociationResult, CommandMeta, ContainerType, InferenceRule, ResolutionContext,
    ResolutionEngine as CoreResolutionEngine, SelectorError, SelectorResolver, TargetRequirement,
    find_associated_control, get_inference_rules, is_actionable_label, is_inside,
    validate_requirement,
};

pub use backend_adapter::BackendSelectorResolver;
pub use engine::ResolutionEngine;
pub use result::{Resolution, ResolutionError};

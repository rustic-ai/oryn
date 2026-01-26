pub mod engine;
pub mod result;

// Re-export resolution logic from oryn-core
pub use oryn_core::resolution::{
    AssociationResult, CommandMeta, ContainerType, ResolutionContext, TargetRequirement,
    find_associated_control, get_inference_rules, is_actionable_label, is_inside,
    validate_requirement, InferenceRule,
};

pub use engine::ResolutionEngine;
pub use result::{Resolution, ResolutionError};

pub mod command_meta;
pub mod context;
pub mod engine;
pub mod inference;
pub mod requirement;
pub mod result;

pub use command_meta::CommandMeta;
pub use context::{ResolutionContext, is_inside};
pub use engine::ResolutionEngine;
pub use inference::InferenceRule;
pub use requirement::{ContainerType, TargetRequirement};
pub use result::{Resolution, ResolutionError};

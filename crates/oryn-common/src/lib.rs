pub mod agent;
pub mod error;
pub mod error_mapping;
pub mod formatter;
pub mod protocol;
pub mod resolver;
pub mod v2;
pub mod v3;
pub mod v4;
pub mod v5;
pub mod v6;

/// Shared parent/worker compatibility pin for the production style pipeline.
/// Keeping this in the V8-free contract crate prevents manifest verification
/// from drifting from the worker implementation.
pub const STYLE_ENGINE_VERSION: &str = "stylo-computed-values-0.20+taffy-grid-v1";

pub mod intent {
    pub mod define_parser;
    pub mod definition;
    pub mod registry;
}

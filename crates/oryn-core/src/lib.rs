pub mod api;
pub mod ast;
pub mod normalizer;
pub mod parser;
pub mod translator;
pub mod resolution;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use api::{process_command, ProcessError, ProcessedCommand};
pub use ast::*;
pub use normalizer::normalize;
pub use parser::{parse, OilParser, Rule};

#[cfg(target_arch = "wasm32")]
pub use wasm::*;

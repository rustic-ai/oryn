//! Narrow JavaScript-engine boundary for Oryn.

pub mod async_ops;
pub mod bindings;
pub mod generated;
pub mod indexed_db;
pub mod mime_type;
pub mod navigation;
pub mod origin;
pub mod platform;
pub mod platform_admission;
pub mod realm;
pub mod scheduler;
pub mod trusted_types;

#[cfg(feature = "v8-host")]
mod pattern_service;
#[cfg(feature = "v8-host")]
pub mod v8_bindings;

pub const BINDING_ABI_VERSION: u16 = 3;

#[cfg(feature = "v8-host")]
pub mod v8_context_bridge;

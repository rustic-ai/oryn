//! Narrow JavaScript-engine boundary for Oryn.

pub mod async_ops;
pub mod bindings;
pub mod generated;
pub mod indexed_db;
pub mod platform;
pub mod realm;
pub mod scheduler;
pub mod trusted_types;

#[cfg(feature = "v8-host")]
pub mod v8_bindings;

pub const BINDING_ABI_VERSION: u16 = 3;

#[cfg(feature = "v8-host")]
pub mod v8_context_bridge;

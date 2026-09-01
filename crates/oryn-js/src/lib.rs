//! Narrow JavaScript-engine boundary for Oryn.

pub mod async_ops;
pub mod bindings;
pub mod generated;
pub mod realm;
pub mod scheduler;
pub mod trusted_types;

#[cfg(feature = "v8-host")]
pub mod v8_bindings;

pub const BINDING_ABI_VERSION: u16 = 2;

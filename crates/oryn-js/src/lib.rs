//! Narrow JavaScript-engine boundary for Oryn.

pub mod async_ops;
pub mod bindings;
pub mod crypto_key;
pub mod generated;
pub mod import_maps;
pub mod indexed_db;
pub mod integrity;
pub mod message_wire;
pub mod mime_type;
pub mod navigation;
pub mod navigation_history;
pub mod navigation_timing;
pub mod origin;

pub mod fetch_later;
pub mod permissions_policy;
pub mod platform;
pub mod platform_admission;
pub mod prompts;
pub mod realm;
pub mod scheduler;
pub mod script_source;
pub mod speculation_rules;
pub mod trusted_types;
pub mod user_activation;
pub mod worker_wire;

#[cfg(feature = "v8-host")]
mod pattern_service;
#[cfg(feature = "v8-host")]
pub mod v8_bindings;
#[cfg(feature = "v8-host")]
pub use v8_bindings::navigation_api;
#[cfg(feature = "v8-host")]
pub mod joint_history;

pub const BINDING_ABI_VERSION: u16 = 3;

#[cfg(feature = "v8-host")]
pub mod v8_context_bridge;

#[cfg(all(feature = "v8-host", unix))]
pub mod v8_shared_memory;
#[cfg(all(feature = "v8-host", unix))]
pub mod v8_shared_wait;

pub mod data_transfer;

pub mod form_state;

pub mod clipboard;

pub mod service_worker_wire;

pub mod drag_image;

pub mod cache_wire;

pub mod accessibility;

pub mod shared_memory;

pub mod composition;

pub mod shared_wait_custody;

pub mod platform_control;

pub mod media_capture;

pub mod server_timing;

#[cfg(feature = "v8-host")]
pub mod v8_initialization;

#[cfg(feature = "v8-host")]
pub mod v8_termination;

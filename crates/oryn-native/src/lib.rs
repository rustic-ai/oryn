//! Oryn's native browser runtime.
//!
//! This crate begins as a deliberately compact G0 spine. Browser-owned DOM,
//! scheduling, security policy, semantics, and tracing live here until
//! measurements justify narrower packages.

use serde::Serialize;

pub mod dom;
pub mod event_loop;
#[cfg(feature = "v8-host")]
pub mod executable;
pub mod host;
pub mod html;
pub mod network;
pub mod oil;
pub mod runtime;
pub mod security;
pub mod semantic;
pub mod trace;
pub mod worker;

pub use runtime::{
    Browser, BrowserConfig, BrowserContext, ContentKind, ContextOptions, LifecycleState,
    NavigationResult, PageHandle, WaitPredicate,
};

/// Machine-readable fingerprint for callers that must prove they launched the
/// production native runtime rather than the deliberately small no-V8 probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeBuildInfo {
    pub contract_version: u16,
    pub native_v8: bool,
    pub v8_version: Option<&'static str>,
    pub build_profile: &'static str,
    pub capabilities: Vec<&'static str>,
}

pub fn runtime_build_info() -> RuntimeBuildInfo {
    let mut capabilities = vec![
        "native_dom",
        "html5ever",
        "rustls_network_broker",
        "oil",
        "semantic_v2",
    ];
    #[cfg(feature = "v8-host")]
    capabilities.push("v8_javascript");

    RuntimeBuildInfo {
        contract_version: oryn_common::v2::CONTRACT_VERSION,
        native_v8: cfg!(feature = "v8-host"),
        #[cfg(feature = "v8-host")]
        v8_version: Some(v8::V8::get_version()),
        #[cfg(not(feature = "v8-host"))]
        v8_version: None,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        capabilities,
    }
}

//! Oryn's native browser runtime.
//!
//! This crate begins as a deliberately compact G0 spine. Browser-owned DOM,
//! scheduling, security policy, semantics, and tracing live here until
//! measurements justify narrower packages.

use serde::{Deserialize, Serialize};

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
#[cfg(unix)]
pub mod worker_client;

pub use runtime::{
    Browser, BrowserConfig, BrowserContext, ContentKind, ContextOptions, LifecycleState,
    NavigationResult, PageHandle, WaitPredicate,
};

/// Machine-readable fingerprint for callers that must prove they launched the
/// production native runtime rather than the deliberately small no-V8 probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBuildInfo {
    pub contract_version: u16,
    pub native_v8: bool,
    pub v8_version: Option<String>,
    pub build_profile: String,
    pub capabilities: Vec<String>,
    pub execution_mode: String,
    pub sandboxed: bool,
    pub worker_protocol_version: Option<u32>,
    pub worker_binary_sha256: Option<String>,
    pub worker_bundle_id: Option<String>,
    pub worker_team_id: Option<String>,
    pub worker_signing_cert_sha1: Option<String>,
    pub policy_digest: Option<String>,
    pub sandbox_state: String,
    pub limits: Option<worker::WorkerLimits>,
}

pub fn runtime_build_info() -> RuntimeBuildInfo {
    let capabilities = vec![
        "native_dom".into(),
        "html5ever".into(),
        "rustls_network_broker".into(),
        "oil".into(),
        "semantic_v2".into(),
    ];
    #[cfg(feature = "v8-host")]
    let capabilities = {
        let mut capabilities = capabilities;
        capabilities.push("v8_javascript".into());
        capabilities
    };

    RuntimeBuildInfo {
        contract_version: oryn_common::v2::CONTRACT_VERSION,
        native_v8: cfg!(feature = "v8-host"),
        #[cfg(feature = "v8-host")]
        v8_version: Some(v8::V8::get_version().into()),
        #[cfg(not(feature = "v8-host"))]
        v8_version: None,
        build_profile: if cfg!(debug_assertions) {
            "debug".into()
        } else {
            "release".into()
        },
        capabilities,
        execution_mode: "in_process_probe".into(),
        sandboxed: false,
        worker_protocol_version: None,
        worker_binary_sha256: None,
        worker_bundle_id: None,
        worker_team_id: None,
        worker_signing_cert_sha1: None,
        policy_digest: None,
        sandbox_state: "in_process_probe".into(),
        limits: None,
    }
}

/// Report the execution boundary used by production native callers. On Unix
/// release builds this performs the same verified worker handshake as a page.
pub fn production_runtime_build_info() -> Result<RuntimeBuildInfo, runtime::RuntimeError> {
    production_runtime_build_info_with_options(ContextOptions::default())
}

pub fn production_runtime_build_info_with_options(
    options: ContextOptions,
) -> Result<RuntimeBuildInfo, runtime::RuntimeError> {
    #[cfg(unix)]
    if runtime::uses_worker_process() {
        return worker_client::runtime_report(&BrowserConfig::default(), &options);
    }
    Ok(runtime_build_info())
}

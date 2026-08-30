//! Oryn's native browser runtime.
//!
//! This crate provides the compact native-runtime foundation. Browser-owned DOM,
//! scheduling, security policy, semantics, and tracing live here until
//! measurements justify narrower packages.

// Cargo's `--all-features` validation intentionally enables both profile
// markers. In that tooling-only combination the stricter `core-lite` guards
// take precedence; release builds select exactly one profile.

use serde::{Deserialize, Serialize};

pub mod challenge;
#[cfg(unix)]
pub mod context_service;
pub mod dom;
pub mod download;
pub mod event_loop;
#[cfg(feature = "v8-host")]
#[path = "rust_executable.rs"]
pub mod executable;
pub mod execution;
pub mod host;
pub mod html;
#[cfg(target_os = "linux")]
pub mod linux_sandbox;
pub mod network;
pub mod oil;
pub mod replay;
pub mod runtime;
pub mod security;
pub mod semantic;
pub mod storage;
pub mod trace;
pub mod worker;
#[cfg(unix)]
pub mod worker_client;

pub use challenge::{
    CHALLENGE_DETECTOR_VERSION, ChallengeAssessment, ChallengeDisposition, ChallengeSignal,
    ChallengeSignalSource,
};
pub use download::{DownloadDescriptor, DownloadId};
pub use execution::{
    CapabilityId, CapabilityLedger, ConsequenceLevel, DomainDecision, DomainDescriptor,
    DomainReasonCode, DomainSelectionError, ExecutionDomainAdapter, ExecutionDomainPolicy,
    ExecutionRequirements, FormValueState, HandoffBundle, HandoffLoss, HandoffOptions,
    HandoffReceipt, OriginStorageState, select_execution_domain,
};
pub use oryn_common::v3::{PageDescriptor, PageId, PageLifecycleEvent, PagePredicate, PageStatus};
pub use replay::{
    CheckpointPage, CheckpointPayload, CheckpointRestore, ContextCheckpoint, DeterminismConfig,
    FailureArtifact, FailureArtifactPayload, IntentTraceRecord, NetworkArchive,
    NetworkReplayController, NondeterministicRegion, RecordedExchange, ReplayBranch,
    ReplayDivergence, ReplayDivergenceKind, ReplayRun, RestoredPage, TRACE_ARCHIVE_VERSION,
    TaskTraceArchive,
};
pub use runtime::{
    Browser, BrowserConfig, BrowserContext, CacheMode, ContentKind, ContextOptions, LifecycleState,
    NativeAction, NavigationResult, PageHandle, RuntimeError, RuntimeProfile, ScrollRequest,
    WaitPredicate,
};
pub use security::{
    ActionPolicy, ConfirmationToken, ConsequentialAction, NetworkPolicy, PolicyDecision,
    PolicyRequest, PolicyRule, SecretHandle, SecurityAuditRecord,
};
pub use storage::{PersistentProfile, StorageLimits};

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
    pub style_engine_version: Option<String>,
    pub box_engine_version: Option<String>,
    pub inline_engine_version: Option<String>,
    pub embedded_font_sha256: Option<String>,
    pub runtime_profile: String,
    pub binding_abi_version: u16,
}

pub fn runtime_build_info() -> RuntimeBuildInfo {
    let capabilities = vec![
        "native_dom".into(),
        "html5ever".into(),
        "rustls_network_broker".into(),
        "oil".into(),
        "semantic_v3_compat".into(),
        "semantic_v4".into(),
        "execution_domain_handoff".into(),
        "trace_recording".into(),
        "network_record_replay".into(),
        "sealed_context_checkpoint".into(),
        "replay_branching".into(),
    ];
    #[cfg(feature = "v8-host")]
    let capabilities = {
        let mut capabilities = capabilities;
        capabilities.push("v8_javascript".into());
        capabilities.push("generated_webidl_registry".into());
        capabilities.push("webassembly".into());
        capabilities.push(
            if cfg!(feature = "core-lite") {
                "intl_root_en_en_us"
            } else {
                "intl_full_icu"
            }
            .into(),
        );
        capabilities.extend([
            "computed_style".into(),
            "layout_box".into(),
            "layout_inline_text".into(),
            "viewport_scroll".into(),
            "deterministic_embedded_font".into(),
            "paint_snapshot".into(),
        ]);
        capabilities
    };

    RuntimeBuildInfo {
        contract_version: oryn_common::v4::CONTRACT_VERSION,
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
        #[cfg(feature = "v8-host")]
        style_engine_version: Some(oryn_layout::STYLE_ENGINE_VERSION.into()),
        #[cfg(not(feature = "v8-host"))]
        style_engine_version: None,
        #[cfg(feature = "v8-host")]
        box_engine_version: Some(oryn_layout::BOX_ENGINE_VERSION.into()),
        #[cfg(not(feature = "v8-host"))]
        box_engine_version: None,
        #[cfg(feature = "v8-host")]
        inline_engine_version: Some(oryn_layout::INLINE_ENGINE_VERSION.into()),
        #[cfg(not(feature = "v8-host"))]
        inline_engine_version: None,
        #[cfg(feature = "v8-host")]
        embedded_font_sha256: Some(oryn_layout::DEFAULT_FONT_SHA256.into()),
        #[cfg(not(feature = "v8-host"))]
        embedded_font_sha256: None,
        runtime_profile: if cfg!(feature = "core-lite") {
            "core_lite_english".into()
        } else {
            "core_full".into()
        },
        binding_abi_version: oryn_js::BINDING_ABI_VERSION,
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

//! Oryn's native browser runtime.
//!
//! This crate provides the compact native-runtime foundation. Browser-owned DOM,
//! scheduling, security policy, semantics, and tracing live here until
//! measurements justify narrower packages.

// Cargo's `--all-features` validation intentionally enables both profile
// markers. In that tooling-only combination the stricter `core-lite` guards
// take precedence; release builds select exactly one profile.

use serde::{Deserialize, Serialize};

#[cfg(all(feature = "v8-host", oryn_icu_data))]
#[repr(align(16))]
struct AlignedIcuData<const N: usize>([u8; N]);

#[cfg(all(feature = "v8-host", oryn_icu_data))]
static ICU_DATA: AlignedIcuData<{ include_bytes!(env!("ORYN_ICU_DATA_PATH")).len() }> =
    AlignedIcuData(*include_bytes!(env!("ORYN_ICU_DATA_PATH")));

/// Install the profile-specific ICU package before V8 or any ICU consumer is
/// initialized. Full builds link their data into V8 and therefore need no
/// host-provided package.
#[cfg(feature = "v8-host")]
pub fn initialize_v8_icu_data() {
    #[cfg(oryn_icu_data)]
    v8::icu::set_common_data_77(&ICU_DATA.0)
        .unwrap_or_else(|status| panic!("profile ICU data was rejected with status {status}"));
}

#[cfg(feature = "v8-host")]
mod agent;
#[cfg(feature = "v8-host")]
mod automation_trace;
pub mod challenge;
#[cfg(unix)]
pub mod context_service;
pub mod cookies;
pub mod csp;
pub mod dom;
pub mod download;
pub mod event_loop;
#[cfg(feature = "v8-host")]
#[path = "rust_executable.rs"]
pub mod executable;
pub mod execution;
pub mod host;
pub mod html;
#[cfg(feature = "v8-host")]
mod import_maps;
pub mod intent;
#[cfg(target_os = "linux")]
pub mod linux_sandbox;
pub mod network;
mod observable;
pub mod oil;
pub mod referrer;
pub mod replay;
pub mod resource_audit;
pub mod runtime;
pub mod security;
pub mod semantic;
pub mod storage;
pub mod trace;
pub mod upload;
pub mod worker;
#[cfg(unix)]
pub mod worker_client;
#[cfg(feature = "v8-host")]
pub mod worker_process;
pub mod worker_route;

pub use challenge::{
    CHALLENGE_DETECTOR_VERSION, ChallengeAssessment, ChallengeDisposition, ChallengeSignal,
    ChallengeSignalSource,
};
pub use cookies::{
    CookieCredentials, CookieDecision, CookieRequestContext, CookieRequestKind, SameSiteProfile,
    SchemefulSite,
};
pub use csp::{
    CspDecision, CspDestination, CspDisposition, CspPolicySet, CspReportFormat, CspReportTarget,
    CspRequest, CspViolation, SandboxFlags, TrustedTypesDirective,
};
pub use download::{DownloadDescriptor, DownloadId};
pub use execution::{
    CapabilityId, CapabilityLedger, ConsequenceLevel, DomainDecision, DomainDescriptor,
    DomainReasonCode, DomainSelectionError, ExecutionDomainAdapter, ExecutionDomainPolicy,
    ExecutionRequirements, FormValueState, HandoffBundle, HandoffLoss, HandoffOptions,
    HandoffReceipt, OriginStorageState, select_execution_domain,
};
pub use intent::IntentResult;
pub use oryn_common::v3::{PageDescriptor, PageId, PageLifecycleEvent, PagePredicate, PageStatus};
pub use referrer::ReferrerPolicy;
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
    WaitPredicate, WorkerLifecycle,
};
pub use security::{
    ActionPolicy, ConfirmationToken, ConsequentialAction, NetworkPolicy, PolicyDecision,
    PolicyRequest, PolicyRule, SecretHandle, SecurityAuditRecord,
};
pub use storage::{PersistentProfile, StorageLimits};
pub use upload::{UploadDescriptor, UploadFile, UploadHandle};

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
    pub worker_lifecycle: String,
    pub sandboxed: bool,
    pub worker_protocol_version: Option<u32>,
    pub worker_binary_sha256: Option<String>,
    pub worker_code_directory_hash: Option<String>,
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
    pub transport_mode: String,
    pub streaming_parser: bool,
    pub semantic_graph_version: u16,
    pub webidl_manifest_sha256: Option<String>,
    pub v8_heap_sandbox: bool,
    pub icu_data_sha256: Option<String>,
    pub startup_snapshot_sha256: Option<String>,
    pub external_references_sha256: Option<String>,
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
        capabilities.push("generated_web_platform_algorithms".into());
        capabilities.push("typed_platform_host_operations".into());
        capabilities.push("webassembly".into());
        capabilities.push("same_site_cookies".into());
        capabilities.push("csp3_agent_critical".into());
        capabilities.push("trusted_types_core".into());
        capabilities.push(
            if cfg!(feature = "core-lite") && option_env!("ORYN_ICU_DATA_SHA256").is_some() {
                "intl_root_en_en_us"
            } else if cfg!(feature = "core-lite") {
                "intl_lite_unattested_probe"
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
        worker_lifecycle: "isolated_on_demand".into(),
        sandboxed: false,
        worker_protocol_version: None,
        worker_binary_sha256: None,
        worker_code_directory_hash: None,
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
        runtime_profile: if cfg!(feature = "core-lite")
            && option_env!("ORYN_ICU_DATA_SHA256").is_some()
        {
            "core_lite_english".into()
        } else if cfg!(feature = "core-lite") {
            "core_lite_unattested_probe".into()
        } else {
            "core_full".into()
        },
        binding_abi_version: oryn_js::BINDING_ABI_VERSION,
        transport_mode: "async_streaming_parent_broker".into(),
        streaming_parser: true,
        semantic_graph_version: crate::semantic::SEMANTIC_GRAPH_VERSION,
        webidl_manifest_sha256: option_env!("ORYN_WEBIDL_MANIFEST_SHA256").map(str::to_owned),
        v8_heap_sandbox: cfg!(all(feature = "v8-host", feature = "v8-sandbox")),
        icu_data_sha256: option_env!("ORYN_ICU_DATA_SHA256").map(str::to_owned),
        startup_snapshot_sha256: option_env!("ORYN_STARTUP_SNAPSHOT_SHA256").map(str::to_owned),
        external_references_sha256: option_env!("ORYN_EXTERNAL_REFERENCES_SHA256")
            .map(str::to_owned),
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
    production_runtime_build_info_with_config(BrowserConfig::default(), options)
}

pub fn production_runtime_build_info_with_config(
    config: BrowserConfig,
    options: ContextOptions,
) -> Result<RuntimeBuildInfo, runtime::RuntimeError> {
    config.worker_lifecycle.validate()?;
    #[cfg(unix)]
    if runtime::uses_worker_process() {
        return worker_client::runtime_report(&config, &options);
    }
    let mut runtime = runtime_build_info();
    runtime.worker_lifecycle = config.worker_lifecycle.name().into();
    Ok(runtime)
}

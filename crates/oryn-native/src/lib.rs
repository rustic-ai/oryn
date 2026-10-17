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

/// Runtime and startup snapshots must use the same staged language features.
pub const V8_RUNTIME_FLAGS: &str = "--js-defer-import-eval";

/// Install the profile-specific ICU package before V8 or any ICU consumer is
/// initialized. Full builds link their data into V8 and therefore need no
/// host-provided package.
#[cfg(feature = "v8-host")]
pub fn initialize_v8_icu_data() {
    #[cfg(oryn_icu_data)]
    v8::icu::set_common_data_77(&ICU_DATA.0)
        .unwrap_or_else(|status| panic!("profile ICU data was rejected with status {status}"));
}

pub mod feature_profile;
pub mod fetch_metadata;
pub use feature_profile::{NativeFeatureProfile, UserMediaElementMode};
pub mod accessibility_platform;
#[cfg(feature = "v8-host")]
mod agent;
#[cfg(unix)]
mod agent_io;
#[cfg(unix)]
pub mod agent_ipc;
pub mod agent_platform;
pub mod agent_platform_service;
#[cfg(unix)]
pub mod agent_process;
pub mod agent_registry;
#[cfg(unix)]
pub mod agent_script;
#[cfg(unix)]
mod agent_service;
pub mod agent_thread;
#[cfg(feature = "v8-host")]
mod automation_trace;
pub mod challenge;
pub mod execution_placement;
pub(crate) mod http_cleanup_custody;
pub(crate) mod keepalive;
pub(crate) mod native_input_allowance;
#[doc(hidden)]
pub mod owned_action_request;
pub(crate) mod keepalive_drain;
mod local_resource_response;
pub mod navigation_control;
#[cfg(all(unix, any(feature = "v8-host", test)))]
mod navigation_execution;
#[cfg(unix)]
pub mod queued_navigation;
#[path = "resource_io_executor.rs"]
pub(crate) mod resource_io_executor;
#[cfg(feature = "v8-host")]
mod svg_capture;
#[cfg(feature = "v8-host")]
mod webdriver_action_execution;
pub mod webdriver_actions;
/// Lossless UTF-16 conversion for native String-backed DOM and prompt values.
pub use oryn_dom::dom_string;
/// V8-free native modal control types shared by parent and worker.
pub use oryn_js::prompts as user_prompts;
#[cfg(unix)]
pub mod context_service;
pub mod cookies;
pub mod csp;
pub mod dom;
pub mod download;
pub(crate) mod embedded_resource_type;
pub mod event_loop;
#[cfg(feature = "v8-host")]
#[path = "rust_executable.rs"]
pub mod executable;
pub mod execution;
mod frame_embedding;
pub(crate) mod frame_resource_response;
pub mod host;
pub mod html;
#[cfg(feature = "v8-host")]
mod import_maps;
mod integrity;
mod mixed_content;
mod module_types;
mod script_close_custody;
mod script_fetch;
pub mod script_load_failure;
mod script_load_lease;
mod support_descriptor;
mod support_response_state;
mod support_broker_occurrence;
pub use support_descriptor::PinnedSupportDescriptor;
mod script_policy_snapshot;
mod standalone_csp_reporting;
mod standalone_document_policy;
pub(crate) mod stylesheet_response_proof;
pub use module_types::ModuleType;
#[cfg(any(feature = "v8-host", test))]
mod html_encoding_replay;
pub mod intent;
mod ipc_progress;
#[cfg(target_os = "linux")]
pub mod linux_sandbox;
pub mod native_timing;
pub mod network;
mod observable;
pub mod oil;
mod page_terminal_control;
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
pub mod worker_lifecycle_audit;
#[cfg(feature = "v8-host")]
pub mod worker_process;
pub mod worker_route;
#[cfg(feature = "v8-host")]
mod xslt_process;

pub use challenge::{
    CHALLENGE_DETECTOR_VERSION, ChallengeAssessment, ChallengeDisposition, ChallengeSignal,
    ChallengeSignalSource,
};
pub use cookies::{
    CookieCredentials, CookieDecision, CookieRequestContext, CookieRequestKind, SameSiteProfile,
    SchemefulSite, ThirdPartyCookiePolicy,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeBuildInfo {
    pub render_contract_version: Option<u16>,
    pub local_effects_contract_version: u16,
    pub contract_version: u16,
    pub native_v8: bool,
    pub v8_version: Option<String>,
    pub build_profile: String,
    pub capabilities: Vec<String>,
    pub execution_mode: String,
    pub worker_lifecycle: String,
    pub sandboxed: bool,
    pub worker_protocol_version: Option<u32>,
    /// Compiled wire contracts, separate from evidence of a running worker.
    #[serde(default)]
    pub context_service_protocol_version: Option<u32>,
    #[serde(default)]
    pub agent_ipc_protocol_version: Option<u32>,
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
    #[serde(default)]
    pub v8_archive_sha256: Option<String>,
    /// Exact compiler/header ABI contract used by the native V8 bridge.
    #[serde(default)]
    pub v8_bridge_abi_sha256: Option<String>,
    #[serde(default)]
    pub v8_patch_chain_sha256: Option<String>,
    #[serde(default)]
    pub rusty_v8_source_manifest_sha256: Option<String>,
    #[serde(default)]
    pub rusty_v8_source_tree_sha256: Option<String>,
    #[serde(default)]
    pub rusty_v8_weak_teardown_patch_sha256: Option<String>,
    #[serde(default)]
    pub rusty_v8_patch_metadata_sha256: Option<String>,
    #[serde(default)]
    pub v8_binding_sha256: Option<String>,
    /// Required worker attestation: never infer this privilege from a parent build.
    pub historical_test_input: bool,
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
        #[cfg(any(feature = "v8-host", feature = "accessibility-host"))]
        render_contract_version: Some(oryn_render_core::RENDER_CONTRACT_VERSION),
        #[cfg(not(any(feature = "v8-host", feature = "accessibility-host")))]
        render_contract_version: None,
        local_effects_contract_version: oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION,
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
        #[cfg(unix)]
        context_service_protocol_version: Some(context_service::CONTEXT_SERVICE_PROTOCOL_VERSION),
        #[cfg(not(unix))]
        context_service_protocol_version: None,
        #[cfg(unix)]
        agent_ipc_protocol_version: Some(agent_ipc::VERSION),
        #[cfg(not(unix))]
        agent_ipc_protocol_version: None,
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
        historical_test_input: cfg!(feature = "historical-test-input"),
        v8_archive_sha256: option_env!("ORYN_V8_ARCHIVE_SHA256").map(str::to_owned),
        v8_bridge_abi_sha256: option_env!("ORYN_V8_BRIDGE_ABI_SHA256").map(str::to_owned),
        v8_patch_chain_sha256: option_env!("ORYN_V8_PATCH_CHAIN_SHA256").map(str::to_owned),
        rusty_v8_source_manifest_sha256: option_env!("ORYN_RUSTY_V8_SOURCE_MANIFEST_SHA256")
            .map(str::to_owned),
        rusty_v8_source_tree_sha256: option_env!("ORYN_RUSTY_V8_SOURCE_TREE_SHA256")
            .map(str::to_owned),
        rusty_v8_weak_teardown_patch_sha256: option_env!(
            "ORYN_RUSTY_V8_WEAK_TEARDOWN_PATCH_SHA256"
        )
        .map(str::to_owned),
        rusty_v8_patch_metadata_sha256: option_env!("ORYN_RUSTY_V8_PATCH_METADATA_SHA256")
            .map(str::to_owned),
        v8_binding_sha256: option_env!("ORYN_V8_BINDING_SHA256").map(str::to_owned),
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

pub mod workflow_instrumentation;

#[cfg(unix)]
mod service_registry;

#[cfg(unix)]
mod service_network;

#[cfg(test)]
mod historical_input_attestation_tests {
    #[test]
    fn runtime_wire_contracts_are_distinct_and_legacy_absence_is_not_invented() {
        let runtime = super::runtime_build_info();
        #[cfg(unix)]
        {
            assert_eq!(
                runtime.context_service_protocol_version,
                Some(super::context_service::CONTEXT_SERVICE_PROTOCOL_VERSION)
            );
            assert_eq!(
                runtime.agent_ipc_protocol_version,
                Some(super::agent_ipc::VERSION)
            );
        }
        let mut wire = serde_json::to_value(&runtime).unwrap();
        wire.as_object_mut()
            .unwrap()
            .remove("context_service_protocol_version");
        wire.as_object_mut()
            .unwrap()
            .remove("agent_ipc_protocol_version");
        let old: super::RuntimeBuildInfo = serde_json::from_value(wire).unwrap();
        assert_eq!(old.context_service_protocol_version, None);
        assert_eq!(old.agent_ipc_protocol_version, None);
    }
    #[test]
    fn runtime_attests_feature_and_missing_attribution_is_not_backward_compatible() {
        let runtime = super::runtime_build_info();
        assert_eq!(
            runtime.historical_test_input,
            cfg!(feature = "historical-test-input")
        );
        let mut wire = serde_json::to_value(&runtime).unwrap();
        assert_eq!(
            wire["historical_test_input"],
            cfg!(feature = "historical-test-input")
        );
        wire.as_object_mut()
            .unwrap()
            .remove("historical_test_input");
        assert!(serde_json::from_value::<super::RuntimeBuildInfo>(wire).is_err());
    }
}

mod shared_memory;

#[cfg(unix)]
mod shared_memory_fd;

mod agent_isolation;
mod coep_reporting;

mod shared_wait;

mod isolation_permissions;

pub mod browsing_groups;

mod native_locale;
mod native_locale_data;
mod platform_control;

pub use oryn_js::platform_control as platform_control_contract;

mod media_capture;
mod media_constraints;
pub use oryn_js::media_capture::DeviceConfiguration as MediaDeviceConfiguration;

#[cfg(test)]
mod render_contract_metadata_regressions {
    #[test]
    fn runtime_metadata_distinguishes_display_list_from_snapshot_extension() {
        let info = super::runtime_build_info();
        assert_eq!(
            info.local_effects_contract_version,
            oryn_common::v4::LOCAL_EFFECTS_CONTRACT_VERSION
        );
        #[cfg(any(feature = "v8-host", feature = "accessibility-host"))]
        assert_eq!(
            info.render_contract_version,
            Some(oryn_render_core::RENDER_CONTRACT_VERSION)
        );
        #[cfg(not(any(feature = "v8-host", feature = "accessibility-host")))]
        assert_eq!(info.render_contract_version, None);
        let serialized = serde_json::to_value(&info).unwrap();
        let restored: super::RuntimeBuildInfo = serde_json::from_value(serialized.clone()).unwrap();
        assert_eq!(restored, info);
        let mut historical = serialized;
        historical
            .as_object_mut()
            .unwrap()
            .remove("local_effects_contract_version");
        assert!(serde_json::from_value::<super::RuntimeBuildInfo>(historical).is_err());
    }
}

mod local_resource_preload;
#[cfg(unix)]
mod preload_cache;
mod render_request_status;

#[cfg(unix)]
mod context_event_journal;

#[cfg(all(test, unix, feature = "v8-host"))]
mod protocol_pending_contract_tests;
mod retained_size;

#[cfg(feature = "v8-host")]
mod navigation_timing_custody;

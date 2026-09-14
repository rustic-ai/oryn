//! Stable public façade for Oryn's native agent-browser runtime.
//!
//! Process protocol and worker implementation details remain in
//! `oryn-native`; product callers depend on this crate and use the same typed
//! page/context contracts as the CLI and language adapters.

pub mod mcp;
pub mod secret_input;

pub use oryn_common::agent::{
    AGENT_INTENT_CONTRACT_VERSION, ExtractKind, Input, Intent, IntentRequest,
    LEGACY_AGENT_INTENT_CONTRACT_VERSION, ObserveOptions, Target,
};
pub use oryn_common::v3::{
    CapabilityDiagnostic, Observation, ObservationDelta, PageDescriptor, PageId,
    PageLifecycleEvent, PagePredicate, PageStatus, ProjectionProfile, Revision, SemanticAction,
    SemanticRef,
};
pub use oryn_common::v5::{
    AccessibilityEvidence, AccessibilityEvidenceKind, AccessibilityNode, AccessibilityRelation,
    SemanticGroup, SemanticObservation, SemanticStructure, SemanticStructureKind,
};
pub use oryn_common::v6::{
    GeometryPolicy, ObservationScope, ProjectionSummary, ResolutionCandidate, ResolutionResult,
    SemanticDelta, SemanticField, SemanticFieldEvidence, SemanticNode,
    SemanticObservation as SemanticObservationV6, SemanticObservationOptions,
};
pub use oryn_native::{
    ActionPolicy, Browser, BrowserConfig, BrowserContext, CacheMode, ChallengeAssessment,
    ChallengeDisposition, ChallengeSignal, ChallengeSignalSource, ConfirmationToken,
    ConsequentialAction, ContentKind, ContextOptions, IntentResult, LifecycleState, NativeAction,
    NavigationResult, PageHandle, PersistentProfile, PolicyDecision, PolicyRequest, PolicyRule,
    RuntimeBuildInfo, RuntimeError, RuntimeProfile, ScrollRequest, SecretHandle, StorageLimits,
    WaitPredicate, WorkerLifecycle, production_runtime_build_info,
    production_runtime_build_info_with_config, production_runtime_build_info_with_options,
};

/// The versioned OIL adapter used by compatibility callers.
///
/// New Rust integrations should prefer typed `PageHandle` methods. OIL remains
/// a supported parser adapter over the same native runtime.
pub mod oil {
    pub use oryn_native::oil::{
        NativeContextOilSession, NativeOilError, NativeOilOutput, NativeOilSession,
    };
}

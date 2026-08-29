//! Stable public façade for Oryn's native agent-browser runtime.
//!
//! Process protocol and worker implementation details remain in
//! `oryn-native`; product callers depend on this crate and use the same typed
//! page/context contracts as the CLI and language adapters.

pub use oryn_common::v3::{
    CapabilityDiagnostic, Observation, ObservationDelta, PageDescriptor, PageId,
    PageLifecycleEvent, PagePredicate, PageStatus, ProjectionProfile, Revision, SemanticAction,
    SemanticRef,
};
pub use oryn_native::{
    ActionPolicy, Browser, BrowserConfig, BrowserContext, ConfirmationToken, ConsequentialAction,
    ContentKind, ContextOptions, LifecycleState, NativeAction, NavigationResult, PageHandle,
    PersistentProfile, PolicyDecision, PolicyRequest, PolicyRule, RuntimeBuildInfo, RuntimeError,
    ScrollRequest, SecretHandle, StorageLimits, WaitPredicate, production_runtime_build_info,
    production_runtime_build_info_with_options,
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

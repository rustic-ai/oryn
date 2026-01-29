//! Resolution Engine Wrapper for Backend Integration
//!
//! Thin wrapper around `oryn_core::resolution::ResolutionEngine` that adapts it
//! for use with the Backend trait by creating a `BackendSelectorResolver` internally.

use super::BackendSelectorResolver;
use super::result::ResolutionError;
use crate::backend::Backend;
use oryn_common::protocol::ScanResult;
use oryn_core::ast;
use oryn_core::resolution::ResolutionEngine as CoreEngine;

/// Resolution engine that works with Backend implementations.
pub struct ResolutionEngine;

impl ResolutionEngine {
    /// Resolve a command using a backend for selector queries.
    pub async fn resolve<B: Backend + ?Sized>(
        cmd: ast::Command,
        scan: &ScanResult,
        backend: &mut B,
    ) -> Result<ast::Command, ResolutionError> {
        let mut resolver = BackendSelectorResolver::new(backend);
        CoreEngine::resolve(cmd, scan, &mut resolver).await
    }
}

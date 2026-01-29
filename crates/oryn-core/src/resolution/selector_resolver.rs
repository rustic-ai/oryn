//! Selector Resolution Abstraction
//!
//! This module defines the trait for resolving CSS/XPath selectors to element IDs.
//! This is the ONLY backend-dependent operation in semantic target resolution.
//!
//! Implementations:
//! - Native backends: Use Backend::execute_scanner() to query the browser
//! - WASM: Use web_sys::querySelector() directly since it runs in-browser
//! - Tests: Use mock implementations

use thiserror::Error;

/// Resolve CSS/XPath selectors to element IDs.
///
/// This is the single abstraction point for backend-dependent selector queries.
/// All other resolution logic (text matching, inference, label association) is
/// backend-independent and operates on scan data.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait SelectorResolver {
    /// Returns `Ok(Some(id))` if matched, `Ok(None)` if not found, or `Err` on failure.
    async fn resolve_selector(&mut self, selector: &str) -> Result<Option<u32>, SelectorError>;
}

/// Errors that can occur during selector resolution.
#[derive(Debug, Clone, Error)]
pub enum SelectorError {
    /// Backend execution error (network, timeout, etc.)
    #[error("Backend error: {0}")]
    Backend(String),

    /// Invalid selector syntax
    #[error("Invalid selector: {0}")]
    InvalidSelector(String),

    /// Selector resolution not supported in this context
    #[error("Selector resolution not available: {0}")]
    NotAvailable(String),
}

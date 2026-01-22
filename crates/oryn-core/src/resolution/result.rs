use thiserror::Error;

/// Result of a resolution attempt.
#[derive(Debug)]
pub enum Resolution {
    /// Fully resolved to an element ID
    Resolved(u32),

    /// Needs a sub-resolution step (e.g., CSS selector query)
    /// The resolver will handle this and retry
    NeedsBrowserQuery(String), // CSS selector to query

    /// Resolution failed with reason
    Failed(ResolutionError),
}

#[derive(Debug, Clone, Error)]
#[error("Resolution failed for target '{target}': {reason}")]
pub struct ResolutionError {
    pub target: String,
    pub reason: String,
    pub attempted: Vec<String>, // Strategies tried
}

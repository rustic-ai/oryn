use crate::protocol::{ScannerProtocolResponse, ScannerRequest};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status: u16, // generic status code (e.g. 200)
}

#[derive(thiserror::Error, Debug)]
pub enum BackendError {
    #[error("Navigation failed: {0}")]
    Navigation(String),
    #[error("Scanner execution failed: {0}")]
    Scanner(String),
    #[error("Connection lost")]
    ConnectionLost,
    #[error("Timeout")]
    Timeout,
    #[error("Not ready")]
    NotReady,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Other: {0}")]
    Other(String),
}

/// The Backend trait is the unified interface all three binaries must implement.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Launch the backend (start browser, connect to remote, etc.)
    async fn launch(&mut self) -> Result<(), BackendError>;

    /// Close the backend and cleanup resources.
    async fn close(&mut self) -> Result<(), BackendError>;

    /// Check if the backend is ready to accept commands.
    async fn is_ready(&self) -> bool;

    /// Navigate to a specific URL.
    async fn navigate(&mut self, url: &str) -> Result<NavigationResult, BackendError>;

    /// Execute a raw scanner command.
    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError>;

    /// Capture a screenshot of the current viewport.
    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError>;
}

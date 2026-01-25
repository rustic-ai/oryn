use async_trait::async_trait;
pub use oryn_common::error::backend_error::BackendError;
use oryn_common::protocol::{ScannerAction, ScannerProtocolResponse};

#[derive(Debug, Clone)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status: u16, // generic status code (e.g. 200)
}

// Error definitions moved to oryn_common::error::backend_error

// Navigation Errors
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
        command: ScannerAction,
    ) -> Result<ScannerProtocolResponse, BackendError>;

    /// Execute a script in the browser context.
    /// This is a convenience method that might wrap a scanner request or call the backend directly.
    async fn execute_script(&mut self, _script: &str) -> Result<serde_json::Value, BackendError> {
        // Default implementation via Scanner request?
        // Or just return NotSupported by default.
        // Since it's used by Execute action, it should probably be supported or optional.
        // Let's make it return NotSupported by default.
        Err(BackendError::NotSupported("execute_script".into()))
    }

    /// Capture a screenshot of the current viewport.
    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError>;

    /// Generate a PDF of the current page.
    async fn pdf(&mut self) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::NotSupported("pdf".into()))
    }

    /// Get all cookies from the current session.
    async fn get_cookies(&mut self) -> Result<Vec<oryn_common::protocol::Cookie>, BackendError> {
        Err(BackendError::NotSupported("get_cookies".into()))
    }

    /// Set a cookie in the current session.
    async fn set_cookie(
        &mut self,
        _cookie: oryn_common::protocol::Cookie,
    ) -> Result<(), BackendError> {
        Err(BackendError::NotSupported("set_cookie".into()))
    }

    /// Get all open tabs/windows.
    async fn get_tabs(&mut self) -> Result<Vec<oryn_common::protocol::TabInfo>, BackendError> {
        Err(BackendError::NotSupported("get_tabs".into()))
    }

    /// Navigate back in browser history.
    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        Err(BackendError::NotSupported("go_back".into()))
    }

    /// Navigate forward in browser history.
    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        Err(BackendError::NotSupported("go_forward".into()))
    }

    /// Refresh the current page.
    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        Err(BackendError::NotSupported("refresh".into()))
    }

    /// Press a key (with optional modifiers).
    async fn press_key(&mut self, _key: &str, _modifiers: &[String]) -> Result<(), BackendError> {
        Err(BackendError::NotSupported("press_key".into()))
    }
}

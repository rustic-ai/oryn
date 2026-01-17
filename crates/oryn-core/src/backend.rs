use crate::protocol::{ScannerProtocolResponse, ScannerRequest};
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct NavigationResult {
    pub url: String,
    pub title: String,
    pub status: u16, // generic status code (e.g. 200)
}

/// Error codes aligned with SPEC-SCANNER-PROTOCOL.md
#[derive(thiserror::Error, Debug, Clone)]
pub enum BackendError {
    // ============================================================
    // Navigation Errors
    // ============================================================
    #[error("Navigation failed: {0}")]
    Navigation(String),

    // ============================================================
    // Element Errors (aligned with SPEC-SCANNER-PROTOCOL §2.3)
    // ============================================================
    #[error("Element {id} not found")]
    ElementNotFound { id: u32 },

    #[error("Element {id} is stale (removed from DOM)")]
    ElementStale { id: u32 },

    #[error("Element {id} is not visible")]
    ElementNotVisible { id: u32 },

    #[error("Element {id} is disabled")]
    ElementDisabled { id: u32 },

    #[error("Element {id} is not interactable: {reason}")]
    ElementNotInteractable { id: u32, reason: String },

    #[error("Invalid element type: expected {expected}, got {got}")]
    InvalidElementType { expected: String, got: String },

    #[error("Option not found: {value}")]
    OptionNotFound { value: String },

    #[error("Invalid selector: {selector}")]
    SelectorInvalid { selector: String },

    // ============================================================
    // Execution Errors
    // ============================================================
    #[error("Script execution error: {0}")]
    ScriptError(String),

    #[error("Timeout: {operation}")]
    TimeoutWithContext { operation: String },

    #[error("Timeout")]
    Timeout,

    // ============================================================
    // Protocol Errors
    // ============================================================
    #[error("Unknown command: {0}")]
    UnknownCommand(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    // ============================================================
    // System Errors
    // ============================================================
    #[error("Scanner execution failed: {0}")]
    Scanner(String),

    #[error("Connection lost")]
    ConnectionLost,

    #[error("Not ready")]
    NotReady,

    #[error("IO error: {0}")]
    Io(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Other: {0}")]
    Other(String),

    #[error("Not supported: {0}")]
    NotSupported(String),
}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        BackendError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for BackendError {
    fn from(err: serde_json::Error) -> Self {
        BackendError::Serialization(err.to_string())
    }
}

impl BackendError {
    /// Returns the SPEC-SCANNER-PROTOCOL error code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            BackendError::Navigation(_) => "NAVIGATION_ERROR",
            BackendError::ElementNotFound { .. } => "ELEMENT_NOT_FOUND",
            BackendError::ElementStale { .. } => "ELEMENT_STALE",
            BackendError::ElementNotVisible { .. } => "ELEMENT_NOT_VISIBLE",
            BackendError::ElementDisabled { .. } => "ELEMENT_DISABLED",
            BackendError::ElementNotInteractable { .. } => "ELEMENT_NOT_INTERACTABLE",
            BackendError::InvalidElementType { .. } => "INVALID_ELEMENT_TYPE",
            BackendError::OptionNotFound { .. } => "OPTION_NOT_FOUND",
            BackendError::SelectorInvalid { .. } => "SELECTOR_INVALID",
            BackendError::ScriptError(_) => "SCRIPT_ERROR",
            BackendError::TimeoutWithContext { .. } | BackendError::Timeout => "TIMEOUT",
            BackendError::UnknownCommand(_) => "UNKNOWN_COMMAND",
            BackendError::InvalidRequest(_) => "INVALID_REQUEST",
            BackendError::Scanner(_) => "SCANNER_ERROR",
            BackendError::ConnectionLost => "CONNECTION_LOST",
            BackendError::NotReady => "NOT_READY",
            BackendError::Io(_) => "IO_ERROR",
            BackendError::Serialization(_) => "SERIALIZATION_ERROR",
            BackendError::Other(_) => "INTERNAL_ERROR",
            BackendError::NotSupported(_) => "NOT_SUPPORTED",
        }
    }

    /// Returns a recovery hint for this error per SPEC-SCANNER-PROTOCOL §2.3.
    pub fn recovery_hint(&self) -> &'static str {
        match self {
            BackendError::ElementNotFound { .. } | BackendError::ElementStale { .. } => {
                "Run scan to refresh element map"
            }
            BackendError::ElementNotVisible { .. } => "Scroll element into view or wait",
            BackendError::ElementDisabled { .. } => "Wait for element to become enabled",
            BackendError::ElementNotInteractable { .. } => "Use force option or wait",
            BackendError::Timeout | BackendError::TimeoutWithContext { .. } => {
                "Increase timeout or verify condition"
            }
            BackendError::SelectorInvalid { .. } => "Fix selector syntax",
            BackendError::Navigation(_) => "Check URL and network connectivity",
            _ => "Check command parameters",
        }
    }
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

    /// Generate a PDF of the current page.
    async fn pdf(&mut self) -> Result<Vec<u8>, BackendError> {
        Err(BackendError::NotSupported("pdf".into()))
    }

    /// Get all cookies from the current session.
    async fn get_cookies(&mut self) -> Result<Vec<crate::protocol::Cookie>, BackendError> {
        Err(BackendError::NotSupported("get_cookies".into()))
    }

    /// Set a cookie in the current session.
    async fn set_cookie(&mut self, _cookie: crate::protocol::Cookie) -> Result<(), BackendError> {
        Err(BackendError::NotSupported("set_cookie".into()))
    }

    /// Get all open tabs/windows.
    async fn get_tabs(&mut self) -> Result<Vec<crate::protocol::TabInfo>, BackendError> {
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

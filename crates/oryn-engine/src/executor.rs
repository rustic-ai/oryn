//! Shared command execution pipeline for all Oryn backends.
//!
//! This module provides a `CommandExecutor` that handles the full command pipeline:
//! input → process (parse+resolve+translate) → execute → format
//!
//! All Oryn binaries (oryn-h, oryn-e, oryn-r) should use this shared executor.

use crate::backend::Backend;
use crate::formatter::format_response;
use oryn_common::protocol::{
    Action, BrowserAction, Cookie, ScanRequest, ScanResult, ScannerAction, ScannerData,
    ScannerProtocolResponse, SessionAction,
};
use oryn_common::resolver::{ResolverContext, ResolverError};
use oryn_parser::{ProcessError, process};

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("Process error: {0}")]
    Process(#[from] ProcessError),

    #[error("No scan context. Run 'observe' first to enable semantic targeting.")]
    NoScanContext,

    #[error("Backend error: {0}")]
    Backend(#[from] oryn_common::error::backend_error::BackendError),

    #[error("Navigation error: {0}")]
    Navigation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Result of executing a command.
pub struct ExecutionResult {
    /// Formatted output string for display.
    pub output: String,
    /// Whether execution was successful.
    pub success: bool,
}

pub struct CommandExecutor {
    last_scan: Option<ScanResult>,
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor {
    pub fn new() -> Self {
        Self { last_scan: None }
    }

    pub fn get_last_scan(&self) -> Option<&ScanResult> {
        self.last_scan.as_ref()
    }

    /// Execute a line of input.
    pub async fn execute_line<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        line: &str,
    ) -> Result<ExecutionResult, ExecutorError> {
        // 1. Process (Parse + Resolve + Translate)
        let ctx = self.get_resolver_context();
        let actions_result = process(line, &ctx);

        let actions = match actions_result {
            Ok(a) => a,
            Err(ProcessError::Resolution(ResolverError::NoMatch(_)))
            | Err(ProcessError::Resolution(ResolverError::StaleContext)) => {
                // Retry with fresh scan
                let req = ScannerAction::Scan(ScanRequest::default());
                let resp = backend.execute_scanner(req).await?;
                self.update_from_response(&resp);

                let ctx = self.get_resolver_context();
                process(line, &ctx).map_err(ExecutorError::Process)?
            }
            Err(e) => return Err(ExecutorError::Process(e)),
        };

        // 2. Execute Actions
        let mut outputs = Vec::new();
        for action in actions {
            let output = self.execute_action(backend, action).await?;
            outputs.push(output);
        }

        Ok(ExecutionResult {
            output: outputs.join("\n"),
            success: true,
        })
    }

    async fn execute_action<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        action: Action,
    ) -> Result<String, ExecutorError> {
        match action {
            // Scanner Actions -> execute_scanner
            Action::Scanner(sa) => {
                let resp = backend.execute_scanner(sa).await?;
                self.update_from_response(&resp);
                Ok(format_response(&resp))
            }

            // Browser Actions -> backend methods
            Action::Browser(ba) => self.execute_browser_action(backend, ba).await,

            // Session Actions -> backend methods
            Action::Session(sa) => self.execute_session_action(backend, sa).await,

            // Meta Actions -> Not supported yet
            Action::Meta(ma) => Err(ExecutorError::NotImplemented(format!(
                "Meta action: {:?}",
                ma
            ))),
        }
    }

    async fn execute_browser_action<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        action: BrowserAction,
    ) -> Result<String, ExecutorError> {
        match action {
            BrowserAction::Navigate(req) => {
                let res = backend
                    .navigate(&req.url)
                    .await
                    .map_err(|e| ExecutorError::Navigation(e.to_string()))?;
                Ok(format!("Navigated to {}", res.url))
            }
            BrowserAction::Back(_) => {
                let res = backend.go_back().await?;
                Ok(format!("Navigated back to {}", res.url))
            }
            BrowserAction::Forward(_) => {
                let res = backend.go_forward().await?;
                Ok(format!("Navigated forward to {}", res.url))
            }
            BrowserAction::Refresh(_) => {
                let res = backend.refresh().await?;
                Ok(format!("Refreshed: {}", res.url))
            }
            BrowserAction::Screenshot(req) => {
                let data = backend.screenshot().await?;
                let output_path = req.output.unwrap_or_else(|| "screenshot.png".to_string());
                std::fs::write(&output_path, &data)?;
                Ok(format!(
                    "Screenshot saved to {} ({} bytes)",
                    output_path,
                    data.len()
                ))
            }
            BrowserAction::Pdf(req) => {
                let data = backend.pdf().await?;
                std::fs::write(&req.path, &data)?;
                Ok(format!("PDF saved to {} ({} bytes)", req.path, data.len()))
            }
            BrowserAction::Press(req) => {
                backend.press_key(&req.key, &req.modifiers).await?;
                let mod_str = if req.modifiers.is_empty() {
                    "".to_string()
                } else {
                    format!("+{:?}", req.modifiers)
                };
                Ok(format!("Pressed {}{}", req.key, mod_str))
            }
            BrowserAction::Tab(req) => match req.action.as_str() {
                "list" => {
                    let tabs = backend.get_tabs().await?;
                    if tabs.is_empty() {
                        Ok("No tabs".into())
                    } else {
                        let titles: Vec<String> = tabs.iter().map(|t| t.title.clone()).collect();
                        Ok(format!("Tabs: {}", titles.join(", ")))
                    }
                }
                _ => Err(ExecutorError::NotImplemented(format!(
                    "Tab action: {}",
                    req.action
                ))),
            },
            // Frame, Dialog -> NotSupported
            _ => Err(ExecutorError::NotImplemented(format!(
                "Browser action: {:?}",
                action
            ))),
        }
    }

    async fn execute_session_action<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        action: SessionAction,
    ) -> Result<String, ExecutorError> {
        match action {
            SessionAction::Cookie(req) => {
                match req.action.as_str() {
                    "list" => {
                        let cookies = backend.get_cookies().await?;
                        if cookies.is_empty() {
                            Ok("No cookies".into())
                        } else {
                            let names: Vec<String> =
                                cookies.iter().map(|c| c.name.clone()).collect();
                            Ok(format!("Cookies: {}", names.join(", ")))
                        }
                    }
                    "get" => {
                        // For now we just call get_cookies and filter by name?
                        // Or does backend have get_cookie(name)?
                        // Backend trait has get_cookies() -> Vec<Cookie>.
                        // So we filter.
                        let name = req.name.unwrap_or_default();
                        let cookies = backend.get_cookies().await?;
                        if let Some(c) = cookies.into_iter().find(|c| c.name == name) {
                            Ok(format!("Cookie: {}={}", c.name, c.value))
                        } else {
                            Ok(format!("Cookie {} not found", name))
                        }
                    }
                    "set" => {
                        let c = Cookie {
                            name: req.name.unwrap_or_default(),
                            value: req.value.unwrap_or_default(),
                            domain: req.domain,
                            path: Some("/".into()),
                            expires: None,
                            http_only: None,
                            secure: None,
                        };
                        backend.set_cookie(c).await?;
                        Ok("Cookie set".into())
                    }
                    "delete" => {
                        // Delete usually means setting expiry to past
                        let name = req.name.unwrap_or_default();
                        let c = Cookie {
                            name: name.clone(),
                            value: "".into(),
                            domain: req.domain,
                            path: Some("/".into()),
                            expires: Some(0.0), // Expired
                            http_only: None,
                            secure: None,
                        };
                        backend.set_cookie(c).await?;
                        Ok(format!("Cookie {} deleted", name))
                    }
                    // Implement other cases as needed or return error
                    _ => Err(ExecutorError::NotImplemented(format!(
                        "Cookie action: {}",
                        req.action
                    ))),
                }
            }
            _ => Err(ExecutorError::NotImplemented(format!(
                "Session action: {:?}",
                action
            ))),
        }
    }

    fn update_from_response(&mut self, resp: &ScannerProtocolResponse) {
        if let ScannerProtocolResponse::Ok { data, .. } = resp
            && let ScannerData::Scan(result) = data.as_ref()
        {
            self.last_scan = Some(*result.clone());
        }
    }

    fn get_resolver_context(&self) -> ResolverContext {
        self.last_scan
            .as_ref()
            .map(ResolverContext::new)
            .unwrap_or_else(ResolverContext::empty)
    }
}

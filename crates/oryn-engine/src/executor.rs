//! Shared command execution pipeline for all Oryn backends.
//!
//! This module provides a `CommandExecutor` that handles the full command pipeline:
//! input → process (parse+resolve+translate) → execute → format
//!
//! All Oryn binaries (oryn-h, oryn-e, oryn-r) should use this shared executor.

use crate::backend::Backend;
use crate::formatter::format_response;
use crate::resolution::ResolutionEngine;
use oryn_common::protocol::{
    Action, BrowserAction, Cookie, ScanRequest, ScanResult, ScannerAction, ScannerData,
    ScannerProtocolResponse, SessionAction,
};
use oryn_parser::{
    normalize, parse,
    parser::ParseError,
    translator::{self, TranslationError},
};

#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Resolution error: {0}")]
    Resolution(#[from] crate::resolution::result::ResolutionError),

    #[error("Translation error: {0}")]
    Translation(#[from] TranslationError),

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
        // 1. Parse
        let normalized = normalize(line);
        let script = parse(&normalized)?;

        // 2. Resolve + Translate + Execute each command
        let mut outputs = Vec::new();
        for script_line in script.lines {
            if let Some(cmd) = script_line.command {
                let cmd_clone = cmd.clone();

                // Try to resolve the command
                let resolved_cmd = match self.resolve_command(cmd, backend).await {
                    Ok(c) => c,
                    Err(ExecutorError::Resolution(_)) | Err(ExecutorError::NoScanContext) => {
                        // If resolution fails, try with fresh scan
                        let req = ScannerAction::Scan(ScanRequest::default());
                        let resp = backend.execute_scanner(req).await?;
                        self.update_from_response(&resp);

                        // Retry resolution
                        self.resolve_command(cmd_clone, backend).await?
                    }
                    Err(e) => return Err(e),
                };

                // Translate the resolved command to an action
                let action = translator::translate(&resolved_cmd)?;

                // Execute the action
                let output = self.execute_action(backend, action).await?;
                outputs.push(output);
            }
        }

        Ok(ExecutionResult {
            output: outputs.join("\n"),
            success: true,
        })
    }

    /// Resolve a command using the sophisticated resolution engine.
    async fn resolve_command<B: Backend + ?Sized>(
        &self,
        cmd: oryn_parser::ast::Command,
        backend: &mut B,
    ) -> Result<oryn_parser::ast::Command, ExecutorError> {
        if let Some(scan) = &self.last_scan {
            ResolutionEngine::resolve(cmd, scan, backend)
                .await
                .map_err(ExecutorError::Resolution)
        } else {
            // No scan context, return command as-is (for commands that don't need resolution)
            Ok(cmd)
        }
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
                if req.modifiers.is_empty() {
                    Ok(format!("Pressed {}", req.key))
                } else {
                    Ok(format!("Pressed {}+{:?}", req.key, req.modifiers))
                }
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
            SessionAction::Cookie(req) => match req.action.as_str() {
                "list" => {
                    let cookies = backend.get_cookies().await?;
                    if cookies.is_empty() {
                        Ok("No cookies".into())
                    } else {
                        let names: Vec<String> = cookies.iter().map(|c| c.name.clone()).collect();
                        Ok(format!("Cookies: {}", names.join(", ")))
                    }
                }
                "get" => {
                    let name = req.name.unwrap_or_default();
                    let cookies = backend.get_cookies().await?;
                    match cookies.into_iter().find(|c| c.name == name) {
                        Some(c) => Ok(format!("Cookie: {}={}", c.name, c.value)),
                        None => Ok(format!("Cookie {} not found", name)),
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
                    let name = req.name.unwrap_or_default();
                    let c = Cookie {
                        name: name.clone(),
                        value: String::new(),
                        domain: req.domain,
                        path: Some("/".into()),
                        expires: Some(0.0),
                        http_only: None,
                        secure: None,
                    };
                    backend.set_cookie(c).await?;
                    Ok(format!("Cookie {} deleted", name))
                }
                _ => Err(ExecutorError::NotImplemented(format!(
                    "Cookie action: {}",
                    req.action
                ))),
            },
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
}

//! Shared command execution pipeline for all Oryn backends.
//!
//! This module provides a `CommandExecutor` that handles the full command pipeline:
//! parse → resolve → translate → execute → format
//!
//! All Oryn binaries (oryn-h, oryn-e, oryn-r) should use this shared executor
//! rather than implementing their own command handling.

use crate::backend::Backend;
use crate::command::{Command, CookieAction, TabAction, Target};
use crate::formatter::format_response;
use crate::parser::Parser;
use crate::protocol::{Cookie, ScannerData, ScannerProtocolResponse, ScannerRequest};
use crate::resolver::{ResolutionStrategy, ResolverContext, resolve_target};
use crate::translator::{TranslationError, translate};

/// Truncate a string value for display, adding ellipsis if needed.
fn truncate_value(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Error type for command execution.
#[derive(Debug, thiserror::Error)]
pub enum ExecutorError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Resolution error: {0} (hint: run 'observe' first)")]
    Resolution(String),

    #[error("No scan context. Run 'observe' first to enable semantic targeting.")]
    NoScanContext,

    #[error("Translation error: {0}")]
    Translation(#[from] TranslationError),

    #[error("Backend error: {0}")]
    Backend(#[from] crate::backend::BackendError),

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

/// Shared command executor that maintains state across commands.
///
/// This executor handles the full pipeline:
/// 1. Parse the input line into commands
/// 2. Resolve semantic targets to element IDs
/// 3. Translate commands to scanner requests
/// 4. Execute via the backend
/// 5. Update resolver context from scan responses
/// 6. Format output for display
pub struct CommandExecutor {
    resolver_context: Option<ResolverContext>,
}

impl Default for CommandExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandExecutor {
    /// Create a new command executor.
    pub fn new() -> Self {
        Self {
            resolver_context: None,
        }
    }

    /// Execute a line of input, which may contain multiple commands.
    ///
    /// Returns the formatted output string and whether execution succeeded.
    pub async fn execute_line<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        line: &str,
    ) -> Result<ExecutionResult, ExecutorError> {
        let mut parser = Parser::new(line);
        let commands = parser
            .parse()
            .map_err(|e| ExecutorError::Parse(e.to_string()))?;

        let mut outputs = Vec::new();
        for cmd in commands {
            let output = self.execute_command(backend, cmd).await?;
            outputs.push(output);
        }

        Ok(ExecutionResult {
            output: outputs.join("\n"),
            success: true,
        })
    }

    /// Execute a single command.
    pub async fn execute_command<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        cmd: Command,
    ) -> Result<String, ExecutorError> {
        match &cmd {
            // ============================================================
            // Navigation Commands - Route to Backend trait methods
            // ============================================================
            Command::GoTo(url) => {
                let res = backend
                    .navigate(url)
                    .await
                    .map_err(|e| ExecutorError::Navigation(e.to_string()))?;
                Ok(format!("Navigated to {}", res.url))
            }

            Command::Back => {
                let res = backend.go_back().await?;
                Ok(format!("Navigated back to {}", res.url))
            }

            Command::Forward => {
                let res = backend.go_forward().await?;
                Ok(format!("Navigated forward to {}", res.url))
            }

            Command::Refresh(_opts) => {
                let res = backend.refresh().await?;
                Ok(format!("Refreshed: {}", res.url))
            }

            // ============================================================
            // Media Capture Commands - Route to Backend trait methods
            // ============================================================
            Command::Screenshot(opts) => {
                let data = backend.screenshot().await?;
                // Handle output path from options
                let output_path = opts
                    .get("output")
                    .or_else(|| opts.get("file"))
                    .cloned()
                    .unwrap_or_else(|| "screenshot.png".to_string());

                std::fs::write(&output_path, &data)?;
                Ok(format!(
                    "Screenshot saved to {} ({} bytes)",
                    output_path,
                    data.len()
                ))
            }

            Command::Pdf(path) => {
                let data = backend.pdf().await?;
                let output_path = if path.is_empty() { "page.pdf" } else { path };
                std::fs::write(output_path, &data)?;
                Ok(format!(
                    "PDF saved to {} ({} bytes)",
                    output_path,
                    data.len()
                ))
            }

            // ============================================================
            // Keyboard Commands - Route to Backend trait methods
            // ============================================================
            Command::Press(key, opts) => {
                let modifiers: Vec<String> = opts.keys().cloned().collect();
                backend.press_key(key, &modifiers).await?;
                if modifiers.is_empty() {
                    Ok(format!("Pressed {}", key))
                } else {
                    Ok(format!("Pressed {}+{}", modifiers.join("+"), key))
                }
            }

            // ============================================================
            // Session Commands - Route to Backend cookie methods
            // ============================================================
            Command::Cookies(action) => match action {
                CookieAction::List => {
                    let cookies = backend.get_cookies().await?;
                    if cookies.is_empty() {
                        Ok("No cookies".to_string())
                    } else {
                        let mut output = format!("Cookies ({}):\n", cookies.len());
                        for cookie in cookies {
                            output.push_str(&format!(
                                "  {} = {}{}\n",
                                cookie.name,
                                truncate_value(&cookie.value, 50),
                                cookie
                                    .domain
                                    .as_ref()
                                    .map(|d| format!(" ({})", d))
                                    .unwrap_or_default()
                            ));
                        }
                        Ok(output.trim_end().to_string())
                    }
                }
                CookieAction::Get(name) => {
                    let cookies = backend.get_cookies().await?;
                    match cookies.iter().find(|c| c.name == *name) {
                        Some(cookie) => Ok(format!("{} = {}", cookie.name, cookie.value)),
                        None => Ok(format!("Cookie '{}' not found", name)),
                    }
                }
                CookieAction::Set(name, value) => {
                    let cookie = Cookie {
                        name: name.clone(),
                        value: value.clone(),
                        domain: None,
                        path: Some("/".to_string()),
                        expires: None,
                        http_only: None,
                        secure: None,
                    };
                    backend.set_cookie(cookie).await?;
                    Ok(format!("Cookie '{}' set", name))
                }
                CookieAction::Delete(name) => {
                    // Delete by setting an expired cookie
                    let cookie = Cookie {
                        name: name.clone(),
                        value: String::new(),
                        domain: None,
                        path: Some("/".to_string()),
                        expires: Some(0.0), // Expired
                        http_only: None,
                        secure: None,
                    };
                    backend.set_cookie(cookie).await?;
                    Ok(format!("Cookie '{}' deleted", name))
                }
            },

            // ============================================================
            // Tab Commands - Route to Backend tab methods
            // ============================================================
            Command::Tabs(action) => match action {
                TabAction::List => {
                    let tabs = backend.get_tabs().await?;
                    if tabs.is_empty() {
                        Ok("No tabs".to_string())
                    } else {
                        let mut output = format!("Tabs ({}):\n", tabs.len());
                        for (i, tab) in tabs.iter().enumerate() {
                            let active_marker = if tab.active { " *" } else { "" };
                            output.push_str(&format!(
                                "  [{}]{} {} - {}\n",
                                i,
                                active_marker,
                                truncate_value(&tab.title, 40),
                                truncate_value(&tab.url, 60)
                            ));
                        }
                        Ok(output.trim_end().to_string())
                    }
                }
                TabAction::New(url) => {
                    // Tab creation requires backend support - delegate to not implemented for now
                    Err(ExecutorError::NotImplemented(format!(
                        "tab new {} - requires backend implementation",
                        url
                    )))
                }
                TabAction::Switch(id) => {
                    // Tab switching requires backend support
                    Err(ExecutorError::NotImplemented(format!(
                        "tab switch {} - requires backend implementation",
                        id
                    )))
                }
                TabAction::Close(id) => {
                    // Tab closing requires backend support
                    Err(ExecutorError::NotImplemented(format!(
                        "tab close {} - requires backend implementation",
                        id
                    )))
                }
            },

            // ============================================================
            // Intent Management Commands
            // ============================================================
            Command::Intents(filter) => {
                use crate::command::IntentFilter;
                // For now, return a placeholder - intent system integration needed
                let scope = match filter {
                    IntentFilter::All => "all",
                    IntentFilter::Session => "session",
                };
                Ok(format!(
                    "Intent listing ({}) - requires intent registry integration",
                    scope
                ))
            }

            Command::Define(_body) => {
                // Intent definition requires intent system integration
                Err(ExecutorError::NotImplemented(
                    "define - requires intent system integration".to_string(),
                ))
            }

            Command::Undefine(name) => {
                // Intent removal requires intent system integration
                Err(ExecutorError::NotImplemented(format!(
                    "undefine {} - requires intent system integration",
                    name
                )))
            }

            Command::Export(name, path) => {
                // Intent export requires intent system integration
                Err(ExecutorError::NotImplemented(format!(
                    "export {} to {} - requires intent system integration",
                    name, path
                )))
            }

            Command::RunIntent(name, _params) => {
                // Intent execution requires intent system integration
                Err(ExecutorError::NotImplemented(format!(
                    "run {} - requires intent system integration",
                    name
                )))
            }

            // ============================================================
            // Pack Management Commands
            // ============================================================
            Command::Packs => Err(ExecutorError::NotImplemented(
                "packs - requires pack system integration".to_string(),
            )),

            Command::PackLoad(name) => Err(ExecutorError::NotImplemented(format!(
                "pack load {} - requires pack system integration",
                name
            ))),

            Command::PackUnload(name) => Err(ExecutorError::NotImplemented(format!(
                "pack unload {} - requires pack system integration",
                name
            ))),

            // ============================================================
            // Learning Commands
            // ============================================================
            Command::Learn(action) => {
                use crate::command::LearnAction;
                let action_desc = match action {
                    LearnAction::Status => "status",
                    LearnAction::Refine(s) => s,
                    LearnAction::Save(s) => s,
                    LearnAction::Ignore(s) => s,
                };
                Err(ExecutorError::NotImplemented(format!(
                    "learn {} - requires learning system integration",
                    action_desc
                )))
            }

            // ============================================================
            // Default: Commands that go through translator → scanner
            // ============================================================
            _ => {
                // Resolve semantic targets if needed
                let resolved_cmd = if Self::command_needs_resolution(&cmd) {
                    match &self.resolver_context {
                        Some(ctx) => Self::resolve_command(&cmd, ctx)?,
                        None => {
                            return Err(ExecutorError::NoScanContext);
                        }
                    }
                } else {
                    cmd
                };

                // Resolve CSS selectors to IDs for commands that require numeric IDs
                let final_cmd = Self::resolve_selectors_to_ids(backend, resolved_cmd).await?;

                // Translate to scanner request
                let req = translate(&final_cmd)?;

                // Execute via backend
                let resp = backend.execute_scanner(req).await?;

                // Update resolver context if this was a scan
                self.update_from_response(&resp);

                // Format output
                Ok(format_response(&resp))
            }
        }
    }

    /// Resolve CSS selectors to element IDs by executing JavaScript in the browser.
    async fn resolve_selectors_to_ids<B: Backend + ?Sized>(
        backend: &mut B,
        cmd: Command,
    ) -> Result<Command, ExecutorError> {
        match cmd {
            Command::Click(Target::Selector(selector), opts) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Click(Target::Id(id), opts))
            }
            Command::Type(Target::Selector(selector), text, opts) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Type(Target::Id(id), text, opts))
            }
            Command::Clear(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Clear(Target::Id(id)))
            }
            Command::Check(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Check(Target::Id(id)))
            }
            Command::Uncheck(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Uncheck(Target::Id(id)))
            }
            Command::Focus(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Focus(Target::Id(id)))
            }
            Command::Hover(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Hover(Target::Id(id)))
            }
            Command::Select(Target::Selector(selector), value) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Select(Target::Id(id), value))
            }
            Command::Submit(Target::Selector(selector)) => {
                let id = Self::resolve_selector(backend, &selector).await?;
                Ok(Command::Submit(Target::Id(id)))
            }
            // Commands that don't need selector-to-ID resolution pass through
            _ => Ok(cmd),
        }
    }

    /// Resolve a CSS selector to an element ID by executing JavaScript.
    ///
    /// This function tries multiple selector strategies in order:
    /// 1. The original selector as-is (for CSS selectors like `input[type="text"]`)
    /// 2. As an ID selector (`#selector`)
    /// 3. As a name attribute (`[name="selector"]`)
    /// 4. As a placeholder contains (`[placeholder*="selector"]`)
    /// 5. As an aria-label contains (`[aria-label*="selector"]`)
    async fn resolve_selector<B: Backend + ?Sized>(
        backend: &mut B,
        selector: &str,
    ) -> Result<usize, ExecutorError> {
        // Escape the selector for use in JavaScript
        let escaped_selector = selector.replace('\\', "\\\\").replace('\'', "\\'");

        // Note: The scanner uses new Function() to execute scripts, which creates
        // a function wrapper. We need 'return' at the top level.
        //
        // We try multiple selector strategies to handle text targets like "coupon-code"
        // that might be an ID, name attribute, placeholder text, or aria-label.
        let script = format!(
            r#"
            var selectors = [
                '{}',                       // Original selector as-is
                '#{}',                      // As ID selector
                '[name="{}"]',              // As name attribute
                '[placeholder*="{}"]',      // Placeholder contains
                '[aria-label*="{}"]'        // aria-label contains
            ];

            var el = null;
            for (var i = 0; i < selectors.length; i++) {{
                try {{
                    el = document.querySelector(selectors[i]);
                    if (el) break;
                }} catch (e) {{
                    // Invalid selector syntax, try next
                }}
            }}

            if (!el) return {{ found: false }};

            // Check if element already has an ID in our state
            var id = Oryn.State.inverseMap.get(el);
            if (id !== undefined) return {{ found: true, id: id }};

            // Assign new ID
            id = Oryn.State.nextId++;
            Oryn.State.inverseMap.set(el, id);
            Oryn.State.elementMap.set(id, el);
            return {{ found: true, id: id }};
            "#,
            escaped_selector,
            escaped_selector,
            escaped_selector,
            escaped_selector,
            escaped_selector
        );

        let req = ScannerRequest::Execute(crate::protocol::ExecuteRequest {
            script,
            args: vec![],
        });

        let resp = backend.execute_scanner(req).await?;

        // Parse the response to get the ID
        match resp {
            ScannerProtocolResponse::Ok { data, .. } => {
                // Execute results come back as Value variant with { result: <actual_value> }
                if let ScannerData::Value(result) = data.as_ref()
                    && let Some(inner) = result.get("result")
                    && let Some(obj) = inner.as_object()
                    && obj.get("found").and_then(|v| v.as_bool()) == Some(true)
                    && let Some(id) = obj.get("id").and_then(|v| v.as_u64())
                {
                    return Ok(id as usize);
                }
                Err(ExecutorError::Resolution(format!(
                    "Element not found for selector: {}",
                    selector
                )))
            }
            ScannerProtocolResponse::Error { message, .. } => Err(ExecutorError::Resolution(
                format!("Failed to resolve selector '{}': {}", selector, message),
            )),
        }
    }

    /// Update resolver context from a scan response.
    fn update_from_response(&mut self, resp: &ScannerProtocolResponse) {
        if let ScannerProtocolResponse::Ok { data, .. } = resp
            && let ScannerData::Scan(result) = data.as_ref()
        {
            self.resolver_context = Some(ResolverContext::new(result));
        }
    }

    /// Get the current resolver context, if any.
    pub fn get_context(&self) -> Option<&ResolverContext> {
        self.resolver_context.as_ref()
    }

    /// Check if a target needs resolution (is not already an ID).
    fn needs_resolution(target: &Target) -> bool {
        !matches!(target, Target::Id(_))
    }

    /// Check if command has a target that needs resolution.
    fn command_needs_resolution(cmd: &Command) -> bool {
        match cmd {
            Command::Click(t, _)
            | Command::Type(t, _, _)
            | Command::Clear(t)
            | Command::Check(t)
            | Command::Uncheck(t)
            | Command::Hover(t)
            | Command::Focus(t)
            | Command::Select(t, _)
            | Command::Submit(t) => Self::needs_resolution(t),
            Command::Scroll(Some(t), _) => Self::needs_resolution(t),
            _ => false,
        }
    }

    /// Resolve targets in a command using appropriate strategy based on command type.
    fn resolve_command(cmd: &Command, ctx: &ResolverContext) -> Result<Command, ExecutorError> {
        match cmd {
            // Click command: prefer clickable elements (buttons, links)
            Command::Click(target, opts) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferClickable)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Click(resolved, opts.clone()))
            }
            // Type command: prefer input elements (input, textarea, select)
            Command::Type(target, text, opts) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferInput)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Type(resolved, text.clone(), opts.clone()))
            }
            // Clear command: prefer input elements
            Command::Clear(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferInput)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Clear(resolved))
            }
            // Check command: prefer checkable elements (checkbox, radio)
            Command::Check(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferCheckable)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Check(resolved))
            }
            // Uncheck command: prefer checkable elements
            Command::Uncheck(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferCheckable)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Uncheck(resolved))
            }
            Command::Hover(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Hover(resolved))
            }
            // Focus command: prefer input elements
            Command::Focus(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferInput)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Focus(resolved))
            }
            // Select command: prefer select elements
            Command::Select(target, value) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferInput)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Select(resolved, value.clone()))
            }
            // Submit command: prefer clickable (button) elements
            Command::Submit(target) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::PreferClickable)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Submit(resolved))
            }
            Command::Scroll(Some(target), opts) => {
                let resolved = resolve_target(target, ctx, ResolutionStrategy::First)
                    .map_err(|e| ExecutorError::Resolution(e.to_string()))?;
                Ok(Command::Scroll(Some(resolved), opts.clone()))
            }
            // Commands without targets pass through
            _ => Ok(cmd.clone()),
        }
    }
}

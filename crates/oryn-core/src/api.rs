use crate::ast::{Command, Target, TargetAtomic};
use crate::resolution::{ResolutionEngine, ResolutionError, WasmSelectorResolver};
use oryn_common::protocol::{Action, ScanResult};
use oryn_common::resolver::{self, ResolutionStrategy, ResolverContext};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessedCommand {
    /// Command resolved to an action ready to execute
    Resolved(Action),
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("Parse error: {0}")]
    Parse(#[from] crate::parser::ParseError),

    #[error("Translation error: {0}")]
    Translation(#[from] crate::translator::TranslationError),

    #[error("Resolution error (legacy): {0}")]
    Resolution(#[from] oryn_common::resolver::ResolverError),

    #[error("Resolution error (advanced): {0}")]
    AdvancedResolution(#[from] ResolutionError),

    #[error("Empty script")]
    EmptyScript,
}

/// Process an OIL command (legacy version - basic resolution only)
///
/// This version uses basic text/role resolution from oryn_common::resolver.
/// It does NOT include:
/// - Label association
/// - Target inference rules
/// - Requirement validation
///
/// For advanced features, use `process_command_advanced()` instead.
///
/// This version does:
/// 1. Normalize input
/// 2. Parse to AST
/// 3. Resolve targets using basic scan data matching
/// 4. Translate to Action
pub fn process_command(
    oil_input: &str,
    scan: &ScanResult,
) -> Result<ProcessedCommand, ProcessError> {
    let normalized = crate::normalizer::normalize(oil_input);
    let script = crate::parser::parse(&normalized)?;

    let Some(script_line) = script.lines.first() else {
        return Err(ProcessError::EmptyScript);
    };
    let Some(cmd) = &script_line.command else {
        return Err(ProcessError::EmptyScript);
    };

    #[cfg(target_arch = "wasm32")]
    {
        web_sys::console::log_1(&format!("[WASM] Resolving command: {:?}", cmd).into());
        web_sys::console::log_1(
            &format!("[WASM] Scan has {} elements", scan.elements.len()).into(),
        );
        if let Command::Click(ref c) = cmd {
            web_sys::console::log_1(&format!("[WASM] Click target: {:?}", c.target).into());
        }
        for (i, elem) in scan.elements.iter().take(5).enumerate() {
            web_sys::console::log_1(
                &format!(
                    "[WASM] Element {}: id={}, type={}, text={:?}, label={:?}",
                    i, elem.id, elem.element_type, elem.text, elem.label
                )
                .into(),
            );
        }
    }

    let resolved_cmd = resolve_command_targets(cmd, scan)?;

    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&format!("[WASM] Resolved to: {:?}", resolved_cmd).into());

    let action = crate::translator::translate(&resolved_cmd)?;
    Ok(ProcessedCommand::Resolved(action))
}

/// Resolve all targets in a command using the scan data
fn resolve_command_targets(cmd: &Command, scan: &ScanResult) -> Result<Command, ProcessError> {
    let ctx = ResolverContext::new(scan);
    let strategy = ResolutionStrategy::First;

    // Helper to resolve a single target
    let resolve_target = |target: &Target| -> Result<Target, ProcessError> {
        // Convert AST target to resolver target
        let resolver_target = target.to_resolver_target();

        // Resolve using oryn_common::resolver
        let resolved = resolver::resolve_target(&resolver_target, &ctx, strategy)?;

        // Convert back to AST target
        match resolved {
            resolver::Target::Id(id) => Ok(Target {
                atomic: TargetAtomic::Id(id),
                relation: None,
            }),
            // If not resolved to ID, return original target (for selectors, etc.)
            _ => Ok(target.clone()),
        }
    };

    // Resolve targets based on command type
    match cmd {
        Command::Click(c) => Ok(Command::Click(crate::ast::ClickCmd {
            target: resolve_target(&c.target)?,
            ..c.clone()
        })),
        Command::Type(c) => Ok(Command::Type(crate::ast::TypeCmd {
            target: resolve_target(&c.target)?,
            ..c.clone()
        })),
        Command::Check(c) => Ok(Command::Check(crate::ast::CheckCmd {
            target: resolve_target(&c.target)?,
        })),
        Command::Select(c) => Ok(Command::Select(crate::ast::SelectCmd {
            target: resolve_target(&c.target)?,
            value: c.value.clone(),
        })),
        Command::Hover(c) => Ok(Command::Hover(crate::ast::HoverCmd {
            target: resolve_target(&c.target)?,
        })),
        Command::Focus(c) => Ok(Command::Focus(crate::ast::FocusCmd {
            target: resolve_target(&c.target)?,
        })),
        Command::Clear(c) => Ok(Command::Clear(crate::ast::ClearCmd {
            target: resolve_target(&c.target)?,
        })),
        Command::Text(c) => Ok(Command::Text(crate::ast::TextCmd {
            target: c.target.as_ref().map(resolve_target).transpose()?,
            selector: c.selector.clone(),
        })),
        Command::Uncheck(c) => Ok(Command::Uncheck(crate::ast::UncheckCmd {
            target: resolve_target(&c.target)?,
        })),
        // These commands don't have resolvable targets
        Command::Html(_) | Command::Wait(_) => Ok(cmd.clone()),
        _ => Ok(cmd.clone()),
    }
}

/// Process an OIL command with advanced resolution (async version).
///
/// Uses the full `ResolutionEngine` with label association, target inference,
/// requirement validation, and dismiss/accept inference.
///
/// Note: Requires `WasmSelectorResolver` to be fully implemented for CSS selector
/// support. Currently, selectors fail gracefully (return `None`).
pub async fn process_command_advanced(
    oil_input: &str,
    scan: &ScanResult,
) -> Result<ProcessedCommand, ProcessError> {
    let normalized = crate::normalizer::normalize(oil_input);
    let script = crate::parser::parse(&normalized)?;

    let Some(script_line) = script.lines.first() else {
        return Err(ProcessError::EmptyScript);
    };
    let Some(cmd) = &script_line.command else {
        return Err(ProcessError::EmptyScript);
    };

    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&format!("[WASM Advanced] Resolving command: {:?}", cmd).into());

    let mut resolver = WasmSelectorResolver::new();
    let resolved_cmd = ResolutionEngine::resolve(cmd.clone(), scan, &mut resolver).await?;

    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&format!("[WASM Advanced] Resolved to: {:?}", resolved_cmd).into());

    let action = crate::translator::translate(&resolved_cmd)?;
    Ok(ProcessedCommand::Resolved(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oryn_common::protocol::{PageInfo, ScanStats, ScrollInfo, ViewportInfo};

    fn create_test_scan() -> ScanResult {
        ScanResult {
            page: PageInfo {
                url: "https://test.com".to_string(),
                title: "Test".to_string(),
                viewport: ViewportInfo::default(),
                scroll: ScrollInfo::default(),
                ready_state: None,
            },
            elements: vec![],
            stats: ScanStats {
                total: 0,
                scanned: 0,
                iframes: None,
            },
            patterns: None,
            changes: None,
            available_intents: None,
            full_mode: false,
            settings_applied: None,
            timing: None,
        }
    }

    #[test]
    fn test_process_observe() {
        let scan = create_test_scan();
        let result = process_command("observe", &scan);
        match result {
            Ok(ProcessedCommand::Resolved(_action)) => {
                // Success
            }
            Err(e) => {
                panic!("Expected success, got error: {}", e);
            }
        }
    }

    #[test]
    fn test_process_goto() {
        let scan = create_test_scan();
        let result = process_command("goto https://example.com", &scan);
        assert!(result.is_ok());
        if let Ok(ProcessedCommand::Resolved(_action)) = result {
            // Success
        } else {
            panic!("Expected resolved command");
        }
    }
}

use crate::ast::{Command, Target, TargetAtomic};
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

    #[error("Resolution error: {0}")]
    Resolution(#[from] oryn_common::resolver::ResolverError),

    #[error("Empty script")]
    EmptyScript,
}

/// Process an OIL command (WASM version)
///
/// This version does:
/// 1. Normalize input
/// 2. Parse to AST
/// 3. Resolve targets using scan data
/// 4. Translate to Action
pub fn process_command(
    oil_input: &str,
    scan: &ScanResult,
) -> Result<ProcessedCommand, ProcessError> {
    // 1. Normalize
    let normalized = crate::normalizer::normalize(oil_input);

    // 2. Parse
    let script = crate::parser::parse(&normalized)?;

    // 3. Extract first command
    let Some(script_line) = script.lines.first() else {
        return Err(ProcessError::EmptyScript);
    };

    let Some(cmd) = &script_line.command else {
        return Err(ProcessError::EmptyScript);
    };

    // 4. Resolve targets in the command
    let resolved_cmd = resolve_command_targets(cmd, scan)?;

    // 5. Translate to protocol action
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
        Command::Check(_c) => Ok(Command::Check(crate::ast::CheckCmd {
            target: resolve_target(&_c.target)?,
        })),
        Command::Select(c) => Ok(Command::Select(crate::ast::SelectCmd {
            target: resolve_target(&c.target)?,
            value: c.value.clone(),
        })),
        Command::Hover(_c) => Ok(Command::Hover(crate::ast::HoverCmd {
            target: resolve_target(&_c.target)?,
        })),
        Command::Focus(_c) => Ok(Command::Focus(crate::ast::FocusCmd {
            target: resolve_target(&_c.target)?,
        })),
        Command::Clear(_c) => Ok(Command::Clear(crate::ast::ClearCmd {
            target: resolve_target(&_c.target)?,
        })),
        Command::Text(c) => Ok(Command::Text(crate::ast::TextCmd {
            target: c.target.as_ref().map(resolve_target).transpose()?,
            selector: c.selector.clone(),
        })),
        Command::Html(_c) => Ok(cmd.clone()), // HtmlCmd doesn't have target, just selector
        Command::Uncheck(c) => Ok(Command::Uncheck(crate::ast::UncheckCmd {
            target: resolve_target(&c.target)?,
        })),
        // Wait has target embedded in condition - more complex resolution needed
        Command::Wait(_c) => Ok(cmd.clone()),
        // Commands without targets - return as-is
        _ => Ok(cmd.clone()),
    }
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
            },
            elements: vec![],
            stats: ScanStats {
                total: 0,
                scanned: 0,
            },
            patterns: None,
            changes: None,
            available_intents: None,
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

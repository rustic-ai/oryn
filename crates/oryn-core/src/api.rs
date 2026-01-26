use oryn_common::protocol::{Action, ScanResult};
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

    #[error("Empty script")]
    EmptyScript,
}

/// Process an OIL command (minimal version for WASM)
///
/// This version does:
/// 1. Normalize input
/// 2. Parse to AST
/// 3. Translate to Action
///
/// Note: Resolution happens on the JS side via the scanner's capabilities
pub fn process_command(
    oil_input: &str,
    _scan: &ScanResult,
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

    // 4. Translate to protocol action
    let action = crate::translator::translate(cmd)?;

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

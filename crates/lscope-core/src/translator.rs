use crate::command::{Command, Target};
use crate::protocol::{
    ClickRequest, MouseButton, ScanRequest, ScannerRequest, ScrollDirection, ScrollRequest,
    TypeRequest, WaitRequest,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TranslationError {
    #[error("Unsupported command: {0}")]
    Unsupported(String),
    #[error("Invalid target for command: {0}")]
    InvalidTarget(String),
    #[error("Missing required parameter: {0}")]
    MissingParameter(String),
}

/// Translates a high-level Intent Command into a low-level ScannerRequest.
pub fn translate(command: &Command) -> Result<ScannerRequest, TranslationError> {
    match command {
        Command::Observe(_) => Ok(ScannerRequest::Scan(ScanRequest {
            max_elements: None, // TODO: support options
            monitor_changes: false,
            include_hidden: false,
            view_all: false,
        })),

        Command::Click(target, _options) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Click(ClickRequest {
                    id: *id as u32,
                    button: MouseButton::Left,
                    double: false,
                    modifiers: vec![],
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Click requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Type(target, text, _options) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Type(TypeRequest {
                    id: *id as u32,
                    text: text.clone(),
                    clear: false,
                    submit: false,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Type requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Submit(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Submit(crate::protocol::SubmitRequest {
                    id: *id as u32,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Submit requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Scroll(target, _options) => {
            // If target is ID, scroll that element. If None, scroll window.
            let id = if let Some(Target::Id(id)) = target {
                Some(*id as u32)
            } else {
                None
            };

            // TODO: Extract direction from options if present
            Ok(ScannerRequest::Scroll(ScrollRequest {
                id,
                direction: ScrollDirection::Down, // Default
                amount: Some("page".to_string()),
            }))
        }

        Command::Wait(condition, _options) => {
            // Basic support for wait
            let _ = condition; // usage to avoid unused variable warning if we drop match
            Ok(ScannerRequest::Wait(WaitRequest {
                condition: "visible".to_string(), // placeholder, need mapping
                target: None,
                timeout_ms: Some(5000),
            }))
        }

        // Navigation commands are handled by Backend::navigate usually, but if we need a scanner equivalent:
        // Command::GoTo(_) => ...
        _ => Err(TranslationError::Unsupported(format!("{:?}", command))),
    }
}

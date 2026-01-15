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

        Command::Scroll(target, options) => {
            // Map ID if present
            let id = if let Some(Target::Id(id)) = target {
                Some(*id as u32)
            } else {
                None
            };

            // Map direction
            let direction = if let Some(dir_str) = options.get("direction") {
                match dir_str.as_str() {
                    "up" => ScrollDirection::Up,
                    "down" => ScrollDirection::Down,
                    "left" => ScrollDirection::Left,
                    "right" => ScrollDirection::Right,
                    _ => ScrollDirection::Down,
                }
            } else {
                ScrollDirection::Down
            };

            Ok(ScannerRequest::Scroll(ScrollRequest {
                id,
                direction,
                amount: Some("page".to_string()), // Default amount
            }))
        }

        Command::Wait(condition, _options) => {
            // Map WaitCondition enum to protocol strings
            // Protocol expects: "exists", "visible", "hidden", "gone", "navigation"
            // WaitCondition: Load, Idle, Visible(T), Hidden(T), Exists(selector/id), Gone(selector/id), Url(s)

            let (cond_str, target, timeout) = match condition {
                crate::command::WaitCondition::Visible(t) => match t {
                    Target::Id(id) => ("visible", Some(id.to_string()), None::<u64>),
                    Target::Selector(s) => ("visible", Some(s.clone()), None::<u64>),
                    _ => {
                        return Err(TranslationError::InvalidTarget(
                            "Wait visible requires ID or Selector".into(),
                        ));
                    }
                },
                crate::command::WaitCondition::Hidden(t) => match t {
                    Target::Id(id) => ("hidden", Some(id.to_string()), None::<u64>),
                    Target::Selector(s) => ("hidden", Some(s.clone()), None::<u64>),
                    _ => {
                        return Err(TranslationError::InvalidTarget(
                            "Wait hidden requires ID or Selector".into(),
                        ));
                    }
                },
                crate::command::WaitCondition::Exists(s) => {
                    ("exists", Some(s.clone()), None::<u64>)
                }
                crate::command::WaitCondition::Gone(s) => ("gone", Some(s.clone()), None::<u64>),
                crate::command::WaitCondition::Url(_) => ("navigation", None, None::<u64>), // Simple mapping for now
                crate::command::WaitCondition::Load => ("load", None, None::<u64>), // Not supported by scanner directly usually
                crate::command::WaitCondition::Idle => ("idle", None, None::<u64>),
            };

            Ok(ScannerRequest::Wait(WaitRequest {
                condition: cond_str.to_string(),
                target,
                timeout_ms: timeout.or(Some(30000)), // Default 30s
            }))
        }

        Command::Storage(op) => {
            // Map 'storage clear' etc to Execute script
            // This is a naive implementation; proper support might need a dedicated protocol message
            // or just using Execute.
            let script = match op.as_str() {
                "clear" => {
                    "localStorage.clear(); sessionStorage.clear(); return 'Storage cleared';"
                }
                "ls_clear" => "localStorage.clear(); return 'Local storage cleared';",
                "ss_clear" => "sessionStorage.clear(); return 'Session storage cleared';",
                _ => return Err(TranslationError::Unsupported(format!("Storage op: {}", op))),
            };

            Ok(ScannerRequest::Execute(crate::protocol::ExecuteRequest {
                script: script.to_string(),
                args: vec![],
            }))
        }

        // Navigation commands are handled by Backend::navigate usually, but if we need a scanner equivalent:
        // Command::GoTo(_) => ...
        _ => Err(TranslationError::Unsupported(format!("{:?}", command))),
    }
}

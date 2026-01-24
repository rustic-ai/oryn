use crate::command::{Command, Target};
use crate::protocol::{
    AcceptRequest, CheckRequest, ClearRequest, ClickRequest, DismissRequest, ExecuteRequest,
    ExtractRequest, FocusRequest, GetHtmlRequest, GetTextRequest, HoverRequest, LoginRequest,
    MouseButton, ScanRequest, ScannerRequest, ScrollDirection, ScrollRequest, SearchRequest,
    SelectRequest, TypeRequest, WaitRequest,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TranslationError {
    #[error("Unknown command: {0}")]
    UnknownCommand(String),
    #[error("Missing argument: {0}")]
    MissingArgument(String),
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    #[error("Invalid target: {0}")]
    InvalidTarget(String),
    #[error("Unsupported target type for command: {0}")]
    UnsupportedTarget(String),
    #[error("Unsupported command: {0}")]
    Unsupported(String),
}

/// Extracts the numeric ID from a Target, returning an error if the target is not an ID.
fn extract_id(target: &Target, command_name: &str) -> Result<u32, TranslationError> {
    match target {
        Target::Id(id) => Ok(*id as u32),
        _ => Err(TranslationError::InvalidTarget(format!(
            "{} requires a resolved numeric ID target",
            command_name
        ))),
    }
}

/// Result of extracting a wait target.
struct WaitTargetInfo {
    selector: Option<String>,
    text: Option<String>,
}

/// Extracts target info (selector or text) for wait conditions.
fn extract_wait_target(target: &Target) -> Result<WaitTargetInfo, TranslationError> {
    match target {
        Target::Id(id) => Ok(WaitTargetInfo {
            selector: Some(id.to_string()),
            text: None,
        }),
        Target::Selector(s) => Ok(WaitTargetInfo {
            selector: Some(s.clone()),
            text: None,
        }),
        Target::Text(t) => Ok(WaitTargetInfo {
            selector: None,
            text: Some(t.clone()),
        }),
        _ => Err(TranslationError::InvalidTarget(
            "Wait requires ID, Selector, or Text target".to_string(),
        )),
    }
}

/// Escapes a string for use in JavaScript single-quoted strings.
fn js_escape(s: &str) -> String {
    s.replace('\'', "\\'")
}

/// Translates a high-level Intent Command into a low-level ScannerRequest.
pub fn translate(command: &Command) -> Result<ScannerRequest, TranslationError> {
    match command {
        Command::Observe(options) => {
            let max_elements = options.get("max").and_then(|v| v.parse::<usize>().ok());
            let near = options.get("near").cloned();
            let viewport_only = options.contains_key("viewport");
            let view_all = options.contains_key("full");
            let include_hidden = options.contains_key("hidden");

            Ok(ScannerRequest::Scan(ScanRequest {
                max_elements,
                monitor_changes: false,
                include_hidden,
                view_all,
                near,
                viewport_only,
            }))
        }

        Command::Click(target, options) => {
            let id = extract_id(target, "Click")?;
            let button = if options.contains_key("right") {
                MouseButton::Right
            } else if options.contains_key("middle") {
                MouseButton::Middle
            } else {
                MouseButton::Left
            };

            Ok(ScannerRequest::Click(ClickRequest {
                id,
                button,
                double: options.contains_key("double"),
                modifiers: vec![],
                force: options.contains_key("force"),
            }))
        }

        Command::Type(target, text, options) => {
            let id = extract_id(target, "Type")?;
            Ok(ScannerRequest::Type(TypeRequest {
                id,
                text: text.clone(),
                clear: !options.contains_key("append"),
                submit: options.contains_key("enter"),
                delay: options.get("delay").and_then(|v| v.parse::<u64>().ok()),
            }))
        }

        Command::Submit(target) => {
            let id = extract_id(target, "Submit")?;
            Ok(ScannerRequest::Submit(crate::protocol::SubmitRequest {
                id,
            }))
        }

        Command::Scroll(target, options) => {
            let id = target.as_ref().and_then(|t| {
                if let Target::Id(id) = t {
                    Some(*id as u32)
                } else {
                    None
                }
            });

            let direction = options
                .get("direction")
                .map(|dir| match dir.as_str() {
                    "up" => ScrollDirection::Up,
                    "left" => ScrollDirection::Left,
                    "right" => ScrollDirection::Right,
                    _ => ScrollDirection::Down,
                })
                .unwrap_or(ScrollDirection::Down);

            Ok(ScannerRequest::Scroll(ScrollRequest {
                id,
                direction,
                amount: options.get("amount").cloned().or(Some("page".to_string())),
            }))
        }

        Command::Wait(condition, options) => {
            use crate::command::WaitCondition;

            let (cond_str, selector, text) = match condition {
                WaitCondition::Visible(t) => {
                    let info = extract_wait_target(t)?;
                    ("visible", info.selector, info.text)
                }
                WaitCondition::Hidden(t) => {
                    let info = extract_wait_target(t)?;
                    ("hidden", info.selector, info.text)
                }
                WaitCondition::Exists(s) => ("exists", Some(s.clone()), None),
                WaitCondition::Gone(s) => ("gone", Some(s.clone()), None),
                WaitCondition::Url(_) => ("navigation", None, None),
                WaitCondition::Load => ("load", None, None),
                WaitCondition::Idle => ("idle", None, None),
            };

            let timeout_ms = options
                .get("timeout")
                .and_then(|t| t.parse::<u64>().ok())
                .or(Some(30000));

            Ok(ScannerRequest::Wait(WaitRequest {
                condition: cond_str.to_string(),
                target: selector,
                text,
                timeout_ms,
            }))
        }

        Command::Storage(action) => {
            use crate::command::{StorageAction, StorageType};

            let script = match action {
                StorageAction::Get { storage_type, key } => {
                    let k = js_escape(key);
                    match storage_type {
                        StorageType::Local => format!("return localStorage.getItem('{}');", k),
                        StorageType::Session => format!("return sessionStorage.getItem('{}');", k),
                        StorageType::Both => format!(
                            "var v = localStorage.getItem('{k}'); \
                             if (v !== null) return {{ local: v }}; \
                             v = sessionStorage.getItem('{k}'); \
                             return v !== null ? {{ session: v }} : null;"
                        ),
                    }
                }
                StorageAction::Set { storage_type, key, value } => {
                    let k = js_escape(key);
                    let v = js_escape(value);
                    match storage_type {
                        StorageType::Local => format!(
                            "localStorage.setItem('{k}', '{v}'); return 'Set in localStorage';"
                        ),
                        StorageType::Session => format!(
                            "sessionStorage.setItem('{k}', '{v}'); return 'Set in sessionStorage';"
                        ),
                        StorageType::Both => format!(
                            "localStorage.setItem('{k}', '{v}'); \
                             sessionStorage.setItem('{k}', '{v}'); \
                             return 'Set in both storages';"
                        ),
                    }
                }
                StorageAction::List { storage_type } => match storage_type {
                    StorageType::Local => "return Object.keys(localStorage);".into(),
                    StorageType::Session => "return Object.keys(sessionStorage);".into(),
                    StorageType::Both => "return { local: Object.keys(localStorage), session: Object.keys(sessionStorage) };".into(),
                },
                StorageAction::Clear { storage_type } => match storage_type {
                    StorageType::Local => "localStorage.clear(); return 'Local storage cleared';".into(),
                    StorageType::Session => "sessionStorage.clear(); return 'Session storage cleared';".into(),
                    StorageType::Both => "localStorage.clear(); sessionStorage.clear(); return 'Both storages cleared';".into(),
                },
            };

            Ok(ScannerRequest::Execute(ExecuteRequest {
                script,
                args: vec![],
            }))
        }

        Command::Check(target) => {
            let id = extract_id(target, "Check")?;
            Ok(ScannerRequest::Check(CheckRequest { id, state: true }))
        }

        Command::Uncheck(target) => {
            let id = extract_id(target, "Uncheck")?;
            Ok(ScannerRequest::Check(CheckRequest { id, state: false }))
        }

        Command::Clear(target) => {
            let id = extract_id(target, "Clear")?;
            Ok(ScannerRequest::Clear(ClearRequest { id }))
        }

        Command::Focus(target) => {
            let id = extract_id(target, "Focus")?;
            Ok(ScannerRequest::Focus(FocusRequest { id }))
        }

        Command::Hover(target) => {
            let id = extract_id(target, "Hover")?;
            Ok(ScannerRequest::Hover(HoverRequest { id }))
        }

        Command::Select(target, value) => {
            let id = extract_id(target, "Select")?;
            // Smart detection: if value is a pure number, use as index; otherwise use as label
            let (index, label) = match value.parse::<usize>() {
                Ok(idx) => (Some(idx), None),
                Err(_) => (None, Some(value.clone())),
            };

            Ok(ScannerRequest::Select(SelectRequest {
                id,
                value: None,
                index,
                label,
            }))
        }

        // Group C: Content extraction via Execute
        Command::Url => Ok(ScannerRequest::Execute(ExecuteRequest {
            script: "return window.location.href;".to_string(),
            args: vec![],
        })),

        Command::Title => Ok(ScannerRequest::Execute(ExecuteRequest {
            script: "return document.title;".to_string(),
            args: vec![],
        })),

        Command::Text(options) => Ok(ScannerRequest::GetText(GetTextRequest {
            selector: options.get("selector").cloned(),
        })),

        Command::Html(options) => Ok(ScannerRequest::GetHtml(GetHtmlRequest {
            selector: options.get("selector").cloned(),
            outer: true, // Default to outerHTML for consistency with previous behavior
        })),

        Command::Extract(source) => {
            use crate::command::ExtractSource;
            let (source_str, selector) = match source {
                ExtractSource::Links => ("links", None),
                ExtractSource::Images => ("images", None),
                ExtractSource::Tables => ("tables", None),
                ExtractSource::Meta => ("meta", None),
                ExtractSource::Css(s) => ("css", Some(s.clone())),
            };
            Ok(ScannerRequest::Extract(ExtractRequest {
                source: source_str.into(),
                selector,
            }))
        }

        Command::Login(user, pass, _opts) => Ok(ScannerRequest::Login(LoginRequest {
            username: user.clone(),
            password: pass.clone(),
        })),

        Command::Search(query, _opts) => Ok(ScannerRequest::Search(SearchRequest {
            query: query.clone(),
        })),

        Command::Dismiss(target, _opts) => {
            let target_str = match target {
                Target::Text(s) | Target::Role(s) => s.clone(),

                Target::Infer => "popups".to_string(), // Default if not resolved

                _ => {
                    return Err(TranslationError::UnsupportedTarget(format!(
                        "Dismiss requires text target, got {:?}",
                        target
                    )));
                }
            };

            Ok(ScannerRequest::Dismiss(DismissRequest {
                target: target_str,
            }))
        }

        Command::Accept(target, _opts) => {
            let target_str = match target {
                Target::Text(s) | Target::Role(s) => s.clone(),

                Target::Infer => "cookies".to_string(), // Default if not resolved

                _ => {
                    return Err(TranslationError::UnsupportedTarget(format!(
                        "Accept requires text target, got {:?}",
                        target
                    )));
                }
            };

            Ok(ScannerRequest::Accept(AcceptRequest { target: target_str }))
        }

        Command::ScrollUntil(target, direction, options) => {
            // ScrollUntil scrolls in a direction until the target becomes visible
            // Extract target ID for the scroll request
            let id = match target {
                Target::Id(id) => Some(*id as u32),
                _ => None,
            };

            let scroll_dir = match direction {
                crate::command::ScrollDirection::Up => ScrollDirection::Up,
                crate::command::ScrollDirection::Down => ScrollDirection::Down,
                crate::command::ScrollDirection::Left => ScrollDirection::Left,
                crate::command::ScrollDirection::Right => ScrollDirection::Right,
            };

            Ok(ScannerRequest::Scroll(ScrollRequest {
                id,
                direction: scroll_dir,
                amount: options.get("amount").cloned().or(Some("page".to_string())),
            }))
        }

        // Commands handled directly by executor (backend trait methods):
        // GoTo, Back, Forward, Refresh, Screenshot, Pdf, Press, Cookies, Tabs,
        // Intents, Define, Undefine, Export, RunIntent, Packs, PackLoad, PackUnload, Learn
        _ => Err(TranslationError::Unsupported(format!("{:?}", command))),
    }
}

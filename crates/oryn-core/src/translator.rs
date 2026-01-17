use crate::command::{Command, Target};
use crate::protocol::{
    AcceptRequest, CheckRequest, ClearRequest, ClickRequest, DismissRequest, ExecuteRequest,
    ExtractRequest, FocusRequest, HoverRequest, LoginRequest, MouseButton, ScanRequest,
    ScannerRequest, ScrollDirection, ScrollRequest, SearchRequest, SelectRequest, TypeRequest,
    WaitRequest,
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
            if let Target::Id(id) = target {
                // Parse mouse button from options
                let button = if options.contains_key("right") {
                    MouseButton::Right
                } else if options.contains_key("middle") {
                    MouseButton::Middle
                } else {
                    MouseButton::Left
                };

                // Parse double-click
                let double = options.contains_key("double");

                // Parse force flag
                let force = options.contains_key("force");

                Ok(ScannerRequest::Click(ClickRequest {
                    id: *id as u32,
                    button,
                    double,
                    modifiers: vec![],
                    force,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Click requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Type(target, text, options) => {
            if let Target::Id(id) = target {
                // --append means don't clear (inverse logic)
                let clear = !options.contains_key("append");

                // --enter means submit after typing
                let submit = options.contains_key("enter");

                // --delay N for character-by-character typing
                let delay = options.get("delay").and_then(|v| v.parse::<u64>().ok());

                Ok(ScannerRequest::Type(TypeRequest {
                    id: *id as u32,
                    text: text.clone(),
                    clear,
                    submit,
                    delay,
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

            // Read amount from options, default to "page"
            let amount = options
                .get("amount")
                .cloned()
                .or_else(|| Some("page".to_string()));

            Ok(ScannerRequest::Scroll(ScrollRequest {
                id,
                direction,
                amount,
            }))
        }

        Command::Wait(condition, options) => {
            // Map WaitCondition enum to protocol strings
            // Protocol expects: "exists", "visible", "hidden", "gone", "navigation"
            // WaitCondition: Load, Idle, Visible(T), Hidden(T), Exists(selector/id), Gone(selector/id), Url(s)

            let (cond_str, target) = match condition {
                crate::command::WaitCondition::Visible(t) => match t {
                    Target::Id(id) => ("visible", Some(id.to_string())),
                    Target::Selector(s) => ("visible", Some(s.clone())),
                    _ => {
                        return Err(TranslationError::InvalidTarget(
                            "Wait visible requires ID or Selector".into(),
                        ));
                    }
                },
                crate::command::WaitCondition::Hidden(t) => match t {
                    Target::Id(id) => ("hidden", Some(id.to_string())),
                    Target::Selector(s) => ("hidden", Some(s.clone())),
                    _ => {
                        return Err(TranslationError::InvalidTarget(
                            "Wait hidden requires ID or Selector".into(),
                        ));
                    }
                },
                crate::command::WaitCondition::Exists(s) => ("exists", Some(s.clone())),
                crate::command::WaitCondition::Gone(s) => ("gone", Some(s.clone())),
                crate::command::WaitCondition::Url(_) => ("navigation", None), // Simple mapping for now
                crate::command::WaitCondition::Load => ("load", None), // Not supported by scanner directly usually
                crate::command::WaitCondition::Idle => ("idle", None),
            };

            // Read timeout from options, default to 30000ms (30s)
            let timeout_ms = options
                .get("timeout")
                .and_then(|t| t.parse::<u64>().ok())
                .or(Some(30000));

            Ok(ScannerRequest::Wait(WaitRequest {
                condition: cond_str.to_string(),
                target,
                timeout_ms,
            }))
        }

        Command::Storage(action) => {
            use crate::command::{StorageAction, StorageType};

            let script = match action {
                StorageAction::Get { storage_type, key } => {
                    let key_escaped = key.replace('\'', "\\'");
                    match storage_type {
                        StorageType::Local => {
                            format!("return localStorage.getItem('{}');", key_escaped)
                        }
                        StorageType::Session => {
                            format!("return sessionStorage.getItem('{}');", key_escaped)
                        }
                        StorageType::Both => {
                            format!(
                                "var v = localStorage.getItem('{}'); \
                                 if (v !== null) return {{ local: v }}; \
                                 v = sessionStorage.getItem('{}'); \
                                 return v !== null ? {{ session: v }} : null;",
                                key_escaped, key_escaped
                            )
                        }
                    }
                }
                StorageAction::Set {
                    storage_type,
                    key,
                    value,
                } => {
                    let key_escaped = key.replace('\'', "\\'");
                    let value_escaped = value.replace('\'', "\\'");
                    match storage_type {
                        StorageType::Local => {
                            format!(
                                "localStorage.setItem('{}', '{}'); return 'Set in localStorage';",
                                key_escaped, value_escaped
                            )
                        }
                        StorageType::Session => {
                            format!(
                                "sessionStorage.setItem('{}', '{}'); return 'Set in sessionStorage';",
                                key_escaped, value_escaped
                            )
                        }
                        StorageType::Both => {
                            format!(
                                "localStorage.setItem('{}', '{}'); \
                                 sessionStorage.setItem('{}', '{}'); \
                                 return 'Set in both storages';",
                                key_escaped, value_escaped, key_escaped, value_escaped
                            )
                        }
                    }
                }
                StorageAction::List { storage_type } => match storage_type {
                    StorageType::Local => {
                        "return Object.keys(localStorage);".to_string()
                    }
                    StorageType::Session => {
                        "return Object.keys(sessionStorage);".to_string()
                    }
                    StorageType::Both => {
                        "return { local: Object.keys(localStorage), session: Object.keys(sessionStorage) };".to_string()
                    }
                },
                StorageAction::Clear { storage_type } => match storage_type {
                    StorageType::Local => {
                        "localStorage.clear(); return 'Local storage cleared';".to_string()
                    }
                    StorageType::Session => {
                        "sessionStorage.clear(); return 'Session storage cleared';".to_string()
                    }
                    StorageType::Both => {
                        "localStorage.clear(); sessionStorage.clear(); return 'Both storages cleared';".to_string()
                    }
                },
            };

            Ok(ScannerRequest::Execute(crate::protocol::ExecuteRequest {
                script,
                args: vec![],
            }))
        }

        // Group A: Direct protocol mapping (existing types)
        Command::Check(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Check(CheckRequest {
                    id: *id as u32,
                    state: true,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Check requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Uncheck(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Check(CheckRequest {
                    id: *id as u32,
                    state: false,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Uncheck requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Clear(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Clear(ClearRequest { id: *id as u32 }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Clear requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Focus(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Focus(FocusRequest { id: *id as u32 }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Focus requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Hover(target) => {
            if let Target::Id(id) = target {
                Ok(ScannerRequest::Hover(HoverRequest { id: *id as u32 }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Hover requires a resolved numeric ID target".into(),
                ))
            }
        }

        Command::Select(target, value) => {
            if let Target::Id(id) = target {
                // Smart detection: if value is a pure number, use as index; otherwise use as label
                let (value_opt, index_opt, label_opt) = if let Ok(idx) = value.parse::<usize>() {
                    // Pure numeric value - treat as index
                    (None, Some(idx), None)
                } else {
                    // Text value - treat as label
                    (None, None, Some(value.clone()))
                };

                Ok(ScannerRequest::Select(SelectRequest {
                    id: *id as u32,
                    value: value_opt,
                    index: index_opt,
                    label: label_opt,
                }))
            } else {
                Err(TranslationError::InvalidTarget(
                    "Select requires a resolved numeric ID target".into(),
                ))
            }
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

        Command::Text(options) => {
            let script = if let Some(selector) = options.get("selector") {
                format!(
                    "var el = document.querySelector('{}'); return el ? el.innerText : null;",
                    selector.replace('\'', "\\'")
                )
            } else {
                "return document.body.innerText;".to_string()
            };
            Ok(ScannerRequest::Execute(ExecuteRequest {
                script,
                args: vec![],
            }))
        }

        Command::Html(options) => {
            let script = if let Some(selector) = options.get("selector") {
                format!(
                    "var el = document.querySelector('{}'); return el ? el.outerHTML : null;",
                    selector.replace('\'', "\\'")
                )
            } else {
                "return document.documentElement.outerHTML;".to_string()
            };
            Ok(ScannerRequest::Execute(ExecuteRequest {
                script,
                args: vec![],
            }))
        }

        Command::Extract(source) => {
            let (source_str, selector) = match source {
                crate::command::ExtractSource::Links => ("links".to_string(), None),
                crate::command::ExtractSource::Images => ("images".to_string(), None),
                crate::command::ExtractSource::Tables => ("tables".to_string(), None),
                crate::command::ExtractSource::Meta => ("meta".to_string(), None),
                crate::command::ExtractSource::Css(s) => ("css".to_string(), Some(s.clone())),
            };
            Ok(ScannerRequest::Extract(ExtractRequest {
                source: source_str,
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

        Command::Dismiss(target, _opts) => Ok(ScannerRequest::Dismiss(DismissRequest {
            target: target.clone(),
        })),

        Command::Accept(target, _opts) => Ok(ScannerRequest::Accept(AcceptRequest {
            target: target.clone(),
        })),

        // Navigation commands are handled by Backend trait methods
        // Command::GoTo(_) => handled by backend.navigate()
        // Command::Back => needs backend.go_back()
        // Command::Forward => needs backend.go_forward()
        // Command::Refresh(_) => needs backend.refresh()
        // Command::Screenshot(_) => handled by backend.screenshot()
        _ => Err(TranslationError::Unsupported(format!("{:?}", command))),
    }
}

use crate::ast::{Command, ExtractWhat, Target, TargetAtomic, WaitCondition};
use oryn_common::protocol::{
    AcceptRequest, Action, BackRequest, BrowserAction, CheckRequest, ClearRequest, ClickRequest,
    CookieRequest, DismissRequest, ExecuteRequest, ExtractRequest, FocusRequest, ForwardRequest,
    GetHtmlRequest, GetTextRequest, HoverRequest, LoginRequest, MouseButton, NavigateRequest,
    PdfRequest, RefreshRequest, ScanRequest, ScannerAction, ScreenshotRequest, ScrollDirection,
    ScrollRequest, SearchRequest, SelectRequest, SessionAction, SubmitRequest, TabRequest,
    TypeRequest, WaitRequest,
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
    #[error("Unsupported target atomic type for command: {0}")]
    UnsupportedTarget(String),
    #[error("Unsupported command: {0}")]
    Unsupported(String),
}

struct ActionTarget {
    id: Option<u32>,
    selector: Option<String>,
}

struct WaitTarget {
    id: Option<u32>,
    selector: Option<String>,
    text: Option<String>,
}

fn extract_id(target: &Target, command_name: &str) -> Result<u32, TranslationError> {
    match &target.atomic {
        TargetAtomic::Id(id) => Ok(*id as u32),
        _ => Err(TranslationError::InvalidTarget(format!(
            "{} requires a resolved numeric ID target",
            command_name
        ))),
    }
}

fn extract_action_target(
    target: &Target,
    command_name: &str,
) -> Result<ActionTarget, TranslationError> {
    match &target.atomic {
        TargetAtomic::Id(id) => Ok(ActionTarget {
            id: Some(*id as u32),
            selector: None,
        }),
        TargetAtomic::Selector { kind, value } => {
            if kind != "css" {
                return Err(TranslationError::UnsupportedTarget(format!(
                    "{} does not support {} selectors",
                    command_name, kind
                )));
            }
            Ok(ActionTarget {
                id: None,
                selector: Some(value.clone()),
            })
        }
        _ => Err(TranslationError::InvalidTarget(format!(
            "{} requires a resolved numeric ID or CSS selector target",
            command_name
        ))),
    }
}

fn extract_wait_target(target: &Target) -> Result<WaitTarget, TranslationError> {
    match &target.atomic {
        TargetAtomic::Id(id) => Ok(WaitTarget {
            id: Some(*id as u32),
            selector: None,
            text: None,
        }),
        TargetAtomic::Selector { kind, value } => {
            if kind != "css" {
                return Err(TranslationError::UnsupportedTarget(format!(
                    "Wait does not support {} selectors",
                    kind
                )));
            }
            Ok(WaitTarget {
                id: None,
                selector: Some(value.clone()),
                text: None,
            })
        }
        TargetAtomic::Text(text) => Ok(WaitTarget {
            id: None,
            selector: None,
            text: Some(text.clone()),
        }),
        _ => Err(TranslationError::InvalidTarget(
            "Wait requires ID, CSS selector, or text target".into(),
        )),
    }
}

fn parse_duration_ms(value: &str) -> Option<u64> {
    if let Some(ms) = value.strip_suffix("ms") {
        return ms.parse::<u64>().ok();
    }
    if let Some(s) = value.strip_suffix('s') {
        return s.parse::<u64>().ok().and_then(|v| v.checked_mul(1000));
    }
    if let Some(m) = value.strip_suffix('m') {
        return m.parse::<u64>().ok().and_then(|v| v.checked_mul(60_000));
    }
    value.parse::<u64>().ok()
}

pub fn translate(command: &Command) -> Result<Action, TranslationError> {
    match command {
        // --- Navigation ---
        Command::Goto(cmd) => Ok(Action::Browser(BrowserAction::Navigate(NavigateRequest {
            url: cmd.url.clone(),
        }))),
        Command::Back => Ok(Action::Browser(BrowserAction::Back(BackRequest::default()))),
        Command::Forward => Ok(Action::Browser(BrowserAction::Forward(
            ForwardRequest::default(),
        ))),
        Command::Refresh(cmd) => Ok(Action::Browser(BrowserAction::Refresh(RefreshRequest {
            hard: cmd.hard,
        }))),
        Command::Url => Ok(Action::Scanner(ScannerAction::Execute(ExecuteRequest {
            script: "return window.location.href;".into(),
            args: vec![],
        }))),

        // --- Observation ---
        Command::Observe(cmd) => Ok(Action::Scanner(ScannerAction::Scan(ScanRequest {
            max_elements: None, // TODO
            monitor_changes: false,
            include_hidden: cmd.hidden,
            view_all: cmd.full,
            near: cmd.near.clone(),
            viewport_only: cmd.viewport,
        }))),
        Command::Html(cmd) => Ok(Action::Scanner(ScannerAction::GetHtml(GetHtmlRequest {
            selector: cmd.selector.clone(),
            outer: true,
        }))),
        Command::Text(cmd) => Ok(Action::Scanner(ScannerAction::GetText(GetTextRequest {
            selector: cmd.selector.clone(),
        }))),
        Command::Title => Ok(Action::Scanner(ScannerAction::Execute(ExecuteRequest {
            script: "return document.title;".into(),
            args: vec![],
        }))),
        Command::Screenshot(cmd) => Ok(Action::Browser(BrowserAction::Screenshot(
            ScreenshotRequest {
                output: cmd.output.clone(),
                format: cmd.format.clone(),
                selector: None, // target?
                fullpage: cmd.fullpage,
            },
        ))),

        // --- Actions ---
        Command::Click(cmd) => {
            let target = extract_action_target(&cmd.target, "Click")?;
            let button = if cmd.right {
                MouseButton::Right
            } else if cmd.middle {
                MouseButton::Middle
            } else {
                MouseButton::Left
            };
            Ok(Action::Scanner(ScannerAction::Click(ClickRequest {
                id: target.id,
                selector: target.selector,
                button,
                double: cmd.double,
                modifiers: vec![], // TODO: extract from options if parser supported
                force: cmd.force,
            })))
        }
        Command::Type(cmd) => {
            let target = extract_action_target(&cmd.target, "Type")?;
            Ok(Action::Scanner(ScannerAction::Type(TypeRequest {
                id: target.id,
                selector: target.selector,
                text: cmd.text.clone(),
                clear: cmd.clear,
                submit: cmd.enter,
                delay: cmd.delay.map(|d| d as u64),
            })))
        }
        Command::Clear(cmd) => {
            let target = extract_action_target(&cmd.target, "Clear")?;
            Ok(Action::Scanner(ScannerAction::Clear(ClearRequest {
                id: target.id,
                selector: target.selector,
            })))
        }
        Command::Select(cmd) => {
            let target = extract_action_target(&cmd.target, "Select")?;
            let (index, label) = if let Ok(idx) = cmd.value.parse::<usize>() {
                (Some(idx), None)
            } else {
                (None, Some(cmd.value.clone()))
            };
            Ok(Action::Scanner(ScannerAction::Select(SelectRequest {
                id: target.id,
                selector: target.selector,
                value: None,
                index,
                label,
            })))
        }
        Command::Check(cmd) => {
            let target = extract_action_target(&cmd.target, "Check")?;
            Ok(Action::Scanner(ScannerAction::Check(CheckRequest {
                id: target.id,
                selector: target.selector,
                state: true,
            })))
        }
        Command::Uncheck(cmd) => {
            let target = extract_action_target(&cmd.target, "Uncheck")?;
            Ok(Action::Scanner(ScannerAction::Check(CheckRequest {
                id: target.id,
                selector: target.selector,
                state: false,
            })))
        }
        Command::Hover(cmd) => {
            let target = extract_action_target(&cmd.target, "Hover")?;
            Ok(Action::Scanner(ScannerAction::Hover(HoverRequest {
                id: target.id,
                selector: target.selector,
            })))
        }
        Command::Focus(cmd) => {
            let target = extract_action_target(&cmd.target, "Focus")?;
            Ok(Action::Scanner(ScannerAction::Focus(FocusRequest {
                id: target.id,
                selector: target.selector,
            })))
        }
        Command::Submit(cmd) => {
            let (id, selector) = if let Some(t) = &cmd.target {
                let target = extract_action_target(t, "Submit")?;
                (target.id, target.selector)
            } else {
                (None, None)
            };
            Ok(Action::Scanner(ScannerAction::Submit(SubmitRequest {
                id,
                selector,
            })))
        }
        Command::Scroll(cmd) => {
            let id = if let Some(t) = &cmd.target {
                Some(extract_id(t, "Scroll")?)
            } else {
                None
            };
            let direction = match cmd.direction.as_deref() {
                Some("up") => ScrollDirection::Up,
                Some("left") => ScrollDirection::Left,
                Some("right") => ScrollDirection::Right,
                _ => ScrollDirection::Down,
            };
            Ok(Action::Scanner(ScannerAction::Scroll(ScrollRequest {
                id,
                direction,
                amount: cmd.amount.map(|v| v.to_string()).or(if cmd.page {
                    Some("page".into())
                } else {
                    None
                }),
            })))
        }

        // --- Wait ---
        Command::Wait(cmd) => {
            let mut target = WaitTarget {
                id: None,
                selector: None,
                text: None,
            };
            let mut expression = None;
            let mut count = None;
            let cond_str = match &cmd.condition {
                WaitCondition::Visible(t) => {
                    target = extract_wait_target(t)?;
                    "visible"
                }
                WaitCondition::Hidden(t) => {
                    target = extract_wait_target(t)?;
                    "hidden"
                }
                WaitCondition::Exists(s) => {
                    target.selector = Some(s.clone());
                    "exists"
                }
                WaitCondition::Gone(s) => {
                    target.selector = Some(s.clone());
                    "gone"
                }
                WaitCondition::Url(_) => "navigation", // pattern?
                WaitCondition::Navigation => "navigation",
                WaitCondition::Load => "load",
                WaitCondition::Idle => "idle",
                WaitCondition::Until(s) => {
                    expression = Some(s.clone());
                    "custom"
                }
                WaitCondition::Items { selector, count: c } => {
                    target.selector = Some(selector.clone());
                    count = Some(c.round() as u64);
                    "count"
                }
                _ => "unknown",
            };
            Ok(Action::Scanner(ScannerAction::Wait(WaitRequest {
                condition: cond_str.into(),
                id: target.id,
                selector: target.selector,
                text: target.text,
                expression,
                count,
                timeout: cmd.timeout.as_ref().and_then(|t| parse_duration_ms(t)),
            })))
        }

        // --- Extract ---
        Command::Extract(cmd) => {
            let (source, sel) = match &cmd.what {
                ExtractWhat::Links => ("links", None),
                ExtractWhat::Images => ("images", None),
                ExtractWhat::Tables => ("tables", None),
                ExtractWhat::Meta => ("meta", None),
                ExtractWhat::Text => ("text", None),
                ExtractWhat::Css(s) => ("css", Some(s.clone())),
            };
            Ok(Action::Scanner(ScannerAction::Extract(ExtractRequest {
                source: source.into(),
                selector: cmd.selector.clone().or(sel),
            })))
        }

        // --- Session ---
        Command::Cookies(cmd) => {
            use crate::ast::CookiesAction as CA;
            let (action, name, value, domain) = match &cmd.action {
                CA::List => ("list", None, None, None),
                CA::Get(n) => ("get", Some(n.clone()), None, None),
                CA::Set { name, value } => ("set", Some(name.clone()), Some(value.clone()), None),
                CA::Delete(n) => ("delete", Some(n.clone()), None, None),
                CA::Clear => ("clear", None, None, None),
            };
            Ok(Action::Session(SessionAction::Cookie(CookieRequest {
                action: action.into(),
                name,
                value,
                domain,
            })))
        }
        // ... (Storage, Headers, etc)

        // --- Intents ---
        Command::Login(cmd) => Ok(Action::Scanner(ScannerAction::Login(LoginRequest {
            username: cmd.user.clone(),
            password: cmd.pass.clone(),
        }))),

        Command::Search(cmd) => Ok(Action::Scanner(ScannerAction::Search(SearchRequest {
            query: cmd.query.clone(),
        }))),

        Command::Dismiss(cmd) => Ok(Action::Scanner(ScannerAction::Dismiss(DismissRequest {
            target: cmd.target.clone(),
        }))),

        Command::AcceptCookies => Ok(Action::Scanner(ScannerAction::Accept(AcceptRequest {
            target: "cookies".into(),
        }))),

        Command::Press(cmd) => {
            if cmd.keys.is_empty() {
                return Err(TranslationError::MissingArgument("keys".into()));
            }
            // Simple heuristic: last key is the key, rest are modifiers
            // e.g. "Ctrl", "A" -> modifiers=["Ctrl"], key="A"
            let key = cmd.keys.last().unwrap().clone();
            let modifiers = cmd.keys[0..cmd.keys.len() - 1].to_vec();
            Ok(Action::Browser(BrowserAction::Press(
                oryn_common::protocol::PressRequest { key, modifiers },
            )))
        }

        Command::Tabs => Ok(Action::Browser(BrowserAction::Tab(TabRequest {
            action: "list".into(),
            url: None,
            tab_id: None,
            index: None,
        }))),

        Command::Tab(cmd) => {
            // Ast TabActionCmd -> Protocol TabAction -> TabRequest
            let (action, url, tab_id, index) = match &cmd.action {
                crate::ast::TabAction::New(u) => ("new", Some(u.clone()), None, None),
                crate::ast::TabAction::Switch(idx) => ("switch", None, None, Some(*idx as usize)),
                crate::ast::TabAction::Close(idx) => ("close", None, None, idx.map(|i| i as usize)),
            };
            Ok(Action::Browser(BrowserAction::Tab(TabRequest {
                action: action.into(),
                url,
                tab_id,
                index,
            })))
        }

        Command::Pdf(cmd) => Ok(Action::Browser(BrowserAction::Pdf(PdfRequest {
            path: cmd.path.clone(),
            format: cmd.format.clone(),
            landscape: cmd.landscape,
            margin: cmd.margin.clone(),
            scale: None, // AST doesn't have scale? Protocol does. Default None.
        }))),

        _ => Err(TranslationError::Unsupported(format!("{:?}", command))),
    }
}

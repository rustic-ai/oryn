use crate::ast::{Command, Target, TargetAtomic, WaitCondition, ExtractWhat};
use oryn_common::protocol::{
    Action, ScannerAction, BrowserAction, SessionAction, MetaAction,
    ScanRequest, ClickRequest, TypeRequest, ScrollRequest, ScrollDirection,
    WaitRequest, CheckRequest, SelectRequest, SubmitRequest, HoverRequest,
    FocusRequest, ClearRequest, ExecuteRequest, ExtractRequest,
    LoginRequest, SearchRequest, DismissRequest, AcceptRequest, 
    GetTextRequest, GetHtmlRequest,
    NavigateRequest, BackRequest, ForwardRequest, RefreshRequest, 
    ScreenshotRequest, PdfRequest, TabRequest, FrameRequest, DialogRequest,
    CookieRequest, StorageRequest, HeadersRequest, // ProxyRequest not in AST yet?
    PackRequest, IntentRequest, LearnRequest,
    MouseButton,
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

fn extract_id(target: &Target, command_name: &str) -> Result<u32, TranslationError> {
    match &target.atomic {
        TargetAtomic::Id(id) => Ok(*id as u32),
        _ => Err(TranslationError::InvalidTarget(format!(
            "{} requires a resolved numeric ID target",
            command_name
        ))),
    }
}

pub fn translate(command: &Command) -> Result<Action, TranslationError> {
    match command {
        // --- Navigation ---
        Command::Goto(cmd) => Ok(Action::Browser(BrowserAction::Navigate(NavigateRequest {
            url: cmd.url.clone(),
        }))),
        Command::Back => Ok(Action::Browser(BrowserAction::Back(BackRequest::default()))),
        Command::Forward => Ok(Action::Browser(BrowserAction::Forward(ForwardRequest::default()))),
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
        Command::Screenshot(cmd) => Ok(Action::Browser(BrowserAction::Screenshot(ScreenshotRequest {
            output: cmd.output.clone(),
            format: cmd.format.clone(),
            selector: None, // target?
            fullpage: cmd.fullpage,
        }))),

        // --- Actions ---
        Command::Click(cmd) => {
            let id = extract_id(&cmd.target, "Click")?;
            let button = if cmd.right { MouseButton::Right } 
                         else if cmd.middle { MouseButton::Middle } 
                         else { MouseButton::Left };
            Ok(Action::Scanner(ScannerAction::Click(ClickRequest {
                id,
                button,
                double: cmd.double,
                modifiers: vec![], // TODO: extract from options if parser supported
                force: cmd.force,
            })))
        }
        Command::Type(cmd) => {
            let id = extract_id(&cmd.target, "Type")?;
            Ok(Action::Scanner(ScannerAction::Type(TypeRequest {
                id,
                text: cmd.text.clone(),
                clear: cmd.clear,
                submit: cmd.enter,
                delay: cmd.delay.map(|d| d as u64),
            })))
        },
        Command::Clear(cmd) => {
             let id = extract_id(&cmd.target, "Clear")?;
             Ok(Action::Scanner(ScannerAction::Clear(ClearRequest { id })))
        },
        Command::Select(cmd) => {
            let id = extract_id(&cmd.target, "Select")?;
            let (index, label) = if let Ok(idx) = cmd.value.parse::<usize>() {
                (Some(idx), None)
            } else {
                (None, Some(cmd.value.clone()))
            };
            Ok(Action::Scanner(ScannerAction::Select(SelectRequest {
                id,
                value: None,
                index,
                label,
            })))
        },
        Command::Check(cmd) => {
            let id = extract_id(&cmd.target, "Check")?;
            Ok(Action::Scanner(ScannerAction::Check(CheckRequest { id, state: true })))
        },
        Command::Uncheck(cmd) => {
            let id = extract_id(&cmd.target, "Uncheck")?;
            Ok(Action::Scanner(ScannerAction::Check(CheckRequest { id, state: false })))
        },
        Command::Hover(cmd) => {
            let id = extract_id(&cmd.target, "Hover")?;
            Ok(Action::Scanner(ScannerAction::Hover(HoverRequest { id })))
        },
        Command::Focus(cmd) => {
            let id = extract_id(&cmd.target, "Focus")?;
            Ok(Action::Scanner(ScannerAction::Focus(FocusRequest { id })))
        },
        Command::Submit(cmd) => {
            let id = if let Some(t) = &cmd.target {
                extract_id(t, "Submit")?
            } else {
                0 // TODO: Global submit? Or translate error?
            };
            Ok(Action::Scanner(ScannerAction::Submit(SubmitRequest { id })))
        },
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
                amount: cmd.amount.map(|v| v.to_string()).or(if cmd.page { Some("page".into()) } else { None }),
            })))
        },

        // --- Wait ---
        Command::Wait(cmd) => {
            let (cond_str, selector, text) = match &cmd.condition {
                WaitCondition::Visible(t) => ("visible", target_to_selector(t), target_to_text(t)),
                WaitCondition::Hidden(t) => ("hidden", target_to_selector(t), target_to_text(t)),
                WaitCondition::Exists(s) => ("exists", Some(s.clone()), None),
                WaitCondition::Gone(s) => ("gone", Some(s.clone()), None),
                WaitCondition::Url(_) => ("navigation", None, None), // pattern?
                WaitCondition::Navigation => ("navigation", None, None),
                WaitCondition::Load => ("load", None, None),
                WaitCondition::Idle => ("idle", None, None),
                WaitCondition::Until(s) => ("until", Some(s.clone()), None), // custom js?
                // ...
                _ => ("unknown", None, None),
            };
            Ok(Action::Scanner(ScannerAction::Wait(WaitRequest {
                condition: cond_str.into(),
                target: selector,
                text,
                timeout_ms: cmd.timeout.as_ref().and_then(|t| t.strip_suffix("ms").or(Some(t)).and_then(|v| v.parse().ok())), 
            })))
        },

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
        },
        
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
        },
        // ... (Storage, Headers, etc)
        
        // --- Intents ---
        Command::Login(cmd) => Ok(Action::Scanner(ScannerAction::Login(LoginRequest {
            username: cmd.user.clone(),
            password: cmd.pass.clone(),
        }))),
        
        Command::Search(cmd) => Ok(Action::Scanner(ScannerAction::Search(SearchRequest {
            query: cmd.query.clone(),
        }))),

        // ...

        Command::Press(cmd) => {
            if cmd.keys.is_empty() {
                return Err(TranslationError::MissingArgument("keys".into()));
            }
            // Simple heuristic: last key is the key, rest are modifiers
            // e.g. "Ctrl", "A" -> modifiers=["Ctrl"], key="A"
            let key = cmd.keys.last().unwrap().clone();
            let modifiers = cmd.keys[0..cmd.keys.len()-1].to_vec();
            Ok(Action::Browser(BrowserAction::Press(
                oryn_common::protocol::PressRequest {
                    key,
                    modifiers,
                }
            )))
        },
        
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
        },

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

fn target_to_selector(t: &Target) -> Option<String> {
    match &t.atomic {
        TargetAtomic::Id(id) => Some(id.to_string()),
        TargetAtomic::Selector { value, .. } => Some(value.clone()),
        _ => None
    }
}

fn target_to_text(t: &Target) -> Option<String> {
    match &t.atomic {
        TargetAtomic::Text(s) => Some(s.clone()),
        _ => None
    }
}

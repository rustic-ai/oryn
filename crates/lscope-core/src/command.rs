use std::collections::HashMap;

/// Represents a target element in the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A numbered element ID (e.g., from an observation).
    Id(usize),
    /// A semantic text match (e.g., "Sign in").
    Text(String),
    /// A semantic role (e.g., email, submit).
    Role(String),
    /// A raw CSS or XPath selector.
    Selector(String),
}

/// Supported wait conditions for the `wait` command.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitCondition {
    Load,
    Idle,
    Visible(Target),
    Hidden(Target),
    Exists(String), // Selector
    Gone(String),   // Selector
    Url(String),    // Pattern
}

/// Sub-commands for data extraction.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtractSource {
    Links,
    Images,
    Tables,
    Css(String), // Selector
    Meta,
}

/// Sub-commands for cookie management.
#[derive(Debug, Clone, PartialEq)]
pub enum CookieAction {
    List,
    Get(String),
    Set(String, String),
    Delete(String),
}

/// Sub-commands for tab management.
#[derive(Debug, Clone, PartialEq)]
pub enum TabAction {
    List, // Corresponds to `tabs` command or `tab list`? Spec implies `tabs` is separate command, but we can unify.
    New(String),
    Switch(String),
    Close(String),
}

/// The core intent command enum.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // Navigation
    GoTo(String),
    Back,
    Forward,
    Refresh(HashMap<String, String>), // Options: --hard
    Url,

    // Observation
    Observe(HashMap<String, String>), // Options: --full, --minimal, --near
    Html(HashMap<String, String>),    // Options: --selector
    Text(HashMap<String, String>),    // Options: --selector
    Title,
    Screenshot(HashMap<String, String>), // Options: format, selector, file

    // Action
    Click(Target, HashMap<String, String>), // Options: --force, double/right/middle
    Type(Target, String, HashMap<String, String>), // Options: --append, --enter, --delay
    Clear(Target),
    Press(String, HashMap<String, String>), // Key, modifiers
    Select(Target, String), // Value/Text/Index. How to distinguish? Parser can resolve.
    Check(Target),
    Uncheck(Target),
    Hover(Target),
    Focus(Target),
    Scroll(Option<Target>, HashMap<String, String>), // Target or direction/amount in options

    // Wait
    Wait(WaitCondition, HashMap<String, String>), // Options: --timeout

    // Extraction
    Extract(ExtractSource),

    // Session
    Cookies(CookieAction),
    Storage(String), // "Manage localStorage", spec is vague. Maybe just subcmd?

    // Tabs
    Tabs(TabAction),
}

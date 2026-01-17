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
    /// Relational: Target is near another target.
    Near {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    /// Relational: Target is inside another target.
    Inside {
        target: Box<Target>,
        container: Box<Target>,
    },
    /// Relational: Target is after another target.
    After {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    /// Relational: Target is before another target.
    Before {
        target: Box<Target>,
        anchor: Box<Target>,
    },
    /// Relational: Target contains another target.
    Contains {
        target: Box<Target>,
        content: Box<Target>,
    },
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

/// Storage type for localStorage/sessionStorage operations.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum StorageType {
    Local,
    Session,
    #[default]
    Both,
}

/// Sub-commands for storage management.
#[derive(Debug, Clone, PartialEq)]
pub enum StorageAction {
    Get {
        storage_type: StorageType,
        key: String,
    },
    Set {
        storage_type: StorageType,
        key: String,
        value: String,
    },
    List {
        storage_type: StorageType,
    },
    Clear {
        storage_type: StorageType,
    },
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
    Storage(StorageAction),

    // Tabs
    Tabs(TabAction),

    // Missing Basic Commands
    Submit(Target),

    // Level 3 Composite Commands
    Login(String, String, HashMap<String, String>), // User, Pass, Options
    Search(String, HashMap<String, String>),        // Query, Options
    Dismiss(String, HashMap<String, String>),       // "popups", etc.
    Accept(String, HashMap<String, String>),        // "cookies", etc.
    ScrollUntil(Target, ScrollDirection, HashMap<String, String>), // Target, Direction

    // Browser Features
    Pdf(String), // Output path
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

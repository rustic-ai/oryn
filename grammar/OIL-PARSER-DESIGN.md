# OIL Parser: Design & Implementation Plan

## Version 1.0 — Based on OIL v1.8.1 Canonical Grammar

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Architecture Overview](#2-architecture-overview)
3. [Grammar Corrections](#3-grammar-corrections)
4. [AST Design](#4-ast-design)
5. [Builder Implementation](#5-builder-implementation)
6. [Error Handling](#6-error-handling)
7. [Testing Strategy](#7-testing-strategy)
8. [Implementation Phases](#8-implementation-phases)
9. [Integration Points](#9-integration-points)
10. [Appendices](#10-appendices)

---

## 1. Executive Summary

### 1.1 Purpose

This document defines the design and implementation plan for the Oryn Intent Language (OIL) parser. The parser transforms canonical OIL text into a strongly-typed Abstract Syntax Tree (AST) that the Oryn runtime can execute.

### 1.2 Goals

| Goal | Success Criteria |
|------|------------------|
| **Correctness** | 100% of 343 test vectors pass |
| **Performance** | Parse simple commands in <100μs, complex in <1ms |
| **Error Quality** | Actionable error messages with source locations |
| **Maintainability** | Clear separation of grammar, AST, and builder |
| **Testability** | Property-based tests, roundtrip verification |

### 1.3 Technology Choice: Pest

We use [Pest](https://pest.rs/) (PEG parser generator) because:

- **PEG ordered-choice semantics** align with OIL's design
- **Zero-copy parsing** — spans reference original input
- **Excellent error locations** — built-in position tracking
- **Rust-native** — no FFI, integrates with async ecosystem
- **Grammar as code** — `.pest` file is the specification

### 1.4 Non-Goals

- **Phase 1 normalization**: Raw → canonical transformation is a separate preprocessor
- **Semantic validation**: Target resolution happens in the resolver module
- **Execution**: Parser only builds AST; execution is the engine's job

---

## 2. Architecture Overview

### 2.1 Pipeline Diagram

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                        OIL Processing Pipeline                                │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ┌────────────┐     ┌────────────┐     ┌────────────┐     ┌────────────┐    │
│  │  Raw OIL   │────▶│ Normalizer │────▶│ Canonical  │────▶│   Parser   │    │
│  │   Input    │     │ (Phase 1)  │     │    OIL     │     │   (Pest)   │    │
│  │            │     │            │     │            │     │            │    │
│  │ "CLICK 5"  │     │ lowercase  │     │ "click 5"  │     │ Pair<Rule> │    │
│  └────────────┘     │ quote norm │     └────────────┘     └────────────┘    │
│                     └────────────┘                              │            │
│                                                                 ▼            │
│                                                          ┌────────────┐      │
│                                                          │    AST     │      │
│                                                          │  Builder   │      │
│                                                          │            │      │
│                                                          │ Command    │      │
│                                                          └────────────┘      │
│                                                                 │            │
│                    ┌────────────────────────────────────────────┤            │
│                    │                    │                       │            │
│                    ▼                    ▼                       ▼            │
│             ┌────────────┐       ┌────────────┐          ┌────────────┐      │
│             │  Resolver  │       │ Translator │          │  Formatter │      │
│             │ (Semantic) │       │ (Scanner)  │          │ (Response) │      │
│             └────────────┘       └────────────┘          └────────────┘      │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Module Structure

```
oryn-parser/
├── Cargo.toml
├── src/
│   ├── lib.rs                  # Public API
│   ├── grammar.rs              # Pest derive macro
│   ├── oil.pest                # PEG grammar (corrected)
│   │
│   ├── ast/                    # AST type definitions
│   │   ├── mod.rs              # Re-exports
│   │   ├── command.rs          # Command enum + variants
│   │   ├── target.rs           # Target, TargetAtom, Relation
│   │   ├── primitives.rs       # Duration, Number, StringValue
│   │   └── span.rs             # Source location tracking
│   │
│   ├── builder/                # Pest Pair → AST conversion
│   │   ├── mod.rs              # Entry point + dispatch
│   │   ├── navigation.rs       # goto, back, forward, refresh, url
│   │   ├── observation.rs      # observe, html, text, screenshot, box
│   │   ├── action.rs           # click, type, scroll, press, etc.
│   │   ├── wait.rs             # wait conditions
│   │   ├── session.rs          # cookies, storage, state, headers
│   │   ├── network.rs          # intercept, requests
│   │   ├── intent.rs           # login, search, dismiss
│   │   ├── pack.rs             # packs, define, export, run
│   │   ├── viewport.rs         # viewport, device, media
│   │   ├── recording.rs        # trace, record, highlight
│   │   └── utility.rs          # pdf, learn, exit, help
│   │
│   └── error.rs                # ParseError types
│
├── tests/
│   ├── vectors.rs              # Test vector runner (343 vectors)
│   ├── roundtrip.rs            # AST → serialize → AST
│   ├── errors.rs               # Error message quality
│   └── fixtures/
│       └── oil-test-vectors.v1.8.1.yaml
│
└── benches/
    └── parser.rs               # Performance benchmarks
```

### 2.3 Data Flow

```
Input: "click 5 --double --timeout 10s"
                │
                ▼
┌─────────────────────────────────────────┐
│  Pest Parser (oil.pest)                 │
│                                         │
│  Produces: Pairs<Rule>                  │
│  ┌─────────────────────────────────┐    │
│  │ click_cmd                       │    │
│  │ ├─ target: "5"                  │    │
│  │ ├─ "--double"                   │    │
│  │ └─ timeout_opt                  │    │
│  │    └─ duration: "10s"           │    │
│  └─────────────────────────────────┘    │
└─────────────────────────────────────────┘
                │
                ▼
┌─────────────────────────────────────────┐
│  AST Builder (builder/action.rs)        │
│                                         │
│  Produces: Command::Click(ClickCmd)     │
│  ┌─────────────────────────────────┐    │
│  │ ClickCmd {                      │    │
│  │   target: Target {              │    │
│  │     primary: TargetAtom::Id(5), │    │
│  │     relations: [],              │    │
│  │   },                            │    │
│  │   options: ClickOptions {       │    │
│  │     double: true,               │    │
│  │     timeout: Some(10000ms),     │    │
│  │     ..default()                 │    │
│  │   },                            │    │
│  │   span: Span { ... },           │    │
│  │ }                               │    │
│  └─────────────────────────────────┘    │
└─────────────────────────────────────────┘
```

---

## 3. Grammar Corrections

Before implementation, the Pest grammar requires these fixes:

### 3.1 CRITICAL: Function Key Ordering

**Problem**: PEG ordered-choice is greedy. `"f1"` matches before `"f10"` can be tried.

**Location**: Line 169

```pest
// BEFORE (BUG)
function_key = { "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" }

// AFTER (FIXED)
function_key = { "f10" | "f11" | "f12" | "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" }
```

**Test Case**:
```
press f10     # Would parse as "f1" + "0" (FAIL) → now parses as "f10" (OK)
press f12     # Would parse as "f1" + "2" (FAIL) → now parses as "f12" (OK)
```

### 3.2 RECOMMENDED: Scroll Command Ordering

**Problem**: `scroll_cmd` matches `scroll` alone, shadowing `scroll_until_cmd`.

**Current dispatch order**:
- Line 49: `action_cmd` (contains `scroll_cmd`)
- Line 54: `intent_cmd` (contains `scroll_until_cmd`)

**Fix Option**: Move `scroll_until_cmd` to `action_cmd` before `scroll_cmd`:

```pest
// Line 121-136: action_cmd
action_cmd = _{
    click_cmd |
    type_cmd |
    clear_cmd |
    press_cmd |
    keydown_cmd |
    keyup_cmd |
    keys_cmd |
    select_cmd |
    check_cmd |
    uncheck_cmd |
    hover_cmd |
    focus_cmd |
    scroll_until_cmd |    // ADDED: more specific, before scroll_cmd
    scroll_cmd |
    submit_cmd
}

// Line 296: Remove from intent_cmd
intent_cmd = _{ login_cmd | search_cmd | dismiss_cmd | accept_cookies_cmd }
```

---

## 4. AST Design

### 4.1 Design Principles

1. **Strongly typed**: Each command has explicit fields, no `HashMap<String, Value>`
2. **Flat structure**: Avoid deep nesting; prefer structs over nested enums
3. **Source spans**: Every node carries location info for error reporting
4. **Owned data**: AST owns all strings (no lifetime parameters)
5. **Serde support**: `Serialize`/`Deserialize` for testing and debugging

### 4.2 Core Types

```rust
// ═══════════════════════════════════════════════════════════════════════════
// ast/span.rs — Source Location
// ═══════════════════════════════════════════════════════════════════════════

/// Source location for error reporting and debugging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// Byte offset of start (0-indexed)
    pub start: usize,
    /// Byte offset of end (exclusive)
    pub end: usize,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
}

impl Span {
    /// Create from a Pest pair
    pub fn from_pest<R: pest::RuleType>(pair: &pest::iterators::Pair<R>) -> Self {
        let span = pair.as_span();
        let (line, column) = span.start_pos().line_col();
        Self {
            start: span.start(),
            end: span.end(),
            line,
            column,
        }
    }

    /// Merge two spans into one covering both
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            column: if self.line <= other.line { self.column } else { other.column },
        }
    }

    /// Extract source text this span covers
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }
}
```

```rust
// ═══════════════════════════════════════════════════════════════════════════
// ast/primitives.rs — Basic Types
// ═══════════════════════════════════════════════════════════════════════════

/// A quoted string value with source span
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StringValue {
    pub value: String,
    pub span: Span,
}

/// A numeric value (integer or decimal)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Number {
    pub value: f64,
    pub span: Span,
}

impl Number {
    pub fn as_u32(&self) -> Option<u32> {
        if self.value >= 0.0 && self.value.fract() == 0.0 && self.value <= u32::MAX as f64 {
            Some(self.value as u32)
        } else {
            None
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if self.value.fract() == 0.0 {
            Some(self.value as i64)
        } else {
            None
        }
    }
}

/// A duration: `5s`, `100ms`, `2m`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Duration {
    pub millis: u64,
    pub span: Span,
}

impl Duration {
    pub fn as_std(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.millis)
    }
}

/// A URL value (quoted or bare)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UrlValue {
    pub value: String,
    pub span: Span,
}

/// A file path (quoted or bare)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilePath {
    pub value: String,
    pub span: Span,
}

/// An identifier (unquoted name)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Identifier {
    pub value: String,
    pub span: Span,
}

/// A key combination: `ctrl+shift+a`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyCombo {
    pub raw: String,  // Original text for echo
    pub keys: Vec<KeyName>,
    pub span: Span,
}

/// Individual key names
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyName {
    // Modifiers
    Control, Shift, Alt, Meta,
    // Function keys
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    // Navigation
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Home, End, PageUp, PageDown,
    // Editing
    Enter, Tab, Escape, Space, Backspace, Delete,
    // Character
    Char(char),
}
```

### 4.3 Target Types

```rust
// ═══════════════════════════════════════════════════════════════════════════
// ast/target.rs — Element Targeting
// ═══════════════════════════════════════════════════════════════════════════

/// A target with optional relational modifiers
/// Example: `"Submit" inside "Login Form" near "Username"`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub primary: TargetAtom,
    pub relations: Vec<(Relation, TargetAtom)>,
    pub span: Span,
}

/// A single target specifier
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TargetAtom {
    /// Numeric element ID: `5`, `42`
    Id(u32),
    
    /// Semantic role: `email`, `password`, `submit`
    Role(TargetRole),
    
    /// CSS selector: `css(".btn-primary")`
    Css(StringValue),
    
    /// XPath selector: `xpath("//button[@id='submit']")`
    XPath(StringValue),
    
    /// Text match: `"Sign in"`, `"Submit"`
    Text(StringValue),
}

/// Semantic roles for targeting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetRole {
    Email,
    Password,
    Search,
    Submit,
    Username,
    Phone,
    Url,
}

impl TargetRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetRole::Email => "email",
            TargetRole::Password => "password",
            TargetRole::Search => "search",
            TargetRole::Submit => "submit",
            TargetRole::Username => "username",
            TargetRole::Phone => "phone",
            TargetRole::Url => "url",
        }
    }
}

/// Relational modifiers for targets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Relation {
    Near,
    Inside,
    After,
    Before,
    Contains,
}

impl Relation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Relation::Near => "near",
            Relation::Inside => "inside",
            Relation::After => "after",
            Relation::Before => "before",
            Relation::Contains => "contains",
        }
    }
}
```

### 4.4 Command Enum

```rust
// ═══════════════════════════════════════════════════════════════════════════
// ast/command.rs — Main Command Enum
// ═══════════════════════════════════════════════════════════════════════════

/// All OIL commands (65 total)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    // ─────────────────────────────────────────────────────────────────────
    // Navigation (5)
    // ─────────────────────────────────────────────────────────────────────
    GoTo(GoToCmd),
    Back(SpanOnly),
    Forward(SpanOnly),
    Refresh(RefreshCmd),
    Url(SpanOnly),

    // ─────────────────────────────────────────────────────────────────────
    // Observation (6)
    // ─────────────────────────────────────────────────────────────────────
    Observe(ObserveCmd),
    Html(HtmlCmd),
    Text(TextCmd),
    Title(SpanOnly),
    Screenshot(ScreenshotCmd),
    Box(BoxCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Actions (14)
    // ─────────────────────────────────────────────────────────────────────
    Click(ClickCmd),
    Type(TypeCmd),
    Clear(ClearCmd),
    Press(PressCmd),
    KeyDown(KeyDownCmd),
    KeyUp(KeyUpCmd),
    Keys(SpanOnly),
    Select(SelectCmd),
    Check(CheckCmd),
    Uncheck(UncheckCmd),
    Hover(HoverCmd),
    Focus(FocusCmd),
    Scroll(ScrollCmd),
    Submit(SubmitCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Wait (1 with 11 condition variants)
    // ─────────────────────────────────────────────────────────────────────
    Wait(WaitCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Extraction (1 with 6 type variants)
    // ─────────────────────────────────────────────────────────────────────
    Extract(ExtractCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Session Management (6)
    // ─────────────────────────────────────────────────────────────────────
    Cookies(CookiesCmd),
    Storage(StorageCmd),
    Sessions(SpanOnly),
    Session(SessionCmd),
    State(StateCmd),
    Headers(HeadersCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Tabs (2)
    // ─────────────────────────────────────────────────────────────────────
    Tabs(SpanOnly),
    Tab(TabCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Intents (5)
    // ─────────────────────────────────────────────────────────────────────
    Login(LoginCmd),
    Search(SearchCmd),
    Dismiss(DismissCmd),
    AcceptCookies(SpanOnly),
    ScrollUntil(ScrollUntilCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Pack Management (7)
    // ─────────────────────────────────────────────────────────────────────
    Packs(SpanOnly),
    Pack(PackCmd),
    Intents(IntentsCmd),
    Define(DefineCmd),
    Undefine(UndefineCmd),
    Export(ExportCmd),
    Run(RunCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Network (2)
    // ─────────────────────────────────────────────────────────────────────
    Intercept(InterceptCmd),
    Requests(RequestsCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Console/Errors (2)
    // ─────────────────────────────────────────────────────────────────────
    Console(ConsoleCmd),
    Errors(ErrorsCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Frames (2)
    // ─────────────────────────────────────────────────────────────────────
    Frames(SpanOnly),
    Frame(FrameCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Dialog (1)
    // ─────────────────────────────────────────────────────────────────────
    Dialog(DialogCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Viewport/Device/Media (4)
    // ─────────────────────────────────────────────────────────────────────
    Viewport(ViewportCmd),
    Device(DeviceCmd),
    Devices(SpanOnly),
    Media(MediaCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Recording (3)
    // ─────────────────────────────────────────────────────────────────────
    Trace(TraceCmd),
    Record(RecordCmd),
    Highlight(HighlightCmd),

    // ─────────────────────────────────────────────────────────────────────
    // Utility (4)
    // ─────────────────────────────────────────────────────────────────────
    Pdf(PdfCmd),
    Learn(LearnCmd),
    Exit(SpanOnly),
    Help(HelpCmd),
}

/// For commands with no arguments (span only)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanOnly {
    pub span: Span,
}

impl Command {
    /// Get the command name for error messages
    pub fn name(&self) -> &'static str {
        match self {
            Command::GoTo(_) => "goto",
            Command::Back(_) => "back",
            Command::Forward(_) => "forward",
            Command::Refresh(_) => "refresh",
            Command::Url(_) => "url",
            Command::Observe(_) => "observe",
            Command::Html(_) => "html",
            Command::Text(_) => "text",
            Command::Title(_) => "title",
            Command::Screenshot(_) => "screenshot",
            Command::Box(_) => "box",
            Command::Click(_) => "click",
            Command::Type(_) => "type",
            Command::Clear(_) => "clear",
            Command::Press(_) => "press",
            Command::KeyDown(_) => "keydown",
            Command::KeyUp(_) => "keyup",
            Command::Keys(_) => "keys",
            Command::Select(_) => "select",
            Command::Check(_) => "check",
            Command::Uncheck(_) => "uncheck",
            Command::Hover(_) => "hover",
            Command::Focus(_) => "focus",
            Command::Scroll(_) => "scroll",
            Command::Submit(_) => "submit",
            Command::Wait(_) => "wait",
            Command::Extract(_) => "extract",
            Command::Cookies(_) => "cookies",
            Command::Storage(_) => "storage",
            Command::Sessions(_) => "sessions",
            Command::Session(_) => "session",
            Command::State(_) => "state",
            Command::Headers(_) => "headers",
            Command::Tabs(_) => "tabs",
            Command::Tab(_) => "tab",
            Command::Login(_) => "login",
            Command::Search(_) => "search",
            Command::Dismiss(_) => "dismiss",
            Command::AcceptCookies(_) => "accept_cookies",
            Command::ScrollUntil(_) => "scroll until",
            Command::Packs(_) => "packs",
            Command::Pack(_) => "pack",
            Command::Intents(_) => "intents",
            Command::Define(_) => "define",
            Command::Undefine(_) => "undefine",
            Command::Export(_) => "export",
            Command::Run(_) => "run",
            Command::Intercept(_) => "intercept",
            Command::Requests(_) => "requests",
            Command::Console(_) => "console",
            Command::Errors(_) => "errors",
            Command::Frames(_) => "frames",
            Command::Frame(_) => "frame",
            Command::Dialog(_) => "dialog",
            Command::Viewport(_) => "viewport",
            Command::Device(_) => "device",
            Command::Devices(_) => "devices",
            Command::Media(_) => "media",
            Command::Trace(_) => "trace",
            Command::Record(_) => "record",
            Command::Highlight(_) => "highlight",
            Command::Pdf(_) => "pdf",
            Command::Learn(_) => "learn",
            Command::Exit(_) => "exit",
            Command::Help(_) => "help",
        }
    }

    /// Get the span for this command
    pub fn span(&self) -> Span {
        match self {
            Command::GoTo(c) => c.span,
            Command::Back(c) => c.span,
            // ... etc for all variants
            _ => todo!("implement span() for all variants"),
        }
    }
}
```

### 4.5 Command Structs (Selected Examples)

```rust
// ═══════════════════════════════════════════════════════════════════════════
// Navigation Commands
// ═══════════════════════════════════════════════════════════════════════════

/// goto <url> [--headers <json>] [--timeout <duration>]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoToCmd {
    pub url: UrlValue,
    pub headers: Option<StringValue>,
    pub timeout: Option<Duration>,
    pub span: Span,
}

/// refresh [--hard]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefreshCmd {
    pub hard: bool,
    pub span: Span,
}

// ═══════════════════════════════════════════════════════════════════════════
// Action Commands
// ═══════════════════════════════════════════════════════════════════════════

/// click <target> [--double] [--right] [--force] [--ctrl] [--shift] [--alt] [--timeout]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickCmd {
    pub target: Target,
    pub double: bool,
    pub right: bool,
    pub middle: bool,
    pub force: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub timeout: Option<Duration>,
    pub span: Span,
}

/// type <target> <text> [--append] [--enter] [--clear] [--delay <ms>] [--timeout]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeCmd {
    pub target: Target,
    pub text: StringValue,
    pub append: bool,
    pub enter: bool,
    pub clear: bool,
    pub delay: Option<Number>,
    pub timeout: Option<Duration>,
    pub span: Span,
}

/// press <key_combo>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PressCmd {
    pub key_combo: KeyCombo,
    pub span: Span,
}

/// scroll [<direction>] [<target>] [--amount <n>] [--page] [--timeout]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollCmd {
    pub direction: Option<ScrollDirection>,
    pub target: Option<Target>,
    pub amount: Option<Number>,
    pub page: bool,
    pub timeout: Option<Duration>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

// ═══════════════════════════════════════════════════════════════════════════
// Wait Command
// ═══════════════════════════════════════════════════════════════════════════

/// wait <condition> [--timeout <duration>]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitCmd {
    pub condition: WaitCondition,
    pub timeout: Option<Duration>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WaitCondition {
    Load,
    Idle,
    Navigation,
    Ready,
    Visible(Target),
    Hidden(Target),
    Exists(StringValue),      // CSS selector
    Gone(StringValue),        // CSS selector
    Url(StringValue),         // URL pattern
    Until(StringValue),       // JS expression
    Items {
        selector: StringValue,
        count: Number,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Session Commands
// ═══════════════════════════════════════════════════════════════════════════

/// storage <action> [--local|--session]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageCmd {
    pub action: StorageAction,
    pub storage_type: Option<StorageType>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StorageAction {
    List,
    Get(StringValue),
    Set { key: StringValue, value: StringValue },
    Delete(StringValue),
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageType {
    Local,
    Session,
}

/// intercept <pattern> [--block|--respond|--status] | intercept clear [<pattern>]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterceptCmd {
    pub action: InterceptAction,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InterceptAction {
    Clear { pattern: Option<StringValue> },
    Rule {
        pattern: StringValue,
        block: bool,
        respond: Option<StringValue>,
        respond_file: Option<FilePath>,
        status: Option<Number>,
    },
}

// ═══════════════════════════════════════════════════════════════════════════
// Intent Commands
// ═══════════════════════════════════════════════════════════════════════════

/// login <username> <password> [--no-submit] [--wait <duration>] [--timeout]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginCmd {
    pub username: StringValue,
    pub password: StringValue,
    pub no_submit: bool,
    pub wait: Option<Duration>,
    pub timeout: Option<Duration>,
    pub span: Span,
}

/// dismiss <target>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DismissCmd {
    pub target: DismissTarget,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DismissTarget {
    Popups,
    Modals,
    Modal,
    Banner,
    Custom(StringValue),
    Identifier(Identifier),
}

/// scroll until <target> [--amount <n>] [--page] [--timeout]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrollUntilCmd {
    pub target: Target,
    pub amount: Option<Number>,
    pub page: bool,
    pub timeout: Option<Duration>,
    pub span: Span,
}
```

### 4.6 Program Structure

```rust
// ═══════════════════════════════════════════════════════════════════════════
// ast/mod.rs — Top-Level Program
// ═══════════════════════════════════════════════════════════════════════════

/// A parsed OIL program (one or more lines)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub lines: Vec<Line>,
    pub span: Span,
}

/// A single line in an OIL program
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub content: LineContent,
    pub trailing_comment: Option<Comment>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LineContent {
    Empty,
    Comment(Comment),
    Command(Command),
}

/// A comment (full-line or trailing)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub text: String,
    pub span: Span,
}
```

---

## 5. Builder Implementation

### 5.1 Entry Point

```rust
// ═══════════════════════════════════════════════════════════════════════════
// builder/mod.rs
// ═══════════════════════════════════════════════════════════════════════════

use pest::Parser;
use pest_derive::Parser;

use crate::ast::*;
use crate::error::{ParseError, ParseResult};

#[derive(Parser)]
#[grammar = "oil.pest"]
pub struct OilParser;

/// Parse OIL source text into a Program AST
pub fn parse(source: &str) -> ParseResult<Program> {
    let pairs = OilParser::parse(Rule::oil_input, source)
        .map_err(|e| ParseError::from_pest(e, source))?;

    build_program(pairs, source)
}

/// Parse a single command (convenience for REPL)
pub fn parse_command(source: &str) -> ParseResult<Command> {
    let program = parse(source)?;
    
    for line in program.lines {
        if let LineContent::Command(cmd) = line.content {
            return Ok(cmd);
        }
    }
    
    Err(ParseError::NoCommand { 
        span: Span { start: 0, end: source.len(), line: 1, column: 1 }
    })
}

fn build_program(pairs: pest::iterators::Pairs<Rule>, source: &str) -> ParseResult<Program> {
    let mut lines = Vec::new();
    let mut span_start = 0;
    let mut span_end = 0;

    for pair in pairs {
        match pair.as_rule() {
            Rule::line => {
                let line = build_line(pair)?;
                if lines.is_empty() {
                    span_start = line.span.start;
                }
                span_end = line.span.end;
                lines.push(line);
            }
            Rule::EOI => break,
            _ => {}
        }
    }

    Ok(Program {
        lines,
        span: Span {
            start: span_start,
            end: span_end,
            line: 1,
            column: 1,
        },
    })
}

fn build_line(pair: pest::iterators::Pair<Rule>) -> ParseResult<Line> {
    let span = Span::from_pest(&pair);
    let mut content = LineContent::Empty;
    let mut trailing_comment = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::comment => {
                let comment_span = Span::from_pest(&inner);
                let text = inner.as_str()[1..].to_string(); // Skip '#'
                let comment = Comment { text, span: comment_span };
                
                if matches!(content, LineContent::Empty) {
                    content = LineContent::Comment(comment);
                } else {
                    trailing_comment = Some(comment);
                }
            }
            _ => {
                // Any command rule
                content = LineContent::Command(dispatch_command(inner)?);
            }
        }
    }

    Ok(Line { content, trailing_comment, span })
}

/// Dispatch to specific command builders based on rule
fn dispatch_command(pair: pest::iterators::Pair<Rule>) -> ParseResult<Command> {
    // The `command` rule is silent (_), so we receive the actual command rule
    match pair.as_rule() {
        // Navigation
        Rule::goto_cmd => navigation::build_goto(pair),
        Rule::back_cmd => Ok(Command::Back(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::forward_cmd => Ok(Command::Forward(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::refresh_cmd => navigation::build_refresh(pair),
        Rule::url_cmd => Ok(Command::Url(SpanOnly { span: Span::from_pest(&pair) })),

        // Observation
        Rule::observe_cmd => observation::build_observe(pair),
        Rule::html_cmd => observation::build_html(pair),
        Rule::text_cmd => observation::build_text(pair),
        Rule::title_cmd => Ok(Command::Title(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::screenshot_cmd => observation::build_screenshot(pair),
        Rule::box_cmd => observation::build_box(pair),

        // Actions
        Rule::click_cmd => action::build_click(pair),
        Rule::type_cmd => action::build_type(pair),
        Rule::clear_cmd => action::build_clear(pair),
        Rule::press_cmd => action::build_press(pair),
        Rule::keydown_cmd => action::build_keydown(pair),
        Rule::keyup_cmd => action::build_keyup(pair),
        Rule::keys_cmd => Ok(Command::Keys(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::select_cmd => action::build_select(pair),
        Rule::check_cmd => action::build_check(pair),
        Rule::uncheck_cmd => action::build_uncheck(pair),
        Rule::hover_cmd => action::build_hover(pair),
        Rule::focus_cmd => action::build_focus(pair),
        Rule::scroll_cmd => action::build_scroll(pair),
        Rule::scroll_until_cmd => intent::build_scroll_until(pair),
        Rule::submit_cmd => action::build_submit(pair),

        // Wait
        Rule::wait_cmd => wait::build_wait(pair),

        // Extraction
        Rule::extraction_cmd => extraction::build_extract(pair),

        // Session
        Rule::cookies_cmd => session::build_cookies(pair),
        Rule::storage_cmd => session::build_storage(pair),
        Rule::sessions_cmd => Ok(Command::Sessions(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::session_mgmt_cmd => session::build_session(pair),
        Rule::state_cmd => session::build_state(pair),
        Rule::headers_cmd => session::build_headers(pair),

        // Tabs
        Rule::tabs_cmd => Ok(Command::Tabs(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::tab_action_cmd => tab::build_tab(pair),

        // Intents
        Rule::login_cmd => intent::build_login(pair),
        Rule::search_cmd => intent::build_search(pair),
        Rule::dismiss_cmd => intent::build_dismiss(pair),
        Rule::accept_cookies_cmd => Ok(Command::AcceptCookies(SpanOnly { span: Span::from_pest(&pair) })),

        // Pack Management
        Rule::packs_cmd => Ok(Command::Packs(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::pack_action_cmd => pack::build_pack(pair),
        Rule::intents_cmd => pack::build_intents(pair),
        Rule::define_cmd => pack::build_define(pair),
        Rule::undefine_cmd => pack::build_undefine(pair),
        Rule::export_cmd => pack::build_export(pair),
        Rule::run_cmd => pack::build_run(pair),

        // Network
        Rule::intercept_cmd => network::build_intercept(pair),
        Rule::requests_cmd => network::build_requests(pair),

        // Console/Errors
        Rule::console_cmd | Rule::console_clear | Rule::console_view => console::build_console(pair),
        Rule::errors_cmd | Rule::errors_clear | Rule::errors_view => console::build_errors(pair),

        // Frames
        Rule::frames_cmd => Ok(Command::Frames(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::frame_switch_cmd => frame::build_frame(pair),

        // Dialog
        Rule::dialog_cmd => dialog::build_dialog(pair),

        // Viewport/Device/Media
        Rule::viewport_size_cmd => viewport::build_viewport(pair),
        Rule::device_cmd => viewport::build_device(pair),
        Rule::devices_cmd => Ok(Command::Devices(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::media_cmd => viewport::build_media(pair),

        // Recording
        Rule::trace_cmd => recording::build_trace(pair),
        Rule::record_cmd => recording::build_record(pair),
        Rule::highlight_cmd => recording::build_highlight(pair),

        // Utility
        Rule::pdf_cmd => utility::build_pdf(pair),
        Rule::learn_cmd => utility::build_learn(pair),
        Rule::exit_cmd => Ok(Command::Exit(SpanOnly { span: Span::from_pest(&pair) })),
        Rule::help_cmd => utility::build_help(pair),

        _ => Err(ParseError::UnknownRule {
            rule: format!("{:?}", pair.as_rule()),
            span: Span::from_pest(&pair),
        }),
    }
}

// Sub-modules
mod action;
mod console;
mod dialog;
mod extraction;
mod frame;
mod intent;
mod navigation;
mod network;
mod observation;
mod pack;
mod primitives;
mod recording;
mod session;
mod tab;
mod target;
mod utility;
mod viewport;
mod wait;
```

### 5.2 Target Builder

```rust
// ═══════════════════════════════════════════════════════════════════════════
// builder/target.rs
// ═══════════════════════════════════════════════════════════════════════════

use crate::ast::*;
use crate::error::{ParseError, ParseResult};
use super::primitives::*;
use super::Rule;

pub fn build_target(pair: pest::iterators::Pair<Rule>) -> ParseResult<Target> {
    let span = Span::from_pest(&pair);
    let mut atoms: Vec<TargetAtom> = Vec::new();
    let mut relations: Vec<(Relation, TargetAtom)> = Vec::new();
    let mut pending_relation: Option<Relation> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target_id => {
                let id: u32 = inner.as_str().parse().map_err(|_| {
                    ParseError::InvalidNumber {
                        value: inner.as_str().to_string(),
                        span: Span::from_pest(&inner),
                    }
                })?;
                let atom = TargetAtom::Id(id);
                
                if let Some(rel) = pending_relation.take() {
                    relations.push((rel, atom));
                } else {
                    atoms.push(atom);
                }
            }
            
            Rule::target_role => {
                let role = match inner.as_str() {
                    "email" => TargetRole::Email,
                    "password" => TargetRole::Password,
                    "search" => TargetRole::Search,
                    "submit" => TargetRole::Submit,
                    "username" => TargetRole::Username,
                    "phone" => TargetRole::Phone,
                    "url" => TargetRole::Url,
                    other => return Err(ParseError::InvalidRole {
                        role: other.to_string(),
                        span: Span::from_pest(&inner),
                    }),
                };
                let atom = TargetAtom::Role(role);
                
                if let Some(rel) = pending_relation.take() {
                    relations.push((rel, atom));
                } else {
                    atoms.push(atom);
                }
            }
            
            Rule::target_text => {
                let string = build_string_value(inner.into_inner().next().unwrap())?;
                let atom = TargetAtom::Text(string);
                
                if let Some(rel) = pending_relation.take() {
                    relations.push((rel, atom));
                } else {
                    atoms.push(atom);
                }
            }
            
            Rule::relation => {
                pending_relation = Some(match inner.as_str() {
                    "near" => Relation::Near,
                    "inside" => Relation::Inside,
                    "after" => Relation::After,
                    "before" => Relation::Before,
                    "contains" => Relation::Contains,
                    other => return Err(ParseError::InvalidRelation {
                        relation: other.to_string(),
                        span: Span::from_pest(&inner),
                    }),
                });
            }
            
            // Handle css() and xpath() via target_selector (silent rule)
            _ if inner.as_str().starts_with("css(") || inner.as_str().starts_with("css (") => {
                let selector_str = inner.into_inner()
                    .find(|p| p.as_rule() == Rule::string_value)
                    .map(build_string_value)
                    .transpose()?
                    .ok_or_else(|| ParseError::MissingArgument {
                        command: "target",
                        argument: "CSS selector",
                        span,
                    })?;
                let atom = TargetAtom::Css(selector_str);
                
                if let Some(rel) = pending_relation.take() {
                    relations.push((rel, atom));
                } else {
                    atoms.push(atom);
                }
            }
            
            _ if inner.as_str().starts_with("xpath(") || inner.as_str().starts_with("xpath (") => {
                let selector_str = inner.into_inner()
                    .find(|p| p.as_rule() == Rule::string_value)
                    .map(build_string_value)
                    .transpose()?
                    .ok_or_else(|| ParseError::MissingArgument {
                        command: "target",
                        argument: "XPath selector",
                        span,
                    })?;
                let atom = TargetAtom::XPath(selector_str);
                
                if let Some(rel) = pending_relation.take() {
                    relations.push((rel, atom));
                } else {
                    atoms.push(atom);
                }
            }
            
            _ => {}
        }
    }

    let primary = atoms.into_iter().next().ok_or_else(|| {
        ParseError::EmptyTarget { span }
    })?;

    Ok(Target { primary, relations, span })
}
```

### 5.3 Primitives Builder

```rust
// ═══════════════════════════════════════════════════════════════════════════
// builder/primitives.rs
// ═══════════════════════════════════════════════════════════════════════════

use crate::ast::*;
use crate::error::{ParseError, ParseResult};
use super::Rule;

pub fn build_string_value(pair: pest::iterators::Pair<Rule>) -> ParseResult<StringValue> {
    let span = Span::from_pest(&pair);
    
    // Find string_inner (the content without quotes)
    let inner = pair.into_inner()
        .find(|p| p.as_rule() == Rule::string_inner)
        .map(|p| p.as_str())
        .unwrap_or("");
    
    // Process escape sequences
    let value = unescape(inner, span)?;
    
    Ok(StringValue { value, span })
}

fn unescape(s: &str, span: Span) -> ParseResult<String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some(other) => {
                    return Err(ParseError::InvalidEscape {
                        sequence: format!("\\{}", other),
                        span,
                    });
                }
                None => {
                    return Err(ParseError::UnterminatedEscape { span });
                }
            }
        } else {
            result.push(c);
        }
    }
    
    Ok(result)
}

pub fn build_number(pair: pest::iterators::Pair<Rule>) -> ParseResult<Number> {
    let span = Span::from_pest(&pair);
    let value: f64 = pair.as_str().parse().map_err(|_| {
        ParseError::InvalidNumber {
            value: pair.as_str().to_string(),
            span,
        }
    })?;
    
    Ok(Number { value, span })
}

pub fn build_duration(pair: pest::iterators::Pair<Rule>) -> ParseResult<Duration> {
    let span = Span::from_pest(&pair);
    let s = pair.as_str();
    
    let millis = if let Some(ms) = s.strip_suffix("ms") {
        ms.parse::<u64>().map_err(|_| ParseError::InvalidDuration { 
            value: s.to_string(), 
            span 
        })?
    } else if let Some(secs) = s.strip_suffix('s') {
        let secs: u64 = secs.parse().map_err(|_| ParseError::InvalidDuration { 
            value: s.to_string(), 
            span 
        })?;
        secs * 1000
    } else if let Some(mins) = s.strip_suffix('m') {
        let mins: u64 = mins.parse().map_err(|_| ParseError::InvalidDuration { 
            value: s.to_string(), 
            span 
        })?;
        mins * 60 * 1000
    } else {
        return Err(ParseError::InvalidDuration { 
            value: s.to_string(), 
            span 
        });
    };
    
    Ok(Duration { millis, span })
}

pub fn build_timeout(pair: pest::iterators::Pair<Rule>) -> ParseResult<Duration> {
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::duration {
            return build_duration(inner);
        }
    }
    Err(ParseError::MissingDuration { 
        span: Span { start: 0, end: 0, line: 0, column: 0 } 
    })
}

pub fn build_url_value(pair: pest::iterators::Pair<Rule>) -> ParseResult<UrlValue> {
    let span = Span::from_pest(&pair);
    
    let inner = pair.into_inner().next().unwrap();
    let value = match inner.as_rule() {
        Rule::string_value => build_string_value(inner)?.value,
        Rule::url_bare => inner.as_str().to_string(),
        _ => inner.as_str().to_string(),
    };
    
    Ok(UrlValue { value, span })
}

pub fn build_file_path(pair: pest::iterators::Pair<Rule>) -> ParseResult<FilePath> {
    let span = Span::from_pest(&pair);
    
    let inner = pair.into_inner().next().unwrap();
    let value = match inner.as_rule() {
        Rule::string_value => build_string_value(inner)?.value,
        Rule::path_bare => inner.as_str().to_string(),
        _ => inner.as_str().to_string(),
    };
    
    Ok(FilePath { value, span })
}

pub fn build_identifier(pair: pest::iterators::Pair<Rule>) -> ParseResult<Identifier> {
    Ok(Identifier {
        value: pair.as_str().to_string(),
        span: Span::from_pest(&pair),
    })
}

pub fn build_key_combo(pair: pest::iterators::Pair<Rule>) -> ParseResult<KeyCombo> {
    let span = Span::from_pest(&pair);
    let raw = pair.as_str().to_string();
    
    let keys: Vec<KeyName> = raw
        .split('+')
        .map(|k| parse_key_name(k.trim()))
        .collect::<Result<_, _>>()?;
    
    Ok(KeyCombo { raw, keys, span })
}

fn parse_key_name(s: &str) -> ParseResult<KeyName> {
    Ok(match s.to_lowercase().as_str() {
        // Modifiers
        "control" | "ctrl" => KeyName::Control,
        "shift" => KeyName::Shift,
        "alt" => KeyName::Alt,
        "meta" | "cmd" | "command" | "win" | "super" => KeyName::Meta,
        
        // Function keys (note: f10/f11/f12 handled first in grammar)
        "f1" => KeyName::F1,
        "f2" => KeyName::F2,
        "f3" => KeyName::F3,
        "f4" => KeyName::F4,
        "f5" => KeyName::F5,
        "f6" => KeyName::F6,
        "f7" => KeyName::F7,
        "f8" => KeyName::F8,
        "f9" => KeyName::F9,
        "f10" => KeyName::F10,
        "f11" => KeyName::F11,
        "f12" => KeyName::F12,
        
        // Navigation
        "arrowup" | "up" => KeyName::ArrowUp,
        "arrowdown" | "down" => KeyName::ArrowDown,
        "arrowleft" | "left" => KeyName::ArrowLeft,
        "arrowright" | "right" => KeyName::ArrowRight,
        "home" => KeyName::Home,
        "end" => KeyName::End,
        "pageup" => KeyName::PageUp,
        "pagedown" => KeyName::PageDown,
        
        // Editing
        "enter" | "return" => KeyName::Enter,
        "tab" => KeyName::Tab,
        "escape" | "esc" => KeyName::Escape,
        "space" => KeyName::Space,
        "backspace" => KeyName::Backspace,
        "delete" | "del" => KeyName::Delete,
        
        // Single character
        s if s.len() == 1 => KeyName::Char(s.chars().next().unwrap()),
        
        other => return Err(ParseError::InvalidKeyName {
            key: other.to_string(),
            span: Span { start: 0, end: 0, line: 0, column: 0 },
        }),
    })
}
```

### 5.4 Example: Action Builder

```rust
// ═══════════════════════════════════════════════════════════════════════════
// builder/action.rs
// ═══════════════════════════════════════════════════════════════════════════

use crate::ast::*;
use crate::error::ParseResult;
use super::primitives::*;
use super::target::build_target;
use super::Rule;

pub fn build_click(pair: pest::iterators::Pair<Rule>) -> ParseResult<Command> {
    let span = Span::from_pest(&pair);
    let mut target = None;
    let mut double = false;
    let mut right = false;
    let mut middle = false;
    let mut force = false;
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => {
                target = Some(build_target(inner)?);
            }
            Rule::timeout_opt => {
                timeout = Some(build_timeout(inner)?);
            }
            _ => {
                // Handle flag options
                match inner.as_str() {
                    "--double" => double = true,
                    "--right" => right = true,
                    "--middle" => middle = true,
                    "--force" => force = true,
                    "--ctrl" => ctrl = true,
                    "--shift" => shift = true,
                    "--alt" => alt = true,
                    _ => {}
                }
            }
        }
    }

    Ok(Command::Click(ClickCmd {
        target: target.expect("target required by grammar"),
        double,
        right,
        middle,
        force,
        ctrl,
        shift,
        alt,
        timeout,
        span,
    }))
}

pub fn build_type(pair: pest::iterators::Pair<Rule>) -> ParseResult<Command> {
    let span = Span::from_pest(&pair);
    let mut target = None;
    let mut text = None;
    let mut append = false;
    let mut enter = false;
    let mut clear = false;
    let mut delay = None;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::target => {
                target = Some(build_target(inner)?);
            }
            Rule::string_value => {
                text = Some(build_string_value(inner)?);
            }
            Rule::timeout_opt => {
                timeout = Some(build_timeout(inner)?);
            }
            Rule::number => {
                // This follows --delay
                delay = Some(build_number(inner)?);
            }
            _ => {
                match inner.as_str() {
                    "--append" => append = true,
                    "--enter" => enter = true,
                    "--clear" => clear = true,
                    _ => {}
                }
            }
        }
    }

    Ok(Command::Type(TypeCmd {
        target: target.expect("target required by grammar"),
        text: text.expect("text required by grammar"),
        append,
        enter,
        clear,
        delay,
        timeout,
        span,
    }))
}

pub fn build_scroll(pair: pest::iterators::Pair<Rule>) -> ParseResult<Command> {
    let span = Span::from_pest(&pair);
    let mut direction = None;
    let mut target = None;
    let mut amount = None;
    let mut page = false;
    let mut timeout = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::scroll_direction => {
                direction = Some(match inner.as_str() {
                    "up" => ScrollDirection::Up,
                    "down" => ScrollDirection::Down,
                    "left" => ScrollDirection::Left,
                    "right" => ScrollDirection::Right,
                    _ => unreachable!(),
                });
            }
            Rule::target => {
                target = Some(build_target(inner)?);
            }
            Rule::number => {
                amount = Some(build_number(inner)?);
            }
            Rule::timeout_opt => {
                timeout = Some(build_timeout(inner)?);
            }
            _ => {
                if inner.as_str() == "--page" {
                    page = true;
                }
            }
        }
    }

    Ok(Command::Scroll(ScrollCmd {
        direction,
        target,
        amount,
        page,
        timeout,
        span,
    }))
}

pub fn build_press(pair: pest::iterators::Pair<Rule>) -> ParseResult<Command> {
    let span = Span::from_pest(&pair);
    
    let key_combo = pair.into_inner()
        .find(|p| p.as_rule() == Rule::key_combo)
        .map(build_key_combo)
        .transpose()?
        .expect("key_combo required by grammar");

    Ok(Command::Press(PressCmd { key_combo, span }))
}

// ... similar builders for clear, select, check, uncheck, hover, focus, submit
```

---

## 6. Error Handling

### 6.1 Error Types

```rust
// ═══════════════════════════════════════════════════════════════════════════
// error.rs
// ═══════════════════════════════════════════════════════════════════════════

use crate::ast::Span;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    // ─────────────────────────────────────────────────────────────────────
    // Syntax Errors
    // ─────────────────────────────────────────────────────────────────────
    
    #[error("syntax error at line {}, column {}: {message}", span.line, span.column)]
    Syntax {
        message: String,
        span: Span,
        expected: Vec<String>,
    },

    #[error("unknown command: `{command}`")]
    UnknownCommand {
        command: String,
        span: Span,
        suggestions: Vec<String>,
    },

    #[error("unknown rule: `{rule}`")]
    UnknownRule {
        rule: String,
        span: Span,
    },

    // ─────────────────────────────────────────────────────────────────────
    // Target Errors
    // ─────────────────────────────────────────────────────────────────────
    
    #[error("empty target specification")]
    EmptyTarget {
        span: Span,
    },

    #[error("invalid target role: `{role}`")]
    InvalidRole {
        role: String,
        span: Span,
    },

    #[error("invalid relation: `{relation}`")]
    InvalidRelation {
        relation: String,
        span: Span,
    },

    // ─────────────────────────────────────────────────────────────────────
    // Primitive Errors
    // ─────────────────────────────────────────────────────────────────────
    
    #[error("invalid number: `{value}`")]
    InvalidNumber {
        value: String,
        span: Span,
    },

    #[error("invalid duration: `{value}` (expected format: 5s, 100ms, 2m)")]
    InvalidDuration {
        value: String,
        span: Span,
    },

    #[error("missing duration")]
    MissingDuration {
        span: Span,
    },

    #[error("invalid escape sequence: `{sequence}`")]
    InvalidEscape {
        sequence: String,
        span: Span,
    },

    #[error("unterminated escape sequence")]
    UnterminatedEscape {
        span: Span,
    },

    #[error("invalid key name: `{key}`")]
    InvalidKeyName {
        key: String,
        span: Span,
    },

    // ─────────────────────────────────────────────────────────────────────
    // Argument Errors
    // ─────────────────────────────────────────────────────────────────────
    
    #[error("missing required argument `{argument}` for command `{command}`")]
    MissingArgument {
        command: &'static str,
        argument: &'static str,
        span: Span,
    },

    #[error("no command found in input")]
    NoCommand {
        span: Span,
    },

    // ─────────────────────────────────────────────────────────────────────
    // Pest Integration
    // ─────────────────────────────────────────────────────────────────────
    
    #[error("parse error: {message}")]
    Pest {
        message: String,
        span: Span,
        expected: Vec<String>,
    },
}

impl ParseError {
    /// Create from Pest error with enhanced message
    pub fn from_pest(err: pest::error::Error<super::builder::Rule>, source: &str) -> Self {
        let (line, column) = match err.line_col {
            pest::error::LineColLocation::Pos((l, c)) => (l, c),
            pest::error::LineColLocation::Span((l, c), _) => (l, c),
        };

        let (start, end) = match err.location {
            pest::error::InputLocation::Pos(p) => (p, p + 1),
            pest::error::InputLocation::Span((s, e)) => (s, e),
        };

        let expected = match &err.variant {
            pest::error::ErrorVariant::ParsingError { positives, .. } => {
                positives.iter().map(|r| format!("{:?}", r)).collect()
            }
            _ => vec![],
        };

        let message = match &err.variant {
            pest::error::ErrorVariant::ParsingError { positives, negatives } => {
                if !positives.is_empty() {
                    format!("expected {}", positives.iter()
                        .map(|r| format!("{:?}", r))
                        .collect::<Vec<_>>()
                        .join(" or "))
                } else if !negatives.is_empty() {
                    format!("unexpected {}", negatives.iter()
                        .map(|r| format!("{:?}", r))
                        .collect::<Vec<_>>()
                        .join(" or "))
                } else {
                    "syntax error".to_string()
                }
            }
            pest::error::ErrorVariant::CustomError { message } => message.clone(),
        };

        ParseError::Pest {
            message,
            span: Span { start, end, line, column },
            expected,
        }
    }

    /// Get the span for this error
    pub fn span(&self) -> Span {
        match self {
            ParseError::Syntax { span, .. } => *span,
            ParseError::UnknownCommand { span, .. } => *span,
            ParseError::UnknownRule { span, .. } => *span,
            ParseError::EmptyTarget { span } => *span,
            ParseError::InvalidRole { span, .. } => *span,
            ParseError::InvalidRelation { span, .. } => *span,
            ParseError::InvalidNumber { span, .. } => *span,
            ParseError::InvalidDuration { span, .. } => *span,
            ParseError::MissingDuration { span } => *span,
            ParseError::InvalidEscape { span, .. } => *span,
            ParseError::UnterminatedEscape { span } => *span,
            ParseError::InvalidKeyName { span, .. } => *span,
            ParseError::MissingArgument { span, .. } => *span,
            ParseError::NoCommand { span } => *span,
            ParseError::Pest { span, .. } => *span,
        }
    }

    /// Format error with source context
    pub fn format_with_source(&self, source: &str) -> String {
        let span = self.span();
        let line_text = source.lines().nth(span.line.saturating_sub(1)).unwrap_or("");
        let pointer = " ".repeat(span.column.saturating_sub(1)) + "^";
        let underline = if span.end > span.start {
            "~".repeat((span.end - span.start).min(line_text.len()))
        } else {
            "^".to_string()
        };

        format!(
            "{}\n  --> line {}:{}\n   |\n{:>3}| {}\n   | {}{}",
            self,
            span.line,
            span.column,
            span.line,
            line_text,
            " ".repeat(span.column.saturating_sub(1)),
            underline
        )
    }
}

pub type ParseResult<T> = Result<T, ParseError>;
```

### 6.2 Error Examples

```
syntax error at line 1, column 7: expected target
  --> line 1:7
   |
  1| click
   |       ^

unknown command: `clck`
  --> line 1:1
   |
  1| clck 5
   | ~~~~

invalid duration: `10` (expected format: 5s, 100ms, 2m)
  --> line 1:25
   |
  1| click 5 --timeout 10
   |                   ~~
```

---

## 7. Testing Strategy

### 7.1 Test Categories

| Category | Description | Count |
|----------|-------------|-------|
| **Vector Tests** | YAML test vectors from spec | 343 |
| **Unit Tests** | Per-rule, per-builder | ~200 |
| **Roundtrip Tests** | AST → serialize → parse → compare | ~100 |
| **Error Tests** | Error message quality | ~50 |
| **Fuzz Tests** | Random input generation | Continuous |
| **Property Tests** | Invariant checking | ~30 |

### 7.2 Test Vector Runner

```rust
// tests/vectors.rs

use oryn_parser::{parse_command, Command};
use serde::Deserialize;
use std::fs;

#[derive(Deserialize)]
struct TestSuite {
    metadata: Metadata,
    vectors: Vec<TestVector>,
}

#[derive(Deserialize)]
struct Metadata {
    description: String,
    version: String,
}

#[derive(Deserialize)]
struct TestVector {
    id: String,
    raw: String,
    canonical: String,
    #[serde(default)]
    expect: Expectation,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum Expectation {
    #[default]
    Ok,
    Error,
}

#[test]
fn test_all_vectors() {
    let yaml = fs::read_to_string("tests/fixtures/oil-test-vectors.v1.8.1.yaml")
        .expect("Failed to read test vectors");
    
    let suites: std::collections::HashMap<String, TestSuite> = 
        serde_yaml::from_str(&yaml).expect("Failed to parse YAML");
    
    let mut passed = 0;
    let mut failed = 0;
    let mut failures = Vec::new();
    
    for (suite_name, suite) in &suites {
        for vector in &suite.vectors {
            let result = parse_command(&vector.canonical);
            
            let ok = match (&vector.expect, &result) {
                (Expectation::Ok, Ok(_)) => true,
                (Expectation::Error, Err(_)) => true,
                _ => false,
            };
            
            if ok {
                passed += 1;
            } else {
                failed += 1;
                failures.push(format!(
                    "[{}::{}] canonical: `{}`, expected: {:?}, got: {:?}",
                    suite_name, vector.id, vector.canonical,
                    vector.expect, result
                ));
            }
        }
    }
    
    if !failures.is_empty() {
        for f in &failures[..failures.len().min(20)] {
            eprintln!("{}", f);
        }
        if failures.len() > 20 {
            eprintln!("... and {} more", failures.len() - 20);
        }
    }
    
    assert_eq!(failed, 0, "{}/{} vectors failed", failed, passed + failed);
    println!("✓ All {} test vectors passed", passed);
}
```

### 7.3 Roundtrip Tests

```rust
// tests/roundtrip.rs

use oryn_parser::{parse_command, Command};

/// Serialize AST back to canonical OIL text
fn serialize(cmd: &Command) -> String {
    match cmd {
        Command::Click(c) => {
            let mut s = format!("click {}", serialize_target(&c.target));
            if c.double { s.push_str(" --double"); }
            if c.right { s.push_str(" --right"); }
            if c.middle { s.push_str(" --middle"); }
            if c.force { s.push_str(" --force"); }
            if c.ctrl { s.push_str(" --ctrl"); }
            if c.shift { s.push_str(" --shift"); }
            if c.alt { s.push_str(" --alt"); }
            if let Some(t) = &c.timeout {
                s.push_str(&format!(" --timeout {}ms", t.millis));
            }
            s
        }
        Command::GoTo(c) => {
            let mut s = format!("goto \"{}\"", c.url.value.escape_default());
            if let Some(h) = &c.headers {
                s.push_str(&format!(" --headers \"{}\"", h.value.escape_default()));
            }
            if let Some(t) = &c.timeout {
                s.push_str(&format!(" --timeout {}ms", t.millis));
            }
            s
        }
        // ... other commands
        _ => format!("{:?}", cmd), // Fallback for incomplete serialization
    }
}

#[test]
fn roundtrip_navigation() {
    let cases = [
        "goto example.com",
        "goto \"https://example.com/path?q=1\"",
        "goto example.com --timeout 10s",
        "back",
        "forward",
        "refresh",
        "refresh --hard",
        "url",
    ];
    
    for input in cases {
        let cmd = parse_command(input).expect(&format!("parse failed: {}", input));
        let output = serialize(&cmd);
        let reparsed = parse_command(&output).expect(&format!("reparse failed: {}", output));
        
        // Compare AST (ignoring spans)
        let cmd_debug = format!("{:?}", cmd);
        let reparsed_debug = format!("{:?}", reparsed);
        
        // Remove span info for comparison
        let normalize = |s: &str| {
            s.split("span:").map(|p| p.split_whitespace().next().unwrap_or(""))
             .collect::<Vec<_>>().join(" ")
        };
        
        assert_eq!(
            normalize(&cmd_debug),
            normalize(&reparsed_debug),
            "roundtrip failed: {} → {} → {:?}",
            input, output, reparsed
        );
    }
}
```

---

## 8. Implementation Phases

### Phase 1: Foundation (Week 1) — 15 hours

| Task | Description | Hours |
|------|-------------|-------|
| 1.1 | Create `oryn-parser` crate, configure Cargo.toml | 1 |
| 1.2 | Copy Pest grammar, apply function_key fix | 1 |
| 1.3 | Define `Span` type with Pest integration | 2 |
| 1.4 | Define `ParseError` types | 3 |
| 1.5 | Implement primitive builders (string, number, duration) | 4 |
| 1.6 | Set up test harness, CI integration | 2 |
| 1.7 | Write unit tests for primitives | 2 |

**Milestone**: Parse `"hello"`, `42`, `10s`, `3.14`

### Phase 2: Targets & Navigation (Week 2) — 18 hours

| Task | Description | Hours |
|------|-------------|-------|
| 2.1 | Define `Target`, `TargetAtom`, `Relation` types | 2 |
| 2.2 | Implement target builder with all variants | 4 |
| 2.3 | Define navigation command structs | 2 |
| 2.4 | Implement navigation builders (goto, back, forward, refresh, url) | 3 |
| 2.5 | Define observation command structs | 2 |
| 2.6 | Implement observation builders | 3 |
| 2.7 | Write tests for targets and navigation | 2 |

**Milestone**: Parse `click 5`, `goto example.com`, `observe --full`

### Phase 3: Actions (Week 3) — 20 hours

| Task | Description | Hours |
|------|-------------|-------|
| 3.1 | Define action command structs (click, type, scroll, etc.) | 4 |
| 3.2 | Implement click builder with all options | 2 |
| 3.3 | Implement type builder with all options | 2 |
| 3.4 | Implement scroll builder | 2 |
| 3.5 | Implement press, keydown, keyup builders | 3 |
| 3.6 | Implement remaining action builders | 4 |
| 3.7 | Write tests for all action commands | 3 |

**Milestone**: Parse all 14 action commands

### Phase 4: Wait & Session (Week 4) — 18 hours

| Task | Description | Hours |
|------|-------------|-------|
| 4.1 | Define wait condition types | 2 |
| 4.2 | Implement wait builder with all conditions | 4 |
| 4.3 | Define session command structs | 3 |
| 4.4 | Implement session builders (cookies, storage, state, headers) | 4 |
| 4.5 | Implement tab builders | 2 |
| 4.6 | Write tests for wait and session | 3 |

**Milestone**: Parse `wait visible 5`, `storage set "key" "value"`

### Phase 5: Intents & Packs (Week 5) — 16 hours

| Task | Description | Hours |
|------|-------------|-------|
| 5.1 | Define intent command structs | 2 |
| 5.2 | Implement intent builders (login, search, dismiss) | 4 |
| 5.3 | Implement scroll_until builder | 2 |
| 5.4 | Define pack management command structs | 2 |
| 5.5 | Implement pack builders (define, export, run) | 3 |
| 5.6 | Write tests for intents and packs | 3 |

**Milestone**: Parse `login "user" "pass"`, `define my_intent:`

### Phase 6: Network & Console (Week 6) — 14 hours

| Task | Description | Hours |
|------|-------------|-------|
| 6.1 | Define network command structs | 2 |
| 6.2 | Implement intercept builder | 3 |
| 6.3 | Implement requests builder | 2 |
| 6.4 | Define console/errors command structs | 2 |
| 6.5 | Implement console/errors builders | 2 |
| 6.6 | Write tests | 3 |

**Milestone**: Parse `intercept "*.js" --block`, `console --level error`

### Phase 7: Remaining Commands (Week 7) — 16 hours

| Task | Description | Hours |
|------|-------------|-------|
| 7.1 | Implement frame builders | 2 |
| 7.2 | Implement dialog builder | 2 |
| 7.3 | Implement viewport/device/media builders | 3 |
| 7.4 | Implement recording builders (trace, record, highlight) | 3 |
| 7.5 | Implement utility builders (pdf, learn, exit, help) | 3 |
| 7.6 | Write tests for remaining commands | 3 |

**Milestone**: 100% command coverage

### Phase 8: Polish & Integration (Week 8) — 20 hours

| Task | Description | Hours |
|------|-------------|-------|
| 8.1 | Run full test vector suite, fix failures | 6 |
| 8.2 | Implement error message improvements | 3 |
| 8.3 | Add command suggestions for typos | 2 |
| 8.4 | Performance benchmarks and optimization | 3 |
| 8.5 | Integration with oryn-core REPL | 3 |
| 8.6 | Documentation and examples | 3 |

**Milestone**: Production-ready parser

### Total Estimated Effort

| Phase | Weeks | Hours |
|-------|-------|-------|
| Foundation | 1 | 15 |
| Targets & Navigation | 1 | 18 |
| Actions | 1 | 20 |
| Wait & Session | 1 | 18 |
| Intents & Packs | 1 | 16 |
| Network & Console | 1 | 14 |
| Remaining Commands | 1 | 16 |
| Polish & Integration | 1 | 20 |
| **Total** | **8 weeks** | **137 hours** |

---

## 9. Integration Points

### 9.1 REPL Integration

```rust
// oryn-core/src/repl.rs

use oryn_parser::{parse_command, Command, ParseError};

impl Repl {
    pub async fn handle_input(&mut self, input: &str) -> Result<Response, Error> {
        // 1. Normalize (Phase 1 - separate module)
        let canonical = self.normalizer.normalize(input)?;
        
        // 2. Parse to AST
        let command = parse_command(&canonical).map_err(|e| {
            Error::Parse(oryn_parser::format_error(&e, &canonical))
        })?;
        
        // 3. Resolve semantic targets
        let resolved = self.resolver.resolve(&command, &self.element_map)?;
        
        // 4. Translate to scanner protocol
        let scanner_cmd = self.translator.translate(&resolved)?;
        
        // 5. Execute via backend
        let result = self.backend.execute(scanner_cmd).await?;
        
        // 6. Format response
        self.formatter.format(result)
    }
}
```

### 9.2 Resolver Integration

```rust
// oryn-core/src/resolver.rs

use oryn_parser::ast::{Target, TargetAtom, TargetRole, Relation};

impl Resolver {
    pub fn resolve_target(
        &self, 
        target: &Target, 
        elements: &ElementMap
    ) -> Result<ElementId, ResolveError> {
        // Resolve primary target
        let mut id = match &target.primary {
            TargetAtom::Id(n) => {
                elements.get(*n).ok_or(ResolveError::ElementNotFound(*n))?
            }
            TargetAtom::Role(role) => {
                self.find_by_role(role, elements)?
            }
            TargetAtom::Text(s) => {
                self.find_by_text(&s.value, elements)?
            }
            TargetAtom::Css(s) => {
                self.find_by_css(&s.value, elements)?
            }
            TargetAtom::XPath(s) => {
                self.find_by_xpath(&s.value, elements)?
            }
        };
        
        // Apply relational filters
        for (relation, atom) in &target.relations {
            id = self.apply_relation(id, relation, atom, elements)?;
        }
        
        Ok(id)
    }
}
```

### 9.3 Translator Integration

```rust
// oryn-core/src/translator.rs

use oryn_parser::Command;

impl Translator {
    pub fn translate(&self, command: &Command) -> Result<ScannerRequest, TranslateError> {
        match command {
            Command::Click(cmd) => {
                Ok(ScannerRequest::Click {
                    id: cmd.resolved_id,
                    button: if cmd.right { "right" } else if cmd.middle { "middle" } else { "left" },
                    click_count: if cmd.double { 2 } else { 1 },
                    modifiers: self.build_modifiers(cmd.ctrl, cmd.shift, cmd.alt),
                    force: cmd.force,
                })
            }
            Command::Type(cmd) => {
                Ok(ScannerRequest::Type {
                    id: cmd.resolved_id,
                    text: cmd.text.value.clone(),
                    clear: !cmd.append,
                    delay: cmd.delay.as_ref().map(|n| n.value as u64),
                })
            }
            // ... other commands
            _ => todo!(),
        }
    }
}
```

---

## 10. Appendices

### 10.1 Command Coverage Matrix

| Category | Commands | Count |
|----------|----------|-------|
| Navigation | goto, back, forward, refresh, url | 5 |
| Observation | observe, html, text, title, screenshot, box | 6 |
| Actions | click, type, clear, press, keydown, keyup, keys, select, check, uncheck, hover, focus, scroll, submit | 14 |
| Wait | wait (11 conditions) | 1 |
| Extraction | extract (6 types) | 1 |
| Session | cookies, storage, sessions, session, state, headers | 6 |
| Tabs | tabs, tab | 2 |
| Intents | login, search, dismiss, accept_cookies, scroll until | 5 |
| Pack Management | packs, pack, intents, define, undefine, export, run | 7 |
| Network | intercept, requests | 2 |
| Console/Errors | console, errors | 2 |
| Frames | frames, frame | 2 |
| Dialog | dialog | 1 |
| Viewport/Device | viewport, device, devices, media | 4 |
| Recording | trace, record, highlight | 3 |
| Utility | pdf, learn, exit, help | 4 |
| **Total** | | **65** |

### 10.2 Dependencies

```toml
# Cargo.toml

[package]
name = "oryn-parser"
version = "0.1.0"
edition = "2021"
description = "OIL (Oryn Intent Language) parser"

[dependencies]
pest = "2.7"
pest_derive = "2.7"
thiserror = "1.0"
serde = { version = "1.0", features = ["derive"] }

[dev-dependencies]
serde_yaml = "0.9"
proptest = "1.4"
criterion = "0.5"

[[bench]]
name = "parser"
harness = false
```

### 10.3 Performance Targets

| Input | Target | Measurement |
|-------|--------|-------------|
| Simple command (`click 5`) | <100μs | p99 latency |
| Complex command (5 options) | <500μs | p99 latency |
| Very complex (nested targets) | <1ms | p99 latency |
| 100-line script | <10ms | total time |
| Memory per command | <2KB | heap allocation |

### 10.4 Glossary

| Term | Definition |
|------|------------|
| **AST** | Abstract Syntax Tree — typed representation of parsed input |
| **Canonical OIL** | Normalized OIL text (lowercase verbs, `--kebab-case` options) |
| **PEG** | Parsing Expression Grammar — formal grammar notation |
| **Pest** | Rust parsing library using PEG |
| **Span** | Source location (byte offset, line, column) |
| **Target** | Element reference (ID, role, text, CSS, XPath) |
| **Relation** | Target modifier (near, inside, after, before, contains) |

---

*Document Version: 1.0*
*Created: January 2026*
*Based on: OIL v1.8.1 Canonical Grammar*

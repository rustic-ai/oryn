# Oryn Intent Language - Command Reference

Extracted from SPEC-INTENT-LANGUAGE.md

## Implementation Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully implemented and wired end-to-end |
| ⚠️ | Partially implemented (see notes) |
| ❌ | Not implemented |

Pipeline stages: **Parser** → **Resolver** → **Translator** → **Backend/Scanner**

---

## Command Syntax

```
command [target] [arguments] [--options]
```

## Comments

Lines starting with `#` are comments and ignored by the parser:

```
# This is a comment
goto example.com  # Inline comment
```

---

## Navigation Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `goto` | Navigate to a URL (accepts full URLs, domain-only, or relative paths) | | ✅ |
| `back` | Navigate to previous page in history | | ✅ |
| `forward` | Navigate to next page in history | | ✅ |
| `refresh` | Reload the current page | `--hard` (clears cache) | ✅ |
| `url` | Return the current URL | | ✅ |

**Implementation Notes:**
- `goto`: Parser → REPL → `backend.navigate()` → CDP/Browser
- `back/forward/refresh`: Parser → REPL → Backend trait methods → CDP
- `url`: Parser → Translator → `Execute` JS (`window.location.href`)

---

## Observation Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `observe` | Scan and return interactive elements | `--full`, `--minimal`, `--near "text"`, `--viewport`, `--hidden` | ✅ |
| `html` | Get raw HTML content | `--selector` | ✅ |
| `text` | Get text content of page or element | `--selector` | ✅ |
| `title` | Get page title | | ✅ |
| `screenshot` | Capture visual representation | file output, format selection, element-specific capture | ✅ |

**Implementation Notes:**
- `observe`: Parser → Translator (`ScanRequest`) → Scanner.scan() with pattern detection
- `text`: Parser → Translator (`GetTextRequest`) → Scanner.get_text()
- `html`: Parser → Translator (`GetHtmlRequest`) → Scanner.get_html()
- `title`: Parser → Translator → `Execute` JS
- `screenshot`: Parser → REPL → `backend.screenshot()` → CDP

---

## Action Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `click` | Click an element | `--force`, `--double`, `--right`, `--middle` | ✅ |
| `type` | Enter text into an input | `--append`, `--enter`, `--delay` | ✅ |
| `clear` | Clear an input field | | ✅ |
| `press` | Press a keyboard key (supports modifiers like Control+A) | | ✅ |
| `select` | Choose from dropdown/select element (by value, text, or index) | | ✅ |
| `check` | Check a checkbox | | ✅ |
| `uncheck` | Uncheck a checkbox | | ✅ |
| `hover` | Move mouse over element (triggers hover states) | | ✅ |
| `focus` | Set keyboard focus to element | | ✅ |
| `scroll` | Scroll viewport or container (by direction, element, or page) | `--direction`, `--amount` | ✅ |

**Implementation Notes:**
- All action commands: Parser → Resolver (semantic → ID) → Translator → Scanner
- `press`: Parser → REPL → `backend.press_key()` → CDP `Input.dispatchKeyEvent`
- Resolver supports: ID, Text, Role, Selector, and relational targets (Near, Inside, After, Before)

---

## Wait Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `wait load` | Wait for page load complete | `--timeout` | ✅ |
| `wait idle` | Wait for network idle | `--timeout` | ✅ |
| `wait visible <target>` | Wait for element visibility | `--timeout` | ✅ |
| `wait hidden <target>` | Wait for element to hide | `--timeout` | ✅ |
| `wait exists <selector>` | Wait for element in DOM | `--timeout` | ✅ |
| `wait gone <selector>` | Wait for element removal | `--timeout` | ✅ |
| `wait url <pattern>` | Wait for URL match | `--timeout` | ✅ |

**Implementation Notes:**
- Parser → Translator (`WaitRequest`) → Scanner.wait_for() with polling

---

## Data Extraction Commands

| Command | Description | Status |
|---------|-------------|--------|
| `extract links` | Extract all hyperlinks | ✅ |
| `extract images` | Extract all images with src/alt | ✅ |
| `extract tables` | Extract table data as structured output | ✅ |
| `extract css(<selector>)` | Custom element extraction | ✅ |
| `extract meta` | Extract page metadata | ✅ |
| `extract text` | Alias for `text` command (supports `--selector`) | ✅ |

**Implementation Notes:**
- Parser → Translator (`ExtractRequest`) → Scanner.extract()
- `extract text`: Parser redirects to `Command::Text` → Translator (`GetTextRequest`) → Scanner.get_text()

---

## Session Commands

| Command | Description | Status |
|---------|-------------|--------|
| `cookies list` | Show all cookies | ✅ |
| `cookies get <name>` | Get specific cookie | ⚠️ |
| `cookies set <name> <value>` | Set cookie | ⚠️ |
| `cookies delete <name>` | Remove cookie | ⚠️ |
| `storage get` | Get localStorage/sessionStorage value | ✅ |
| `storage set` | Set localStorage/sessionStorage value | ✅ |
| `storage list` | List storage keys | ✅ |
| `storage clear` | Clear storage | ✅ |

**Implementation Notes:**
- `cookies list`: Parser → REPL → `backend.get_cookies()` → CDP
- `cookies get/set/delete`: Parser ✅, REPL only calls `get_cookies()` (partial wiring)
- `storage *`: Parser → Translator → `Execute` JS

---

## Tab Commands

| Command | Description | Status |
|---------|-------------|--------|
| `tabs` | List open tabs | ✅ |
| `tab new <url>` | Open new tab | ⚠️ |
| `tab switch <id>` | Switch to tab | ⚠️ |
| `tab close <id>` | Close tab | ⚠️ |

**Implementation Notes:**
- `tabs`: Parser → REPL → `backend.get_tabs()` → CDP
- `tab new/switch/close`: Parser ✅, but REPL/Backend methods not wired

---

## Intent Commands (Level 3)

High-level intents that execute multiple actions:

| Command | Description | Status |
|---------|-------------|--------|
| `login <email> <password>` | Find credentials fields, type values, submit form, wait for navigation | ✅ |
| `search <query>` | Find search input, type query, submit | ✅ |
| `dismiss <target>` | Close overlays matching the target (popups, modals, modal, banner, or any string) | ✅ |
| `accept cookies` | Find and click cookie consent | ✅ |
| `scroll until <target>` | Scroll until element is visible | ✅ |

**dismiss examples:**
- `dismiss popups` — Close all detected popups
- `dismiss modals` — Close modal dialogs
- `dismiss "modal"` — Close element matching "modal"
- `dismiss banner` — Close banner overlays

**Implementation Notes:**
- `login/search/dismiss/accept`: Parser → Translator → Scanner (uses pattern detection)
- `scroll until`: Parser → REPL custom loop (scroll + scan + resolve until found)

---

## Pack & Intent Management Commands

| Command | Description | Status |
|---------|-------------|--------|
| `packs` | List loaded intent packs | ✅ |
| `pack load <name>` | Load an intent pack | ✅ |
| `pack unload <name>` | Unload an intent pack | ✅ |
| `intents` | List all registered intents | ✅ |
| `intents session` | List session-defined intents | ✅ |
| `define <body>` | Define a new session intent | ✅ |
| `undefine <name>` | Remove a session intent | ✅ |
| `export <name> <path>` | Export intent to file | ✅ |
| `run <intent> [params]` | Execute a registered intent | ✅ |

**Implementation Notes:**
- All handled in REPL via `PackManager` and `SessionIntentManager`

---

## Additional Commands

| Command | Description | Status |
|---------|-------------|--------|
| `pdf <path>` | Generate PDF of current page | ✅ |
| `submit <target>` | Submit a form | ✅ |
| `learn status` | Show learning status for current domain | ✅ |
| `learn save <name>` | Save proposed intent | ✅ |

**Implementation Notes:**
- `pdf`: Parser → REPL → `backend.pdf()` → CDP
- `submit`: Parser → Resolver → Translator → Scanner.submit()
- `learn`: Parser → REPL → Observer/Recognizer/Proposer modules

---

## Target Resolution Methods

| Method | Example | Description | Status |
|--------|---------|-------------|--------|
| ID | `click 5` | Target element by numbered label [5] | ✅ |
| Text | `click "Sign in"` | Match by visible/accessible text | ✅ |
| Role | `type email "user@test.com"` | Reference by semantic role | ✅ |
| Selector | `click css(".btn-primary")` | Explicit CSS selector | ✅ |
| Near | `click "Add" near "Product"` | Relational targeting | ✅ |
| Inside | `click "Submit" inside "Form"` | Container-scoped targeting | ✅ |

**Implementation Notes:**
- Resolver (`resolver.rs`) converts semantic targets to element IDs using scan results
- Requires `observe` to be run first to populate resolver context

---

## Reserved Words

**Roles:** email, password, search, submit, username, phone, url

**Directions:** up, down, left, right, top, bottom

**Conditions:** visible, hidden, exists, gone, idle, load

**Modifiers:** near, after, before, inside, contains

**Key Names:** Enter, Tab, Escape, Space, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, F1-F12, Control, Shift, Alt, Meta

---

## Pipeline Summary

```
┌─────────┐    ┌──────────┐    ┌────────────┐    ┌─────────┐    ┌─────────┐
│ Parser  │───►│ Resolver │───►│ Translator │───►│ Backend │───►│ Scanner │
│         │    │          │    │            │    │  Trait  │    │   (JS)  │
└─────────┘    └──────────┘    └────────────┘    └─────────┘    └─────────┘
     │              │                │                │              │
     │         Semantic→ID      Command→Request   Direct calls   Execute
     │         resolution       translation       (nav, keys)    commands
     │                                                │
     └─────────────────────────────────────────────────┘
                    Some commands bypass translator
                    and go directly to backend methods
```

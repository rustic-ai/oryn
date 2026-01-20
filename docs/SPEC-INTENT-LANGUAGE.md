# Oryn Intent Language Specification

## Version 1.0

---

## 1. Overview

The Oryn Intent Language (LIL) is a token-efficient, human-readable protocol designed specifically for AI agents to control web browsers. Unlike traditional approaches that force agents to interpret screenshots, parse raw HTML, or construct complex function calls, LIL provides a semantic abstraction layer that speaks the language of web interaction.

### 1.1 Design Philosophy

LIL is built on four foundational principles:

**Forgiving Syntax**
Multiple syntactic variations are accepted for the same action. The parser prioritizes understanding agent intent over enforcing rigid formatting rules. This dramatically reduces failed commands due to minor syntax variations.

**Semantic Targeting**  
Agents can reference elements by meaning rather than implementation details. Instead of hunting for CSS selectors or XPath expressions, agents simply say "click login" or "type email" and let Oryn resolve the target.

**Token Efficiency**
Every character counts in an agent's context window. LIL minimizes verbosity while maximizing expressiveness, allowing agents to accomplish more within their token budgets.

**Human Readability**
Commands and responses are designed to be immediately comprehensible to human operators, facilitating debugging, auditing, and collaborative development.

### 1.2 What LIL Is Not

LIL deliberately rejects several common paradigms:

- **Not JSON**: JSON is verbose and error-prone for language models. Bracket matching, quote escaping, and strict formatting requirements create unnecessary failure modes.

- **Not Screenshot-Based**: Agents should not need computer vision to understand a web page. Oryn provides structured observations that convey semantic meaning directly.

- **Not HTML Parsing**: Raw HTML is designed for browsers, not agents. Oryn abstracts away the complexity of DOM structure, presenting only the interactive elements that matter.

- **Not Function Calls**: Complex tool schemas with rigid type requirements create friction. LIL's natural language-inspired syntax reduces cognitive load on agents.

### 1.3 Protocol Flow

The interaction model follows a simple request-response pattern:

**Agent to Oryn:**
The agent generates commands in plain text. These might be single commands or batched sequences. Commands express intent at the appropriate level of abstraction.

**Oryn Processing:**
Commands are parsed with forgiveness, targets are resolved semantically, actions are executed against the browser backend, and responses are formatted consistently.

**Oryn to Agent:**
Structured text responses provide the information agents need to make decisions. Changes are clearly marked, errors include recovery hints, and observations present a digestible view of page state.

---

## 2. Command Syntax

### 2.1 General Format

Commands follow a consistent structure:

```
command [target] [arguments] [--options]
```

**Command**: The action to perform (case-insensitive)

**Target**: Element identifier—can be an ID number, semantic reference, text match, or selector

**Arguments**: Additional parameters, with strings enclosed in quotes

**Options**: Flags prefixed with `--` that modify behavior

### 2.2 Target Resolution

Oryn supports multiple targeting strategies, automatically resolving the most specific match:

**ID Targeting**
Direct reference by numbered label from observations. Example: `click 5` targets the element labeled [5].

**Text Targeting**
Match elements by visible or accessible text. Example: `click "Sign in"` finds elements containing that text.

**Role Targeting**
Reference by semantic role. Example: `type email "user@test.com"` finds the email input field.

**Selector Targeting**
Explicit CSS or XPath for edge cases. Example: `click css(".btn-primary")` uses CSS selection.

### 2.3 String Handling

Strings can use single or double quotes interchangeably. Single words without spaces may omit quotes entirely. Standard escape sequences are supported for including quotes within strings.

### 2.4 Comments

Lines beginning with `#` are treated as comments and ignored by the parser. Comments can also appear after commands on the same line.

```
# This is a comment
goto example.com  # Navigate to example
observe           # Check page state
```

---

## 3. Command Reference

### 3.1 Navigation Commands

**goto** — Navigate to a URL
- Accepts full URLs, domain-only (https implied), or relative paths
- Waits for page load before completing

**back** — Navigate to previous page in history

**forward** — Navigate to next page in history

**refresh** — Reload the current page
- `--hard` option clears cache

**url** — Return the current URL

### 3.2 Observation Commands

**observe** — Scan and return interactive elements

This is the primary command for understanding page state. Observations include:
- Page location and title
- All interactive elements with numbered labels
- Element types, roles, and states
- Detected patterns (login forms, search boxes, etc.)

Verbosity options:
- Default (compact): Essential information for decision-making
- `--full`: Includes selectors, positions, and detailed attributes
- `--minimal`: Just counts for quick status checks
- `--near "text"`: Filter to elements near specific content

**html** — Get raw HTML content
- Use sparingly; prefer `observe` for most tasks
- Supports `--selector` to extract specific portions

**text** — Get text content of page or element

**title** — Get page title

**screenshot** — Capture visual representation
- Supports file output, format selection, and element-specific capture
- Useful for verification and debugging rather than primary navigation

### 3.3 Action Commands

**click** — Click an element
- Supports double-click, right-click, middle-click via options
- `--force` option clicks even if element is obscured

**type** — Enter text into an input
- `--append` to add without clearing
- `--enter` to submit after typing
- `--delay` for character-by-character timing

**clear** — Clear an input field

**press** — Press a keyboard key
- Supports all standard keys (Enter, Tab, Escape, Arrow keys, etc.)
- Supports modifier combinations (Control+A, Shift+Tab, etc.)

**select** — Choose from dropdown/select element
- By value, text, or index

**check** / **uncheck** — Toggle checkbox state

**hover** — Move mouse over element (triggers hover states)

**focus** — Set keyboard focus to element

**scroll** — Scroll the viewport or specific container
- By direction and amount
- By element reference (scroll element into view)
- By page increments

### 3.4 Wait Commands

**wait** — Pause for conditions

Supported conditions:
- `load` — Wait for page load complete
- `idle` — Wait for network idle
- `visible <target>` — Wait for element visibility
- `hidden <target>` — Wait for element to hide
- `exists <selector>` — Wait for element in DOM
- `gone <selector>` — Wait for element removal
- `url <pattern>` — Wait for URL match

Timeout configurable via `--timeout` option.

### 3.5 Data Extraction Commands

**extract** — Pull structured data from page
- `links` — All hyperlinks
- `images` — All images with src/alt
- `tables` — Table data as structured output
- `css(<selector>)` — Custom element extraction
- `meta` — Page metadata
- `text` — Alias for the `text` command (supports `--selector`)

### 3.6 Session Commands

**cookies** — Read or manage cookies
- `list` — Show all cookies
- `get <name>` — Get specific cookie
- `set <name> <value>` — Set cookie
- `delete <name>` — Remove cookie

**storage** — Manage localStorage/sessionStorage

### 3.7 Tab Commands

**tabs** — List open tabs

**tab** — Switch to, open, or close tabs
- `new <url>` — Open new tab
- `switch <id>` — Switch to tab
- `close <id>` — Close tab

---

## 4. Response Format

### 4.1 Success Responses

Successful commands return confirmation followed by relevant data:

```
ok <command> [details]

[response data]
```

### 4.2 Observation Format

Observations use a structured notation:

**Page Header**
```
@ domain.com/path "Page Title"
```

**Element Notation**
```
[id] type/role "text" {modifiers}
```

Elements include:
- Numbered ID for targeting
- Type (input, button, link, select, checkbox, etc.)
- Role (email, password, submit, search, etc.)
- Visible or accessible text
- State modifiers (required, disabled, checked, primary, etc.)

**Pattern Detection**
Observations include detected UI patterns:
- Login forms with identified fields
- Search forms
- Pagination controls
- Cookie consent banners
- Modal dialogs

### 4.3 Error Responses

Errors provide actionable information:

```
error <command>: <message>

# hint
<recovery suggestion>
```

Error categories:
- Element not found (with available IDs listed)
- Element not visible/disabled/interactable
- Navigation failures
- Timeout exceeded
- Invalid syntax (with correction suggestions)

### 4.4 Change Notation

After actions that modify page state, responses indicate changes:

| Symbol | Meaning |
|--------|---------|
| `+` | Element appeared |
| `-` | Element disappeared |
| `~` | Element changed |
| `@` | Page/URL changed |

---

## 5. Multi-Level Abstraction

LIL supports commands at different levels of abstraction, allowing agents to operate at whatever level suits their task:

### Level 1: Direct Commands

Operate on element IDs with no ambiguity:
```
click 5
type 3 "hello"
check 7
```

### Level 2: Semantic Commands

Operate on roles or text; Oryn resolves to IDs:
```
click "Sign in"
type email "user@test.com"
check "Remember me"
```

### Level 3: Intent Commands

High-level intents that execute multiple actions:
```
login "user@test.com" "password123"
search "rust programming"
dismiss popups
accept cookies
scroll until "Load more"
```

Intent commands encapsulate common workflows:
- **login** — Finds credentials fields, types values, submits form, waits for navigation
- **search** — Finds search input, types query, submits
- **dismiss `<target>`** — Closes overlays matching the target. Accepts: `popups`, `modals`, `modal`, `banner`, or any descriptive string. Examples: `dismiss popups`, `dismiss "modal"`, `dismiss modals`
- **accept cookies** — Finds and clicks cookie consent

### Level 4: Goal Commands (LLM/Agent Layer)

Natural language goals for agent-driven planning:
```
goal: add "Blue T-Shirt Size M" to cart
goal: find the contact email on this page
goal: subscribe to the newsletter
```

> **Note**: Goal commands are not part of the intent engine. They belong in the LLM/agent orchestration layer that sits above the intent engine. The intent engine provides deterministic execution of well-defined intents; an LLM agent would use these intents as tools to accomplish higher-level goals.

---

## 6. Parser Behavior

### 6.1 Forgiving Parsing

The parser accepts reasonable variations:

**Case Insensitivity**
Commands are case-insensitive. `click`, `CLICK`, and `Click` are equivalent.

**Quote Flexibility**
Single and double quotes are interchangeable.

**Command Aliases**
Common synonyms are accepted: `goto` and `navigate` and `go to` all work.

**Option Flexibility**
Options can use `--option`, `-option`, or just `option` after the command.

### 6.2 Error Recovery

When parsing fails, Oryn provides helpful corrections:

- Unknown commands suggest similar valid commands
- Unterminated strings indicate where quotes should be added
- Missing required arguments show usage patterns
- Invalid targets explain available targeting methods

---

## 7. Configuration

### 7.1 Defaults

| Setting | Default | Description |
|---------|---------|-------------|
| `timeout` | 30s | Command timeout |
| `verbosity` | compact | Observation detail level |
| `auto_wait` | true | Automatic wait after navigation |
| `screenshot_format` | png | Default screenshot format |
| `typing_delay` | 0ms | Delay between keystrokes |

### 7.2 Per-Command Options

Most commands accept `--timeout` to override the default:
```
click 5 --timeout 10s
wait visible 5 --timeout 60s
goto example.com --timeout 45s
```

---

## 8. Reserved Words

### Roles
email, password, search, submit, username, phone, url

### Directions
up, down, left, right, top, bottom

### Conditions
visible, hidden, exists, gone, idle, load

### Modifiers
near, after, before, inside, contains

### Key Names
Enter, Tab, Escape, Space, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, F1-F12, Control, Shift, Alt, Meta

---

*Document Version: 1.1*
*Last Updated: January 2026*

# Oryn Intent Language Specification

## Version 1.1

---

## 1. Overview

The Oryn Intent Language (OIL) is a token-efficient, human-readable protocol designed specifically for AI agents to control web browsers. Unlike traditional approaches that force agents to interpret screenshots, parse raw HTML, or construct complex function calls, OIL provides a semantic abstraction layer that speaks the language of web interaction.

### 1.1 Design Philosophy

OIL is built on four foundational principles:

**Forgiving Syntax**
Multiple syntactic variations are accepted for the same action. The parser prioritizes understanding agent intent over enforcing rigid formatting rules. This dramatically reduces failed commands due to minor syntax variations.

**Semantic Targeting**  
Agents can reference elements by meaning rather than implementation details. Instead of hunting for CSS selectors or XPath expressions, agents simply say "click login" or "type email" and let Oryn resolve the target.

**Token Efficiency**
Every character counts in an agent's context window. OIL minimizes verbosity while maximizing expressiveness, allowing agents to accomplish more within their token budgets.

**Human Readability**
Commands and responses are designed to be immediately comprehensible to human operators, facilitating debugging, auditing, and collaborative development.

### 1.2 What OIL Is Not

OIL deliberately rejects several common paradigms:

- **Not JSON**: JSON is verbose and error-prone for language models. Bracket matching, quote escaping, and strict formatting requirements create unnecessary failure modes.

- **Not Screenshot-Based**: Agents should not need computer vision to understand a web page. Oryn provides structured observations that convey semantic meaning directly.

- **Not HTML Parsing**: Raw HTML is designed for browsers, not agents. Oryn abstracts away the complexity of DOM structure, presenting only the interactive elements that matter.

- **Not Function Calls**: Complex tool schemas with rigid type requirements create friction. OIL's natural language-inspired syntax reduces cognitive load on agents.

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
- Options: `--headers <json>` for custom request headers

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
- `--positions`: Include bounding box coordinates

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
- `--ctrl`, `--shift`, `--alt` for modifier key combinations

**type** — Enter text into an input
- `--append` to add without clearing
- `--enter` to submit after typing
- `--delay` for character-by-character timing

**clear** — Clear an input field

**press** — Press a keyboard key
- Supports all standard keys (Enter, Tab, Escape, Arrow keys, etc.)
- Supports modifier combinations (Control+A, Shift+Tab, etc.)

**keydown** — Hold a key down
- Use for modifier key sequences
- Example: `keydown Control`

**keyup** — Release a held key
- Example: `keyup Control`
- `keyup all` releases all held keys

**keys** — Show currently held keys

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
- `until "<js expression>"` — Wait for JavaScript expression to be truthy
- `ready` — Wait for common app-ready patterns
- `items "<selector>" <count>` — Wait for N elements matching selector

Timeout configurable via `--timeout` option.

**Examples**:
```
wait until "window.appReady === true"
wait until "document.querySelectorAll('.item').length >= 10"
wait until "!document.querySelector('.loading')" --timeout 30s
wait items ".card" 10
```

### 3.5 Data Extraction Commands

**extract** — Pull structured data from page
- `links` — All hyperlinks
- `images` — All images with src/alt
- `tables` — Table data as structured output
- `css(<selector>)` — Custom element extraction
- `meta` — Page metadata
- `text` — Alias for the `text` command (supports `--selector`)

**box** — Get bounding box of an element
```
box 5
box "Submit"
```

Response:
```
ok box 5

# bounds
x: 100
y: 200
width: 150
height: 40
visible: true
viewport: inside    # inside, partial, outside
```

### 3.6 Session Commands

**cookies** — Read or manage cookies
- `list` — Show all cookies
- `get <name>` — Get specific cookie
- `set <name> <value>` — Set cookie
- `delete <name>` — Remove cookie

**storage** — Manage localStorage/sessionStorage
- `get <key>` — Get value
- `set <key> <value>` — Set value
- `list` — List all keys
- `clear` — Clear all storage

### 3.7 Tab Commands

**tabs** — List open tabs

**tab** — Switch to, open, or close tabs
- `new <url>` — Open new tab
- `switch <id>` — Switch to tab
- `close <id>` — Close tab

### 3.8 Session Management Commands

**sessions** — List all active sessions

**Syntax**: `sessions`

**Response**:
```
ok sessions

# active sessions
- default (current)
- agent1
- agent2
```

**session** — Show or manage current session context

**Syntax**: 
- `session` — Show current session info
- `session <name>` — Switch context to named session

**Response**:
```
ok session

# session
name: agent1
mode: headless
started: 2026-01-24T10:30:00Z
pages: 3
url: https://example.com
```

**session new** — Create a new named session

**Syntax**: `session new <name> [--mode <mode>]`

**Options**:
- `--mode`: embedded, headless, or remote (default: current mode)

**session close** — Close a named session

**Syntax**: `session close <name>`

### 3.9 State Persistence Commands

**state save** — Save current authentication state to file

**Syntax**: `state save <path> [--options]`

**Options**:
- `--cookies-only`: Save only cookies, skip storage
- `--domain <domain>`: Filter to specific domain
- `--include-session`: Include sessionStorage (excluded by default)

**Saved Data**:
- Cookies (all, with full attributes)
- localStorage (all key-value pairs)
- sessionStorage (optional)

**Response**:
```
ok state save auth.json

# saved
cookies: 12
localStorage: 5 keys
sessionStorage: 2 keys
domain: github.com
```

**state load** — Restore authentication state from file

**Syntax**: `state load <path> [--options]`

**Options**:
- `--merge`: Add to existing state instead of replacing
- `--cookies-only`: Load only cookies

**Behavior**:
- Clears existing cookies/storage for affected domains (unless --merge)
- Validates state file format before applying
- Reports count of restored items

**State File Format** (JSON):
```json
{
  "version": 1,
  "created": "2026-01-24T10:30:00Z",
  "domain": "github.com",
  "cookies": [
    {
      "name": "session",
      "value": "abc123",
      "domain": ".github.com",
      "path": "/",
      "expires": 1737800000,
      "httpOnly": true,
      "secure": true,
      "sameSite": "Lax"
    }
  ],
  "localStorage": {
    "theme": "dark",
    "user_prefs": "{\"notifications\":true}"
  },
  "sessionStorage": {}
}
```

### 3.10 HTTP Headers Commands

**headers set** — Configure custom HTTP headers

**Syntax**: 
- `headers set <json>` — Global headers (all requests)
- `headers set <domain> <json>` — Domain-scoped headers

**Examples**:
```
headers set {"Authorization": "Bearer token", "X-Custom": "value"}
headers set example.com {"Authorization": "Bearer token"}
```

**Behavior**:
- Domain-scoped headers only sent to matching origins
- Global headers sent to all requests
- Domain-scoped takes precedence over global
- Headers persist for session duration

**headers** — View configured headers

**Syntax**:
- `headers` — Show all configured headers
- `headers <domain>` — Show headers for specific domain

**headers clear** — Remove configured headers

**Syntax**:
- `headers clear` — Clear all headers
- `headers clear <domain>` — Clear domain-specific headers

**Inline Header Option**:
Navigation commands accept `--headers` option:
```
goto api.example.com --headers {"Authorization": "Bearer token"}
```

### 3.11 Network Commands

**intercept** — Set up request interception rules

**Syntax**:
```
intercept "<url-pattern>"                           # Log matching requests
intercept "<url-pattern>" --block                   # Block matching requests
intercept "<url-pattern>" --respond <json>          # Mock response with JSON
intercept "<url-pattern>" --respond-file <path>     # Mock response from file
intercept "<url-pattern>" --status <code>           # Mock response with status
intercept clear                                      # Clear all rules
intercept clear "<url-pattern>"                     # Clear specific rule
```

**Examples**:
```
intercept "https://api.example.com/*"
intercept "https://analytics.com/*" --block
intercept "https://api.example.com/user" --respond {"name": "Test User"}
intercept "https://api.example.com/data" --status 404
```

**requests** — View captured network requests

**Syntax**:
```
requests                    # Show recent requests
requests --filter <text>    # Filter by URL
requests --method <method>  # Filter by HTTP method
requests --last <n>         # Show last N requests
```

**Response**:
```
ok requests

# captured (last 50)
1. GET https://api.example.com/user → 200 (45ms)
2. POST https://api.example.com/login → 200 (120ms)
3. GET https://analytics.com/track → BLOCKED
```

**Mode Availability**:

| Mode | Support |
|------|---------|
| oryn-h | Full (CDP Network domain) |
| oryn-e | Limited (WebDriver doesn't support well) |
| oryn-r | Partial (extension can intercept) |

### 3.12 Console & Error Commands

**console** — View browser console output

**Syntax**:
```
console                     # Show all console messages
console --level <level>     # Filter by level (log, warn, error)
console --filter "<text>"   # Filter by content
console --last <n>          # Show last N messages
console clear               # Clear console buffer
```

**Response**:
```
ok console

# messages (last 20)
[log] 10:30:01 Application initialized
[log] 10:30:02 User data loaded
[warn] 10:30:02 Deprecation: componentWillMount
[error] 10:30:05 Failed to fetch: /api/notifications
```

**errors** — View JavaScript errors

**Syntax**:
```
errors                # Show all errors
errors --last <n>     # Show last N errors
errors clear          # Clear error buffer
```

### 3.13 Frame Commands

**frames** — List all frames in the page

**Response**:
```
ok frames

# frames
- main (current)
- [1] iframe#widget src="https://widget.com/embed"
- [2] iframe.ad src="https://ads.com/banner"
```

**frame** — Switch frame context

**Syntax**:
```
frame "#iframe-id"    # Switch to iframe by selector
frame 3               # Switch to iframe by element ID
frame main            # Return to main frame
frame parent          # Go up one level (for nested iframes)
```

### 3.14 Dialog Commands

**dialog** — Handle browser dialogs (alert, confirm, prompt)

**Syntax**:
```
dialog accept                  # Accept dialog
dialog accept "input text"     # Accept prompt with text
dialog dismiss                 # Dismiss/cancel dialog
dialog auto accept             # Auto-accept all dialogs
dialog auto dismiss            # Auto-dismiss all dialogs
dialog auto off                # Manual handling (default)
```

### 3.15 Viewport & Device Commands

**viewport** — Set viewport size

**Syntax**: `viewport <width> <height>`

**Example**:
```
viewport 1920 1080
```

**device** — Emulate a device

**Syntax**:
```
device "<device-name>"    # Emulate named device
device reset              # Reset to defaults
```

**Examples**:
```
device "iPhone 14"
device "Pixel 7"
device "iPad Pro"
```

**devices** — List available device presets

**media** — Set media features

**Syntax**:
```
media color-scheme dark       # Set prefers-color-scheme
media color-scheme light
media reduced-motion reduce   # Set prefers-reduced-motion
media reset                   # Reset all media settings
```

### 3.16 Recording Commands

**trace** — Record execution trace (oryn-h only)

**Syntax**:
```
trace start                    # Start recording
trace start <path>             # Start with custom path
trace stop                     # Stop and save trace
trace stop <path>              # Stop and save to specific path
```

**Note**: Traces can be viewed with Playwright trace viewer.

**record** — Record video of session

**Syntax**:
```
record start <path>            # Start video recording
record start <path> --quality high
record stop                    # Stop recording
```

**highlight** — Highlight element for debugging

**Syntax**:
```
highlight <target>                    # Highlight element
highlight <target> --duration 3s      # With custom duration
highlight <target> --color red        # With custom color
highlight clear                       # Remove all highlights
```

**Note**: Most useful in oryn-r (Remote) where user sees browser.

### 3.17 PDF Generation

**pdf** — Generate PDF of current page

**Syntax**: `pdf <path>`

**Options**:
- `--format <size>`: Paper size (A4, Letter, etc.)
- `--landscape`: Landscape orientation
- `--margin <size>`: Page margins

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

**With Positions** (when `--positions` specified):
```
[1] button "Submit" {primary} @(100,200,150x40)
```

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

| Symbol | Meaning             |
| ------ | ------------------- |
| `+`    | Element appeared    |
| `-`    | Element disappeared |
| `~`    | Element changed     |
| `@`    | Page/URL changed    |

---

## 5. Multi-Level Abstraction

OIL supports commands at different levels of abstraction, allowing agents to operate at whatever level suits their task:

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
- **dismiss `<target>`** — Closes overlays matching the target. Accepts: `popups`, `modals`, `modal`, `banner`, or any descriptive string
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

| Setting             | Default | Description                     |
| ------------------- | ------- | ------------------------------- |
| `timeout`           | 30s     | Command timeout                 |
| `verbosity`         | compact | Observation detail level        |
| `auto_wait`         | true    | Automatic wait after navigation |
| `screenshot_format` | png     | Default screenshot format       |
| `typing_delay`      | 0ms     | Delay between keystrokes        |

### 7.2 Per-Command Options

Most commands accept `--timeout` to override the default:
```
click 5 --timeout 10s
wait visible 5 --timeout 60s
goto example.com --timeout 45s
```

### 7.3 Session Configuration

Sessions can be configured at startup:

```bash
oryn headless --session agent1
```

Environment variable alternative:
```bash
ORYN_SESSION=agent1 oryn headless
```

---

## 8. Reserved Words

### Roles
email, password, search, submit, username, phone, url

### Directions
up, down, left, right, top, bottom

### Conditions
visible, hidden, exists, gone, idle, load, until, ready

### Modifiers
near, after, before, inside, contains

### Key Names
Enter, Tab, Escape, Space, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, F1-F12, Control, Shift, Alt, Meta

### Session Commands
sessions, session, state

### Network Commands
headers, intercept, requests

### Recording Commands
trace, record, highlight

---

## Appendix A: Command Quick Reference

### Navigation
| Command | Description |
|---------|-------------|
| `goto <url>` | Navigate to URL |
| `back` | Go back |
| `forward` | Go forward |
| `refresh` | Reload page |
| `url` | Get current URL |

### Observation
| Command | Description |
|---------|-------------|
| `observe` | Scan page elements |
| `html` | Get HTML content |
| `text` | Get text content |
| `title` | Get page title |
| `screenshot` | Capture screenshot |
| `box <target>` | Get element bounds |

### Actions
| Command | Description |
|---------|-------------|
| `click <target>` | Click element |
| `type <target> "text"` | Type into input |
| `clear <target>` | Clear input |
| `press <key>` | Press key |
| `keydown <key>` | Hold key |
| `keyup <key>` | Release key |
| `select <target> <value>` | Select option |
| `check <target>` | Check checkbox |
| `uncheck <target>` | Uncheck checkbox |
| `hover <target>` | Hover over element |
| `focus <target>` | Focus element |
| `scroll` | Scroll viewport |

### Waiting
| Command | Description |
|---------|-------------|
| `wait load` | Wait for page load |
| `wait idle` | Wait for network idle |
| `wait visible <target>` | Wait for visibility |
| `wait hidden <target>` | Wait for hidden |
| `wait exists <selector>` | Wait for existence |
| `wait gone <selector>` | Wait for removal |
| `wait url <pattern>` | Wait for URL |
| `wait until "<js>"` | Wait for JS condition |

### Sessions
| Command | Description |
|---------|-------------|
| `sessions` | List sessions |
| `session` | Current session info |
| `session new <name>` | Create session |
| `session close <name>` | Close session |
| `state save <path>` | Save auth state |
| `state load <path>` | Load auth state |

### Network
| Command | Description |
|---------|-------------|
| `headers set <json>` | Set HTTP headers |
| `headers` | View headers |
| `headers clear` | Clear headers |
| `intercept <pattern>` | Intercept requests |
| `requests` | View captured requests |

### Frames & Dialogs
| Command | Description |
|---------|-------------|
| `frames` | List frames |
| `frame <target>` | Switch frame |
| `dialog accept` | Accept dialog |
| `dialog dismiss` | Dismiss dialog |

### Device & Viewport
| Command | Description |
|---------|-------------|
| `viewport <w> <h>` | Set viewport size |
| `device "<name>"` | Emulate device |
| `media <feature> <value>` | Set media feature |

### Recording & Debug
| Command | Description |
|---------|-------------|
| `trace start` | Start trace recording |
| `trace stop` | Stop trace |
| `record start <path>` | Start video |
| `record stop` | Stop video |
| `highlight <target>` | Highlight element |
| `console` | View console |
| `errors` | View JS errors |

---

*Document Version: 1.1*
*Last Updated: January 2026*

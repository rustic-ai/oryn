# Oryn Intent Language - Command Reference

Extracted and expanded from SPEC-INTENT-LANGUAGE.md v1.1

## Implementation Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Fully implemented and wired end-to-end |
| ⚠️ | Partially implemented (see notes) |
| 🆕 | New in v1.1 (implementation pending) |
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
| `goto` | Navigate to a URL (accepts full URLs, domain-only, or relative paths) | `--headers <json>` | ✅ |
| `back` | Navigate to previous page in history | | ✅ |
| `forward` | Navigate to next page in history | | ✅ |
| `refresh` | Reload the current page | `--hard` (clears cache) | ✅ |
| `url` | Return the current URL | | ✅ |

**Implementation Notes:**
- `goto`: Parser → REPL → `backend.navigate()` → CDP/Browser
- `goto --headers`: Parser ✅, Backend integration 🆕
- `back/forward/refresh`: Parser → REPL → Backend trait methods → CDP
- `url`: Parser → Translator → `Execute` JS (`window.location.href`)

---

## Observation Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `observe` | Scan and return interactive elements | `--full`, `--minimal`, `--near "text"`, `--viewport`, `--hidden`, `--positions` | ✅ |
| `html` | Get raw HTML content | `--selector` | ✅ |
| `text` | Get text content of page or element | `--selector` | ✅ |
| `title` | Get page title | | ✅ |
| `screenshot` | Capture visual representation | file output, format selection, element-specific capture | ✅ |
| `box` | Get element bounding box | | 🆕 |

**Implementation Notes:**
- `observe`: Parser → Translator (`ScanRequest`) → Scanner.scan() with pattern detection
- `observe --positions`: Scanner already collects bounds, formatter needs update 🆕
- `box`: Parser 🆕, Translator 🆕, Scanner already has bounds data

---

## Action Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `click` | Click an element | `--force`, `--double`, `--right`, `--middle`, `--ctrl`, `--shift`, `--alt` | ✅ |
| `type` | Enter text into an input | `--append`, `--enter`, `--delay` | ✅ |
| `clear` | Clear an input field | | ✅ |
| `press` | Press a keyboard key (supports modifiers like Control+A) | | ✅ |
| `keydown` | Hold a key down | | 🆕 |
| `keyup` | Release a held key | `all` to release all | 🆕 |
| `keys` | Show currently held keys | | 🆕 |
| `select` | Choose from dropdown/select element (by value, text, or index) | | ✅ |
| `check` | Check a checkbox | | ✅ |
| `uncheck` | Uncheck a checkbox | | ✅ |
| `hover` | Move mouse over element (triggers hover states) | | ✅ |
| `focus` | Set keyboard focus to element | | ✅ |
| `scroll` | Scroll viewport or container (by direction, element, or page) | `--direction`, `--amount` | ✅ |

**Implementation Notes:**
- All action commands: Parser → Resolver (semantic → ID) → Translator → Scanner
- `press`: Parser → REPL → `backend.press_key()` → CDP `Input.dispatchKeyEvent`
- `keydown/keyup`: Parser 🆕, Backend keyboard state machine 🆕
- `click --ctrl/--shift/--alt`: Modifier support needs wiring 🆕

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
| `wait until "<js>"` | Wait for JS expression to be truthy | `--timeout` | 🆕 |
| `wait ready` | Wait for common app-ready patterns | `--timeout` | 🆕 |
| `wait items "<selector>" <n>` | Wait for N elements matching selector | `--timeout` | 🆕 |

**Implementation Notes:**
- Existing wait commands: Parser → Translator (`WaitRequest`) → Scanner.wait_for() with polling
- `wait until`: Parser 🆕, Translator 🆕 (execute + polling loop)
- `wait ready`: Parser 🆕, maps to common app-ready JS checks
- `wait items`: Parser 🆕, Translator 🆕 (count-based polling)

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
| `box <target>` | Get element bounding box (x, y, width, height) | 🆕 |

---

## Session Management Commands

| Command | Description | Status |
|---------|-------------|--------|
| `sessions` | List all active sessions | 🆕 |
| `session` | Show current session info | 🆕 |
| `session <n>` | Switch to named session | 🆕 |
| `session new <n>` | Create new named session | 🆕 |
| `session close <n>` | Close named session | 🆕 |

**CLI Flag:**
```bash
oryn --session <n> <mode>
oryn --session agent1 headless
```

**Environment Variable:**
```bash
ORYN_SESSION=agent1 oryn headless
```

**Response Examples:**
```
sessions
# Response:
# ok sessions
# - default (current)
# - agent1
# - agent2

session
# Response:
# ok session
# name: agent1
# mode: headless
# started: 2026-01-24T10:30:00Z
# pages: 3
```

---

## State Persistence Commands

| Command | Description | Options | Status |
|---------|-------------|---------|--------|
| `state save <path>` | Save auth state to file | `--cookies-only`, `--domain`, `--include-session` | 🆕 |
| `state load <path>` | Load auth state from file | `--merge`, `--cookies-only` | 🆕 |

**Examples:**
```
state save auth.json
state save auth.json --cookies-only
state save auth.json --domain github.com
state load auth.json
state load auth.json --merge
```

**Saved Data:**
- Cookies (with full attributes)
- localStorage
- sessionStorage (optional)

---

## Cookie & Storage Commands

| Command | Description | Status |
|---------|-------------|--------|
| `cookies list` | Show all cookies | ✅ |
| `cookies get <n>` | Get specific cookie | ⚠️ |
| `cookies set <n> <value>` | Set cookie | ⚠️ |
| `cookies delete <n>` | Remove cookie | ⚠️ |
| `storage get <key>` | Get localStorage/sessionStorage value | ✅ |
| `storage set <key> <value>` | Set localStorage/sessionStorage value | ✅ |
| `storage list` | List storage keys | ✅ |
| `storage clear` | Clear storage | ✅ |

---

## HTTP Headers Commands

| Command | Description | Status |
|---------|-------------|--------|
| `headers set <json>` | Set global HTTP headers | 🆕 |
| `headers set <domain> <json>` | Set domain-scoped headers | 🆕 |
| `headers` | View all configured headers | 🆕 |
| `headers <domain>` | View headers for domain | 🆕 |
| `headers clear` | Clear all headers | 🆕 |
| `headers clear <domain>` | Clear domain headers | 🆕 |

**Inline with navigation:**
```
goto api.example.com --headers {"Authorization": "Bearer token"}
```

---

## Network Interception Commands

| Command | Description | Status |
|---------|-------------|--------|
| `intercept "<pattern>"` | Intercept and log matching requests | 🆕 |
| `intercept "<pattern>" --block` | Block matching requests | 🆕 |
| `intercept "<pattern>" --respond <json>` | Mock response with JSON | 🆕 |
| `intercept "<pattern>" --respond-file <path>` | Mock from file | 🆕 |
| `intercept "<pattern>" --status <code>` | Mock with status code | 🆕 |
| `intercept clear` | Clear all interception rules | 🆕 |
| `intercept clear "<pattern>"` | Clear specific rule | 🆕 |
| `requests` | View captured requests | 🆕 |
| `requests --filter <text>` | Filter by URL | 🆕 |
| `requests --method <method>` | Filter by HTTP method | 🆕 |
| `requests --last <n>` | Show last N requests | 🆕 |

**Mode Availability:**
| Mode | Support |
|------|---------|
| oryn-h | Full (CDP Network domain) |
| oryn-e | Limited |
| oryn-r | Partial (extension) |

---

## Console & Error Commands

| Command | Description | Status |
|---------|-------------|--------|
| `console` | View console messages | 🆕 |
| `console --level <level>` | Filter by level (log, warn, error) | 🆕 |
| `console --filter "<text>"` | Filter by content | 🆕 |
| `console --last <n>` | Show last N messages | 🆕 |
| `console clear` | Clear console buffer | 🆕 |
| `errors` | View JavaScript errors | 🆕 |
| `errors --last <n>` | Show last N errors | 🆕 |
| `errors clear` | Clear error buffer | 🆕 |

---

## Tab Commands

| Command | Description | Status |
|---------|-------------|--------|
| `tabs` | List open tabs | ✅ |
| `tab new <url>` | Open new tab | ⚠️ |
| `tab switch <id>` | Switch to tab | ⚠️ |
| `tab close <id>` | Close tab | ⚠️ |

---

## Frame Commands

| Command | Description | Status |
|---------|-------------|--------|
| `frames` | List all frames in page | 🆕 |
| `frame "<selector>"` | Switch to iframe by selector | 🆕 |
| `frame <id>` | Switch to iframe by element ID | 🆕 |
| `frame main` | Return to main frame | 🆕 |
| `frame parent` | Go up one level | 🆕 |

---

## Dialog Commands

| Command | Description | Status |
|---------|-------------|--------|
| `dialog accept` | Accept alert/confirm dialog | 🆕 |
| `dialog accept "<text>"` | Accept prompt with input | 🆕 |
| `dialog dismiss` | Dismiss/cancel dialog | 🆕 |
| `dialog auto accept` | Auto-accept all dialogs | 🆕 |
| `dialog auto dismiss` | Auto-dismiss all dialogs | 🆕 |
| `dialog auto off` | Manual handling (default) | 🆕 |

---

## Viewport & Device Commands

| Command | Description | Status |
|---------|-------------|--------|
| `viewport <width> <height>` | Set viewport size | 🆕 |
| `device "<n>"` | Emulate named device | 🆕 |
| `device reset` | Reset to defaults | 🆕 |
| `devices` | List available device presets | 🆕 |
| `media color-scheme <value>` | Set prefers-color-scheme | 🆕 |
| `media reduced-motion <value>` | Set prefers-reduced-motion | 🆕 |
| `media reset` | Reset all media settings | 🆕 |

---

## Recording & Debug Commands

| Command | Description | Status |
|---------|-------------|--------|
| `trace start` | Start trace recording | 🆕 |
| `trace start <path>` | Start with custom path | 🆕 |
| `trace stop` | Stop and save trace | 🆕 |
| `trace stop <path>` | Stop and save to path | 🆕 |
| `record start <path>` | Start video recording | 🆕 |
| `record start <path> --quality <level>` | With quality setting | 🆕 |
| `record stop` | Stop recording | 🆕 |
| `highlight <target>` | Highlight element | 🆕 |
| `highlight <target> --duration <time>` | With duration | 🆕 |
| `highlight <target> --color <color>` | With color | 🆕 |
| `highlight clear` | Remove all highlights | 🆕 |

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

---

## Pack & Intent Management Commands

| Command | Description | Status |
|---------|-------------|--------|
| `packs` | List loaded intent packs | ✅ |
| `pack load <n>` | Load an intent pack | ✅ |
| `pack unload <n>` | Unload an intent pack | ✅ |
| `intents` | List all registered intents | ✅ |
| `intents session` | List session-defined intents | ✅ |
| `define <body>` | Define a new session intent | ✅ |
| `undefine <n>` | Remove a session intent | ✅ |
| `export <n> <path>` | Export intent to file | ✅ |
| `run <intent> [params]` | Execute a registered intent | ✅ |

---

## Additional Commands

| Command | Description | Status |
|---------|-------------|--------|
| `pdf <path>` | Generate PDF of current page | ✅ |
| `submit <target>` | Submit a form | ✅ |
| `learn status` | Show learning status for current domain | ✅ |
| `learn save <n>` | Save proposed intent | ✅ |

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

---

## Reserved Words

**Roles:** email, password, search, submit, username, phone, url

**Directions:** up, down, left, right, top, bottom

**Conditions:** visible, hidden, exists, gone, idle, load, until, ready

**Modifiers:** near, after, before, inside, contains

**Key Names:** Enter, Tab, Escape, Space, Backspace, Delete, ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, PageUp, PageDown, F1-F12, Control, Shift, Alt, Meta

---

## Implementation Roadmap

### Phase 1: Critical (v1.1)

| Feature | Commands | Effort |
|---------|----------|--------|
| Named Sessions | `sessions`, `session`, `session new/close` | High |
| Auth State Persistence | `state save/load` | Medium |
| HTTP Headers | `headers set/clear`, `goto --headers` | Medium |

### Phase 2: Important (v1.2)

| Feature | Commands | Effort |
|---------|----------|--------|
| Network Interception | `intercept`, `requests` | High |
| Console/Error Access | `console`, `errors` | Medium |
| Custom JS Wait | `wait until`, `wait ready`, `wait items` | Low |
| Bounding Box | `box` | Low |
| Key Hold/Release | `keydown`, `keyup`, `keys` | Medium |
| Device Emulation | `viewport`, `device`, `media` | Medium |

### Phase 3: Polish (v1.3+)

| Feature | Commands | Effort |
|---------|----------|--------|
| Trace Recording | `trace start/stop` | Medium |
| Video Recording | `record start/stop` | High |
| Element Highlighting | `highlight` | Low |
| Frame Navigation | `frames`, `frame` | Medium |
| Dialog Handling | `dialog accept/dismiss/auto` | Low |

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
     └────────────────────────────────────────────────┘
                    Some commands bypass translator
                    and go directly to backend methods
```

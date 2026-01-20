# Intent Language

The Oryn Intent Language (OIL) is a token-efficient, human-readable protocol designed specifically for AI agents to control web browsers.

## Design Philosophy

OIL is built on four foundational principles:

### Forgiving Syntax

Multiple syntactic variations are accepted for the same action. The parser prioritizes understanding agent intent over enforcing rigid formatting rules.

```
# All of these work:
click 5
CLICK 5
Click 5

click "Sign in"
click 'Sign in'
click Sign in
```

### Semantic Targeting

Agents can reference elements by meaning rather than implementation details:

```
type email "user@test.com"      # By role
click "Sign in"                  # By text
click submit                     # By role
click css(".btn-primary")        # By selector (fallback)
```

### Token Efficiency

Every character counts in an agent's context window. OIL minimizes verbosity while maximizing expressiveness:

```
# Compact observation format
@ github.com/login "Sign in"
[1] input/email "Username" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}
```

### Human Readability

Commands and responses are designed to be immediately comprehensible to human operators.

## Command Syntax

### General Format

```
command [target] [arguments] [--options]
```

- **Command**: The action to perform (case-insensitive)
- **Target**: Element identifier (ID, text, role, or selector)
- **Arguments**: Additional parameters (strings in quotes)
- **Options**: Flags prefixed with `--`

### Target Resolution

Oryn supports multiple targeting strategies:

| Strategy | Syntax | Example |
|----------|--------|---------|
| ID | Numeric | `click 5` |
| Text | Quoted string | `click "Sign in"` |
| Role | Role name | `type email "user@test.com"` |
| Selector | `css()` or `xpath()` | `click css(".btn")` |
| Relational | `near`, `inside`, `after` | `click "Edit" near "Item 1"` |

### String Handling

Strings can use single or double quotes interchangeably:

```
type 1 "hello world"
type 1 'hello world'
```

Single words without spaces may omit quotes:

```
type email user@test.com
```

## Command Categories

### Navigation Commands

| Command | Description | Example |
|---------|-------------|---------|
| `goto <url>` | Navigate to URL | `goto github.com` |
| `back` | Go back in history | `back` |
| `forward` | Go forward in history | `forward` |
| `refresh` | Reload page | `refresh` |
| `url` | Get current URL | `url` |

### Observation Commands

| Command | Description | Example |
|---------|-------------|---------|
| `observe` | Scan and list elements | `observe` |
| `observe --full` | Include selectors and positions | `observe --full` |
| `observe --near "text"` | Filter by proximity | `observe --near "Login"` |
| `text` | Get page text content | `text` |
| `title` | Get page title | `title` |
| `screenshot` | Capture screenshot | `screenshot` |

### Action Commands

| Command | Description | Example |
|---------|-------------|---------|
| `click <target>` | Click element | `click 5` |
| `click <target> --double` | Double-click | `click 5 --double` |
| `click <target> --right` | Right-click | `click 5 --right` |
| `type <target> <text>` | Type into input | `type 1 "hello"` |
| `type <target> <text> --enter` | Type and press Enter | `type 1 "hello" --enter` |
| `clear <target>` | Clear input field | `clear 1` |
| `press <key>` | Press keyboard key | `press Enter` |
| `select <target> <value>` | Select dropdown option | `select 3 "Option 1"` |
| `check <target>` | Check checkbox | `check 5` |
| `uncheck <target>` | Uncheck checkbox | `uncheck 5` |
| `hover <target>` | Hover over element | `hover 3` |
| `focus <target>` | Focus element | `focus 1` |
| `scroll [direction] [amount]` | Scroll viewport | `scroll down 500` |

### Wait Commands

| Command | Description | Example |
|---------|-------------|---------|
| `wait load` | Wait for page load | `wait load` |
| `wait idle` | Wait for network idle | `wait idle` |
| `wait visible <target>` | Wait for visibility | `wait visible "Success"` |
| `wait hidden <target>` | Wait for element to hide | `wait hidden "Loading"` |
| `wait url <pattern>` | Wait for URL match | `wait url "/dashboard"` |

### Intent Commands

| Command | Description | Example |
|---------|-------------|---------|
| `login <user> <pass>` | Execute login | `login "me@test.com" "pass"` |
| `search <query>` | Execute search | `search "rust programming"` |
| `accept_cookies` | Dismiss cookie banner | `accept_cookies` |
| `dismiss_popups` | Close modal dialogs | `dismiss_popups` |
| `fill_form <data>` | Fill multiple fields | `fill_form {...}` |
| `submit_form` | Submit current form | `submit_form` |

## Response Format

### Success Responses

```
ok <command> [details]

[response data]
```

### Observation Format

```
@ domain.com/path "Page Title"

[id] type/role "text" {modifiers}

# patterns
- pattern_name: field=[id] ...
```

**Element Types**: `input`, `button`, `link`, `select`, `textarea`, `checkbox`, `radio`, `generic`

**Element Roles**: `email`, `password`, `search`, `tel`, `url`, `username`, `submit`, `primary`, `generic`

**Modifiers**: `{required}`, `{disabled}`, `{readonly}`, `{checked}`, `{primary}`, `{focused}`

### Error Responses

```
error <command>: <message>

# hint
<recovery suggestion>
```

### Change Notation

| Symbol | Meaning |
|--------|---------|
| `+` | Element appeared |
| `-` | Element disappeared |
| `~` | Element/URL changed |
| `@` | Page navigated |

Example:

```
ok click 3

# changes
~ url: /login → /dashboard
+ [1] nav "Dashboard"
- [3] button "Sign in"
```

## Multi-Level Abstraction

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
```

## Reserved Words

### Roles
`email`, `password`, `search`, `submit`, `username`, `phone`, `url`

### Directions
`up`, `down`, `left`, `right`, `top`, `bottom`

### Conditions
`visible`, `hidden`, `exists`, `gone`, `idle`, `load`

### Modifiers
`near`, `after`, `before`, `inside`, `contains`

### Key Names
`Enter`, `Tab`, `Escape`, `Space`, `Backspace`, `Delete`, `ArrowUp`, `ArrowDown`, `ArrowLeft`, `ArrowRight`, `Home`, `End`, `PageUp`, `PageDown`, `F1`-`F12`, `Control`, `Shift`, `Alt`, `Meta`

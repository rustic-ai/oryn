# Intent Commands Reference

Complete reference for all Oryn Intent Language commands.

## Command Syntax

```
command [target] [arguments] [--options]
```

## Navigation Commands

### goto

Navigate to a URL.

```
goto <url> [--timeout <duration>]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `url` | Full URL, domain, or relative path |

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--timeout` | 30s | Navigation timeout |

**Examples:**

```
goto google.com
goto https://github.com/login
goto /about                      # Relative path
goto example.com --timeout 60s
```

### back

Navigate to the previous page in history.

```
back
```

### forward

Navigate to the next page in history.

```
forward
```

### refresh

Reload the current page.

```
refresh [--hard]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--hard` | Clear cache before refreshing |

### url

Get the current page URL.

```
url
```

**Response:**

```
https://example.com/current/path
```

## Observation Commands

### observe / scan

Scan the page and list interactive elements.

```
observe [--full] [--minimal] [--near <text>] [--viewport-only]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--full` | Include selectors, positions, and detailed attributes |
| `--minimal` | Only counts, no element details |
| `--near <text>` | Filter to elements near specific text |
| `--viewport-only` | Only elements visible in viewport |

**Response:**

```
@ github.com/login "Sign in to GitHub"

[1] input/email "Username or email" {required}
[2] input/password "Password" {required}
[3] button/submit "Sign in" {primary}

# patterns
- login_form: email=[1] password=[2] submit=[3]
```

### text

Get the text content of the page or an element.

```
text [<target>]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `target` | Optional element to get text from |

### title

Get the page title.

```
title
```

### screenshot

Capture a screenshot.

```
screenshot [--output <path>] [--element <target>] [--format <format>]
```

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--output` | ./screenshot.png | Output file path |
| `--element` | (full page) | Specific element to capture |
| `--format` | png | Image format (png, jpeg, webp) |

## Action Commands

### click

Click an element.

```
click <target> [--double] [--right] [--middle] [--force]
```

**Target Types:**

| Type | Example |
|------|---------|
| ID | `click 5` |
| Text | `click "Sign in"` |
| Role | `click submit` |
| Selector | `click css(".btn")` |
| Relational | `click "Edit" near "Item 1"` |

**Options:**

| Option | Description |
|--------|-------------|
| `--double` | Double-click |
| `--right` | Right-click (context menu) |
| `--middle` | Middle-click |
| `--force` | Click even if element is covered |

### type

Type text into an input element.

```
type <target> <text> [--append] [--enter] [--delay <ms>]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `target` | Element to type into |
| `text` | Text to type |

**Options:**

| Option | Description |
|--------|-------------|
| `--append` | Don't clear existing content |
| `--enter` | Press Enter after typing |
| `--delay` | Milliseconds between keystrokes |

**Examples:**

```
type 1 "hello world"
type email "user@test.com"
type "Search" "oryn browser" --enter
type 5 "more text" --append
```

### clear

Clear an input field.

```
clear <target>
```

### press

Press a keyboard key.

```
press <key>
```

**Key Names:**

- `Enter`, `Tab`, `Escape`, `Space`, `Backspace`, `Delete`
- `ArrowUp`, `ArrowDown`, `ArrowLeft`, `ArrowRight`
- `Home`, `End`, `PageUp`, `PageDown`
- `F1` through `F12`
- `Control`, `Shift`, `Alt`, `Meta`

**Combinations:**

```
press Control+A
press Control+C
press Shift+Tab
press Alt+F4
```

### select

Select an option from a dropdown.

```
select <target> <value> [--index]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `target` | Select element |
| `value` | Value or text to select |

**Options:**

| Option | Description |
|--------|-------------|
| `--index` | Select by zero-based index |

**Examples:**

```
select 3 "Option 1"        # By text
select 3 "opt1"            # By value
select 3 --index 2         # By index (third option)
```

### check

Check a checkbox.

```
check <target>
```

### uncheck

Uncheck a checkbox.

```
uncheck <target>
```

### hover

Move mouse over an element.

```
hover <target>
```

### focus

Set keyboard focus to an element.

```
focus <target>
```

### scroll

Scroll the viewport or a container.

```
scroll [<direction>] [<amount>] [--to <target>]
```

**Directions:**

`up`, `down`, `left`, `right`, `top`, `bottom`

**Arguments:**

| Argument | Description |
|----------|-------------|
| `direction` | Scroll direction |
| `amount` | Pixels to scroll |

**Options:**

| Option | Description |
|--------|-------------|
| `--to` | Scroll element into view |

**Examples:**

```
scroll down
scroll down 500
scroll up 200
scroll top
scroll bottom
scroll --to "Footer"
scroll to 15            # Scroll element [15] into view
```

## Wait Commands

### wait

Wait for a condition.

```
wait <condition> [<target>] [--timeout <duration>]
```

**Conditions:**

| Condition | Description |
|-----------|-------------|
| `load` | Page load complete |
| `idle` | Network idle |
| `visible <target>` | Element becomes visible |
| `hidden <target>` | Element becomes hidden |
| `exists <selector>` | Element appears in DOM |
| `gone <selector>` | Element removed from DOM |
| `url <pattern>` | URL matches pattern |
| `enabled <target>` | Element becomes enabled |

**Examples:**

```
wait load
wait idle
wait visible "Success"
wait hidden "Loading..."
wait url "/dashboard"
wait enabled 5 --timeout 10s
```

## Intent Commands

### login

Execute login workflow.

```
login <username> <password> [--no-submit] [--wait <duration>]
```

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--no-submit` | false | Fill fields but don't submit |
| `--wait` | 10s | Wait time after submit |

### logout

Execute logout workflow.

```
logout
```

### search

Execute search workflow.

```
search <query> [--submit enter|click|auto]
```

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--submit` | auto | How to submit (enter, click, auto) |

### accept_cookies

Dismiss cookie consent banner.

```
accept_cookies [--reject]
```

**Options:**

| Option | Description |
|--------|-------------|
| `--reject` | Click reject instead of accept |

### dismiss_popups

Close modal dialogs and overlays.

```
dismiss_popups [--all] [--type <type>]
```

**Options:**

| Option | Default | Description |
|--------|---------|-------------|
| `--all` | true | Dismiss all popups |
| `--type` | any | Filter: modal, overlay, toast, banner |

### fill_form

Fill multiple form fields.

```
fill_form <data> [--pattern <pattern>] [--partial]
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `data` | JSON object with field:value pairs |

**Options:**

| Option | Description |
|--------|-------------|
| `--pattern` | Target specific form pattern |
| `--partial` | Allow filling only some fields |

**Example:**

```
fill_form {"name": "John", "email": "john@test.com", "country": "us"}
```

### submit_form

Submit the current form.

```
submit_form [--pattern <pattern>] [--wait <duration>]
```

### scroll_to

Scroll an element into view.

```
scroll_to <target>
```

## Data Extraction Commands

### extract

Extract data from the page.

```
extract <type> [--selector <selector>]
```

**Types:**

| Type | Description |
|------|-------------|
| `links` | All hyperlinks |
| `images` | All images with src/alt |
| `tables` | Table data |
| `meta` | Page metadata |

## Session Commands

### cookies

Manage cookies.

```
cookies list
cookies get <name>
cookies set <name> <value>
cookies delete <name>
cookies clear
```

### storage

Manage localStorage/sessionStorage.

```
storage get <key>
storage set <key> <value>
storage delete <key>
storage clear
```

## Tab Commands

### tabs

List open tabs.

```
tabs
```

### tab

Manage tabs.

```
tab new [<url>]      # Open new tab
tab switch <id>      # Switch to tab
tab close [<id>]     # Close tab
```

## Intent Management

### intents

List available intents.

```
intents [--session] [--builtin] [--loaded]
```

### define

Define a session intent.

```
define <name>:
  [description: "<description>"]
  [parameters: <params>]
  steps:
    <steps>
  [success: <conditions>]
```

### undefine

Remove a session intent.

```
undefine <name>
```

### export

Export an intent to a file.

```
export <name> --out <path>
```

## System Commands

### exit

Exit Oryn.

```
exit
```

### help

Show help information.

```
help [<command>]
```

### version

Show version information.

```
version
```

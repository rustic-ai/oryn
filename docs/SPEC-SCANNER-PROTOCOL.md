# Lemmascope Scanner Protocol Specification

## Version 1.0

---

## 1. Overview

The Universal Scanner Protocol defines the interface between Lemmascope backends and the JavaScript scanner that runs inside web browsers. This protocol ensures consistent behavior across all three execution modes: Embedded (lscope-e), Headless (lscope-h), and Remote (lscope-r).

### 1.1 Design Goals

**Universality**
The same protocol works identically across WebKit, Chromium, and browser extensions. Backend implementation details are abstracted away; agents experience consistent behavior regardless of which binary they connect to.

**Completeness**
The protocol handles all element types, user actions, and edge cases that agents encounter in real-world web automation. From standard forms to dynamic SPAs, the scanner provides comprehensive coverage.

**Efficiency**
Data transfer is minimized through selective scanning, incremental updates, and configurable verbosity. The protocol respects both network bandwidth and agent context windows.

**Debuggability**
Clear error messages, predictable structure, and explicit state representation make troubleshooting straightforward for both humans and agents.

### 1.2 Architecture

The architecture follows a clean separation of concerns:

**Backend Layer**
Each Lemmascope binary (lscope-e, lscope-h, lscope-r) implements browser communication using the appropriate protocol for its environment:
- lscope-e (Embedded): WebDriver over HTTP
- lscope-h (Headless): Chrome DevTools Protocol over WebSocket
- lscope-r (Remote): Custom protocol over WebSocket to browser extension

**Scanner Layer**
A single JavaScript implementation runs inside all browser contexts. Backends inject this same script regardless of their underlying browser engine. The scanner understands a JSON command vocabulary and returns JSON responses.

**Protocol Layer**
The JSON message format is identical across all transport mechanisms. Backends translate between their native communication methods and the standardized scanner protocol.

### 1.3 Transport Methods

| Binary | Browser Engine | Protocol | Transport |
|--------|----------------|----------|-----------|
| lscope-e | WPE WebKit (COG) | WebDriver | HTTP |
| lscope-h | Chromium | CDP | WebSocket |
| lscope-r | User's Browser | Custom | WebSocket |

All transports use the same JSON message format. The scanner implementation is byte-for-byte identical across all contexts.

---

## 2. Message Format

### 2.1 Request Structure

All requests contain a command identifier and command-specific parameters:

**Required Fields**

| Field | Type | Description |
|-------|------|-------------|
| `cmd` | string | Command name |

**Command-Specific Fields**
Additional fields depend on the command being invoked.

### 2.2 Response Structure

All responses share a common structure:

**Required Fields**

| Field | Type | Description |
|-------|------|-------------|
| `ok` | boolean | True if command succeeded |

**Conditional Fields**

| Field | Type | Description |
|-------|------|-------------|
| `error` | string | Error message (when ok=false) |
| `code` | string | Error code for programmatic handling |
| `data` | object | Command-specific response data |
| `timing` | object | Execution timing information |

### 2.3 Error Codes

| Code | Description | Recovery Strategy |
|------|-------------|-------------------|
| `ELEMENT_NOT_FOUND` | Element ID doesn't exist in element map | Run `scan` to refresh |
| `ELEMENT_STALE` | Element was removed from DOM | Run `scan` to refresh |
| `ELEMENT_NOT_VISIBLE` | Element exists but not visible | Scroll or wait |
| `ELEMENT_DISABLED` | Element is disabled | Wait for enabled state |
| `ELEMENT_NOT_INTERACTABLE` | Cannot interact (covered, etc.) | Use `force` option |
| `SELECTOR_INVALID` | CSS selector syntax error | Fix selector |
| `TIMEOUT` | Operation timed out | Increase timeout or verify condition |
| `NAVIGATION_ERROR` | Page navigation failed | Check URL/network |
| `SCRIPT_ERROR` | JavaScript execution error | Check script syntax |
| `UNKNOWN_COMMAND` | Command not recognized | Check command name |

---

## 3. Command Reference

### 3.1 scan

Scan the page and return all interactive elements.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `max_elements` | number | 200 | Maximum elements to return |
| `include_hidden` | boolean | false | Include hidden elements |
| `near` | string | null | Filter by proximity to text |
| `within` | string | null | Limit to container selector |
| `viewport_only` | boolean | false | Only visible in viewport |

**Response Data**

The response includes:

**Page Information**
- URL and title
- Viewport dimensions
- Scroll position and maximum scroll
- Document ready state

**Element List**
Each element includes:
- Numeric ID for targeting
- Type classification (input, button, link, select, etc.)
- Role classification (email, password, submit, search, etc.)
- Tag name
- Accessible text (truncated to reasonable length)
- Unique CSS selector
- XPath expression
- Bounding rectangle coordinates
- Relevant attributes
- Current state (visible, enabled, focused, value, checked)
- Modifier flags (required, disabled, primary, etc.)

**Detected Patterns**
Recognized UI patterns with element ID references:
- Login forms (email, password, submit, remember fields)
- Search forms (input, submit button)
- Pagination (prev, next, page numbers)
- Modal dialogs (container, close button, title)
- Cookie banners (container, accept, reject buttons)

**Metadata**
- Total elements scanned
- Interactive elements found
- Scan execution time

### 3.2 click

Click an element by ID.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID to click |
| `button` | string | "left" | Mouse button (left, right, middle) |
| `click_count` | number | 1 | Number of clicks (2 for double-click) |
| `modifiers` | array | [] | Modifier keys (Control, Shift, Alt) |
| `offset` | object | center | Click offset from element center |
| `force` | boolean | false | Click even if covered |
| `scroll_into_view` | boolean | true | Scroll element into view first |

**Response Data**
- Action performed
- Target element ID and selector
- Click coordinates
- Whether navigation was triggered
- DOM changes detected (elements added/removed/modified)

### 3.3 type

Type text into an element.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |
| `text` | string | required | Text to type |
| `clear` | boolean | true | Clear existing content first |
| `delay` | number | 0 | Milliseconds between keystrokes |

**Response Data**
- Action performed
- Target element ID
- Text typed
- Final input value

### 3.4 clear

Clear an input element's value.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |

### 3.5 check / uncheck

Set checkbox or radio button state.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |

**Response Data**
- Final checked state

### 3.6 select

Select an option in a dropdown.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |
| `value` | string | null | Value attribute to select |
| `text` | string | null | Visible text to select |
| `index` | number | null | Zero-based index to select |

Only one of value, text, or index should be provided.

**Response Data**
- Selected value
- Selected text
- Previous selection

### 3.7 scroll

Scroll the viewport or container.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `direction` | string | null | Direction (up, down, left, right) |
| `amount` | number | null | Pixels to scroll |
| `element` | number | null | Element ID to scroll into view |
| `container` | string | null | Container selector to scroll |
| `behavior` | string | "instant" | Scroll behavior (instant, smooth) |

**Response Data**
- New scroll position
- Maximum scroll position

### 3.8 focus

Set keyboard focus to an element.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |

### 3.9 hover

Move mouse over an element.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |

### 3.10 submit

Submit a form.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | null | Form or element within form |

If no ID provided, submits the form containing the currently focused element.

### 3.11 get_value

Get the current value of an input element.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |

**Response Data**
- Current value (string, boolean for checkboxes, array for multi-select)

### 3.12 get_text

Get text content of an element.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `selector` | string | required | CSS selector |

**Response Data**
- Text content

### 3.13 exists

Check if an element exists in the DOM.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `selector` | string | required | CSS selector |

**Response Data**
- Boolean existence flag

### 3.14 wait_for

Wait for a condition to be true.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `condition` | string | required | Condition type |
| `selector` | string | null | CSS selector (for element conditions) |
| `id` | number | null | Element ID (alternative to selector) |
| `timeout` | number | 30000 | Maximum wait time in milliseconds |

Condition types:
- `visible` — Element becomes visible
- `hidden` — Element becomes hidden
- `exists` — Element appears in DOM
- `gone` — Element removed from DOM
- `enabled` — Element becomes enabled
- `disabled` — Element becomes disabled

**Response Data**
- Whether condition was met
- Time waited

### 3.15 execute

Execute arbitrary JavaScript.

**Request Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `script` | string | required | JavaScript code |
| `args` | array | [] | Arguments passed to script |

**Response Data**
- Script return value

### 3.16 version

Get scanner protocol version.

**Response Data**
- Protocol version string
- Scanner implementation version
- Supported features list

---

## 4. Element Classification

### 4.1 Element Types

| Type | Description |
|------|-------------|
| `input` | Text input, email, password, tel, url, number, etc. |
| `button` | Button elements and input type=button/submit |
| `link` | Anchor elements with href |
| `select` | Dropdown/select elements |
| `textarea` | Multi-line text input |
| `checkbox` | Checkbox inputs |
| `radio` | Radio button inputs |
| `generic` | Other interactive elements (contenteditable, custom widgets) |

### 4.2 Element Roles

Roles are inferred from type attributes, autocomplete hints, labels, placeholders, and context:

| Role | Detection Signals |
|------|-------------------|
| `email` | type=email, autocomplete=email, label/placeholder contains "email" |
| `password` | type=password, autocomplete=*password |
| `search` | type=search, role=search, label contains "search" |
| `tel` | type=tel, autocomplete=tel |
| `url` | type=url, label contains "website/url" |
| `username` | autocomplete=username, label contains "username" |
| `submit` | type=submit, button in form context |
| `primary` | Primary action button (visual prominence, form submit) |
| `generic` | No specific role detected |

### 4.3 Element Modifiers

| Modifier | Meaning |
|----------|---------|
| `required` | Field is required |
| `disabled` | Element is disabled |
| `readonly` | Input is read-only |
| `hidden` | Element is hidden (include_hidden=true) |
| `primary` | Primary/prominent action |
| `checked` | Checkbox/radio is checked |
| `unchecked` | Checkbox/radio is unchecked |
| `focused` | Element has keyboard focus |

---

## 5. Pattern Detection

The scanner automatically identifies common UI patterns and provides structured references to their component elements.

### 5.1 Login Form Pattern

Detected when page contains:
- Email/username input field
- Password input field
- Submit button

Returns references to all identified elements plus the form container selector.

### 5.2 Search Form Pattern

Detected when page contains:
- Search-type input or search-labeled field
- Optional submit/search button

### 5.3 Pagination Pattern

Detected when page contains:
- Previous/next navigation links
- Numbered page links

Returns references to prev, next, and page number elements.

### 5.4 Modal Dialog Pattern

Detected when page contains:
- Element with role=dialog or aria-modal=true
- Common modal CSS classes
- Close/dismiss button within container

Returns container selector, close button reference, and modal title if present.

### 5.5 Cookie Banner Pattern

Detected when page contains:
- Element with cookie/consent/GDPR-related classes or IDs
- Accept/agree button
- Optional reject/decline button

---

## 6. Element Map Lifecycle

### 6.1 Map Creation

When `scan` is called:
1. Previous element map is cleared
2. DOM is traversed for interactive elements
3. New IDs are assigned sequentially
4. Element references are stored for subsequent commands

### 6.2 Map Usage

When action commands are called:
1. Element ID is looked up in the map
2. If found, action is executed on the stored reference
3. If not found, `ELEMENT_NOT_FOUND` error is returned

### 6.3 Map Staleness

The element map becomes stale when:
- Page navigation occurs
- DOM is modified by JavaScript
- AJAX updates content

Agents should re-scan after:
- Navigation commands
- Actions that trigger page changes
- Before critical interactions
- When `ELEMENT_STALE` errors occur

### 6.4 Best Practices

- Always scan before starting a new task on a page
- Re-scan after navigation
- Re-scan after actions that modify content
- Don't cache element IDs across page loads
- Use pattern detection to verify expected UI is present

---

## 7. Iframe Handling

### 7.1 Same-Origin Iframes

The scanner can access content within same-origin iframes through the contentDocument interface. Elements within accessible iframes are included in scan results with their iframe context noted.

### 7.2 Cross-Origin Iframes

Browser security prevents accessing cross-origin iframe content. For cross-origin iframes:
- The iframe element itself is reported
- Content within cannot be scanned
- Navigation to the iframe URL directly may be required
- Backend-level iframe handling (WebDriver/CDP frame switching) is an alternative

---

## 8. Versioning

### 8.1 Protocol Version

The protocol uses semantic versioning:
- Major version: Breaking changes to existing commands
- Minor version: New commands or optional fields
- Patch version: Bug fixes and clarifications

Backends should check protocol version on connection and handle version mismatches gracefully.

### 8.2 Feature Detection

The `version` command returns a list of supported features, allowing backends to adapt to scanner capabilities and handle partial implementations.

---

*Document Version: 1.0*  
*Last Updated: January 2025*

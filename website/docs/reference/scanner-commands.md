# Scanner Commands Reference

Low-level reference for the Universal Scanner JSON protocol.

## Overview

The scanner protocol is the internal JSON interface between Oryn backends and the JavaScript scanner running in the browser. Most users interact with Oryn via the Intent Language, but understanding the scanner protocol is useful for debugging and advanced use cases.

## Message Format

### Request

```json
{
  "cmd": "command_name",
  // command-specific parameters
}
```

### Success Response

```json
{
  "ok": true,
  "data": { /* command-specific data */ },
  "timing": {
    "start": 1234567890,
    "end": 1234567891
  }
}
```

### Error Response

```json
{
  "ok": false,
  "error": "Error description",
  "code": "ERROR_CODE"
}
```

## Commands

### scan

Scan the page for interactive elements.

**Request:**

```json
{
  "cmd": "scan",
  "max_elements": 200,
  "include_hidden": false,
  "viewport_only": false,
  "near": null,
  "within": null
}
```

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `max_elements` | number | 200 | Maximum elements to return |
| `include_hidden` | boolean | false | Include hidden elements |
| `viewport_only` | boolean | false | Only visible in viewport |
| `near` | string | null | Filter by proximity to text |
| `within` | string | null | Limit to container selector |

**Response:**

```json
{
  "ok": true,
  "data": {
    "page": {
      "url": "https://example.com/page",
      "title": "Page Title",
      "viewport": { "width": 1920, "height": 1080 },
      "scroll": { "x": 0, "y": 0, "maxX": 0, "maxY": 500 },
      "ready_state": "complete"
    },
    "elements": [
      {
        "id": 1,
        "type": "input",
        "role": "email",
        "tag": "input",
        "text": "Email address",
        "selector": "#email",
        "xpath": "//input[@id='email']",
        "rect": { "x": 100, "y": 200, "width": 300, "height": 40 },
        "attributes": {
          "type": "email",
          "required": true,
          "placeholder": "Enter email"
        },
        "state": {
          "visible": true,
          "enabled": true,
          "focused": false,
          "value": ""
        },
        "modifiers": ["required"]
      }
    ],
    "patterns": {
      "login_form": {
        "email": 1,
        "password": 2,
        "submit": 3
      }
    },
    "meta": {
      "total_scanned": 150,
      "interactive_found": 10,
      "scan_time_ms": 45
    }
  }
}
```

### click

Click an element.

**Request:**

```json
{
  "cmd": "click",
  "id": 5,
  "button": "left",
  "click_count": 1,
  "modifiers": [],
  "offset": null,
  "force": false,
  "scroll_into_view": true
}
```

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |
| `button` | string | "left" | Mouse button |
| `click_count` | number | 1 | Number of clicks |
| `modifiers` | array | [] | Modifier keys |
| `offset` | object | null | Click offset {x, y} |
| `force` | boolean | false | Click even if covered |
| `scroll_into_view` | boolean | true | Scroll first |

**Response:**

```json
{
  "ok": true,
  "data": {
    "action": "click",
    "target": {
      "id": 5,
      "selector": "#submit-btn"
    },
    "coordinates": { "x": 450, "y": 300 },
    "navigation_triggered": false,
    "changes": {
      "added": [],
      "removed": [],
      "modified": [5]
    }
  }
}
```

### type

Type text into an element.

**Request:**

```json
{
  "cmd": "type",
  "id": 1,
  "text": "hello world",
  "clear": true,
  "delay": 0
}
```

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `id` | number | required | Element ID |
| `text` | string | required | Text to type |
| `clear` | boolean | true | Clear existing content |
| `delay` | number | 0 | Delay between keystrokes (ms) |

**Response:**

```json
{
  "ok": true,
  "data": {
    "action": "type",
    "target": { "id": 1 },
    "text": "hello world",
    "final_value": "hello world"
  }
}
```

### clear

Clear an input element.

**Request:**

```json
{
  "cmd": "clear",
  "id": 1
}
```

### select

Select a dropdown option.

**Request:**

```json
{
  "cmd": "select",
  "id": 3,
  "value": null,
  "text": "Option 1",
  "index": null
}
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `id` | number | Element ID |
| `value` | string | Value to select |
| `text` | string | Visible text to select |
| `index` | number | Zero-based index |

Only one of `value`, `text`, or `index` should be provided.

**Response:**

```json
{
  "ok": true,
  "data": {
    "selected_value": "opt1",
    "selected_text": "Option 1",
    "previous_value": "opt2"
  }
}
```

### check / uncheck

Set checkbox state.

**Request:**

```json
{
  "cmd": "check",
  "id": 5
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "checked": true
  }
}
```

### scroll

Scroll the viewport or container.

**Request:**

```json
{
  "cmd": "scroll",
  "direction": "down",
  "amount": 500,
  "element": null,
  "container": null,
  "behavior": "instant"
}
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `direction` | string | up, down, left, right |
| `amount` | number | Pixels to scroll |
| `element` | number | Element ID to scroll into view |
| `container` | string | Container selector |
| `behavior` | string | instant, smooth |

**Response:**

```json
{
  "ok": true,
  "data": {
    "scroll_position": { "x": 0, "y": 500 },
    "max_scroll": { "x": 0, "y": 2000 }
  }
}
```

### focus

Focus an element.

**Request:**

```json
{
  "cmd": "focus",
  "id": 1
}
```

### hover

Hover over an element.

**Request:**

```json
{
  "cmd": "hover",
  "id": 3
}
```

### submit

Submit a form.

**Request:**

```json
{
  "cmd": "submit",
  "id": null
}
```

If `id` is null, submits the form containing the focused element.

### get_value

Get an input's value.

**Request:**

```json
{
  "cmd": "get_value",
  "id": 1
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "value": "current input value"
  }
}
```

### get_text

Get text content.

**Request:**

```json
{
  "cmd": "get_text",
  "selector": ".content"
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "text": "Text content of element"
  }
}
```

### exists

Check if element exists.

**Request:**

```json
{
  "cmd": "exists",
  "selector": "#submit-btn"
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "exists": true,
    "element_id": 5
  }
}
```

### wait_for

Wait for a condition.

**Request:**

```json
{
  "cmd": "wait_for",
  "condition": "visible",
  "selector": "#success",
  "id": null,
  "timeout": 30000
}
```

**Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `condition` | string | visible, hidden, exists, gone, enabled, disabled, navigation |
| `selector` | string | CSS selector |
| `id` | number | Element ID (alternative to selector) |
| `timeout` | number | Timeout in milliseconds |

**Response:**

```json
{
  "ok": true,
  "data": {
    "condition_met": true,
    "wait_time_ms": 1500
  }
}
```

### execute

Execute JavaScript.

**Request:**

```json
{
  "cmd": "execute",
  "script": "return document.title;",
  "args": []
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "result": "Page Title"
  }
}
```

### version

Get scanner version.

**Request:**

```json
{
  "cmd": "version"
}
```

**Response:**

```json
{
  "ok": true,
  "data": {
    "protocol_version": "1.0",
    "scanner_version": "1.0.0",
    "features": ["scan", "click", "type", "select", "scroll", "wait_for", "execute"]
  }
}
```

## Element Classification

### Types

| Type | Description |
|------|-------------|
| `input` | Text inputs (text, email, password, etc.) |
| `button` | Button elements |
| `link` | Anchor elements with href |
| `select` | Dropdown selects |
| `textarea` | Multi-line text inputs |
| `checkbox` | Checkbox inputs |
| `radio` | Radio buttons |
| `generic` | Other interactive elements |

### Roles

| Role | Detection |
|------|-----------|
| `email` | type=email, autocomplete=email |
| `password` | type=password |
| `search` | type=search, role=search |
| `tel` | type=tel |
| `url` | type=url |
| `username` | autocomplete=username |
| `submit` | type=submit, form submit button |
| `primary` | Primary action button |

### Modifiers

| Modifier | Meaning |
|----------|---------|
| `required` | Field is required |
| `disabled` | Element is disabled |
| `readonly` | Input is read-only |
| `hidden` | Element is hidden |
| `primary` | Primary action |
| `checked` | Checkbox/radio checked |
| `focused` | Has keyboard focus |

## Pattern Detection

### login_form

```json
{
  "login_form": {
    "email": 1,
    "password": 2,
    "submit": 3,
    "remember": 4
  }
}
```

### search_form

```json
{
  "search_form": {
    "input": 5,
    "submit": 6
  }
}
```

### cookie_banner

```json
{
  "cookie_banner": {
    "container": "#cookie-notice",
    "accept": 7,
    "reject": 8
  }
}
```

### modal_dialog

```json
{
  "modal_dialog": {
    "container": ".modal",
    "close": 10,
    "title": "Dialog Title"
  }
}
```

### pagination

```json
{
  "pagination": {
    "prev": 11,
    "next": 12,
    "pages": [13, 14, 15, 16],
    "current": 14
  }
}
```

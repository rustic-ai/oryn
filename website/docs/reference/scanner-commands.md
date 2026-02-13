# Scanner Commands Reference

Low-level reference for scanner protocol actions used by Oryn backends.

## Message Format

### Request

Use `action` as the canonical command key:

```json
{
  "action": "scan"
}
```

Compatibility: scanner.js also accepts `cmd` for legacy callers.

### Success

```json
{
  "status": "ok",
  "success": true,
  "message": "Action completed"
}
```

### Error

```json
{
  "status": "error",
  "code": "INVALID_REQUEST",
  "message": "Missing command"
}
```

## Core Actions

### `scan`

```json
{
  "action": "scan",
  "max_elements": 200,
  "include_hidden": false,
  "viewport_only": false,
  "near": null,
  "monitor_changes": false,
  "full_mode": false
}
```

Returns `page`, `elements`, `stats`, and optionally `patterns`, `changes`, `available_intents`.

### `click`

```json
{
  "action": "click",
  "id": 5,
  "button": "left",
  "double": false,
  "force": false,
  "modifiers": []
}
```

### `type`

```json
{
  "action": "type",
  "id": 1,
  "text": "hello",
  "clear": true,
  "submit": false,
  "delay": 0
}
```

### `select`

```json
{
  "action": "select",
  "id": 3,
  "value": null,
  "label": "Canada",
  "index": null
}
```

### `wait_for`

```json
{
  "action": "wait_for",
  "condition": "visible",
  "selector": "#success",
  "timeout": 30000
}
```

### `extract`

```json
{
  "action": "extract",
  "source": "links",
  "selector": null
}
```

## Intent-Style Scanner Actions

### `login`

```json
{
  "action": "login",
  "username": "user@example.com",
  "password": "secret"
}
```

### `search`

```json
{
  "action": "search",
  "query": "oryn"
}
```

### `dismiss`

```json
{
  "action": "dismiss",
  "target": "popups"
}
```

### `accept`

```json
{
  "action": "accept",
  "target": "cookies"
}
```

## Script/Data Actions

### `execute`

```json
{
  "action": "execute",
  "script": "return document.title;",
  "args": []
}
```

### `get_text`

```json
{
  "action": "get_text",
  "selector": null
}
```

### `get_html`

```json
{
  "action": "get_html",
  "selector": null,
  "outer": true
}
```

## Error Codes

Common scanner-facing codes:

- `ELEMENT_NOT_FOUND`
- `ELEMENT_STALE`
- `ELEMENT_NOT_VISIBLE`
- `ELEMENT_DISABLED`
- `ELEMENT_NOT_INTERACTABLE`
- `INVALID_ELEMENT_TYPE`
- `OPTION_NOT_FOUND`
- `SELECTOR_INVALID`
- `TIMEOUT`
- `NAVIGATION_ERROR`
- `SCRIPT_ERROR`
- `UNKNOWN_COMMAND`
- `INVALID_REQUEST`
- `INTERNAL_ERROR`

See [Error Codes](error-codes.md) for recovery guidance.

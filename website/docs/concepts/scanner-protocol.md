# Scanner Protocol

The scanner protocol is the JSON contract between Oryn backends and the in-page scanner runtime (`scanner.js`).

## Role in Architecture

All modes (`oryn-h`, `oryn-e`, `oryn-r`, and `extension-w`) use the same scanner behavior model:

- backends send scanner actions,
- scanner executes DOM operations,
- scanner returns structured success/error responses.

## Canonical Wire Shape

### Request

Canonical action envelope:

```json
{
  "action": "scan",
  "viewport_only": true
}
```

Compatibility note: the scanner runtime also accepts `cmd` (`message.cmd || message.action`) for legacy callers.

### Success Response

```json
{
  "status": "ok",
  "page": {
    "url": "https://example.com",
    "title": "Example Domain"
  },
  "elements": [],
  "stats": {
    "total": 0,
    "scanned": 0
  }
}
```

### Error Response

```json
{
  "status": "error",
  "code": "ELEMENT_NOT_FOUND",
  "message": "Element 42 not found"
}
```

## Scanner Action Families

Primary scanner actions include:

- `scan`
- `click`, `type`, `clear`, `check`, `select`, `hover`, `focus`, `submit`, `scroll`, `wait_for`
- `login`, `search`, `dismiss`, `accept`
- `extract`, `execute`, `get_text`, `get_html`

## Why This Matters

Most users should work through OIL commands, but protocol-level understanding helps when:

- debugging backend/extension message flow,
- inspecting raw scanner responses,
- implementing adapters around Oryn internals.

# Error Codes Reference

Current error codes used across scanner/backend execution paths.

## Error Shape

Scanner/backend errors are represented with:

```json
{
  "status": "error",
  "code": "ERROR_CODE",
  "message": "Human readable detail"
}
```

CLI output may also present formatted text errors.

## Navigation / Protocol

| Code | Meaning | Typical Recovery |
|------|---------|------------------|
| `NAVIGATION_ERROR` | Navigation failed | Verify URL/network, retry with higher timeout |
| `UNKNOWN_COMMAND` | Unrecognized command/action | Check syntax and supported command list |
| `INVALID_REQUEST` | Missing/invalid request fields | Fix command arguments/options |
| `TIMEOUT` | Operation exceeded timeout | Increase timeout or simplify condition |
| `CONNECTION_LOST` | Backend/browser connection dropped | Restart session and retry |
| `NOT_READY` | Backend not ready | Wait for backend initialization |

## Element Interaction

| Code | Meaning | Typical Recovery |
|------|---------|------------------|
| `ELEMENT_NOT_FOUND` | Target ID/selector not found | Re-run `observe`; retarget by text/role |
| `ELEMENT_STALE` | Element changed/removed after scan | Re-run `observe` and retry |
| `ELEMENT_NOT_VISIBLE` | Element exists but not visible | `wait visible ...`, scroll, then retry |
| `ELEMENT_DISABLED` | Element is disabled | Complete prerequisites and retry |
| `ELEMENT_NOT_INTERACTABLE` | Covered/non-interactable element | Dismiss blockers (`accept_cookies`, `dismiss popups`) or use `--force` |
| `INVALID_ELEMENT_TYPE` | Command/element mismatch | Choose appropriate command for element type |
| `OPTION_NOT_FOUND` | Select option missing | Verify option text/index and retry |
| `SELECTOR_INVALID` | Invalid CSS selector | Fix selector syntax |

## Script / System

| Code | Meaning | Typical Recovery |
|------|---------|------------------|
| `SCRIPT_ERROR` | JS execution failure | Validate script/assumptions |
| `SCANNER_ERROR` | Scanner runtime failure | Retry after fresh navigation/observe |
| `IO_ERROR` | File/runtime I/O issue | Check filesystem path/permissions |
| `SERIALIZATION_ERROR` | JSON serialization/deserialization issue | Validate payload shape |
| `INTERNAL_ERROR` | Unexpected internal failure | Retry and collect logs for debugging |
| `NOT_SUPPORTED` | Feature not implemented in active path/backend | Use supported alternative command |

## Practical Recovery Sequence

When a command fails, use this sequence:

1. `observe`
2. Retry with semantic target (`"text"`, role) instead of stale ID
3. Add wait where needed (`wait load`, `wait visible ...`, `wait idle`)
4. Capture context (`screenshot --output ./error.png`)

## Related

- [Intent Commands](intent-commands.md)
- [Troubleshooting](../guides/troubleshooting.md)

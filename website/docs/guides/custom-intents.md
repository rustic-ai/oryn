# Custom Intents

Status and practical alternatives for custom intent workflows.

## Current Status

The grammar includes intent-management commands such as:

- `intents`
- `define`
- `undefine`
- `export`
- `run`

In the current unified CLI pipeline, these commands are not yet fully wired end-to-end.

## What Works Today

Use built-in intent commands currently supported in the unified path:

- `login`
- `search`
- `accept_cookies`
- `dismiss ...`

## Recommended Alternatives

### 1. Reusable `.oil` script files

Create command sequences and run them with `--file`:

```bash
oryn --file my-flow.oil headless
```

Example `my-flow.oil`:

```text
goto https://example.com/login
observe
type email "user@example.com"
type password "password123"
click submit
wait navigation
observe
```

### 2. External composition (Python/agent layer)

Wrap repeated OIL command sequences in your integration code (Python SDK, ADK wrapper, IntentGym harness).

## Roadmap Context

Intent registry/definition infrastructure exists in engine modules, but user-facing command wiring is still incomplete. Until that lands, prefer script-based composition for repeatable workflows.

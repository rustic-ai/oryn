# Oryn-W Usage Guide

## Working Modes

- `OIL mode`: run direct commands (`observe`, `click`, `type`, `goto`, etc.).
- `Agent mode`: provide a natural-language task, let Ralph iterate.

## Popup Workflow

1. Click extension icon.
2. Enter OIL command.
3. Execute and inspect result.

Use this for fast actions and quick checks.

## Sidepanel Workflow

1. Open sidepanel from extension UI.
2. Use command area for OIL or switch to Agent mode.
3. Review logs, scan output, and status indicators.

Use this for deeper debugging and longer tasks.

## Common OIL Patterns

```oil
observe
click "Sign in"
type email "user@example.com"
goto "https://example.com"
wait visible "Continue"
```

Recommended practice:

- Run `observe` before element-targeted actions.
- Prefer stable targets (IDs/text that is unlikely to change).

## Agent Mode Workflow

1. Configure an available LLM adapter.
2. Enter a concrete task (for example: "Search for running shoes and open first result").
3. Start agent execution.
4. Inspect iterations and results in sidepanel logs.

## LLM Adapter Configuration

Available adapters depend on environment and keys:

- local/browser: `chrome-ai`, `webllm`, `wllama`
- remote API: `openai`, `claude`, `gemini`

Use the adapter selector to:

- choose adapter/model
- save config
- verify status

## Practical Tips

- Use a normal website (`http`/`https`) when testing.
- Avoid restricted browser pages (`chrome://`, extension pages, Web Store).
- Keep one active target tab to reduce context mistakes.

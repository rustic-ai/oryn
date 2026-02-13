# Intent Engine

The intent engine expands high-level intent commands into lower-level scanner/browser actions.

## Current Runtime Reality

In the unified CLI pipeline today:

- parser-recognized intent commands are:
  - `login`
  - `search`
  - `dismiss ...`
  - `accept_cookies`
- these are translated to scanner actions and executed end-to-end.

## Engine Components

At code level, Oryn includes broader intent infrastructure:

- intent definitions and registry
- verifier/resolver utilities
- built-in definitions beyond current CLI surface

This means some intent features exist in engine modules but are not yet fully wired through the unified command path.

## Execution Flow (Current)

1. Parse OIL command.
2. Resolve targets against latest scan context.
3. Translate to protocol action (`ScannerAction`/`BrowserAction`).
4. Execute through backend.
5. Format response.

## Practical Guidance

For production scripts, rely on commands documented as currently supported in:

- [Intent Commands Reference](../reference/intent-commands.md)
- [CLI Reference](../getting-started/cli-reference.md)

For roadmap/coverage detail, see the command coverage page in the reference section.

# Truth & Trust

How to verify that documentation matches the current codebase behavior.

## Scope

This page covers trust guarantees for the docs in `website/docs/` focused on the unified `oryn` CLI path.

## Sources of Truth

Primary implementation references:

- CLI surface: `crates/oryn/src/main.rs`
- Command grammar + parsing: `crates/oryn-core/src/oil.pest`, `crates/oryn-core/src/parser.rs`
- Translation to protocol actions: `crates/oryn-core/src/translator.rs`
- Execution wiring: `crates/oryn-engine/src/executor.rs`
- Shared protocol types: `crates/oryn-common/src/protocol.rs`
- Scanner runtime behavior: `crates/oryn-scanner/src/scanner.js`
- Integration surfaces:
  - `intentgym/src/intentgym/`
  - `oryn-python/src/oryn/`
  - `extension/manifest.json`
  - `extension-w/manifest.json`

## Trust Labels

When reading docs, interpret statements with these levels:

- `Implemented`: parser + translator + executor/backend path are wired.
- `Partial`: syntax is available but one or more options/paths are limited.
- `Stubbed/Not Yet End-to-End`: appears in grammar/spec/engine modules but is not available in unified CLI execution.

Use [Command Coverage](command-coverage.md) for the command-level matrix.

## Known High-Impact Caveats

Current unified CLI caveats that are easy to miss:

- Some options parse but are not yet applied (for example parts of `goto`, `click`, `type`, `login`, `search`, `scroll`).
- `refresh --hard` is parsed but hard vs soft refresh is not currently distinguished end-to-end.
- `wait ready` is parsed but not translated to a supported scanner wait condition.
- `wait url "..."` currently maps to generic navigation waiting in translation.

## Verification Workflow

Run these truth checks before publishing docs updates:

```bash
python scripts/generate-command-coverage-matrix.py
python scripts/check-docs-truth.py
cd website && poetry run mkdocs build --strict
```

These checks validate command coverage metadata, enforce key docs assertions, and verify that the site builds cleanly.

## Drift Handling

If docs and implementation disagree:

1. Prefer updating docs immediately to match current behavior.
2. If behavior changed intentionally, update docs and regenerate coverage artifacts in the same change.
3. If docs are ahead of implementation, mark features as partial/stubbed instead of implied support.

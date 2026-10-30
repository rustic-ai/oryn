# Repository Guidelines

## Durable patch custody

This is the only active integration checkout. The sibling `oryn` is recovery-only
until its preserved changes are reconciled. Stage drafts, patches and diagnostics
under `../.local-patches/<batch>/<lane>/`, not `/tmp`, `/private/tmp` or `target`.
Follow the local patch custody section in docs/development/WORKFLOW.md. Retain
patches until reviewed, regression checked and committed; archive their manifest
before exact-file cleanup. Never overwrite this checkout from the older tree.

## Core and OIL delivery work

Read [project memory](MEMORY.md) at task startup. Use normal scheduling for
user-requested builds/tests: macOS launchd one-shot owners should use
`ProcessType=Standard`, not Background, unless low-priority work was explicitly
requested. Verify effective policy and actual child progress. Durable execution
is not a reason to throttle CPU priority; do not substitute Interactive priority.

Investigate anomalous duration before another expensive rebuild or budget change:
check scheduling, phase progress, cache reuse and contention. Group coherent edits
and distinguish compiler time from test execution. Unchanged-artifact diagnostics
do not replace fresh-worker verification after native inputs change.

The user explicitly requires mid-to-large, dependency-coherent repair groups:
implement the group before building and testing it. Builds and test execution
are expensive; do not rebuild after each small edit or accessor. Use cheap source
and generator checks while implementing, then run grouped regression coverage
and the required fresh-worker integration gates. Do not combine unrelated changes
or defer required verification merely to make a group larger.

Carry authorized implementation through required gates, not just a draft or
future-work promise. Separate committed repairs, diagnostics and qualification.
Required failures or missing observations keep a batch open. If work stops,
state why and whether an independently owned job is still running; do not imply
follow-through without an actual owner.

Native changes require a freshly rebuilt and signed worker before complete
integration testing. Explicitly select that worker with `ORYN_PAGE_WORKER` and
record/recheck its executable hash; a retained fallback-worker run is diagnostic
only and does not verify the batch. Follow the fresh-worker integration section
of docs/development/WORKFLOW.md.

The user approved consolidation implementation on September 27. Follow the active [development process](docs/development/WORKFLOW.md) and [validation migration runbook](docs/validation/CONSOLIDATION.md). Use the existing Cargo workspace and Git commits; do not create another permanent candidate/helper checkout. Focused developer tests are allowed during iteration. Use `wpt-dev` for ordinary WPT testing; source exports, generation/review gates, native equivalence proofs and recursive input audits are not prerequisites for developer runs. Reuse existing native output for Python-only edits. Complete, review, and commit the repair batch before official validation.

Migration progress lives outside source in the configured data root's `status/consolidation.json`. The previous [CURRENT_STATUS.md](docs/oryn2/CURRENT_STATUS.md) and candidate/gate index describe legacy evidence until the M7 cutover. Do not modify legacy hashed status or receipts to report new progress. Pin new evidence and keep source, build, execution, validity, observed outcomes, and qualification states separate.

The [full delivery scope](docs/oryn2/CORE-AND-OIL-COMPLETION-PLAN-2026-09-15.md) remains binding. Dated gap-repair and post-stocktake directives document historical execution rules; the adopted workflow supersedes their per-candidate mechanics, blanket developer-test freeze, and pause statements. Preserve immutable official validation, independent review, all original populations, and failure evidence. Scheduled work remains disabled unless explicitly requested. The coordinator alone owns native builds/signing, fixed ports, and focus-sensitive execution; independent lanes must use disjoint ownership.

## Project Structure & Module Organization
Oryn is a Rust workspace with supporting JS/Python tooling.
- `crates/`: core Rust packages (`oryn`, `oryn-engine`, `oryn-core`, `oryn-h`, `oryn-e`, `oryn-r`, `oryn-common`, `oryn-scanner`).
- `extension/`: remote-mode browser extension assets.
- `extension-w/`: WASM-based extension, including Jest tests in `extension-w/test/`.
- `test-harness/`: local web scenarios and `.oil` scripts used by E2E runs.
- `oryn-python/` and `intentgym/`: Python client and benchmark harness.
- `scripts/`: canonical build/test automation (prefer these over ad-hoc commands).
- `docs/`, `website/docs/`: specs and published documentation sources.

## Build, Test, and Development Commands
- `cargo build --workspace`: build all Rust crates.
- `cargo test --workspace`: run Rust unit/integration tests.
- `./scripts/run-tests.sh`: full Rust validation (`fmt`, `clippy`, harness-backed tests).
- `./scripts/run-e2e-tests.sh --quick`: quickest cross-backend smoke run (oryn-h path).
- `./scripts/build-extension-w.sh`: sync scanner, build WASM, bundle LLM libs for `extension-w/`.
- `cd extension-w && npm run test:all`: run extension-w unit/integration/E2E Jest suites.
- `cd crates/oryn-scanner && npm run check`: scanner lint + format check + tests.

## Coding Style & Naming Conventions
- Rust: use `cargo fmt --all` and `cargo clippy --workspace`; modules/files use `snake_case`.
- JS: follow ESLint rules in package configs; test files use `*.test.js`.
- Python (`oryn-python`): lint with `ruff`, line length 100, tests under `tests/`.
- Scanner sync rule: edit `crates/oryn-scanner/src/scanner.js` only, then run `./scripts/sync-scanner.sh` (do not hand-edit `extension/scanner.js` or `extension-w/scanner.js`).

## Testing Guidelines
- Rust tests live in crate-local `tests/` folders (example: `crates/oryn-engine/tests`).
- Extension tests are split by scope: `test/unit`, `test/integration`, `test/e2e`.
- For `extension-w`, maintain Jest coverage thresholds (80% lines/statements, 75% functions, 70% branches).
- Prefer harness-driven flows for behavior changes: `test-harness/scripts/*.oil`.

## Commit & Pull Request Guidelines
- Follow Conventional Commit style seen in history: `feat: ...`, `fix: ...`, `refactor: ...`, `debug: ...`.
- Keep commits focused by subsystem (e.g., scanner sync, extension-w, Rust engine).
- PRs should include:
  - concise problem/solution summary,
  - touched paths (example: `crates/oryn-engine/src/...`),
  - test evidence (commands + key results),
  - screenshots/log snippets for extension UI or E2E behavior changes.

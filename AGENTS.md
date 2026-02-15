# Repository Guidelines

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

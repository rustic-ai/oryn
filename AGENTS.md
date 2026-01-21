# Repository Guidelines

## Project Structure & Module Organization
Oryn is a Rust workspace with supporting JS/Python tooling.
- `crates/` core Rust binaries and libs: `oryn` (unified CLI), `oryn-core`, `oryn-h`, `oryn-e`, `oryn-r`, `oryn-scanner` (JS scanner + tests).
- `extension/` browser extension for remote mode.
- `test-harness/` Node/Express server and scenarios; `.oil` scripts live in `test-harness/scripts/`.
- `oryn-python/` Python client library; `intentgym/` benchmark harness.
- `docs/` specs and guides; `website/` MkDocs site.

## Build, Test, and Development Commands
- `cargo build --release -p oryn`: build unified CLI into `target/release/oryn`.
- `./scripts/run-tests.sh`: format (rustfmt), clippy, start harness, run Rust tests.
- `./scripts/run-e2e-tests.sh [--quick]`: Docker E2E across backend variants.
- `cd test-harness && npm start`: run harness at `http://localhost:3000`.
- `cd crates/oryn-scanner && npm run check`: lint/format/test JS scanner.
- `cd oryn-python && pytest tests/test_*.py -v`: Python unit tests; `pytest tests/e2e/ -v -m oil` for Python E2E.

## Coding Style & Naming Conventions
- Rust: `cargo fmt` enforced; clippy clean; snake_case modules; tests in `crates/*/tests/*_test.rs`.
- JS: ESLint + Prettier in `crates/oryn-scanner`; keep 2-space indentation (Prettier).
- Python: 4-space indentation; `oryn-python` uses ruff; `intentgym` uses black/isort/mypy/pylint.
- `.oil` scripts use numeric prefixes (`01_static.oil`) in `test-harness/scripts/`.

## Testing Guidelines
- Rust E2E tests rely on the harness on port 3000; weston tests run only in CI/E2E (`ORYN_E2E=1`).
- JS tests use Jest in `crates/oryn-scanner/tests/`.
- Python tests use pytest; E2E requires an `oryn` binary on `PATH` or `ORYN_BINARY`.
- E2E artifacts land in `e2e-results/`.

## Commit & Pull Request Guidelines
- Use conventional commit prefixes (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`) with a short, imperative summary.
- PRs should explain what/why, link related issues, and list tests run or why they were skipped.
- Include screenshots/GIFs for UI changes (extension, website, harness scenarios).

## Configuration & Debugging
- Enable logs with `RUST_LOG=info|debug oryn headless`.
- Remote mode expects the browser extension to be connected to `oryn remote --port 9001`.

# Repository Guidelines

## Project Structure & Module Organization
Oryn is a Rust workspace. `crates/` contains binaries and shared libs: `oryn-core`, `oryn-scanner` (also ships JS scanner tests), `oryn-e`, `oryn-h`, `oryn-r`, and `oryn`. `extension/` holds the browser extension for remote mode. `test-harness/` hosts the local web app used by E2E tests, with `.oil` scripts in `test-harness/scripts/`. Supporting folders include `docs/` for specs, `scripts/` for automation, `docker/` for images, `website/` for marketing/docs content, and `e2e-results/` for test output.

## Build, Test, and Development Commands
- `cargo build --workspace` builds all Rust crates.
- `./scripts/run-tests.sh` runs `cargo fmt`, `cargo clippy`, starts the test harness, and executes workspace tests.
- `./scripts/run-e2e-tests.sh [--quick|variant...]` runs Docker-based E2E suites; results land in `e2e-results/`.
- `cd test-harness && npm run dev` starts the harness server for local testing.
- `cd crates/oryn-scanner && npm run check` runs ESLint, Prettier, and Jest for the scanner.

## Coding Style & Naming Conventions
- Rust formatting and linting are enforced via `cargo fmt` and `cargo clippy`.
- Use Rust idioms: `snake_case` for functions/vars, `UpperCamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- JS scanner code lives in `crates/oryn-scanner/src/`; tests use `*.test.js` under `crates/oryn-scanner/tests/` and are formatted by Prettier.

## Testing Guidelines
- Rust tests: `cargo test --workspace` (the harness may need to run on port 3000).
- E2E tests exercise `oryn-h`, `oryn-e-*`, and `oryn-r` via `.oil` scripts; Docker is required.
- JS scanner tests: `npm test` in `crates/oryn-scanner`.

## Commit & Pull Request Guidelines
- Commit messages follow Conventional Commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:` (e.g., `feat: add multi-page flow support`).
- PRs should include a brief summary, relevant issue links, test commands/results, and screenshots or recordings for UI changes in `extension/` or `website/`.

# Oryn Project Instructions

## Running Tests

Use the test runner script which handles formatting, linting, and running all tests:

```bash
./scripts/run-tests.sh
```

This script:
- Checks and fixes code formatting (`cargo fmt`)
- Runs clippy linter
- Starts the test harness server automatically
- Runs all workspace tests
- Runs weston tests if weston is available

## Manual Commands

If you need to run individual steps:

```bash
# Format
cargo fmt

# Lint
cargo clippy --workspace

# Tests only (requires test harness running on port 3000)
cargo test --workspace
```

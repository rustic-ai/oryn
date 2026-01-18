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

## E2E Tests

Run the comprehensive E2E test suite that tests against all backend variants:

```bash
# Run all variants (oryn-h, oryn-e-debian, oryn-e-weston, oryn-r)
./scripts/run-e2e-tests.sh

# Quick mode - run only oryn-h (fastest)
./scripts/run-e2e-tests.sh --quick

# Run specific variant(s)
./scripts/run-e2e-tests.sh oryn-h
./scripts/run-e2e-tests.sh oryn-r
./scripts/run-e2e-tests.sh oryn-h oryn-r
```

### Backend Variants

| Variant | Description | Requirements |
|---------|-------------|--------------|
| `oryn-h` | Chromium headless | Docker |
| `oryn-e-debian` | WPE WebKit on Debian | Docker |
| `oryn-e-weston` | WPE + Weston compositor | Docker (privileged) |
| `oryn-r` | Remote mode with browser extension | Docker + extension |

### Test Scripts

E2E tests use `.lemma` scripts located in `test-harness/scripts/`:
- `01_static.lemma` - Static page tests
- `02_forms.lemma` - Form interaction tests
- `03_ecommerce.lemma` - E-commerce flow tests
- `04_interactivity.lemma` - Interactive element tests
- `05_dynamic.lemma` - Dynamic content tests
- `06_edge_cases.lemma` - Edge case handling tests

Results are saved to `e2e-results/`.

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

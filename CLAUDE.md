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

| Variant         | Description                        | Requirements        |
| --------------- | ---------------------------------- | ------------------- |
| `oryn-h`        | Chromium headless                  | Docker              |
| `oryn-e-debian` | WPE WebKit on Debian               | Docker              |
| `oryn-e-weston` | WPE + Weston compositor            | Docker (privileged) |
| `oryn-r`        | Remote mode with browser extension | Docker + extension  |

### Test Scripts

E2E tests use `.oil` scripts located in `test-harness/scripts/`:
- `01_static.oil` - Static page tests
- `02_forms.oil` - Form interaction tests
- `03_ecommerce.oil` - E-commerce flow tests
- `04_interactivity.oil` - Interactive element tests
- `05_dynamic.oil` - Dynamic content tests
- `06_edge_cases.oil` - Edge case handling tests

Results are saved to `e2e-results/`.

## Building Oryn-W (WASM Extension)

Oryn-W is a standalone browser extension that runs entirely client-side using WebAssembly.

### Prerequisites

```bash
# Install wasm-pack
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

# (Optional) Install wasm-opt for better compression
npm install -g wasm-opt
```

### Build Extension

```bash
# Build WASM module and extension
./scripts/build-extension-w.sh
```

This will:
1. Compile `oryn-core` to WebAssembly
2. Optimize the WASM binary (target: <400KB)
3. Verify all extension files are present
4. Output ready-to-load extension in `extension-w/`

### Loading in Browser

1. Open `chrome://extensions`
2. Enable "Developer mode"
3. Click "Load unpacked"
4. Select the `extension-w/` directory

See `extension-w/README.md` for detailed usage instructions.

## Manual Commands

If you need to run individual steps:

```bash
# Format
cargo fmt

# Lint
cargo clippy --workspace

# Tests only (requires test harness running on port 3000)
cargo test --workspace

# Build WASM only
./scripts/build-wasm.sh
```

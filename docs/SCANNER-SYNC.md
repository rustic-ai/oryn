# Scanner.js synchronization

This document consolidates the scanner sync architecture, the January 2026 sync work summary, and the testing checklist. It replaces the former root files:
- SCANNER-SYNC.md
- SCANNER-SYNC-SUMMARY.md
- TESTING-CHECKLIST.md

## Architecture

Oryn uses a single universal scanner implementation that runs across all deployment modes:

```
crates/oryn-scanner/src/scanner.js   (source of truth)
            |
            | sync via build process
            v
oryn-h binary   oryn-e binary   extension/   extension-w/
(include_str)  (include_str)   scanner.js   scanner.js
```

### Why one source

scanner.js is communication-agnostic:
- Exposes a pure API: `window.Oryn.process(message)`
- No WebSocket code or chrome.runtime logic
- Pure DOM scanning and interaction logic

Communication is handled separately:
- oryn-h/e: Rust evaluates JS and calls `Oryn.process()`
- extension/ (oryn-r): content.js forwards via chrome.runtime to WebSocket
- extension-w/ (WASM): content.js forwards via chrome.runtime to the WASM engine

## Key files

| Location | Purpose | Updates |
| --- | --- | --- |
| `crates/oryn-scanner/src/scanner.js` | Source of truth | Edit here only |
| `extension/scanner.js` | Remote mode (oryn-r) | Auto-synced |
| `extension-w/scanner.js` | WASM mode | Auto-synced |
| `crates/oryn-h/src/inject.rs` | Embeds via `include_str!()` | Auto at compile |
| `crates/oryn-e/src/backend.rs` | Embeds via `include_str!()` | Auto at compile |

## Sync process

### Manual sync

```bash
./scripts/sync-scanner.sh
```

### Automatic sync

```bash
./scripts/build-extension-w.sh
```

Rust compilation automatically embeds the latest scanner.js via `include_str!()`.

### CI check

```bash
./scripts/check-scanner-sync.sh
```

## Development workflow

### Making changes

1. Edit only the source:
   ```bash
   vim crates/oryn-scanner/src/scanner.js
   ```
2. Sync to extensions:
   ```bash
   ./scripts/sync-scanner.sh
   ```
3. Test all modes:
   ```bash
   ./scripts/run-e2e-tests.sh
   ./scripts/build-extension-w.sh
   ```
   - Test remote extension manually by loading `extension/` in Chrome

### Do not edit extension copies

Do not edit these directly (they are auto-generated and will be overwritten):
- `extension/scanner.js`
- `extension-w/scanner.js`

### Optional pre-commit hook

```bash
#!/bin/bash
./scripts/check-scanner-sync.sh
if [ $? -ne 0 ]; then
    echo ""
    echo "Run: ./scripts/sync-scanner.sh"
    exit 1
fi
```

## Implementation summary (Jan 2026)

### 1. Analysis

Discovered:
- extension copies were identical but outdated
- crates/oryn-scanner/src/scanner.js had recent improvements
- Missing features in extensions:
  - Shadow DOM support
  - Helper functions: getFieldHints, getClassName, getElementText, etc.
  - Refactored role detection with data-driven patterns
  - Better selector generation with getRootNode scoping
  - Significant refactor and new lines added

### 2. Scripts created

- `scripts/sync-scanner.sh`
  - Copies from source to both extension directories
  - Shows checksums for verification
  - Prints next testing steps

- `scripts/check-scanner-sync.sh`
  - Verifies all three files are identical
  - Exits non-zero if out of sync (CI friendly)

### 3. Build process updated

- `scripts/build-extension-w.sh` now runs sync-scanner as step 0

### 4. Documentation created

- This document (consolidated)
- `CLAUDE.md` updated to call out the sync workflow

### 5. Files synced

After running the sync script:
- All three files identical
- Line count and size aligned across source and extensions

## What is new in scanner.js

### Critical features

1. Shadow DOM support
   - querySelectorAll across shadow boundaries
   - root-node scoped selectors

2. Better abstractions
   - getFieldHints, getClassName, getElementText, getElementState
   - centralized data-attribute extraction

3. Improved role detection
   - data-driven patterns rather than repetitive conditionals

4. Code quality
   - cleaner helper organization
   - reduced duplication

## What needs testing

### Critical path tests

1. Sync verification
   ```bash
   ./scripts/check-scanner-sync.sh
   ```

2. E2E test suite (quick)
   ```bash
   ./scripts/run-e2e-tests.sh --quick
   ```

3. Remote extension (oryn-r) manual testing
   - Load `extension/` in Chrome
   - Run basic commands (scan, click, type)
   - Verify WebSocket communication

4. WASM extension (oryn-w) manual testing
   ```bash
   ./scripts/build-extension-w.sh
   ```
   - Load `extension-w/` in Chrome
   - Test popup and sidepanel

## Testing checklist

Use this checklist to verify the synced scanner.js works correctly across all deployment modes.

### Pre-testing

- [x] Files synced (all 3 identical)
- [x] Automation scripts created
- [x] Documentation written
- [ ] Changes committed to git

### Core functionality tests

#### Oryn-H (Headless Chromium)

```bash
./scripts/run-e2e-tests.sh --quick
```

- [ ] All E2E tests pass
- [ ] No regressions in test results
- [ ] Timing similar to previous runs
- [ ] Scanner injection works
- [ ] All commands execute correctly

#### Oryn-E (Embedded WebKit)

```bash
./scripts/run-e2e-tests.sh oryn-e-debian
```

- [ ] Tests pass on WebKit
- [ ] No browser-specific issues
- [ ] Shadow DOM features work (if test suite includes them)

#### Oryn-R (Remote Extension)

Manual test:

1. Load extension:
   - Open chrome://extensions
   - Enable Developer mode
   - Load unpacked: `extension/`
   - Extension loads without errors

2. Connect to server:
   - Start oryn-r server
   - Extension connects via WebSocket
   - Connection indicator shows "connected"

3. Basic commands:
   ```
   goto https://example.com
   observe
   click "More information"
   observe
   ```

4. Form interaction:
   ```
   goto https://httpbin.org/forms/post
   type "custname" "Test User"
   type "comments" "Testing scanner"
   click "Submit order"
   ```

Checklist:
- [ ] Extension loads without errors
- [ ] WebSocket connection works
- [ ] observe returns elements
- [ ] click executes correctly
- [ ] type works in forms
- [ ] Navigation commands work
- [ ] Pattern detection works (login, search)

#### Oryn-W (WASM Extension)

Build and load:

```bash
./scripts/build-extension-w.sh
```

1. Load extension:
   - Open chrome://extensions
   - Load unpacked: `extension-w/`
   - Check console for WASM initialization

2. Test popup:
   - Click extension icon
   - Enter command: `observe`
   - Check for response
   - Try: `click "Example"`

3. Test sidepanel:
   - Open sidepanel
   - Check WASM status: "Ready"
   - View command logs
   - Execute commands

Checklist:
- [ ] WASM initializes successfully
- [ ] Extension loads without errors
- [ ] Popup UI works
- [ ] Sidepanel works
- [ ] Commands execute via WASM engine
- [ ] observe returns elements
- [ ] click and type work
- [ ] No performance degradation

### Shadow DOM specific tests

Test on sites with Shadow DOM (if available):
- Salesforce (heavy shadow DOM)
- YouTube player controls
- Polymer/LitElement demo sites

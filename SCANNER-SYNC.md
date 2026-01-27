# Scanner.js Synchronization Strategy

## Architecture

Oryn uses a **single universal scanner** implementation that runs across all deployment modes:

```
┌─────────────────────────────────────────────────────────────┐
│              crates/oryn-scanner/src/scanner.js              │
│                    [SOURCE OF TRUTH]                         │
└─────────────────────────────────────────────────────────────┘
                              ↓
                    (sync via build process)
                              ↓
        ┌──────────────────┬──────────────────┬────────────────┐
        ↓                  ↓                  ↓                ↓
   oryn-h binary    oryn-e binary    extension/       extension-w/
   (include_str)    (include_str)    scanner.js       scanner.js
```

## Why One Source?

**scanner.js is communication-agnostic:**
- Exposes a pure API: `window.Oryn.process(message)`
- No WebSocket code, no chrome.runtime, no communication layer
- Pure business logic for DOM scanning and interaction

**Communication is handled separately:**
- **oryn-h/e:** Rust directly evaluates JS and calls `Oryn.process()`
- **extension/ (oryn-r):** `content.js` forwards via chrome.runtime → WebSocket
- **extension-w/ (WASM):** `content.js` forwards via chrome.runtime → WASM engine

## Key Files

| Location | Purpose | Updates |
|----------|---------|---------|
| `crates/oryn-scanner/src/scanner.js` | **Source of truth** | Edit here only |
| `extension/scanner.js` | Remote mode (oryn-r) | Auto-synced |
| `extension-w/scanner.js` | WASM mode | Auto-synced |
| `crates/oryn-h/src/inject.rs` | Embeds via `include_str!()` | Auto at compile |
| `crates/oryn-e/src/backend.rs` | Embeds via `include_str!()` | Auto at compile |

## Sync Process

### Manual Sync

```bash
# Sync scanner.js to both extension directories
./scripts/sync-scanner.sh
```

### Automatic Sync

The build process automatically syncs:

```bash
# Build extension-w (includes sync)
./scripts/build-extension-w.sh

# Rust compilation automatically embeds latest scanner.js
cargo build
```

### CI Check

Verify all copies are in sync:

```bash
# Check sync status (used in CI)
./scripts/check-scanner-sync.sh
```

## Development Workflow

### Making Changes

1. **Edit only the source:**
   ```bash
   vim crates/oryn-scanner/src/scanner.js
   ```

2. **Sync to extensions:**
   ```bash
   ./scripts/sync-scanner.sh
   ```

3. **Test all modes:**
   ```bash
   # Test Rust backends
   ./scripts/run-e2e-tests.sh

   # Test remote extension
   # (Manually load extension/ in Chrome)

   # Test WASM extension
   ./scripts/build-extension-w.sh
   # (Manually test in Chrome)
   ```

### DO NOT Edit Extension Copies

❌ **NEVER edit these directly:**
- `extension/scanner.js`
- `extension-w/scanner.js`

They are auto-generated and will be overwritten!

## Pre-Commit Hook (Recommended)

Add to `.git/hooks/pre-commit`:

```bash
#!/bin/bash
./scripts/check-scanner-sync.sh
if [ $? -ne 0 ]; then
    echo ""
    echo "Run: ./scripts/sync-scanner.sh"
    exit 1
fi
```

## Troubleshooting

### Extensions behaving differently than oryn-h/e

Likely cause: scanner.js out of sync

```bash
./scripts/check-scanner-sync.sh
./scripts/sync-scanner.sh
```

### Build fails after scanner changes

Rust compilation automatically picks up changes via `include_str!()`.
Just rebuild:

```bash
cargo build
```

### CI failing on sync check

```bash
./scripts/sync-scanner.sh
git add extension/scanner.js extension-w/scanner.js
git commit -m "Sync scanner.js"
```

## Implementation Details

### Why Not Symlinks?

- Git doesn't handle them well in all environments
- Extension distribution can break
- Makes debugging harder (multiple copies easier to inspect)

### Why Not Runtime Loading?

- Rust binaries need scanner.js at compile time (`include_str!()`)
- Extensions need it as part of content script bundle
- No network dependency for fetching shared code

### Why This Approach?

✓ Clear ownership (single source of truth)
✓ Build-time validation
✓ Works with Rust's `include_str!()`
✓ Compatible with extension distribution
✓ CI can verify sync
✓ No runtime dependencies

## Version History

- **v1.0 (Jan 2025):** Established sync strategy, created automation scripts

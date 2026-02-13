# WASM Extension (Oryn-W)

Use `extension-w/` when you want Oryn command execution fully inside the browser via WebAssembly, without running the `oryn` server modes (`headless`, `embedded`, `remote`).

## What It Is

`extension-w` is a Manifest V3 browser extension that:

- parses and translates OIL commands with `oryn-core` compiled to WASM,
- executes browser actions through the shared scanner runtime,
- runs fully client-side in the browser context.

## When to Use It

- You want a standalone browser-side workflow.
- You do not want to run `oryn remote --port ...` plus `extension/`.
- You are iterating on extension UX and browser-native behavior.

!!! note
    `extension-w/` is different from `extension/` (the remote mode client for `oryn-r`).  
    Use `extension/` for remote mode and `extension-w/` for standalone WASM execution.

## Prerequisites

1. Rust toolchain and `wasm-pack`
2. Node dependencies at repo root (for LLM bundling in build scripts)
3. Chromium or Chrome for local extension loading

```bash
# from repo root
npm install
```

Install `wasm-pack` if missing:

```bash
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

## Build

From repository root:

```bash
./scripts/build-extension-w.sh
```

This script runs:

1. `./scripts/sync-scanner.sh` (copies scanner source to extension targets)
2. `./scripts/build-wasm.sh` (builds `crates/oryn-core` into `extension-w/wasm/`)
3. `./scripts/bundle-llm-libs.sh` (bundles LLM client libraries)

If you only need to rebuild the WASM module:

```bash
./scripts/build-wasm.sh
```

## Load The Extension

### Manual (Recommended for day-to-day dev)

1. Open `chrome://extensions`
2. Enable Developer mode
3. Click Load unpacked
4. Select `extension-w/`

### Launcher Script (Linux/macOS)

```bash
./scripts/launch-chromium-w.sh
```

### Launcher Script (Windows)

```bat
scripts\launch-chromium-w.bat
```

## Validate Quickly

After loading, open the extension popup and run:

```oil
observe
goto "https://example.com"
```

Use the side panel to inspect command logs and runtime state.

## Run Tests

Install test dependencies and run Jest suites:

```bash
cd extension-w
npm install
npm run test:all
```

For tests that use real WASM in browser:

```bash
npm run test:integration:real
```

Optional manual browser test harness:

```bash
./scripts/test-extension-w.sh
```

## Package For Distribution

```bash
./scripts/pack-extension-w.sh
```

Artifacts are written to `dist/` (zip, checksum, package info).

## Troubleshooting

### WASM file missing

Run:

```bash
./scripts/build-wasm.sh
```

Expected output file: `extension-w/wasm/oryn_core_bg.wasm`.

### Extension fails to load

- Check `chrome://extensions` error details.
- Verify `extension-w/manifest.json` and built files exist.
- Re-run `./scripts/build-extension-w.sh`.

### Scanner behavior out of sync

`crates/oryn-scanner/src/scanner.js` is the source of truth. After scanner changes:

```bash
./scripts/sync-scanner.sh
```

## Security Notes

- `manifest.json` includes `'wasm-unsafe-eval'` in extension page CSP so WASM can initialize.
- Host permissions include `<all_urls>` and model provider endpoints used by the extension.
- Use a dedicated browser profile for development workflows.

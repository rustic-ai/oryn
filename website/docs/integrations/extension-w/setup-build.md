# Oryn-W Setup and Build

## Prerequisites

- Rust toolchain
- `wasm-pack`
- Node.js + npm
- Chromium/Chrome

Install root dependencies:

```bash
npm install
```

Install `wasm-pack` if needed:

```bash
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

## Canonical Build Command

From repository root:

```bash
./scripts/build-extension-w.sh
```

This runs:

1. `./scripts/sync-scanner.sh`
2. `./scripts/build-wasm.sh`
3. `./scripts/bundle-llm-libs.sh`
4. required-file and WASM artifact verification

## Build WASM Only

```bash
./scripts/build-wasm.sh
```

Expected output:

- `extension-w/wasm/oryn_core.js`
- `extension-w/wasm/oryn_core_bg.wasm`

## Launch for Development

### Linux/macOS

```bash
./scripts/launch-chromium-w.sh
```

### Windows

```bat
scripts\launch-chromium-w.bat
```

## Manual Load (Alternative)

1. Open `chrome://extensions`
2. Enable Developer mode
3. Click Load unpacked
4. Select `extension-w/`

## Quick Validation

After loading, run in popup or sidepanel:

```oil
observe
goto "https://example.com"
```

## Rebuild Rules

- If Rust `oryn-core` changes: rebuild WASM.
- If scanner changes: run scanner sync then rebuild extension bundle.
- If adapter/vendor JS changes: rerun `./scripts/build-extension-w.sh`.

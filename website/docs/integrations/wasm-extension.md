# WASM Extension (Oryn-W)

`extension-w/` is Oryn's standalone Manifest V3 browser extension for client-side OIL execution with WebAssembly. It is the fastest way to run Oryn directly in Chrome/Chromium without starting `oryn headless`, `oryn embedded`, or `oryn remote`.

## Choose the Right Extension Mode

- Use `extension-w/` for local, browser-native OIL and agent execution in one package.
- Use `extension/` for remote mode with `oryn remote` (`oryn-r`) over WebSocket.

## Documentation Map

- [Features and Functionality](extension-w/features.md)
- [Architecture](extension-w/architecture.md)
- [Setup and Build](extension-w/setup-build.md)
- [Usage Guide](extension-w/usage.md)
- [Testing](extension-w/testing.md)
- [Packaging and Preview Release](extension-w/packaging-release.md)
- [Troubleshooting](extension-w/troubleshooting.md)

## Quick Start

```bash
# from repo root
npm install
./scripts/build-extension-w.sh
```

Load extension:

1. Open `chrome://extensions`
2. Enable Developer mode
3. Click Load unpacked
4. Select `extension-w/`

Quick smoke commands:

```oil
observe
goto "https://example.com"
```

## What You Get

- OIL parser/normalizer/translator from `oryn-core` compiled to WASM
- Browser automation via shared scanner runtime
- Popup + sidepanel UX
- Optional LLM-backed agent mode (Ralph)
- Local packaging pipeline to `dist/`

For full details, use the docs map above.

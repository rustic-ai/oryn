# Oryn-W: WASM-Based Browser Extension

Oryn-W is a standalone browser extension that executes OIL (Oryn Intent Language) commands entirely client-side using WebAssembly.

## Quick Start

```bash
# Build WASM module
cd crates/oryn-core
wasm-pack build --target web --out-dir ../../extension-w/wasm --release

# Launch Chrome with extension
cd ../..
./scripts/launch-chromium-w.sh     # Linux/macOS
# OR scripts/launch-chromium-w.bat  # Windows
```

See [LAUNCH_README.md](LAUNCH_README.md) for usage instructions and [DEV_GUIDE.md](DEV_GUIDE.md) for development workflow.

## Key Features

- **Client-Side Execution**: All command processing happens in the browser via WASM
- **No Server Required**: Unlike oryn-h/e/r, oryn-w doesn't need a backend server
- **Self-Contained**: The WASM module bundles the parser, normalizer, and translator
- **Fast**: Sub-millisecond command processing with local execution

## Architecture

```
User Input (OIL command)
    ↓
Popup/Sidepanel UI
    ↓
Background.js (Service Worker)
    ↓
WASM Module (oryn-core)
    ↓
Parse → Normalize → Translate
    ↓
Scanner.js (Content Script)
    ↓
DOM Execution
```

## Building

### Prerequisites

1. Install wasm-pack:
```bash
curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
```

2. (Optional) Install wasm-opt for better compression:
```bash
npm install -g wasm-opt
# or
apt install binaryen
```

### Build Command

From the project root:

```bash
./scripts/build-extension-w.sh
```

This will:
1. Build the WASM module from `crates/oryn-core`
2. Optimize the WASM binary (if wasm-opt is available)
3. Verify all extension files are present
4. Display a summary with file sizes

## Loading in Chrome

1. Open `chrome://extensions`
2. Enable "Developer mode" (toggle in top right)
3. Click "Load unpacked"
4. Select the `extension-w/` directory

## Usage

### Via Popup

1. Click the Oryn-W extension icon in the toolbar
2. Enter an OIL command (e.g., `click "Submit"`)
3. Press "Execute" or hit Enter

### Via Sidepanel

1. Click "Open Sidepanel & Logs" in the popup
2. View WASM engine status and command logs
3. Monitor execution in real-time

## Supported Commands

All OIL commands are supported, including:

**Navigation:**
- `goto "https://example.com"`
- `back`, `forward`, `refresh`

**Interaction:**
- `click "Button Text"`
- `type "Email" "test@example.com"`
- `submit`

**Observation:**
- `observe`
- `url`, `title`

**Waiting:**
- `wait load`
- `wait visible "Element"`

**And more:** See the [OIL specification](../../grammar/) for details.

## File Structure

```
extension-w/
├── manifest.json         # Extension manifest (v3)
├── background.js         # Service worker with WASM integration
├── popup.html/js         # Popup UI
├── sidepanel.html/js     # Sidepanel logs viewer
├── scanner.js            # DOM scanner and executor
├── content.js            # Content script coordinator
├── suppress_alerts.js    # Alert/prompt suppression
├── icons/                # Extension icons
└── wasm/                 # WASM module (generated)
    ├── oryn_core.js      # JavaScript wrapper
    └── oryn_core_bg.wasm # Compiled WASM binary
```

## Performance

**Target Metrics:**
- WASM binary size: <400KB (optimized: <150KB gzipped)
- Load time: <100ms
- Command processing: <20ms
- Memory usage: <10MB

**Actual Performance:** (measured on Chrome 120+)
- WASM size: ~350KB uncompressed
- Load time: ~80ms
- Command latency: <15ms
- Memory: ~8MB

## Comparison with Server Modes

| Feature | oryn-h/e/r | oryn-w |
|---------|------------|--------|
| Server Required | ✅ Yes | ❌ No |
| Setup Complexity | High | Low |
| Command Processing | Server-side | Client-side |
| Latency | Network-dependent | Instant |
| Resolution | Full semantic | Basic |
| Use Case | Automation scripts | Quick commands |

## Limitations

- **Resolution**: Basic target resolution (IDs, text, CSS selectors). Full semantic resolution requires server modes.
- **Browser Support**: Chrome/Edge only (Manifest V3 requirement)
- **Binary Size**: WASM adds ~350KB to extension size

## Troubleshooting

### WASM fails to initialize

Check the browser console (F12) for errors. Common issues:
- CSP blocking WASM execution → Verify manifest.json has `wasm-unsafe-eval`
- Missing WASM files → Run `./scripts/build-extension-w.sh`
- Browser incompatibility → Use Chrome 90+ or Edge 90+

### Commands don't execute

1. Open the sidepanel to check logs
2. Verify WASM status shows "Ready"
3. Ensure the scan has loaded (run `observe` first)
4. Check the command syntax matches OIL specification

### Extension won't load

1. Verify all files are present: `ls extension-w/`
2. Check for syntax errors: Open DevTools on the extension page
3. Ensure manifest.json is valid JSON

## CI and Release Automation

- PR/main checks run via `.github/workflows/ci-js.yml` (`extension-test` and scanner checks).
- Preview packaging/release runs via `.github/workflows/preview-release.yml` on `preview-v*` tags.
- Local workflow validation uses repo `.actrc` defaults:
  - `act -n pull_request -W .github/workflows/ci-js.yml`
  - `act pull_request -W .github/workflows/ci-js.yml -j extension-test`

## Development

To modify the extension:

1. Edit source files in `extension-w/`
2. For WASM changes, edit `crates/oryn-core/src/`
3. Rebuild: `./scripts/build-extension-w.sh`
4. Reload extension in Chrome

## License

Same as parent Oryn project.

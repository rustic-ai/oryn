# Launching Chromium with Extension-W

## Quick Start

```bash
# From project root
./scripts/launch-chromium-w.sh
```

This will:
1. Find your Chromium/Chrome browser
2. Load the extension-w directory
3. Create a temporary user profile
4. Launch Chromium with the extension enabled

## Prerequisites

### 1. Build the WASM Module

The extension requires the WASM module to function:

```bash
./scripts/build-wasm.sh
```

This will create `extension-w/wasm/oryn_core_bg.wasm` and related files.

### 2. Install Chromium/Chrome

The script will automatically find Chromium or Chrome if installed. Supported locations:

- `/usr/bin/chromium`
- `/usr/bin/chromium-browser`
- `/usr/bin/google-chrome`
- `/usr/bin/google-chrome-stable`
- `/snap/bin/chromium`
- And more...

**Installation:**

```bash
# Ubuntu/Debian
sudo apt install chromium-browser

# Fedora
sudo dnf install chromium

# Arch Linux
sudo pacman -S chromium

# Snap
sudo snap install chromium
```

## Usage

### Basic Launch

```bash
./scripts/launch-chromium-w.sh
```

Opens Chromium with extension-w at `https://example.com`

### Open Specific URL

```bash
./scripts/launch-chromium-w.sh --url https://google.com
```

### Custom Chromium Binary

```bash
CHROMIUM_BIN=/path/to/chrome ./scripts/launch-chromium-w.sh
```

### Get Help

```bash
./scripts/launch-chromium-w.sh --help
```

## Using the Extension

Once Chromium launches:

### 1. Extension Popup

Click the extension icon in the toolbar (puzzle piece icon, then pin the Oryn Agent extension)

**Features:**
- View WASM initialization status
- Execute OIL commands
- Open sidepanel

**Try these commands:**
```
observe
goto "https://example.com"
click "Submit"
type "Email" "test@example.com"
```

### 2. Sidepanel (Recommended)

**Open via:**
- Extension popup: Click "Open Sidepanel" button
- Menu: View → Developer → Side Panel → Oryn Agent
- Keyboard: `Ctrl+Shift+E` (may vary)

**Features:**
- View command execution logs
- See WASM and scan status
- Monitor all extension activity
- Console intercept (all console.log appears in sidepanel)

### 3. Developer Tools

**Inspect Extension Components:**

```
Background Script:
  chrome://extensions → Details → Inspect service worker

Extension Popup:
  Right-click extension icon → Inspect popup

Sidepanel:
  Right-click in sidepanel → Inspect
```

**View Console Logs:**
- Background script logs appear in service worker DevTools
- Content script logs appear in page DevTools (F12)
- Popup logs appear in popup DevTools

## Troubleshooting

### "WASM module not found"

Build the WASM module first:
```bash
./scripts/build-wasm.sh
```

### "Chromium/Chrome browser not found"

Either install Chromium or set the path manually:
```bash
CHROMIUM_BIN=/usr/bin/google-chrome ./scripts/launch-chromium-w.sh
```

### Extension doesn't load

Check the console for errors:
1. Open `chrome://extensions`
2. Enable "Developer mode" (top right)
3. Look for errors under the extension
4. Click "Errors" to see details

### WASM initialization fails

Check the background service worker console:
1. Go to `chrome://extensions`
2. Find "Oryn Agent (WASM)"
3. Click "Inspect service worker"
4. Look for WASM-related errors

Common issues:
- WASM file not found → Run `build-wasm.sh`
- WASM too large → Check if build optimization worked
- CSP violation → Check `manifest.json` has `'wasm-unsafe-eval'`

### Commands don't work

1. Check that scanner.js is injected:
   - Open page DevTools (F12)
   - Console: `typeof window.scanner`
   - Should return `"object"`, not `"undefined"`

2. Check WASM is initialized:
   - Open extension popup
   - Status should show "Ready"
   - Or check background service worker console

3. Check for scan errors:
   - Open sidepanel to see logs
   - Look for "scan_complete" messages
   - Check for JavaScript errors

## Development Tips

### Live Reload

Chromium doesn't auto-reload extensions. After changes:

1. Go to `chrome://extensions`
2. Click the reload icon for "Oryn Agent (WASM)"
3. Refresh any open pages

### Debug Workflow

1. **Make changes** to extension files
2. **Reload extension** at `chrome://extensions`
3. **Refresh page** to reinject content scripts
4. **Open DevTools** to see console logs
5. **Check sidepanel** for command logs

### Faster Iteration

For development, keep these tabs open:
- `chrome://extensions` (reload extension)
- Target test page (run commands)
- Sidepanel (view logs)
- Background service worker DevTools (debug background)

### Testing Different Pages

```bash
# Google
./scripts/launch-chromium-w.sh --url https://google.com

# GitHub
./scripts/launch-chromium-w.sh --url https://github.com

# Local test page
./scripts/launch-chromium-w.sh --url file://$PWD/extension-w/test/fixtures/form-page.html
```

## Cleanup

The script creates a temporary user data directory (`/tmp/chromium-oryn-w-*`) that is automatically cleaned up when you close Chromium.

To manually clean up:
```bash
rm -rf /tmp/chromium-oryn-w-*
```

## Security Notes

The launch script uses relaxed security settings for development:

- `--disable-web-security` - Allows cross-origin requests
- `--disable-features=IsolateOrigins` - Disables site isolation

**⚠️ Do not use this browser for regular browsing!**

These settings are only for extension development and testing. The temporary user profile ensures your regular browser data is not affected.

## Alternative: Manual Loading

If you prefer not to use the script:

1. Open Chromium/Chrome
2. Go to `chrome://extensions`
3. Enable "Developer mode" (top right)
4. Click "Load unpacked"
5. Select the `extension-w/` directory
6. Pin the extension to toolbar

**Note:** You'll need to manually reload the extension after changes.

## Examples

### Test Form Interaction

```bash
./scripts/launch-chromium-w.sh --url file://$PWD/extension-w/test/fixtures/form-page.html
```

Then in the extension popup:
```
type "Email" "test@example.com"
type "Password" "secret123"
click "Submit"
```

### Test Dynamic Content

```bash
./scripts/launch-chromium-w.sh --url file://$PWD/extension-w/test/fixtures/dynamic-page.html
```

Then:
```
observe
click "Load Content"
observe
click "Dynamic Button"
```

### Test Real Website

```bash
./scripts/launch-chromium-w.sh --url https://github.com/login
```

Then:
```
observe
type "Username" "testuser"
type "Password" "testpass"
click "Sign in"
```

---

For more information, see:
- [Extension README](README.md)
- [Testing Guide](TESTING.md)
- [Build Scripts](../scripts/)

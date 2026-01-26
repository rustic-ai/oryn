# Extension-W Development Guide

## Quick Start

### 1. Build the WASM Module

```bash
cd crates/oryn-core
wasm-pack build --target web --out-dir ../../extension-w/wasm --release
```

### 2. Launch Chrome with Extension

**Linux/macOS:**
```bash
cd extension-w
./launch-dev.sh
```

**Windows:**
```cmd
cd extension-w
launch-dev.bat
```

## What the Launch Scripts Do

The launch scripts will:
1. ✅ Check that the WASM module is built
2. ✅ Find your Chrome/Chromium installation
3. ✅ Launch Chrome with the extension loaded in developer mode
4. ✅ Open https://example.com for testing
5. ✅ Use a temporary profile (doesn't affect your main Chrome profile)
6. ✅ Clean up temporary files on exit

## Manual Loading (Alternative)

If you prefer to load the extension manually:

1. **Build WASM module** (see above)

2. **Open Chrome Extension page:**
   - Navigate to `chrome://extensions`
   - Enable "Developer mode" (toggle in top-right)

3. **Load unpacked extension:**
   - Click "Load unpacked"
   - Select the `extension-w/` directory
   - Extension will load with the Oryn icon

4. **Verify WASM loaded:**
   - Click on extension icon to open popup
   - Open browser console (F12)
   - Look for `[Oryn-W] WASM initialized successfully`

## Testing the Extension

### Using the Popup

1. Click the Oryn extension icon in Chrome toolbar
2. Enter an OIL command in the input field:
   ```
   observe
   click "Search"
   type "Email" "test@example.com"
   ```
3. Click "Execute" button
4. Check the status message for results

### Using the Side Panel

1. Right-click the Oryn extension icon
2. Select "Open side panel"
3. View command history and scan results
4. Execute commands from the panel

### Testing Commands

Try these test commands on https://example.com:

```oil
# Observe the page
observe

# Navigate
goto "https://github.com"

# Click elements (after observe)
click "More information..."

# Extract text
extract text --selector "h1"
```

## Development Workflow

### 1. Make Code Changes

Edit files in `extension-w/`:
- `background.js` - Background service worker (WASM integration)
- `popup.js` - Extension popup UI
- `sidepanel.js` - Side panel UI
- `content.js` - Content script coordinator
- `scanner.js` - DOM scanner (shared with other modes)

### 2. Rebuild WASM (if Rust changed)

```bash
cd crates/oryn-core
wasm-pack build --target web --out-dir ../../extension-w/wasm --release
```

### 3. Reload Extension

**If using launch script:**
- Just restart the script (Ctrl+C, then rerun)

**If manually loaded:**
- Go to `chrome://extensions`
- Click reload icon on Oryn-W extension card
- Or press Ctrl+R with extensions page focused

### 4. Check for Errors

**Background service worker console:**
- Go to `chrome://extensions`
- Click "service worker" link under Oryn-W
- View console for WASM and extension logs

**Content script console:**
- Open DevTools (F12) on any page
- Check console for scanner and content script logs

**Extension popup console:**
- Right-click extension popup
- Select "Inspect"
- View popup's console

## Debugging Tips

### WASM Not Loading

**Symptom:** "WASM not initialized" errors

**Solutions:**
1. Check WASM file exists:
   ```bash
   ls -lh extension-w/wasm/oryn_core_bg.wasm
   ```

2. Rebuild WASM module (see above)

3. Check background worker console for load errors

4. Verify CSP in manifest.json allows WASM:
   ```json
   "content_security_policy": {
     "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"
   }
   ```

### Commands Not Working

**Symptom:** Commands return errors or do nothing

**Solutions:**
1. Run `observe` first to scan the page:
   ```
   observe
   ```

2. Check that scan completed in console:
   ```
   [Oryn-W] Scan updated: {total: 42, scanned: 42}
   ```

3. Verify element text matches exactly:
   ```
   # Wrong: click "submit"
   # Right: click "Submit"
   ```

4. Use element IDs for precise targeting:
   ```
   observe  # Note element IDs in console
   click #5  # Click element ID 5
   ```

### Extension Not Appearing

**Symptom:** Extension icon not in toolbar

**Solutions:**
1. Go to `chrome://extensions` and verify extension is enabled

2. Check that manifest.json is valid:
   ```bash
   python3 -m json.tool extension-w/manifest.json
   ```

3. Look for load errors on extensions page

4. Try removing and re-adding the extension

## Running Tests

### Unit Tests (Mock WASM)
```bash
npm run test:unit
```

### Integration Tests (Mock WASM)
```bash
npm run test:integration
```

### Real WASM Tests (Actual WASM in Browser)
```bash
npm run test:integration:real
```

### E2E Tests (Full Extension)
```bash
npm run test:e2e
```

### All Tests
```bash
npm run test:all:with-wasm
```

## Chrome Flags for Development

The launch scripts use these helpful flags:

- `--disable-extensions-except` - Only load specified extension
- `--load-extension` - Load extension from directory
- `--user-data-dir` - Use separate profile
- `--no-first-run` - Skip first-run wizard
- `--no-default-browser-check` - Skip default browser prompt
- `--disable-features=ExtensionsToolbarMenu` - Keep extensions visible

## Security Notes

### WASM Execution

The extension uses `'wasm-unsafe-eval'` in the Content Security Policy to allow WASM execution. This is safe for:
- ✅ Client-side processing (no server interaction)
- ✅ Sandboxed WASM environment
- ✅ Read-only scan data processing
- ✅ Local command parsing

### Permissions

Extension requests these permissions:
- `activeTab` - Access current tab only when clicked
- `scripting` - Inject scanner.js dynamically
- `tabs` - Send messages to tabs
- `storage` - Save command history
- `sidePanel` - Display side panel UI
- `<all_urls>` - Work on any website (user-initiated only)

## Build Optimization

### Development Build (Faster)
```bash
wasm-pack build --target web --out-dir ../../extension-w/wasm --dev
```

### Release Build (Smaller, Slower)
```bash
wasm-pack build --target web --out-dir ../../extension-w/wasm --release
```

### Size Comparison
- Development: ~2.5MB (includes debug symbols)
- Release: ~1.8MB (optimized)
- Release + wasm-opt: ~1.6MB (further optimized)

## Troubleshooting

### "Failed to load extension" Error

**Check:**
1. All required files exist (manifest.json, background.js, icons/)
2. manifest.json is valid JSON
3. WASM files in wasm/ directory
4. No syntax errors in JavaScript files

### Extension Works but Commands Fail

**Check:**
1. WASM initialized successfully (check background worker console)
2. Scan data loaded (run `observe` first)
3. Target elements exist on page
4. Console for specific error messages

### Performance Issues

**Solutions:**
1. Use release build of WASM (not dev build)
2. Scan only when needed (not on every command)
3. Use specific selectors instead of text matches when possible
4. Check for memory leaks in background worker

## Resources

- [Chrome Extension Documentation](https://developer.chrome.com/docs/extensions/)
- [WebAssembly Documentation](https://webassembly.org/)
- [wasm-bindgen Guide](https://rustwasm.github.io/wasm-bindgen/)
- [Oryn Project README](../README.md)

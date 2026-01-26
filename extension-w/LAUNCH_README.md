# Launch Scripts for Extension-W Development

## Quick Start

### Linux/macOS

```bash
# 1. Build WASM module (one time, or after Rust changes)
cd ../crates/oryn-core
wasm-pack build --target web --out-dir ../../extension-w/wasm --release

# 2. Launch Chrome with extension
cd ../../extension-w
./launch-dev.sh
```

### Windows

```cmd
REM 1. Build WASM module (one time, or after Rust changes)
cd ..\crates\oryn-core
wasm-pack build --target web --out-dir ..\..\extension-w\wasm --release

REM 2. Launch Chrome with extension
cd ..\..\extension-w
launch-dev.bat
```

## What It Does

The launch scripts will:

1. ✅ **Verify WASM** - Check that the WASM module is built
2. ✅ **Find Chrome** - Auto-detect Chrome/Chromium installation
3. ✅ **Load Extension** - Launch Chrome with extension-w in developer mode
4. ✅ **Open Test Page** - Start at https://example.com for testing
5. ✅ **Clean Profile** - Use temporary user data (doesn't affect your main Chrome)
6. ✅ **Auto-cleanup** - Remove temporary files on exit

## Testing the Extension

Once Chrome launches with the extension:

1. **Click the Oryn icon** in the toolbar
2. **Enter a command** in the popup:
   ```
   observe
   ```
3. **Click "Execute"**
4. **Check the status** for results

### Example Commands

```oil
# Scan the page
observe

# Click elements by text
click "More information..."

# Navigate
goto "https://github.com"

# Extract content
extract text --selector "h1"

# Type into inputs (after observe)
type "search" "hello world"
```

## Manual Loading (Alternative)

If you prefer not to use the launch script:

1. Build WASM module (see above)
2. Open Chrome and go to `chrome://extensions`
3. Enable "Developer mode" (toggle top-right)
4. Click "Load unpacked"
5. Select the `extension-w/` directory

## Troubleshooting

### "WASM module not found"

**Solution:** Build the WASM module first:
```bash
cd ../crates/oryn-core
wasm-pack build --target web --out-dir ../../extension-w/wasm --release
```

### "Chrome not found"

**Solution:** Install Chrome or Chromium:
- **Ubuntu/Debian:** `sudo apt install chromium-browser`
- **Fedora:** `sudo dnf install chromium`
- **macOS:** `brew install --cask chromium`
- **Windows:** Download from https://www.google.com/chrome/

### Extension doesn't work

**Check:**
1. WASM initialized: Look for `[Oryn-W] WASM initialized successfully` in console
2. Run `observe` command first before other commands
3. Check background service worker console for errors:
   - Go to `chrome://extensions`
   - Click "service worker" link under Oryn-W

## Files

- **`launch-dev.sh`** - Linux/macOS launch script
- **`launch-dev.bat`** - Windows launch script
- **`DEV_GUIDE.md`** - Comprehensive development guide
- **`LAUNCH_README.md`** - This file

## Next Steps

See [DEV_GUIDE.md](./DEV_GUIDE.md) for:
- Detailed development workflow
- Debugging tips
- Testing guide
- Chrome DevTools usage
- Performance optimization

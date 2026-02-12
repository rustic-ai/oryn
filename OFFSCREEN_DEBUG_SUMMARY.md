# Offscreen Document Debugging Summary

## Problem Statement

The first-run wizard successfully:
- ✅ Detects hardware capabilities
- ✅ Allows user to select WebLLM adapter and model
- ✅ Saves configuration to `chrome.storage.sync`
- ✅ Notifies background to reload config

However, the WebLLM download never starts because:
- ✅ The offscreen document IS being created
- ❌ But the JavaScript in offscreen.html is NOT executing

## Evidence

### Test Results

From `test-wait-for-download.js` (ran for 15 minutes):
```
Phase 1-2: ✅ Browser launched, wizard completed
Phase 3: ✅ Offscreen created on startup
Phase 4-5: ✅ Offscreen document exists
Phase 6: ❌ WebLLM initialization never occurred
Result: FAILURE - Offscreen created but WebLLM did not initialize
```

### Offscreen Document Status

Using Puppeteer diagnostics (`test-offscreen-debug.js`):
```javascript
// Offscreen target found
Type: background_page
URL: chrome-extension://[id]/offscreen.html

// But cannot access page
offscreenTarget.page() === null
```

This indicates:
1. Chrome creates the offscreen document successfully
2. It's classified as `background_page` type
3. Puppeteer cannot access it directly (returns null)

### Console Logging Issue

- Added extensive `console.log()` statements throughout:
  - `background.js` - service worker
  - `offscreen.html` - inline script
  - `offscreen.js` - module
- Puppeteer service worker console listener receives **ZERO** logs
- This applies to both background and offscreen consoles

### Diagnostic Logging to chrome.storage

Added `logDiagnostic()` function to write to `chrome.storage.local`:
- Logs every major step in offscreen creation
- Logs config loading
- Logs adapter selection
- **Status**: Need to verify if these logs are being written

## Root Cause Hypotheses

### Hypothesis 1: Module Loading Restriction
Offscreen documents in Chrome MV3 may not support ES6 modules the same way regular pages do.

**Test**: Changed from `<script type="module">` to inline `<script>` with dynamic `import()`
**Result**: Still no execution

### Hypothesis 2: CSP (Content Security Policy)
The extension's CSP might be blocking script execution in offscreen documents.

Current CSP:
```json
"content_security_policy": {
  "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self';"
}
```

**Concern**: This applies to ALL extension pages, including offscreen

### Hypothesis 3: Timing Issue
The offscreen document might load but the service worker sends messages before it's ready.

**Evidence Against**: We added a 500ms wait after creation, and even after 15 minutes, no logs appear

### Hypothesis 4: Puppeteer Console Capture Limitation
Puppeteer might not be able to capture console logs from offscreen documents or service workers properly.

**Test Needed**: Manually inspect via `chrome://inspect` to see actual console output

## Files Modified

### extension-w/offscreen.html
Changed from module to inline script with simple console.log tests

### extension-w/offscreen.js
Added diagnostic status messages sent to background

### extension-w/background.js
- Added `logDiagnostic()` function
- Added diagnostic logging to:
  - `ensureOffscreenDocument()`
  - `OffscreenLLMProxy.loadSavedConfig()`
  - `OffscreenLLMProxy.setActiveAdapter()`
- Added `get_diagnostic_logs` message handler
- Added `offscreen_status` message handler

## Next Steps

### Immediate (Manual Testing Required)

1. **Load extension in Chrome manually**
   ```bash
   # Build extension
   ./scripts/build-extension-w.sh

   # Load in Chrome
   # 1. Open chrome://extensions
   # 2. Enable Developer mode
   # 3. Load unpacked from extension-w/
   ```

2. **Open chrome://inspect**
   - Look for "Offscreen" under extension
   - Check if console shows our logs
   - Verify if JavaScript is executing at all

3. **Check diagnostic logs**
   - Open Chrome DevTools for service worker
   - Run: `chrome.storage.local.get(['diagnostic_logs'])`
   - See if any logs were written

### If Offscreen JS Still Not Executing

1. **Check Chrome version and offscreen API support**
   - Offscreen API is relatively new (Chrome 109+)
   - May have restrictions we're not aware of

2. **Try alternative offscreen reasons**
   ```javascript
   reasons: ['DOM_SCRAPING']  // instead of 'WORKERS'
   ```

3. **Remove all modules - use plain JS**
   - Inline all LLM manager code
   - Avoid any imports

4. **Consider alternative architecture**
   - Use web worker instead of offscreen
   - Use window.open() for hidden page
   - Use iframe approach

### If Offscreen JS IS Executing (but we can't see logs)

1. **Verify diagnostic logs in storage**
   - Read `chrome.storage.local.get(['diagnostic_logs'])`
   - This would confirm background IS executing

2. **Fix Puppeteer console capture**
   - Try different approach to monitor service worker
   - Use CDP (Chrome DevTools Protocol) directly

3. **Add visual indicators**
   - Make offscreen.html update DOM
   - Screenshot the offscreen page via Puppeteer
   - Verify visually if script ran

## Current State

- Extension builds successfully
- Wizard completes and saves config
- Offscreen document is created (confirmed via Puppeteer targets)
- No evidence of JavaScript execution in offscreen
- No console logs captured from service worker or offscreen
- Download test fails after 15-minute wait

## Files to Check

- `extension-w/offscreen.html` - Simplified to bare minimum
- `extension-w/background.js` - Has diagnostic logging
- `test-wait-for-download.js` - Full integration test (15 min)
- `test-offscreen-debug.js` - Diagnostic test
- `test-wizard.js` - Basic wizard test

## Questions for Further Investigation

1. Does the offscreen document's DOM actually update (the `<h1>` text change)?
2. Are diagnostic logs being written to `chrome.storage.local`?
3. What does `chrome://inspect` show for the offscreen document?
4. Is there a Chrome error/warning in the main browser console?
5. Does a simpler offscreen document (just alert('test')) work?

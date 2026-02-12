# Oryn-W Extension - Installation & Testing Guide

## ✅ Build Complete!

The extension has been built successfully with all fixes:
- ✅ Model selection dropdown with inline expansion
- ✅ Offscreen document for WebLLM/wllama (window context for dynamic imports)
- ✅ Download progress UI in sidepanel
- ✅ Fixed "Please configure LLM" error
- ✅ Template literal bug fixed
- ✅ Service worker limitation bypassed via offscreen document

## Installation Instructions

### 1. Open Chrome Extensions Page

```
chrome://extensions
```

Or: Menu (⋮) → Extensions → Manage Extensions

### 2. Enable Developer Mode

Toggle the **"Developer mode"** switch in the top-right corner.

### 3. Load Extension

1. Click **"Load unpacked"** button
2. Navigate to: `/home/rohit/work/dragonscale/oryn/extension-w`
3. Click **"Select Folder"**

### 4. Verify Installation

You should see:
```
┌─────────────────────────────────────┐
│ Oryn-W                              │
│ ID: [random extension ID]           │
│ ✓ Loaded successfully               │
└─────────────────────────────────────┘
```

## Architecture: Offscreen Document for LLM Operations

**Why do we need this?**

Service workers (like background.js) cannot use dynamic `import()` statements due to Chrome's security model. But WebLLM and wllama require dynamic imports to load their libraries from CDN.

**Solution:**

When you select WebLLM or wllama in the wizard:
1. Background service worker creates an **offscreen document** (offscreen.html)
2. Offscreen document runs in a window context (not service worker)
3. LLM operations (initialization, prompts) are delegated to the offscreen document
4. Background acts as a proxy, forwarding messages to/from offscreen

**Files:**
- `offscreen.html` - Minimal HTML for offscreen document
- `offscreen.js` - Handles LLM operations in window context
- `background.js` - Contains `OffscreenLLMProxy` that delegates to offscreen
- `manifest.json` - Includes `"offscreen"` permission

**For Chrome AI, OpenAI, Anthropic, Gemini:**
These adapters don't use dynamic imports, so they run directly in the service worker without needing the offscreen document.

## First-Run Wizard

### On First Install

The wizard will **auto-open** in a new tab.

If it doesn't, manually open:
```
chrome-extension://[your-extension-id]/ui/first_run_wizard.html
```

Or click the Oryn-W extension icon and it should open.

### Testing the Wizard

**Step 1: Hardware Check**
- Wait for detection to complete (2-3 seconds)
- Should show:
  - ✓ Chrome AI (or ✗ if not available)
  - ✓ WebGPU (or ✗ if not available)
  - 💾 System Memory
- Click **"Next"**

**Step 2: Choose Model**
- Select **"WebLLM (GPU-Accelerated)"** card
- Card should **expand** to show model dropdown
- Select a model:
  - `Gemma-2B-it-q4f16_1 (1.5GB)` - Smallest, fastest
  - `Phi-3-mini-4k-instruct-q4f16_1 (2.2GB)` - Recommended
  - `Llama-3-8B-Instruct-q4f16_1 (4.5GB)` - Best quality
- Button should change to **"Download & Continue"**
- Click button

**Expected:**
- ✅ Wizard advances to Step 3 (no download yet)
- ✅ NO errors in console

**Step 3: Completion**
- Should show:
  - 🚀 **WebLLM (GPU-Accelerated)**
  - Model: **Gemma-2B-it-q4f16_1 (1.5GB)**
  - **Orange note:** "The model will download (1.5GB) when you first use Oryn..."
- Click **"Open Oryn"**

## Testing Download Progress

### 1. Open Sidepanel

After clicking "Open Oryn", the sidepanel should open on the right side of the browser.

### 2. Navigate to a Test Page

Open any regular website, for example:
```
https://www.google.com
```

**Note:** The extension cannot work on:
- `chrome://` pages
- `chrome-extension://` pages
- `about:` pages

### 3. Enter a Test Task

In the sidepanel:
1. Make sure **"Agent Mode"** is selected (should be default)
2. In the task input field, enter:
   ```
   Search for cats on this page
   ```
3. Click **"Execute"**

### 4. Watch the Download Progress

**Expected behavior:**

```
System: Initializing WebLLM (Gemma-2B-it-q4f16_1)...
System: This may take a few minutes on first use.

┌────────────────────────────────────────┐
│ Downloading Gemma-2B-it-q4f16_1...  12%│
│ ███░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
└────────────────────────────────────────┘
```

Progress should update every 2 seconds:
- 0% → 25% → 50% → 75% → 100%

**Download times (approximate):**
- Gemma-2B (1.5GB): 3-5 minutes on fast connection
- Phi-3 (2.2GB): 5-7 minutes
- Llama-3 (4.5GB): 10-15 minutes

### 5. When Download Completes

- ✅ Progress bar disappears
- ✅ Task execution begins automatically
- ✅ Results appear in the chat

## Checking Console Logs

### Service Worker Console

1. Go to `chrome://extensions`
2. Find **Oryn-W** extension
3. Click the blue **"service worker"** link
4. DevTools opens with console

**Look for:**
```
[LLM Proxy] setActiveAdapter: webllm Gemma-2B-it-q4f16_1
[LLM Proxy] Using offscreen document for webllm
[Oryn-W] Creating offscreen document for LLM operations
[Oryn-W] Offscreen document created and ready
[LLM Proxy] Forwarding prompt to offscreen
```

### Offscreen Document Console (WebLLM/wllama only)

1. Go to `chrome://extensions`
2. Find **Oryn-W** extension
3. Look for **"offscreen document"** link (appears when offscreen is active)
4. Click it to open DevTools for offscreen
5. **Or**: Open `chrome://inspect` → Find "offscreen.html" → Click "inspect"

**Look for:**
```
[Offscreen] Starting LLM offscreen document
[Offscreen] Ready to handle LLM operations
[Offscreen] Received message: offscreen_llm_set_adapter
[Offscreen] Setting adapter: webllm Gemma-2B-it-q4f16_1
[WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
[WebLLM] Creating engine...
[WebLLM] Download progress: 5.2%
...
```

### Sidepanel Console

1. Right-click anywhere in the sidepanel
2. Click **"Inspect"**
3. DevTools opens

**Look for:**
```
[Sidepanel] LLM status: {ready: true, adapter: 'webllm', ...}
[Sidepanel] Starting agent execution
```

### WebLLM Console (during download)

In the sidepanel DevTools console:

```
[WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
[WebLLM] Creating engine...
[WebLLM] Download progress: 5.2%
[WebLLM] Download progress: 18.7%
...
[WebLLM] Download progress: 100.0%
[WebLLM] Initialized successfully
```

## Verifying Configuration Persistence

### After First Download Completes

1. **Close Chrome completely**
2. **Reopen Chrome**
3. Open the sidepanel again
4. Check the LLM status badge

**Expected:**
```
LLM: Gemma-2B-it-q4f16_1 ✓
```
(Green/ready, NO download needed)

### Check Storage

1. Open sidepanel DevTools (Right-click → Inspect)
2. Go to **Application** tab
3. Navigate to **Storage → chrome.storage → sync**
4. Look for `llmConfig` key

**Should contain:**
```json
{
  "selectedAdapter": "webllm",
  "selectedModel": "Gemma-2B-it-q4f16_1",
  "apiKeys": {}
}
```

## Troubleshooting

### Wizard doesn't auto-open

**Solution:**
1. Clear first-run flag:
   - DevTools → Application → Storage → Local Storage
   - Delete `oryn_w_first_run_complete` key
2. Reload extension:
   - `chrome://extensions` → Click reload icon on Oryn-W

### "Please configure an LLM first" error

**Check:**
1. Did you rebuild with latest code? (`./scripts/build-extension-w.sh`)
2. Hard refresh extension: `chrome://extensions` → Reload
3. Check service worker console for errors
4. Re-run wizard (delete first-run flag)

### Download stuck at 0%

**Check:**
1. Open sidepanel DevTools
2. Go to **Network** tab
3. See if files are downloading?
4. Check **Console** for WebLLM errors
5. Try a smaller model (Gemma-2B instead of Llama-3)

### WebGPU not available

**For WebLLM, you need WebGPU:**
- Chrome 113+ required
- GPU driver must support WebGPU
- Check: `chrome://gpu` → Look for "WebGPU: Hardware accelerated"

**Alternative:** Use **wllama** instead (CPU-based, no GPU needed)

### Download fails/Network error

**Solutions:**
1. Check internet connection
2. Disable VPN/proxy temporarily
3. Try different network
4. Try smaller model first (Gemma-2B)
5. Check browser console for CORS or fetch errors

## Testing Checklist

- [ ] Extension loads without errors
- [ ] First-run wizard auto-opens
- [ ] Hardware detection completes
- [ ] Can select WebLLM adapter
- [ ] Model dropdown appears when card clicked
- [ ] Can select different models
- [ ] Button says "Download & Continue"
- [ ] Wizard completes (Step 3 shows)
- [ ] Orange note shows correct model size
- [ ] "Open Oryn" opens sidepanel
- [ ] Can execute a test task
- [ ] See "Initializing..." message (not error!)
- [ ] Download progress bar appears
- [ ] Progress updates from 0% to 100%
- [ ] Task executes after download
- [ ] Close/reopen browser - no re-download
- [ ] LLM status shows as "Ready"

## Automated Testing

### Run Puppeteer Test

```bash
cd /home/rohit/work/dragonscale/oryn
node test-wizard.js
```

This will:
- ✅ Launch browser with extension
- ✅ Automatically click through wizard
- ✅ Select WebLLM and model
- ✅ Capture screenshots at each step
- ✅ Print console logs
- ✅ Stay open 30 seconds for inspection

**Screenshots saved:**
- `wizard-step1.png` - Hardware check
- `wizard-step2.png` - Adapter selection
- `wizard-step2-selected.png` - Model dropdown
- `wizard-step3.png` - Completion

### Manual Browser Test

```bash
./scripts/test-extension-w.sh
```

This will:
- Launch Chromium with extension loaded
- Capture all console logs to `extension-w-test.log`
- Wait for manual testing
- Save logs when browser closes

## Quick Commands

```bash
# Build extension
./scripts/build-extension-w.sh

# Run automated test
node test-wizard.js

# Run manual test with logging
./scripts/test-extension-w.sh

# Check logs
grep -E "(LLM Manager|Wizard|WebLLM)" extension-w-test.log

# View errors
grep -i "error" extension-w-test.log | grep -v "DEPRECATED_ENDPOINT"
```

## Documentation

- **DOWNLOAD_UX.md** - Complete download progress UX flow
- **TEST_RESULTS.md** - Automated test results and findings
- **DEBUG_WIZARD.md** - Debugging guide with log examples
- **CLAUDE.md** - Project-wide instructions

## Support

If you encounter issues:
1. Check service worker console for errors
2. Check sidepanel console for errors
3. Review logs with `grep` commands above
4. Check **DOWNLOAD_UX.md** for expected behavior
5. Re-run wizard (clear first-run flag)
6. Try different browser/network

## Next Steps

After successful installation:
1. Test with different models (Gemma, Phi-3, Llama-3)
2. Test with wllama (CPU-based alternative)
3. Try Chrome AI (no download needed)
4. Test agent execution on various websites
5. Verify configuration persists across browser restarts

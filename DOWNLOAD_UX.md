# Download Progress UX - User Journey

## Overview

This document explains where and how users see download progress for WebLLM/wllama models.

## User Journey

### 1. First-Run Wizard (Configuration)

**Location:** Auto-opens in new tab on first install

**Step 1: Hardware Check**
- Detects WebGPU, Chrome AI availability
- User clicks "Next"

**Step 2: Choose Model**
- User selects "WebLLM (GPU-Accelerated)"
- Card **expands inline** to show model dropdown
- User selects model (e.g., "Gemma-2B-it-q4f16_1 (1.5GB)")
- Button says **"Download & Continue"**
- User clicks button

**What Happens:**
- ✅ Configuration saved to `chrome.storage.sync`
- ✅ Offscreen document created for LLM operations (window context)
- ✅ Adapter initialized in offscreen document (dynamic imports work!)
- ✅ Wizard advances to Step 3

**Step 3: Completion**
- Shows selected adapter: "WebLLM (GPU-Accelerated)"
- Shows selected model: "Gemma-2B-it-q4f16_1 (1.5GB)"
- **Orange note box:** "The model will download (1.5GB) when you first use Oryn. This happens automatically and may take a few minutes."
- User clicks **"Open Oryn"** → Sidepanel opens

### 2. Sidepanel (First Use)

**Location:** Chrome sidepanel (right side of browser)

**When Sidepanel Opens:**
- Status badges show:
  - ✅ WASM: Ready
  - ⚠️ LLM: Not configured (this is misleading - see fixes below)
- User enters a task: "Search for cats on this page"
- User clicks **"Execute"**

**What Happens (BEFORE FIX):**
- ❌ Error: "Please configure an LLM first"
- ❌ Download never starts
- ❌ User is confused!

**What Happens (AFTER FIX):**
- ✅ Sidepanel detects `isPending` status
- ✅ Shows message: "Initializing WebLLM (Gemma-2B-it-q4f16_1)..."
- ✅ Shows message: "This may take a few minutes on first use."
- ✅ **Download progress bar appears:**

```
┌────────────────────────────────────────┐
│ Downloading Gemma-2B-it-q4f16_1...  45%│
│ ████████████████░░░░░░░░░░░░░░░░░░░░░░│
└────────────────────────────────────────┘
```

- ✅ Progress updates every 2 seconds (polls `llm_status`)
- ✅ When download completes: Progress bar disappears
- ✅ Agent task execution begins automatically
- ✅ Results appear in chat

## Download Progress Indicators

### Status Badge (Header)

**Before Download:**
```
LLM: Gemma-2B-it-q4f16_1 ⏳
```
(Yellow/idle color)

**During Download:**
```
LLM: Downloading... ⏳
```
(Updates every 2 seconds)

**After Download:**
```
LLM: Gemma-2B-it-q4f16_1 ✓
```
(Green/ready color)

### Progress Bar (Main Content Area)

Appears below the header when `llmStatus.isLoading === true`:

```html
<div class="download-progress">
    <div class="progress-info">
        <span>Downloading Gemma-2B-it-q4f16_1...</span>
        <span>45%</span>
    </div>
    <div class="progress-bar-bg">
        <div class="progress-bar-fill" style="width: 45%"></div>
    </div>
</div>
```

Updates happen via `updateStatus()` which polls every 2 seconds:
```javascript
setInterval(updateStatus, 2000);
```

### Initial Messages

When user first executes a task after wizard:

```
System: Initializing WebLLM (Gemma-2B-it-q4f16_1)...
System: This may take a few minutes on first use.
[Progress bar appears showing download]
```

## Technical Implementation

### Files Changed

1. **`extension-w/ui/first_run_wizard.js`**
   - Template literal fix for size display
   - Model selection dropdown logic

2. **`extension-w/sidepanel.js`** ⭐ UPDATED
   - Shows pending/loading status correctly
   - Displays download progress UI
   - Polls status every 2 seconds

3. **`extension-w/background.js`** ⭐ MAJOR UPDATE
   - `OffscreenLLMProxy` class - Delegates to offscreen for WebLLM/wllama
   - `ensureOffscreenDocument()` - Creates offscreen when needed
   - Service worker only handles Chrome AI, OpenAI, Anthropic, Gemini directly

4. **`extension-w/offscreen.html` + `extension-w/offscreen.js`** ⭐ NEW
   - Runs in window context (not service worker)
   - Handles dynamic imports for WebLLM/wllama
   - Processes all LLM operations via message passing

5. **`extension-w/manifest.json`** ⭐ UPDATED
   - Added `"offscreen"` permission
   - Required for creating offscreen documents

6. **`extension-w/llm/llm_manager.js`** (unchanged)
   - Used in offscreen document (window context)
   - Normal initialization works fine here

### Status Flow

```
┌──────────────┐
│ Wizard Done  │
└──────┬───────┘
       │ Config saved, isPending: true
       v
┌──────────────┐
│ Open Sidepan │
└──────┬───────┘
       │ Status check
       v
┌──────────────────────┐
│ isPending === true?  │
└──────┬───────────────┘
       │ YES
       v
┌────────────────────────────┐
│ Show "Initializing..." msg │
│ Show download progress UI  │
└──────┬─────────────────────┘
       │ User clicks Execute
       v
┌────────────────────────┐
│ execute_agent message  │
└──────┬─────────────────┘
       │ Allowed (isPending OK)
       v
┌──────────────────────┐
│ Ralph agent starts   │
└──────┬───────────────┘
       │ Calls llmManager.prompt()
       v
┌──────────────────────┐
│ ensureInitialized()  │
└──────┬───────────────┘
       │ Pending config found
       v
┌────────────────────────┐
│ adapter.initialize()   │
│ (in window context ✓)  │
└──────┬─────────────────┘
       │ Dynamic import works!
       v
┌──────────────────────┐
│ WebLLM downloads     │
│ Progress: 0% → 100%  │
└──────┬───────────────┘
       │ updateStatus() polls
       v
┌──────────────────────┐
│ Progress bar updates │
│ Status badge updates │
└──────┬───────────────┘
       │ Download complete
       v
┌──────────────────────┐
│ initialized: true    │
│ isPending: false     │
└──────┬───────────────┘
       │
       v
┌──────────────────────┐
│ Prompt gets answered │
│ Task executes        │
└──────────────────────┘
```

### New Architecture: Offscreen Document

With the offscreen document approach, the flow is simplified:

1. **Wizard** → Sends `llm_set_adapter` to background
2. **Background** → Creates offscreen document (window context)
3. **Offscreen** → Initializes WebLLM with dynamic imports (works!)
4. **Sidepanel** → User executes task
5. **Background** → Ralph agent calls `proxy.prompt()`
6. **Proxy** → Forwards `offscreen_llm_prompt` message
7. **Offscreen** → WebLLM processes prompt, returns response
8. **Background** → Ralph agent continues with response

**Key benefit:** Dynamic imports work in offscreen document (window context), avoiding the service worker limitation entirely.

## Console Logs to Expect

### Service Worker Console

```
[LLM Proxy] setActiveAdapter: webllm Gemma-2B-it-q4f16_1
[LLM Proxy] Using offscreen document for webllm
[Oryn-W] Creating offscreen document for LLM operations
[Oryn-W] Offscreen document created and ready

... (when user executes task) ...

[LLM Proxy] Forwarding prompt to offscreen
```

### Offscreen Document Console (NEW!)

To view offscreen console:
- Go to `chrome://extensions` → Find Oryn-W → Click "offscreen document"
- Or: `chrome://inspect` → Find "offscreen.html" → Click "inspect"

```
[Offscreen] Starting LLM offscreen document
[Offscreen] Ready to handle LLM operations
[Offscreen] Received message: offscreen_llm_set_adapter
[Offscreen] Setting adapter: webllm Gemma-2B-it-q4f16_1
[WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
[WebLLM] Creating engine...
... (download happens here) ...
[Offscreen] Received message: offscreen_llm_prompt
[Offscreen] Sending prompt to LLM
```

### Sidepanel Console

```
[Sidepanel] LLM status: {ready: true, adapter: 'webllm', model: 'Gemma-2B-it-q4f16_1', ...}
[Sidepanel] Starting agent execution: Search for cats
```

### WebLLM Console (in window context)

```
[WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
[WebLLM] Creating engine...
[WebLLM] Download progress: 5.2%
[WebLLM] Download progress: 18.7%
[WebLLM] Download progress: 34.1%
...
[WebLLM] Download progress: 98.4%
[WebLLM] Download progress: 100.0%
[WebLLM] Initialized successfully
```

## Troubleshooting

### "Download progress bar doesn't appear"

Check:
1. Is `isPending === true` in status? (Should be after wizard)
2. Is `isLoading === true` during download?
3. Is `updateStatus()` polling every 2 seconds?
4. Check console for `ensureInitialized` logs

### "Download gets stuck at 0%"

Check:
1. WebLLM console for actual progress logs
2. Network tab in DevTools - files downloading?
3. Service worker errors blocking initialization?
4. Try different model (smaller like Gemma-2B)

### "Error: Please configure LLM"

If this still appears AFTER the fixes:
1. Check if both `sidepanel.js` and `background.js` were updated
2. Hard refresh extension (chrome://extensions → Reload)
3. Clear chrome.storage and re-run wizard
4. Check console logs for `isPending` value

## Testing Checklist

- [ ] Complete wizard with WebLLM
- [ ] Step 3 shows orange note about download
- [ ] Open sidepanel
- [ ] Enter task and click Execute
- [ ] See "Initializing..." message (not error)
- [ ] See download progress bar appear
- [ ] Progress bar shows 0% → 100%
- [ ] Download completes, progress bar disappears
- [ ] Task executes successfully
- [ ] Close and reopen browser
- [ ] Sidepanel shows LLM as Ready (no re-download)

## Subsequent Uses

After first download completes:
- ✅ Config persists in `chrome.storage.sync`
- ✅ Model cached by browser (IndexedDB)
- ✅ Next use: NO download needed
- ✅ Status shows as "Ready" immediately
- ✅ Tasks execute instantly

The download only happens **once** on first use after wizard.

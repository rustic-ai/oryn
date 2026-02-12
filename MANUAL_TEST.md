# Manual Download Test - Step by Step

## What We Know So Far

✅ **WORKING:**
- Wizard completes successfully
- Config saves to chrome.storage.sync
- Step 3 shows correct adapter/model

❌ **NOT VERIFIED:**
- Offscreen document creation
- WebLLM initialization
- Download actually starting

## Critical Finding

**Offscreen is NOT created automatically on extension startup**, even though we load the saved config. This means the download will only start when you first execute a task.

## Manual Test Procedure

### 1. Reload Extension

```bash
# In Chrome:
chrome://extensions
# Click reload icon on Oryn-W
```

### 2. Clear First-Run Flag

1. Open service worker DevTools:
   - chrome://extensions → Find Oryn-W → Click "service worker"
2. In console, run:
```javascript
chrome.storage.local.remove('oryn_w_first_run_complete')
```
3. Reload extension again

### 3. Complete Wizard

1. Wizard should auto-open
2. Step 1: Wait for hardware check → Click "Next"
3. Step 2: Click "WebLLM (GPU-Accelerated)"
4. Select model: "Gemma-2B-it-q4f16_1 (1.5GB)"
5. Click "Download & Continue"
6. **Verify:** Step 3 appears with orange note about download on first use

### 4. Check Service Worker Logs

In service worker DevTools console, look for:
```
[LLM Proxy] Initialized with X available adapters
[LLM Proxy] Loading saved configuration...
[LLM Proxy] Found saved config: webllm Gemma-2B-it-q4f16_1
```

**QUESTION: Does it say "Offscreen document created"?**
- ✅ YES → Good, offscreen was created on startup
- ❌ NO → Expected, offscreen will be created on first use

### 5. Open Sidepanel

1. Navigate to a regular page (e.g., https://example.com)
2. Click the Oryn-W extension icon
3. Sidepanel should open on the right

### 6. Check Sidepanel Status

Look at the LLM status badge in sidepanel header:

**What does it say?**
- "LLM: webllm Gemma-2B... ⏳" → GOOD (pending status)
- "LLM: Not configured" → BAD (config not loaded)
- "LLM: Error" → BAD (initialization error)

### 7. Execute a Test Task

1. In sidepanel, make sure "Agent Mode" is selected
2. Type in task input: **"Say hello"**
3. Click **"Execute"** button

### 8. Monitor Offscreen Creation

**Open chrome://inspect in a new tab**

1. Find "Oryn-W" section
2. Look for **"offscreen.html"**
3. Click **"inspect"** to open DevTools for offscreen

**Does offscreen.html appear after clicking Execute?**
- ✅ YES → Click "inspect" and proceed to step 9
- ❌ NO → Offscreen not created, test FAILED

### 9. Monitor Offscreen Console

In the offscreen DevTools console, look for:

**Initial logs:**
```
[Offscreen] Starting LLM offscreen document
[Offscreen] LLM Manager initialized with X adapters
[Offscreen] Ready to handle LLM operations
```

**After Execute clicked:**
```
[Offscreen] Received message: offscreen_llm_set_adapter
[Offscreen] Setting adapter: webllm Gemma-2B-it-q4f16_1
[LLM Manager] Setting active adapter: webllm model: Gemma-2B-it-q4f16_1
```

**WebLLM initialization:**
```
[WebLLM] Initializing with model: Gemma-2B-it-q4f16_1
[WebLLM] Creating engine...
```

**Download progress:**
```
[WebLLM] Download progress: 2.3%
[WebLLM] Download progress: 8.7%
[WebLLM] Download progress: 15.4%
...
```

### 10. Monitor Sidepanel UI

In the sidepanel, you should see:

**Before download:**
```
System: Initializing webllm (Gemma-2B-it-q4f16_1)...
System: This may take a few minutes on first use.
```

**During download:**
```
┌────────────────────────────────────────┐
│ Downloading Gemma-2B-it-q4f16_1...  15%│
│ █████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░│
└────────────────────────────────────────┘
```

**After download:**
```
(Progress bar disappears)
(Agent starts processing task)
```

## Success Criteria

Mark each as PASS or FAIL:

- [ ] Wizard completes to Step 3
- [ ] Config saved (check service worker logs)
- [ ] Offscreen document created (check chrome://inspect)
- [ ] Offscreen console shows initialization logs
- [ ] WebLLM initialization starts
- [ ] Download progress appears (0% → 100%)
- [ ] Download completes
- [ ] Task executes successfully

## Expected Issues

### Issue: "Unknown adapter: webllm"
**Cause:** Base LLM manager didn't detect WebLLM
**Check:** Service worker console for "Initialized with X adapters"
**Fix:** Verify manual adapter addition in proxy.initialize()

### Issue: "Failed to load WebLLM library from CDN"
**Cause:** Dynamic import failing in service worker
**Check:** Is offscreen document created?
**Fix:** Ensure offscreen exists before calling setActiveAdapter

### Issue: Offscreen not created at all
**Cause:** Message passing broken or proxy not routing correctly
**Check:** Service worker logs for "Using offscreen document for webllm"
**Fix:** Debug OffscreenLLMProxy.setActiveAdapter()

### Issue: Download stuck at 0%
**Cause:** WebLLM initialization failed
**Check:** Offscreen console for errors
**Common:** Network error, WebGPU not available, CDN blocked

## Results Template

```
WIZARD: ✅ PASS / ❌ FAIL
  - Completed to Step 3:
  - Config saved:

OFFSCREEN: ✅ PASS / ❌ FAIL
  - Document created:
  - Initialization logs visible:

WEBLLM: ✅ PASS / ❌ FAIL
  - Initialization started:
  - Dynamic import worked:

DOWNLOAD: ✅ PASS / ❌ FAIL
  - Progress started (>0%):
  - Progress updates visible:
  - Completed to 100%:

TASK EXECUTION: ✅ PASS / ❌ FAIL
  - Agent started:
  - Task completed:
```

## Debugging Commands

**Check saved config:**
```javascript
chrome.storage.sync.get(['llmConfig'], r => console.log(r))
```

**Check offscreen exists:**
```javascript
chrome.runtime.getContexts({contextTypes:['OFFSCREEN_DOCUMENT']}).then(console.log)
```

**Force reload config:**
```javascript
// In service worker console:
await llmManager.loadSavedConfig()
```

# Fix: LLM Selector Showing All Adapters as Unavailable

## Problem

After updating the Chrome AI API, ALL adapters were showing as "Unavailable" in the LLM selector, including adapters that don't even exist (WebLLM, wllama, onnx).

## Root Cause

The UI (`ui/llm_selector.js`) was showing **ALL adapters from the metadata object**, not just the adapters available from the backend:

```javascript
// WRONG: Shows all adapters from metadata (including unimplemented ones)
const allAdapterIds = Object.keys(ADAPTER_META);  // 7 adapters
allAdapterIds.forEach(adapterId => {
    const available = availableAdapters.find(a => a.id === adapterId);
    const isAvailable = available !== undefined; // Most will be false!
});
```

This caused:
1. **WebLLM, wllama, onnx** to show as "Unavailable" (they're in metadata but not registered in backend)
2. **OpenAI, Claude, Gemini** might not show at all or show as unavailable
3. **Chrome AI** only shows as available if the Prompt API is enabled

## The Fix

Changed the UI to only show adapters that are **actually returned from the backend**:

```javascript
// CORRECT: Only show adapters from backend
availableAdapters.forEach(adapter => {
    const adapterId = adapter.id;
    const meta = ADAPTER_META[adapterId];

    if (!meta) {
        console.warn('[LLM Selector] No metadata for adapter:', adapterId);
        return;
    }

    const isAvailable = true; // It's in availableAdapters, so it's available!
    // ... create card ...
});
```

Now the UI only shows adapters that are:
1. **Registered** in `llm_manager.js`
2. **Return true** from their `isAvailable()` method

## Which Adapters Should Show

### Backend Registration (llm_manager.js)

Currently registered:
- ✅ `chrome-ai` - ChromeAIAdapter
- ✅ `openai` - OpenAIAdapter
- ✅ `claude` - ClaudeAdapter
- ✅ `gemini` - GeminiAdapter

NOT registered (even though UI has metadata):
- ❌ `webllm` - Not implemented
- ❌ `wllama` - Not implemented
- ❌ `onnx` - Not implemented

### Availability Logic

**Chrome AI:**
```javascript
static async isAvailable() {
    if (typeof LanguageModel === 'undefined') {
        return false;  // Prompt API not enabled
    }
    const availability = await LanguageModel.availability();
    return availability === 'available' || availability === 'after-download';
}
```

**OpenAI, Claude, Gemini:**
```javascript
static async isAvailable() {
    return true;  // Always available (user just needs API key)
}
```

## Testing

### Step 1: Check Background Console Logs

After reloading the extension, you should see:

```
[LLM Manager] Initializing...
[LLM Manager] Checking availability for: chrome-ai
[Chrome AI] Checking availability using current API...
[Chrome AI] Not available: LanguageModel not found
[LLM Manager] chrome-ai available: false
[LLM Manager] Adapter not available: chrome-ai

[LLM Manager] Checking availability for: openai
[LLM Manager] openai available: true
[LLM Manager] Detected available adapter: openai

[LLM Manager] Checking availability for: claude
[LLM Manager] claude available: true
[LLM Manager] Detected available adapter: claude

[LLM Manager] Checking availability for: gemini
[LLM Manager] gemini available: true
[LLM Manager] Detected available adapter: gemini

[LLM Manager] Initialized with 3 available adapters
```

**Expected Result:**
- **Without Chrome AI enabled**: 3 adapters (OpenAI, Claude, Gemini)
- **With Chrome AI enabled**: 4 adapters (+ Chrome AI)

### Step 2: Check LLM Selector UI

1. Reload extension in `chrome://extensions`
2. Open sidepanel
3. Click "Configure LLM"
4. **Should see 3-4 adapters** (depending on Chrome AI availability)
5. **Should NOT see**: WebLLM, llama.cpp, ONNX

### Step 3: Verify Each Adapter

**OpenAI:**
- ✅ Shows as "Ready" (green)
- Has API key input field
- Can select model (GPT-4, GPT-3.5, etc.)

**Claude:**
- ✅ Shows as "Ready" (green)
- Has API key input field
- Can select model (Sonnet, Haiku)

**Gemini:**
- ✅ Shows as "Ready" (green)
- Has API key input field
- Can select model (Pro, Flash)

**Chrome AI** (if available):
- ✅ Shows as "Ready" (green)
- No API key needed
- Shows as local/private/free

## Debugging

### Issue: No adapters showing at all

**Check console:**
```javascript
// In background service worker console
console.log('[LLM Manager] Available adapters:', llmManager.getAvailableAdapters());
```

**Expected:** Array with 3-4 objects

**If empty:**
1. Check if `llmManager.initialize()` was called
2. Check if `isAvailable()` methods are throwing errors

### Issue: Chrome AI not showing

**This is normal!** Chrome AI requires:
1. Chrome 127+
2. Prompt API flag enabled: `chrome://flags/#prompt-api-for-gemini-nano`
3. Model downloaded (happens automatically on first use)

**Check availability:**
```javascript
// In browser console (NOT service worker)
console.log('LanguageModel exists:', typeof LanguageModel);
LanguageModel.availability().then(status => {
    console.log('Status:', status);  // 'available', 'after-download', or 'no'
});
```

### Issue: Remote adapters showing as unavailable

**This should NOT happen** - they always return `true`.

**Debug:**
1. Check browser console for errors during `detectAdapters()`
2. Ensure `isAvailable()` is not throwing exceptions
3. Check if adapters are registered in `llm_manager.js`

## Files Modified

1. **`ui/llm_selector.js`** (line 192-208)
   - Changed from showing all metadata adapters to only showing available adapters
   - Added empty state message when no adapters available

## What's Next

If you want to add WebLLM, wllama, or ONNX support:

1. **Create adapter file**: `extension-w/llm/webllm_adapter.js`
2. **Implement adapter class**: Extend `LLMAdapter`
3. **Register in manager**: Add to `llm_manager.js` initialization
4. **Implement isAvailable()**: Check if required APIs exist
5. **Add dependencies**: Install npm packages if needed

---

**Status:** ✅ Fixed
**Date:** 2026-01-29
**Impact:** UI now only shows actually available adapters (OpenAI, Claude, Gemini by default)

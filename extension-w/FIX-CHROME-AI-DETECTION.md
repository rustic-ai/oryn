# Fix: Chrome AI Availability Detection

## Problem

Chrome AI was showing as "Unavailable" in the LLM selector UI even when `await LanguageModel.availability({})` returned `'available'`.

## Root Causes

### Issue 1: Wrong Message Handler

The LLM selector UI (`ui/llm_selector.js`) was calling the wrong message handler:

**Before:**
```javascript
async function detectAdapters() {
    const response = await chrome.runtime.sendMessage({ type: 'llm_status' });
    availableAdapters = response.adapters || [];  // ❌ llm_status doesn't return adapters!
}
```

The `llm_status` handler only returns status of the **active** adapter (ready, model, error), not the list of **available** adapters.

**After:**
```javascript
async function detectAdapters() {
    const response = await chrome.runtime.sendMessage({ type: 'llm_get_adapters' });
    availableAdapters = response.adapters || [];  // ✓ Correct handler
}
```

### Issue 2: Missing Adapter ID Field

The LLM manager was not including the `id` field in adapter info:

**Before:**
```javascript
this.availableAdapters.push({
    name: name,                    // ❌ UI expects 'id' field
    displayName: ...,
    description: ...,
});
```

**After:**
```javascript
this.availableAdapters.push({
    id: name,                      // ✓ Added id field
    name: name,
    displayName: ...,
    description: ...,
});
```

The UI code checks `a.id === adapterId` (line 201), so without the `id` field, it couldn't match adapters.

### Issue 3: Unclear Availability Status

Chrome AI adapter was returning `true` for availability when status was `'after-download'`, which means the model needs to be downloaded first.

**Before:**
```javascript
static async isAvailable() {
    const capabilities = await LanguageModel.capabilities();
    return capabilities.available !== 'no';  // ❌ Returns true for 'after-download'
}
```

**After:**
```javascript
static async isAvailable() {
    const capabilities = await LanguageModel.capabilities();
    console.log('[Chrome AI] Capabilities:', capabilities);
    const isReady = capabilities.available === 'available';
    console.log('[Chrome AI] Is available:', isReady, '(status:', capabilities.available + ')');
    return isReady;  // ✓ Only returns true when immediately available
}
```

Now the adapter only reports as available when the model is ready to use immediately.

## Chrome AI Availability States

The Chrome AI API returns three possible states:

1. **`'available'`**: Model is downloaded and ready to use ✅
2. **`'after-download'`**: API exists but model needs download ⚠️
3. **`'no'`**: API not available (old Chrome version, feature disabled, etc.) ❌

Our fix only shows Chrome AI as available when state is `'available'`.

## Testing the Fix

### Step 1: Check Chrome AI Availability

Open Chrome DevTools console and run:

```javascript
// Check if API exists
console.log('AI API exists:', !!self.ai?.languageModel);

// Check availability status
self.ai?.languageModel?.capabilities().then(caps => {
    console.log('Chrome AI status:', caps.available);
    console.log('Full capabilities:', caps);
});
```

Expected outputs:
- **Chrome 127+ with flags enabled**: `'available'` or `'after-download'`
- **Chrome 126 or older**: API doesn't exist
- **Flags disabled**: `'no'`

### Step 2: Check Extension Detection

1. Reload the extension (`chrome://extensions` → reload button)
2. Open extension background service worker console
3. Look for logs:

```
[LLM Manager] Checking availability for: chrome-ai
[Chrome AI] Capabilities: { available: 'available', ... }
[Chrome AI] Is available: true (status: available)
[LLM Manager] Detected available adapter: chrome-ai
```

Or if unavailable:

```
[LLM Manager] Checking availability for: chrome-ai
[Chrome AI] Not available: LanguageModel not found
[LLM Manager] Adapter not available: chrome-ai
```

### Step 3: Check LLM Selector UI

1. Click "Configure LLM" button in sidepanel
2. Chrome AI card should show:
   - **✓ Ready** (green) if available
   - **✗ Unavailable** (gray) if not available

## Debugging Chrome AI Issues

### Issue: API exists but shows as unavailable

**Check:**
```javascript
LanguageModel.capabilities().then(caps => {
    console.log('Status:', caps.available);
    // If 'after-download', need to download model first
});
```

**Fix:** If status is `'after-download'`, create a session to trigger download:
```javascript
const session = await LanguageModel.create();
// This will download the model (~1.5GB)
```

### Issue: API doesn't exist

**Possible causes:**
1. Chrome version < 127
2. Feature flags not enabled
3. Not available in your region

**Fix:**
1. Update to Chrome 127+
2. Enable flags:
   - Go to `chrome://flags`
   - Search for "Optimization Guide On Device Model"
   - Set to "Enabled BypassPerfRequirement"
   - Restart Chrome
3. Check regional availability

### Issue: Shows as available but initialization fails

**Check initialization:**
```javascript
try {
    const adapter = new ChromeAIAdapter();
    await adapter.initialize();
    console.log('✓ Initialized');
} catch (error) {
    console.error('✗ Failed:', error.message);
}
```

**Common errors:**
- `"Chrome AI not available"` → API check failed
- `"language model is not available"` → capabilities.available === 'no'
- Timeout → Model downloading in background

## Files Modified

1. **`ui/llm_selector.js`** (line 182)
   - Changed `llm_status` to `llm_get_adapters`

2. **`llm/llm_manager.js`** (line 51-68)
   - Added `id` field to adapter info
   - Added debug logging

3. **`llm/chrome_ai_adapter.js`** (line 162-180)
   - Only return true for `available === 'available'`
   - Added debug logging

## Verification Checklist

After reloading the extension:

- [ ] Background console shows adapter detection logs
- [ ] Chrome AI shows correct status in LLM selector
- [ ] Other adapters (OpenAI, Claude, Gemini) show as available (if configured)
- [ ] Selecting Chrome AI and clicking "Test Prompt" works (if available)
- [ ] UI updates when adapter status changes

## Related Documentation

- [Chrome AI Built-in Docs](https://developer.chrome.com/docs/ai/built-in)
- [Prompt API Guide](https://developer.chrome.com/docs/ai/built-in-apis)
- Extension LLM integration: `docs/LLM-SELECTION-UI.md`

---

**Status:** ✅ Fixed
**Date:** 2026-01-29
**Files Changed:** 3
